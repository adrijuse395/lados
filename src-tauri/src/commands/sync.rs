//! Cloud sync: signing in, binding a vault to the cloud, and one sync pass.

use super::*;

// ----- Cloud sync (optional) -----

/// Whether the user is signed in and whether the open vault syncs.
#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> views::SyncView {
    state.status().sync
}

/// Remembers the account's server-side state — when the master password last changed, and the
/// entitlement — so the interface can show it. Best effort: a network hiccup must never block.
#[cfg(feature = "sync")]
fn remember_account(state: &AppState, base: &str, token: &str) {
    if let Ok(info) = crate::sync::me(base, token) {
        let _ = state.update_settings(|settings| {
            if let Some(account) = settings.sync_account.as_mut() {
                account.password_changed_at = info.password_changed_at;
                account.plan = Some(info.plan);
                account.plan_valid_until = info.plan_valid_until;
            }
        });
    }
}

/// The error shown when a vault cannot be synced because its master password is not the account's.
#[cfg(feature = "sync")]
fn wrong_master_password() -> Error {
    Error::invalid(
        "This vault uses a different master password. Synced vaults must use your sync master password — use “Change master key” to set it, then sync again.",
    )
}

/// Signs in to (or creates) a sync account from the app start, using the master credentials
/// of the account (which are those of the vault it syncs).
#[tauri::command]
pub async fn sync_sign_in(
    app: AppHandle,
    email: String,
    password: String,
    key_file: Option<String>,
) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let email = email.trim().to_lowercase();
            if !email.contains('@') {
                return Err(Error::invalid("Enter a valid email address"));
            }
            let password = Zeroizing::new(password);
            require_password(&password)?;
            let composite = composite_key(&password, key_file.as_deref())?;
            let auth = crate::sync::auth_secret(&composite, &email)?;
            let label = crypto::secret_to_base64(&crypto::label_key(&composite, &email)?);
            let base = crate::sync::DEFAULT_SERVER.to_string();
            // OPAQUE sign-in: the server never sees a password-equivalent.
            let token = opaque_login(&base, &email, auth.as_bytes())?;
            state.update_settings(|settings| {
                settings.sync_label_key = Some(label);
                settings.sync_account = Some(crate::settings::SyncAccount {
                    server_url: base.clone(),
                    email,
                    token: token.clone(),
                    password_changed_at: None,
                    plan: None,
                    plan_valid_until: None,
                });
            })?;
            remember_account(&state, &base, &token);
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, email, password, key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Creates a new sync account. The account password must be very strong (enforced by the
/// interface, which is the only place that sees it). Fails if the email is already used.
#[tauri::command]
pub async fn sync_register(
    app: AppHandle,
    email: String,
    password: String,
    key_file: Option<String>,
) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let email = email.trim().to_lowercase();
            if !email.contains('@') {
                return Err(Error::invalid("Enter a valid email address"));
            }
            let password = Zeroizing::new(password);
            require_password(&password)?;
            let composite = composite_key(&password, key_file.as_deref())?;
            let auth = crate::sync::auth_secret(&composite, &email)?;
            let label = crypto::secret_to_base64(&crypto::label_key(&composite, &email)?);
            let base = crate::sync::DEFAULT_SERVER.to_string();
            // OPAQUE registration: the server never sees a password-equivalent.
            let (state_b64, request) = crate::sync::registration_start(auth.as_bytes())?;
            let response = crate::sync::opaque_register_start(&base, &email, &request)?;
            let record = crate::sync::registration_finish(auth.as_bytes(), &state_b64, &response)?;
            let token = crate::sync::opaque_register_finish(&base, &email, &record)?;
            state.update_settings(|settings| {
                settings.sync_label_key = Some(label);
                settings.sync_account = Some(crate::settings::SyncAccount {
                    server_url: base.clone(),
                    email,
                    token: token.clone(),
                    password_changed_at: None,
                    plan: None,
                    plan_valid_until: None,
                });
            })?;
            remember_account(&state, &base, &token);
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, email, password, key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Signs out of the sync account on this device. Synced vaults stay on disk.
#[tauri::command(async)]
pub fn sync_sign_out(state: State<'_, AppState>) -> Result<AppStatus> {
    state.update_settings(|settings| settings.sync_account = None)?;
    Ok(state.status())
}

/// Marks the open vault as synced: binds it to a server vault and runs a first sync.
#[tauri::command]
pub async fn sync_enable(app: AppHandle) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let (vault_name, local_id, path) = state.with_vault(|open| {
                Ok((
                    open.vault().name.clone(),
                    open.vault().id.to_string(),
                    open.path().to_path_buf(),
                ))
            })?;
            let (base, token) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
                (account.server_url, account.token)
            };

            let label = {
                let composite = state.with_vault(|open| Ok(open.composite()))?;
                let email = {
                    let settings = state.settings();
                    settings.sync_account.as_ref().map(|account| account.email.clone()).unwrap_or_default()
                };
                if !is_the_account_password(&base, &email, &composite) {
                    return Err(wrong_master_password());
                }
                label_key_for(&state, &composite, &email)?
            };
            let name_cipher = crypto::seal(&label, &vault_name)?;
            let server_vault_id = crate::sync::ensure_vault(&base, &token, &local_id, &name_cipher)?;
            state.update_settings(|settings| {
                settings.set_synced_vault(crate::settings::SyncedVault {
                    path: path.clone(),
                    server_vault_id: server_vault_id.clone(),
                    local_vault_id: local_id,
                    name: Some(vault_name),
                    last_version: None,
                    last_synced_at: None,
                });
            })?;

            let remote = crate::sync::HttpRemote::new(&base, &token);
            let (_report, version) = state.with_vault_mut(|open| open.sync_remote(&remote, &server_vault_id, None))?;
            state.update_settings(|settings| {
                if let Some(entry) = settings.synced_vaults.iter_mut().find(|v| v.path == path) {
                    entry.last_synced_at = Some(chrono::Utc::now().timestamp_millis());
                    entry.last_version = Some(version);
                }
            })?;
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

