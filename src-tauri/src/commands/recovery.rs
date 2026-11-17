//! The recovery key: creating one for the account, and opening its vaults with it.
//!
//! A recovery code is **not** tied to a single vault: each vault stores its own salt, nonce and
//! wrap, so the same code can wrap the secret of every vault that shares the master password.
//! Creating a key therefore produces **one code** and applies it to every vault that password
//! opens — the ones on this device and the ones the account holds in the cloud — with one file to
//! keep. See [`crate::recovery`] for the code and [`crate::storage`] for the vault format.

use super::*;

#[cfg(feature = "sync")]
use std::collections::HashSet;

use crate::recovery;

/// What creating a recovery key covered.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryKeyView {
    /// Where the file holding the code was written.
    pub path: String,
    /// The vaults this one code opens, by file name.
    pub vaults: Vec<String>,
    /// The sync account it belongs to, when there is one.
    pub account: Option<String>,
    /// Synced vaults that are present here but do **not** use this master password, so the key
    /// does not cover them and the user must know before relying on it.
    pub uncovered: Vec<String>,
}

/// What using a recovery key restored.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryView {
    pub status: AppStatus,
    /// The vaults re-opened with the key and saved under the new password, by file name.
    pub restored: Vec<String>,
    /// The account credential now uses the new password, so syncing continues.
    pub account_updated: bool,
    /// Synced vaults the key did not cover: they still need the old password.
    pub uncovered: Vec<String>,
    /// Anything else the user should be told (a vault opened in the cloud but not on this device,
    /// or an account that could not be updated).
    pub notice: Option<String>,
}

// ----- Creating a recovery key -----

/// Creates a recovery key for every vault the given master password opens and writes the file the
/// user keeps, returning what it covered. The vaults are re-encrypted so the one code opens them
/// all; this replaces any earlier recovery key on each of them.
#[tauri::command]
pub async fn create_recovery_key(
    app: AppHandle,
    path: String,
    password: String,
    key_file: Option<String>,
) -> Result<RecoveryKeyView> {
    // Refuse an existing file *before* touching any vault: re-keying it and then failing to hand
    // out the code would leave a recovery wrap whose key nobody has.
    let path = PathBuf::from(path);
    if path.exists() {
        return Err(Error::invalid(
            "A file with that name already exists. Choose another name, so no earlier recovery file is overwritten",
        ));
    }
    run_blocking(move || {
        let state = app.state::<AppState>();
        create_for_account(&state, &path, &password, key_file.as_deref())
    })
    .await
}

fn create_for_account(
    state: &AppState,
    file: &Path,
    password: &str,
    key_file: Option<&str>,
) -> Result<RecoveryKeyView> {
    let password = Zeroizing::new(password.to_owned());
    require_password(&password)?;
    let composite = composite_key(&password, key_file)?;

    // The open vault is the one the user asked for: the typed password must open it, or a key made
    // now would not open the very vault it was made from.
    let (open_composite, open_path) = state.with_vault(|open| Ok((open.composite(), open.path().to_path_buf())))?;
    if crypto::secret_to_base64(&composite) != crypto::secret_to_base64(&open_composite) {
        return Err(Error::invalid(
            "That is not the master password of the vault you have open, so a key made now would not open it",
        ));
    }

    // Every vault file on this device whose key this password derives. Anything this password does
    // not open is left alone, keeping its own recovery key.
    let mut covered: Vec<(PathBuf, VaultKey)> = Vec::new();
    for path in candidate_vaults(state) {
        if let Ok(key) = vault_key_for(&path, &composite) {
            covered.push((path, key));
        }
    }
    if !covered.iter().any(|(path, _)| same_file(path, &open_path)) {
        return Err(Error::invalid("No vault on this device is opened by that master password"));
    }
    let covered_paths: Vec<PathBuf> = covered.iter().map(|(path, _)| path.clone()).collect();
    let uncovered = uncovered_synced(state, &covered_paths);

    let (text, code) = recovery::generate()?;
    let names: Vec<String> = covered.iter().map(|(path, _)| views::file_name(path)).collect();
    let account = state.settings().sync_account.as_ref().map(|account| account.email.clone());

    // Write the file *first*: a code with no vault is harmless, a wrap whose code the user never
    // received is not.
    storage::write_atomic(file, recovery::file_contents(&names, account.as_deref(), &text).as_bytes())?;

    // Add the wrap to each covered vault and save it. The open one goes through the session so it
    // stays valid; the others are opened just for this.
    for (path, key) in &covered {
        let wrapped = key.with_recovery(&composite, &code)?;
        if same_file(path, &open_path) {
            state.with_vault_mut(|open| open.change_key(composite.clone(), wrapped.clone()))?;
        } else {
            let mut open = OpenVault::open(path.clone(), composite.clone())?;
            open.change_key(composite.clone(), wrapped)?;
        }
    }

    // Send the wrap to the cloud so the copies on the other devices receive it as well.
    #[cfg(feature = "sync")]
    push_wraps(state, &covered, &composite, &code);

    Ok(RecoveryKeyView {
        path: views::path_text(file),
        vaults: names,
        account,
        uncovered,
    })
}

