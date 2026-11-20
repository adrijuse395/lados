//! The account-wide master password (changed across every synced vault at once) and deletion.

use super::*;

/// Changes the account master password and re-encrypts every synced vault under it, in one
/// blocking operation. Nothing decrypted ever leaves the device: each vault is decrypted here,
/// re-encrypted with the new password and only then uploaded. Cloud vaults that are not on this
/// device are re-encrypted in memory from their ciphertext, without ever being written in clear.
#[tauri::command]
pub async fn change_master_password(
    app: AppHandle,
    current_password: String,
    current_key_file: Option<String>,
    new_password: String,
    new_key_file: Option<String>,
) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let (base, email, token) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
                (account.server_url, account.email, account.token)
            };

            // 1. Prove the current master password.
            let current = Zeroizing::new(current_password);
            require_password(&current)?;
            let current_composite = composite_key(&current, current_key_file.as_deref())?;
            if !is_the_account_password(&base, &email, &current_composite) {
                return Err(Error::invalid("That is not your current master password"));
            }

            // 2. Derive the new credentials.
            let new = Zeroizing::new(new_password);
            require_password(&new)?;
            let new_composite = composite_key(&new, new_key_file.as_deref())?;
            if crypto::secret_to_base64(&new_composite) == crypto::secret_to_base64(&current_composite) {
                return Err(Error::invalid("The new master password is the same as the current one"));
            }
            let new_label = crypto::label_key(&new_composite, &email)?;
            let current_label =
                stored_label_key(&state).or_else(|| crypto::label_key(&current_composite, &email).ok());

            // 3. Re-encrypt every cloud vault and re-seal its label.
            let cloud = crate::sync::list_vaults(&base, &token)?;
            let remote = crate::sync::HttpRemote::new(&base, &token);
            let total = cloud.len();
            for (index, vault) in cloud.iter().enumerate() {
                let local = {
                    let settings = state.settings();
                    settings
                        .synced_vaults
                        .iter()
                        .find(|synced| synced.server_vault_id == vault.id)
                        .cloned()
                };
                let label = local
                    .as_ref()
                    .and_then(|entry| entry.name.clone())
                    .unwrap_or_else(|| vault.name.clone());
                emit_password_change(&app, "vaults", index, total, &label);

                let kdf = KdfParams::calibrated();
                let version = match &local {
                    Some(entry)
                        if matches!(&*state.session(), Session::Unlocked(open) if open.path() == entry.path) =>
                    {
                        state.with_vault_mut(|open| {
                            let (merged, version, new_key) = crate::sync::rekey(
                                &remote,
                                &vault.id,
                                Some(open.vault()),
                                Some(open.key()),
                                &current_composite,
                                &new_composite,
                                kdf,
                            )?;
                            if let Some(merged) = merged {
                                open.adopt(merged, new_composite.clone(), new_key)?;
                            }
                            Ok(version)
                        })?
                    }
                    Some(entry) if entry.path.is_file() => {
                        // A previous interrupted run may already have re-encrypted it.
                        let mut open = match OpenVault::open(entry.path.clone(), current_composite.clone()) {
                            Ok(open) => open,
                            Err(_) => OpenVault::open(entry.path.clone(), new_composite.clone())?,
                        };
                        let (merged, version, new_key) = crate::sync::rekey(
                            &remote,
                            &vault.id,
                            Some(open.vault()),
                            Some(open.key()),
                            &current_composite,
                            &new_composite,
                            kdf,
                        )?;
                        if let Some(merged) = merged {
                            open.adopt(merged, new_composite.clone(), new_key)?;
                        }
                        version
                    }
                    _ => crate::sync::rekey(
                        &remote,
                        &vault.id,
                        None,
                        None,
                        &current_composite,
                        &new_composite,
                        kdf,
                    )?
                    .1,
                };
                if local.is_some() {
                    state.update_settings(|settings| {
                        if let Some(entry) = settings
                            .synced_vaults
                            .iter_mut()
                            .find(|synced| synced.server_vault_id == vault.id)
                        {
                            entry.last_version = Some(version);
                            entry.last_synced_at = Some(chrono::Utc::now().timestamp_millis());
                        }
                    })?;
                }

                // The label the vault carries now: its cached name, or its old ciphertext.
                let name = local
                    .as_ref()
                    .and_then(|entry| entry.name.clone())
                    .or_else(|| {
                        current_label.as_ref().and_then(|key| {
                            vault
                                .name_cipher
                                .as_ref()
                                .and_then(|cipher| crypto::open(key, cipher).ok())
                        })
                    })
                    .unwrap_or_else(|| vault.name.clone());
                if !name.is_empty() {
                    let name_cipher = crypto::seal(&new_label, &name)?;
                    crate::sync::set_vault_label(&base, &token, &vault.id, &name_cipher)?;
                }
            }

            // 4. Re-register the account credential (OPAQUE) under the new password.
            emit_password_change(&app, "account", total, total, "");
            let new_auth = crate::sync::auth_secret(&new_composite, &email)?;
            upgrade_to_opaque(&base, &token, new_auth.as_bytes())?;

            // 5. Revoke every other device's session. Best effort: never fail the change for it.
            emit_password_change(&app, "sessions", total, total, "");
            let _ = crate::sync::sign_out_everywhere(&base, &token);

            // 6. Remember the new label key and when the password changed.
            state.update_settings(|settings| {
                settings.sync_label_key = Some(crypto::secret_to_base64(&new_label));
                // Every synced vault has just been re-keyed, so any earlier mismatch is gone.
                settings.pending_account_password = false;
                if let Some(account) = settings.sync_account.as_mut() {
                    account.password_changed_at = Some(chrono::Utc::now().timestamp_millis());
                }
            })?;

            emit_password_change(&app, "done", total, total, "");
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, current_password, current_key_file, new_password, new_key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Re-seals every cloud vault's label with the current label key, so labels stored in clear by
/// older versions become encrypted. Safe to run again: already-encrypted labels are left alone.
#[tauri::command]
pub async fn resync_labels(app: AppHandle) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let key = stored_label_key(&state)
                .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
            let (base, token) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
                (account.server_url, account.token)
            };
            let cloud = crate::sync::list_vaults(&base, &token)?;
            for vault in &cloud {
                if vault.name_cipher.is_some() || vault.name.is_empty() {
                    continue;
                }
                let name_cipher = crypto::seal(&key, &vault.name)?;
                crate::sync::set_vault_label(&base, &token, &vault.id, &name_cipher)?;
            }
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Deletes the sync account and everything it holds on the server, then signs out locally.
/// The vaults themselves stay on this device, as local-only.
///
/// This is the one irreversible action the app performs on someone else's machine: it destroys the
/// only off-device copy and cancels the subscription. So it asks for the account credential again
/// (M7) — a mis-click, or somebody at an unlocked machine, must not be able to do it.
#[tauri::command]
pub async fn sync_delete_account(app: AppHandle, password: String, key_file: Option<String>) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Not signed in"))?;

            let password = Zeroizing::new(password);
            let composite = composite_key(&password, key_file.as_deref())?;
            // Verify the password with a fresh login and use **that** session for the delete. The
            // server only accepts a recently created session for this irreversible action, so a
            // stolen long-lived token is not enough on its own (the red-team report, M7).
            let fresh = reauth_token(&account.server_url, &account.email, &composite)?;

            crate::sync::delete_account(&account.server_url, &fresh)?;
            state.update_settings(|settings| {
                settings.sync_account = None;
                settings.synced_vaults.clear();
            })?;
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, password, key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