/// Stops syncing the open vault. The local file and the server copy are kept.
#[tauri::command(async)]
pub fn sync_disable(state: State<'_, AppState>) -> Result<AppStatus> {
    let path = state.session().path().map(Path::to_path_buf);
    if let Some(path) = path {
        state.update_settings(|settings| settings.forget_synced_vault(&path))?;
    }
    Ok(state.status())
}

/// Stops syncing a vault chosen from the list. The local file and the server copy are kept.
#[tauri::command(async)]
pub fn sync_disable_file(state: State<'_, AppState>, path: String) -> Result<AppStatus> {
    state.update_settings(|settings| settings.forget_synced_vault(&PathBuf::from(path)))?;
    Ok(state.status())
}

/// Stops syncing a vault (by its own id), removing every local copy's binding.
#[tauri::command(async)]
pub fn sync_disable_vault(state: State<'_, AppState>, local_vault_id: String) -> Result<AppStatus> {
    state.update_settings(|settings| {
        settings
            .synced_vaults
            .retain(|entry| entry.local_vault_id != local_vault_id);
    })?;
    Ok(state.status())
}

/// Deletes a cloud vault (its rows and blobs) and stops syncing it here. Local files are kept.
#[tauri::command]
pub async fn sync_delete_vault(app: AppHandle, server_vault_id: String) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
            crate::sync::delete_vault(&account.server_url, &account.token, &server_vault_id)?;
            state.update_settings(|settings| {
                settings
                    .synced_vaults
                    .retain(|entry| entry.server_vault_id != server_vault_id);
            })?;
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, server_vault_id);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Revokes the sync sessions of every other device, keeping this one signed in.
#[tauri::command]
pub async fn sync_sign_out_everywhere(app: AppHandle) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
            crate::sync::sign_out_everywhere(&account.server_url, &account.token)?;
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