// ----- Using a recovery key -----

/// Opens every vault the recovery key covers with it, saves them under a new master password and,
/// when a session is still valid, adopts that password for the account too — so recovery leaves the
/// user signed in and syncing, the same reach as a normal master-password change.
#[tauri::command]
pub async fn recover_vault(
    app: AppHandle,
    recovery_file: String,
    new_password: String,
    new_key_file: Option<String>,
) -> Result<RecoveryView> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        recover_with_key(&state, &recovery_file, &new_password, new_key_file.as_deref())
    })
    .await
}

fn recover_with_key(
    state: &AppState,
    recovery_file: &str,
    new_password: &str,
    new_key_file: Option<&str>,
) -> Result<RecoveryView> {
    let new_password = Zeroizing::new(new_password.to_owned());
    require_password(&new_password)?;
    let new_composite = composite_key(&new_password, new_key_file)?;
    let kdf = KdfParams::calibrated();

    let text = Zeroizing::new(String::from_utf8_lossy(&storage::read_file(Path::new(recovery_file))?).into_owned());
    let code = recovery::parse_code(&text).ok_or_else(|| Error::invalid("That file does not hold a Lados recovery key"))?;

    let selected = state.session().path().map(Path::to_path_buf);

    // Every vault file on this device that carries this code.
    let mut restored: Vec<PathBuf> = Vec::new();
    for path in candidate_vaults(state) {
        if recover_one(&path, &code, &new_composite, kdf).is_ok() {
            restored.push(path);
        }
    }

    // Re-encrypt the account's cloud copies under the new password, including the vaults that are
    // not on this device. The server still only ever sees ciphertext.
    #[cfg(feature = "sync")]
    let cloud_restored = recover_cloud_vaults(state, &code, &new_composite, kdf);
    #[cfg(not(feature = "sync"))]
    let cloud_restored: Vec<String> = Vec::new();

    if restored.is_empty() && cloud_restored.is_empty() {
        return Err(Error::invalid(
            "This recovery key does not open any vault of this account. It was made for a different account, or for a copy from before the key existed.",
        ));
    }

    // Install the recovered vault as the open session: the one the user had selected when it fits,
    // otherwise the first one the key opened.
    let open_path = selected
        .filter(|path| restored.iter().any(|done| same_file(done, path)))
        .or_else(|| restored.first().cloned());
    if let Some(path) = &open_path {
        let opened = OpenVault::open(path.clone(), new_composite.clone())?;
        *state.session() = Session::Unlocked(Box::new(opened));
        state
            .update_settings(|settings| settings.remember_vault(path, key_file_path(new_key_file).as_deref()))?;
        state.touch();
    }

    // Adopt the new password for the account, so the recovered vaults sync again. Requires a live
    // session: without one, only the local files are recovered.
    #[cfg(feature = "sync")]
    let account_updated = adopt_account(state, &new_composite).unwrap_or(false);
    #[cfg(not(feature = "sync"))]
    let account_updated = false;

    let uncovered = uncovered_synced(state, &restored);
    let logged_in = state.settings().sync_account.is_some();

    let notice = if restored.is_empty() && !cloud_restored.is_empty() {
        Some(format!(
            "This key opened {} vault(s) held in the cloud, but none of them is on this device. They now use your new password; open one from Sync settings to have it here.",
            cloud_restored.len()
        ))
    } else if logged_in && !account_updated {
        Some(
            "Your vaults are open with the new password, but the sync account was not updated. Sign in and change your sync password to resume syncing."
                .to_string(),
        )
    } else {
        None
    };

    Ok(RecoveryView {
        status: state.status(),
        restored: restored.iter().map(|path| views::file_name(path)).collect(),
        account_updated,
        uncovered,
        notice,
    })
}

/// Opens one file with the code and saves it again under `new_composite`, keeping the wrap so the
/// same file opens it next time. `Error::Corrupted` means it carries no recovery wrap at all;
/// `Error::InvalidCredentials` means the wrap belongs to a different code.
fn recover_one(path: &Path, code: &SecretKey, new_composite: &SecretKey, kdf: KdfParams) -> Result<()> {
    let bytes = storage::read_file(path)?;
    let header = storage::Header::parse(&bytes)?;
    if !header.has_recovery() {
        return Err(Error::Corrupted);
    }
    let recovered = storage::key_from_recovery(&header, &bytes, code)?;
    let vault = storage::decrypt_vault(&bytes, &recovered)?;
    let key = recovered.rewrap(new_composite, kdf)?;
    storage::backup(path)?;
    storage::write_atomic(path, &storage::encrypt_vault(&vault, &key)?)?;
    Ok(())
}

// ----- Cloud: reaching the copies that are not here -----

/// Re-encrypts every cloud vault that carries this code under the new password, returning their
/// ids. Best effort: a network problem leaves what it could not reach for the next sign-in.
#[cfg(feature = "sync")]
fn recover_cloud_vaults(state: &AppState, code: &SecretKey, new_composite: &SecretKey, kdf: KdfParams) -> Vec<String> {
    let Some(account) = state.settings().sync_account.clone() else {
        return Vec::new();
    };
    let Ok(cloud) = crate::sync::list_vaults(&account.server_url, &account.token) else {
        return Vec::new();
    };
    let remote = crate::sync::HttpRemote::new(&account.server_url, &account.token);
    let mut restored = Vec::new();
    for vault in cloud {
        if crate::sync::recover_cloud_vault(&remote, &vault.id, code, new_composite, kdf).is_ok() {
            restored.push(vault.id);
        }
    }
    restored
}

/// Gives the recovery wrap to every cloud copy, so the copies on other devices receive it too.
/// Best effort: the local files already carry the wrap, so a network problem must never fail the
/// whole operation.
#[cfg(feature = "sync")]
fn push_wraps(state: &AppState, covered: &[(PathBuf, VaultKey)], composite: &SecretKey, code: &SecretKey) {
    let Some(account) = state.settings().sync_account.clone() else {
        return;
    };
    let remote = crate::sync::HttpRemote::new(&account.server_url, &account.token);

    // Which synced vault each covered file belongs to.
    let bindings: Vec<(PathBuf, String, Option<u64>)> = {
        let settings = state.settings();
        covered
            .iter()
            .filter_map(|(path, _)| {
                settings
                    .synced_vault(path)
                    .map(|entry| (path.clone(), entry.server_vault_id.clone(), entry.last_version))
            })
            .collect()
    };

    for (path, server_vault_id, last_version) in &bindings {
        let open_now = matches!(&*state.session(), Session::Unlocked(open) if same_file(open.path(), path));
        if open_now {
            let _ = state.with_vault_mut(|open| {
                open.sync_remote(&remote, server_vault_id, *last_version)?;
                Ok(())
            });
        } else if let Ok(mut open) = OpenVault::open(path.clone(), composite.clone()) {
            let _ = open.sync_remote(&remote, server_vault_id, *last_version);
        }
    }

    // Vaults the account holds with no file here: give their cloud copy the wrap directly.
    if let Ok(cloud) = crate::sync::list_vaults(&account.server_url, &account.token) {
        let local: HashSet<String> = bindings.iter().map(|(_, id, _)| id.clone()).collect();
        for vault in cloud {
            if local.contains(&vault.id) {
                continue;
            }
            let _ = crate::sync::add_recovery_to_cloud(&remote, &vault.id, composite, code);
        }
    }
}