/// Lists the cloud vaults of the signed-in account.
#[tauri::command]
pub async fn sync_cloud_vaults(app: AppHandle) -> Result<Vec<views::CloudVaultInfo>> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
            let mut vaults = crate::sync::list_vaults(&account.server_url, &account.token)?;
            // Names arrive encrypted; reveal them with the remembered label key. A label this
            // device cannot open (sealed under an old password) is shown as a placeholder.
            let key = stored_label_key(&state);
            for vault in &mut vaults {
                if let Some(cipher) = vault.name_cipher.clone() {
                    vault.name = key
                        .as_ref()
                        .and_then(|key| crypto::open(key, &cipher).ok())
                        .unwrap_or_else(|| "Encrypted vault".to_string());
                }
            }
            Ok(vaults.into_iter().map(views::CloudVaultInfo::from).collect())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// The file name a downloaded vault always gets: its own name, which the user cannot change.
fn vault_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    let stem = cleaned.trim().trim_matches('.').trim();
    let stem = if stem.is_empty() { "Vault" } else { stem };
    format!("{stem}.{}", storage::FILE_EXTENSION)
}

/// A free path inside `dir` for `file_name`, adding " (2)", " (3)"… when it is taken.
fn unique_in_dir(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Vault".into());
    let extension = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| storage::FILE_EXTENSION.to_string());
    for index in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.{}", Uuid::new_v4(), extension))
}

/// Downloads a cloud vault onto this device and binds it. The user chooses the folder; the
/// file is named after the vault, which (being the vault's own name) is not user-editable.
#[tauri::command]
pub async fn sync_download(
    app: AppHandle,
    server_vault_id: String,
    folder: String,
    password: String,
    key_file: Option<String>,
) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        use crate::sync::Remote;
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
            let password = Zeroizing::new(password);
            require_password(&password)?;
            let composite = composite_key(&password, key_file.as_deref())?;

            let remote = crate::sync::HttpRemote::new(&account.server_url, &account.token);
            let remote_vault = remote
                .get(&server_vault_id)?
                .ok_or_else(|| Error::invalid("This vault has no cloud copy yet"))?;
            // Deriving the key from the blob's own header is what lets any device open it.
            let header = storage::Header::parse(&remote_vault.bytes)?;
            let key = VaultKey::for_header(&composite, &header, &remote_vault.bytes)?;
            let vault = storage::decrypt_vault(&remote_vault.bytes, &key)?;

            let dir = PathBuf::from(folder);
            if !dir.is_dir() {
                return Err(Error::invalid("Choose a folder that exists"));
            }
            let dest = unique_in_dir(&dir, &vault_file_name(&vault.name));
            storage::write_atomic(&dest, &remote_vault.bytes)?;

            state.update_settings(|settings| {
                settings.set_synced_vault(crate::settings::SyncedVault {
                    path: dest.clone(),
                    server_vault_id,
                    local_vault_id: vault.id.to_string(),
                    name: Some(vault.name.clone()),
                    last_version: Some(remote_vault.version),
                    last_synced_at: Some(chrono::Utc::now().timestamp_millis()),
                });
                settings.remember_vault(&dest, key_file_path(key_file.as_deref()).as_deref());
            })?;
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, server_vault_id, folder, password, key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Reconciles the open (synced) vault with the server and reports what changed.
#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Result<MergeReport> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let path = state
                .session()
                .path()
                .map(Path::to_path_buf)
                .ok_or(Error::NoVaultSelected)?;
            let (base, email, token, server_vault_id, local_id, last_version) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
                let synced = settings
                    .synced_vault(&path)
                    .cloned()
                    .ok_or_else(|| Error::invalid("This vault is not set up for sync"))?;
                (
                    account.server_url,
                    account.email,
                    account.token,
                    synced.server_vault_id,
                    synced.local_vault_id,
                    synced.last_version,
                )
            };
            let open_id = state.with_vault(|open| Ok(open.vault().id.to_string()))?;
            if !local_id.is_empty() && local_id != open_id {
                return Err(Error::invalid("This vault is not the one set up for sync"));
            }
            let remote = crate::sync::HttpRemote::new(&base, &token);
            let outcome = state.with_vault_mut(|open| open.sync_remote(&remote, &server_vault_id, last_version));
            let (report, version) = match outcome {
                Ok(value) => value,
                Err(error) => {
                    // The cloud copy could not be read with this vault's key. If the vault's
                    // master password is not the account's (for example, it was re-keyed on its
                    // own), say so, so the interface can offer to re-key the vault back.
                    if matches!(error, Error::InvalidCredentials | Error::Corrupted) {
                        let composite = state.with_vault(|open| Ok(open.composite()))?;
                        if !is_the_account_password(&base, &email, &composite) {
                            return Err(wrong_master_password());
                        }
                    }
                    return Err(error);
                }
            };
            let open_name = state.with_vault(|open| Ok(open.vault().name.clone()))?;
            state.update_settings(|settings| {
                if let Some(entry) = settings.synced_vaults.iter_mut().find(|v| v.path == path) {
                    entry.last_synced_at = Some(chrono::Utc::now().timestamp_millis());
                    entry.name = Some(open_name);
                    entry.last_version = Some(version);
                }
            })?;
            Ok(report)
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Turns on sync for a vault chosen from the list: opens its own copy with the master
/// credentials and runs a first sync, without disturbing the vault currently open.
#[tauri::command]
pub async fn sync_enable_file(
    app: AppHandle,
    path: String,
    password: String,
    key_file: Option<String>,
) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let path = PathBuf::from(path);
            if !path.is_file() {
                return Err(Error::invalid(format!("File not found: {}", path.display())));
            }
            let (base, token) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
                (account.server_url, account.token)
            };

            // A vault that is already open is synced in place, so the visible session stays valid.
            let already_open = matches!(&*state.session(), Session::Unlocked(open) if open.path() == path);
            let version;
            if already_open {
                let (vault_name, local_id) =
                    state.with_vault(|open| Ok((open.vault().name.clone(), open.vault().id.to_string())))?;
                let label = {
                    let composite = state.with_vault(|open| Ok(open.composite()))?;
                    let email = {
                        let settings = state.settings();
                        settings.sync_account.as_ref().map(|account| account.email.clone()).unwrap_or_default()
                    };
                    if !is_the_account_password(&base, &email, &composite) {
                        return Err(wrong_master_password());
                    }
                    label_key_for(&state, &composite, &email)?
                };
                let name_cipher = crypto::seal(&label, &vault_name)?;
                let server_vault_id = crate::sync::ensure_vault(&base, &token, &local_id, &name_cipher)?;
                state.update_settings(|settings| {
                    settings.set_synced_vault(crate::settings::SyncedVault {
                        path: path.clone(),
                        server_vault_id: server_vault_id.clone(),
                        local_vault_id: local_id,
                        name: Some(vault_name),
                        last_version: None,
                        last_synced_at: None,
                    });
                })?;
                let remote = crate::sync::HttpRemote::new(&base, &token);
                let (_report, stored) = state.with_vault_mut(|open| open.sync_remote(&remote, &server_vault_id, None))?;
                version = stored;
            } else {
                let password = Zeroizing::new(password);
                require_password(&password)?;
                let composite = composite_key(&password, key_file.as_deref())?;
                let email = {
                    let settings = state.settings();
                    settings.sync_account.as_ref().map(|account| account.email.clone()).unwrap_or_default()
                };
                if !is_the_account_password(&base, &email, &composite) {
                    return Err(wrong_master_password());
                }
                let label = label_key_for(&state, &composite, &email)?;
                let mut open = OpenVault::open(path.clone(), composite)?;
                let vault_name = open.vault().name.clone();
                let local_id = open.vault().id.to_string();
                let name_cipher = crypto::seal(&label, &vault_name)?;
                let server_vault_id = crate::sync::ensure_vault(&base, &token, &local_id, &name_cipher)?;
                let remote = crate::sync::HttpRemote::new(&base, &token);
                let (_report, stored) = open.sync_remote(&remote, &server_vault_id, None)?;
                version = stored;
                state.update_settings(|settings| {
                    settings.set_synced_vault(crate::settings::SyncedVault {
                        path: path.clone(),
                        server_vault_id,
                        local_vault_id: local_id,
                        name: Some(vault_name),
                        last_version: None,
                        last_synced_at: None,
                    });
                })?;
            }
            state.update_settings(|settings| {
                if let Some(entry) = settings.synced_vaults.iter_mut().find(|v| v.path == path) {
                    entry.last_synced_at = Some(chrono::Utc::now().timestamp_millis());
                    entry.last_version = Some(version);
                }
            })?;
            Ok(state.status())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, path, password, key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Re-encrypts a vault with the sync master password, so it can then be synced. The caller