/// Re-registers the account credential under the password the vaults now use, re-seals the cloud
/// labels so they stay readable, and revokes the other sessions. Needs only the session token, not
/// the old password — that is what makes recovery able to leave the user signed in. Returns whether
/// the account was actually updated.
#[cfg(feature = "sync")]
fn adopt_account(state: &AppState, new_composite: &SecretKey) -> Result<bool> {
    let Some(account) = state.settings().sync_account.clone() else {
        return Ok(false);
    };
    let (base, token, email) = (account.server_url.clone(), account.token.clone(), account.email.clone());
    let new_label = crypto::label_key(new_composite, &email)?;

    // Re-seal every cloud label with the new label key, so it stays readable after the change.
    if let Some(old_label) = stored_label_key(state)
        && let Ok(cloud) = crate::sync::list_vaults(&base, &token)
    {
        for vault in cloud {
            let Some(cipher) = vault.name_cipher.clone() else {
                continue;
            };
            let Ok(name) = crypto::open(&old_label, &cipher) else {
                continue;
            };
            if let Ok(sealed) = crypto::seal(&new_label, &name) {
                let _ = crate::sync::set_vault_label(&base, &token, &vault.id, &sealed);
            }
        }
    }

    // Re-register the account credential, and revoke the sessions of the other devices.
    let new_auth = crate::sync::auth_secret(new_composite, &email)?;
    upgrade_to_opaque(&base, &token, new_auth.as_bytes())?;
    let _ = crate::sync::sign_out_everywhere(&base, &token);

    state.update_settings(|settings| {
        settings.sync_label_key = Some(crypto::secret_to_base64(&new_label));
        if let Some(account) = settings.sync_account.as_mut() {
            account.password_changed_at = Some(chrono::Utc::now().timestamp_millis());
        }
    })?;
    Ok(true)
}

// ----- Helpers -----

/// Every vault file on this device worth trying the master password on: the selected or open vault,
/// the vaults in the recent list, and the synced ones. Duplicates are removed by path.
fn candidate_vaults(state: &AppState) -> Vec<PathBuf> {
    fn push(paths: &mut Vec<PathBuf>, path: &Path) {
        if path.is_file() && !paths.iter().any(|known| same_file(known, path)) {
            paths.push(path.to_path_buf());
        }
    }

    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(path) = state.session().path() {
        push(&mut paths, path);
    }
    let settings = state.settings();
    for recent in &settings.recent_vaults {
        push(&mut paths, &recent.path);
    }
    for synced in &settings.synced_vaults {
        push(&mut paths, &synced.path);
    }
    paths
}

/// The key that `composite` derives for the vault at `path`, or an error when it does not open it.
fn vault_key_for(path: &Path, composite: &SecretKey) -> Result<VaultKey> {
    let bytes = storage::read_file(path)?;
    let header = storage::Header::parse(&bytes)?;
    VaultKey::for_header(composite, &header, &bytes)
}

/// Synced vaults present here that are not among `covered`: they do not use this master password,
/// so a key made now would not open them, and they must be reported rather than silently excluded.
fn uncovered_synced(state: &AppState, covered: &[PathBuf]) -> Vec<String> {
    let synced: Vec<PathBuf> = {
        let settings = state.settings();
        settings
            .synced_vaults
            .iter()
            .map(|entry| entry.path.clone())
            .filter(|path| path.is_file())
            .collect()
    };
    synced
        .into_iter()
        .filter(|path| !covered.iter().any(|known| same_file(known, path)))
        .map(|path| views::file_name(&path))
        .collect()
}