/// verifies the master password and re-runs the normal "enable sync" step afterwards.
#[tauri::command]
pub async fn rekey_vault_file(
    app: AppHandle,
    path: String,
    password: Option<String>,
    key_file: Option<String>,
    master_password: String,
    master_key_file: Option<String>,
) -> Result<()> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let (base, email) = {
                let settings = state.settings();
                let account = settings
                    .sync_account
                    .clone()
                    .ok_or_else(|| Error::invalid("Sign in to sync first"))?;
                (account.server_url, account.email)
            };

            let master = Zeroizing::new(master_password);
            require_password(&master)?;
            let master_composite = composite_key(&master, master_key_file.as_deref())?;
            if !is_the_account_password(&base, &email, &master_composite) {
                return Err(Error::invalid("That is not your sync master password"));
            }

            let path = PathBuf::from(path);
            let already_open = matches!(&*state.session(), Session::Unlocked(open) if open.path() == path);
            if already_open {
                state.with_vault_mut(|open| open.rekey(master_composite, KdfParams::calibrated()))?;
            } else {
                if !path.is_file() {
                    return Err(Error::invalid(format!("File not found: {}", path.display())));
                }
                let password = Zeroizing::new(password.unwrap_or_default());
                require_password(&password)?;
                let composite = composite_key(&password, key_file.as_deref())?;
                let mut open = OpenVault::open(path, composite)?;
                open.rekey(master_composite, KdfParams::calibrated())?;
            }
            Ok(())
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = (app, path, password, key_file, master_password, master_key_file);
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Refreshes what this device knows about the account. If the session is no longer valid — for
/// example because the master password was changed on another device — it signs out locally, so
/// the interface can ask the user to sign in again with the current password.
#[tauri::command]
pub async fn sync_refresh(app: AppHandle) -> Result<AppStatus> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let Some(account) = state.settings().sync_account.clone() else {
                return Ok(state.status());
            };
            match crate::sync::me(&account.server_url, &account.token) {
                Ok(info) => {
                    state.update_settings(|settings| {
                        if let Some(account) = settings.sync_account.as_mut() {
                            account.password_changed_at = info.password_changed_at;
                            account.plan = Some(info.plan);
                            account.plan_valid_until = info.plan_valid_until;
                        }
                    })?;
                    Ok(state.status())
                }
                Err(Error::SessionExpired) => {
                    state.update_settings(|settings| {
                        settings.sync_account = None;
                        settings.sync_label_key = None;
                    })?;
                    Ok(state.status())
                }
                Err(error) => Err(error),
            }
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}
