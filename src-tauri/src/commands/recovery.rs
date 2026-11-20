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
    /// `.bak` copies kept beside the vaults, which open with a **previous** password. Named so the
    /// user can delete them (the red-team report, H2).
    pub backups: Vec<String>,
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

/// The outcome of checking a recovery key file against every vault it can reach. Nothing is
/// opened or changed: this only answers "does this file still open my vaults?".
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCheckView {
    /// The vaults this key opens, by name.
    pub opens: Vec<String>,
    /// The vaults that carry a *different* recovery key: an older file was used for them.
    pub other_key: Vec<String>,
    /// The vaults that have no recovery key at all.
    pub no_key: Vec<String>,
    /// Every vault that was inspected is opened by this key.
    pub complete: bool,
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

    // Every vault file on this device whose key this password really opens. Anything this password
    // does not open is left alone, keeping its own recovery key. The check decrypts, so a vault with
    // a *different* password (a common case on an unwrapped v1 file) is correctly left out (N3).
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
    let account = state.settings().sync_account.as_ref().map(|account| account.email.clone());

    // Write the file *first*, with the number of vaults this password really opens: a code with no
    // vault is harmless, a wrap whose code the user never received is not. The file names no vaults
    // (I4); the list is shown in the app instead.
    storage::write_atomic(file, recovery::file_contents(covered.len(), account.as_deref(), &text).as_bytes())?;

    // Add the wrap to each covered vault and save it. The open one goes through the session so it
    // stays valid; the others are opened just for this. A vault that cannot be written is skipped
    // and named, instead of aborting the whole run and leaving the file's count wrong (N3).
    let mut wrapped: Vec<(PathBuf, VaultKey)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (path, key) in &covered {
        let name = views::file_name(path);
        let wrapped_key = match key.with_recovery(&composite, &code) {
            Ok(key) => key,
            Err(_) => {
                failed.push(name);
                continue;
            }
        };
        let result = if same_file(path, &open_path) {
            state.with_vault_mut(|open| open.change_key(composite.clone(), wrapped_key.clone()).map(|_| ()))
        } else {
            OpenVault::open(path.clone(), composite.clone())
                .and_then(|mut open| open.change_key(composite.clone(), wrapped_key).map(|_| ()))
        };
        match result {
            Ok(()) => wrapped.push((path.clone(), key.clone())),
            Err(_) => failed.push(name),
        }
    }
    if wrapped.is_empty() {
        return Err(Error::invalid(
            "Nothing could be re-encrypted, so no recovery key was written. The vault files were left as they were.",
        ));
    }
    // Rewrite the count when a vault was skipped, so the file says what it really opens.
    if !failed.is_empty() {
        storage::write_atomic(file, recovery::file_contents(wrapped.len(), account.as_deref(), &text).as_bytes())?;
    }

    // Send the wrap to the cloud so the copies on the other devices receive it as well.
    #[cfg(feature = "sync")]
    push_wraps(state, &wrapped, &composite, &code);

    // The `.bak` kept beside each vault is a copy from before an earlier change: it opens with the
    // password the vault had then. Name those copies so the user can delete them (the report, H2).
    let backups: Vec<String> = covered
        .iter()
        .map(|(path, _)| storage::backup_path(path))
        .filter(|backup| backup.exists())
        .map(|backup| views::path_text(&backup))
        .collect();

    Ok(RecoveryKeyView {
        path: views::path_text(file),
        vaults: wrapped.iter().map(|(path, _)| views::file_name(path)).collect(),
        account,
        uncovered,
        backups,
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

    // Signed out, a recovered synced vault no longer matches the account password. Remember that, so
    // the sync space can explain it rather than failing later with a bare "Sync failed" (M6).
    if !account_updated
        && let Some(path) = &open_path
        && state.settings().synced_vault(path).is_some()
    {
        state.update_settings(|settings| settings.pending_account_password = true)?;
    }

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

// ----- What a key would touch (shown before anything is written) -----

/// The vaults a recovery key covers, by file name. Shown to the user *before* any vault is
/// re-encrypted, because a key can reach vaults in folders the user has forgotten about.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryTargetsView {
    pub vaults: Vec<String>,
}

/// Every vault this device would re-encrypt if a recovery key were made now: the ones the open
/// vault's master password opens.
#[tauri::command]
pub async fn recovery_key_targets(app: AppHandle) -> Result<RecoveryTargetsView> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let composite = state.with_vault(|open| Ok(open.composite()))?;
        let mut vaults = Vec::new();
        for path in candidate_vaults(&state) {
            if vault_key_for(&path, &composite).is_ok() {
                vaults.push(views::path_text(&path));
            }
        }
        Ok(RecoveryTargetsView { vaults })
    })
    .await
}

/// Every vault on this device that a recovery key file can open.
#[tauri::command]
pub async fn recovery_file_targets(app: AppHandle, recovery_file: String) -> Result<RecoveryTargetsView> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let text = Zeroizing::new(String::from_utf8_lossy(&storage::read_file(Path::new(&recovery_file))?).into_owned());
        let code = recovery::parse_code(&text).ok_or_else(|| Error::invalid("That file does not hold a Lados recovery key"))?;

        let mut vaults = Vec::new();
        for path in candidate_vaults(&state) {
            let Ok(bytes) = storage::read_file(&path) else {
                continue;
            };
            if matches!(wrap_state(&bytes, &code), WrapState::Opens) {
                vaults.push(views::path_text(&path));
            }
        }
        Ok(RecoveryTargetsView { vaults })
    })
    .await
}

// ----- Backups left beside the vaults -----

/// Every `.bak` kept beside a vault on this device. Each is a copy from before an earlier change and
/// opens with the password the vault had then, so it is named after a password change or a recovery
/// and can be deleted (the red-team report, H2).
#[tauri::command(async)]
pub fn existing_backups(state: State<'_, AppState>) -> Result<Vec<String>> {
    Ok(candidate_vaults(&state)
        .into_iter()
        .map(|path| storage::backup_path(&path))
        .filter(|backup| backup.exists())
        .map(|backup| views::path_text(&backup))
        .collect())
}

/// Deletes one such copy, after the user was told what it is. Only `*.lados.bak` is accepted.
#[tauri::command(async)]
pub fn delete_vault_backup(path: String) -> Result<()> {
    let path = PathBuf::from(path);
    let wanted = format!(".{}.bak", storage::FILE_EXTENSION);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !name.ends_with(wanted.as_str()) {
        return Err(Error::invalid("That is not a vault backup"));
    }
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

// ----- Checking a recovery key -----
/// Checks a recovery key file against every vault it can reach, without opening or changing
/// anything. The point is to know *before* it is needed that the file on disk still opens the
/// vaults it claims to.
#[tauri::command]
pub async fn check_recovery_key(app: AppHandle, recovery_file: String) -> Result<RecoveryCheckView> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        check_for_account(&state, &recovery_file)
    })
    .await
}

fn check_for_account(state: &AppState, recovery_file: &str) -> Result<RecoveryCheckView> {
    let text = Zeroizing::new(String::from_utf8_lossy(&storage::read_file(Path::new(recovery_file))?).into_owned());
    let code = recovery::parse_code(&text).ok_or_else(|| Error::invalid("That file does not hold a Lados recovery key"))?;

    let mut check = RecoveryCheckView {
        opens: Vec::new(),
        other_key: Vec::new(),
        no_key: Vec::new(),
        complete: false,
    };

    for path in candidate_vaults(state) {
        let Ok(bytes) = storage::read_file(&path) else {
            continue;
        };
        let name = views::file_name(&path);
        match wrap_state(&bytes, &code) {
            WrapState::Opens => check.opens.push(name),
            WrapState::Different => check.other_key.push(name),
            WrapState::None => check.no_key.push(name),
            WrapState::NotAVault => {}
        }
    }

    #[cfg(feature = "sync")]
    check_cloud(state, &code, &mut check);

    check.complete = check.other_key.is_empty() && check.no_key.is_empty() && !check.opens.is_empty();
    Ok(check)
}

/// How a vault file or blob relates to a recovery code.
enum WrapState {
    /// The code opens this vault.
    Opens,
    /// The vault has a recovery key, but a different one.
    Different,
    /// The vault has no recovery key at all.
    None,
    /// Not a Lados vault.
    NotAVault,
}

fn wrap_state(bytes: &[u8], code: &SecretKey) -> WrapState {
    let Ok(header) = storage::Header::parse(bytes) else {
        return WrapState::NotAVault;
    };
    if !header.has_recovery() {
        return WrapState::None;
    }
    match storage::key_from_recovery(&header, bytes, code) {
        Ok(_) => WrapState::Opens,
        Err(_) => WrapState::Different,
    }
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

/// Checks the account's cloud copies too, so a vault that is not on this device is not left out of
/// the result. Best effort: a network problem simply leaves those copies unchecked.
#[cfg(feature = "sync")]
fn check_cloud(state: &AppState, code: &SecretKey, check: &mut RecoveryCheckView) {
    use crate::sync::Remote;

    let Some(account) = state.settings().sync_account.clone() else {
        return;
    };
    let Ok(cloud) = crate::sync::list_vaults(&account.server_url, &account.token) else {
        return;
    };
    let remote = crate::sync::HttpRemote::new(&account.server_url, &account.token);
    let label_key = stored_label_key(state);

    // Vaults already inspected as files here, so the same vault is not reported twice.
    let local_ids: HashSet<String> = {
        let settings = state.settings();
        settings
            .synced_vaults
            .iter()
            .filter(|entry| entry.path.is_file())
            .map(|entry| entry.server_vault_id.clone())
            .collect()
    };

    for vault in cloud {
        if local_ids.contains(&vault.id) {
            continue;
        }
        let Ok(Some(blob)) = remote.get(&vault.id) else {
            continue;
        };
        let name = vault
            .name_cipher
            .as_ref()
            .and_then(|cipher| label_key.as_ref().and_then(|key| crypto::open(key, cipher).ok()))
            .unwrap_or_else(|| vault.name.clone());
        match wrap_state(&blob.bytes, code) {
            WrapState::Opens => check.opens.push(name),
            WrapState::Different => check.other_key.push(name),
            WrapState::None => check.no_key.push(name),
            WrapState::NotAVault => {}
        }
    }
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
///
/// An **unwrapped** (version-1) file has nothing to authenticate the password until it is decrypted,
/// so `for_header` returns a key for *any* password. That made vaults with a different password look
/// "covered" by a recovery key (the red-team report, N3), so the vault is decrypted here to be sure.
fn vault_key_for(path: &Path, composite: &SecretKey) -> Result<VaultKey> {
    let bytes = storage::read_file(path)?;
    let header = storage::Header::parse(&bytes)?;
    let key = VaultKey::for_header(composite, &header, &bytes)?;
    // Decrypting is the real test. The plaintext is dropped (and zeroed) at once, since only the
    // answer — "this password opens it" — is needed here.
    let _opened = Zeroizing::new(storage::decrypt_vault(&bytes, &key)?);
    Ok(key)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault file wrapped with `code`.
    fn wrapped_with(code: &SecretKey) -> Vec<u8> {
        let composite = crypto::composite_key("pw", None);
        let key = VaultKey::generate(&composite, KdfParams::FOR_TESTS)
            .unwrap()
            .with_recovery(&composite, code)
            .unwrap();
        storage::encrypt_vault(&Vault::new("Test").unwrap(), &key).unwrap()
    }

    /// The checker must tell apart the three answers that matter: this key opens the vault, the
    /// vault has *another* key, or it has none at all. Getting these wrong is what would let a user
    /// believe they are covered when they are not.
    #[test]
    fn a_check_tells_apart_opening_a_different_key_and_no_key() {
        let code = Zeroizing::new([3u8; 32]);
        let other = Zeroizing::new([4u8; 32]);

        assert!(matches!(wrap_state(&wrapped_with(&code), &code), WrapState::Opens));
        assert!(matches!(wrap_state(&wrapped_with(&code), &other), WrapState::Different));

        // A version-1 vault carries no recovery key at all.
        let composite = crypto::composite_key("pw", None);
        let plain = VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap();
        let version_one = storage::encrypt_vault(&Vault::new("Test").unwrap(), &plain).unwrap();
        assert!(matches!(wrap_state(&version_one, &code), WrapState::None));

        // Anything that is not a vault is ignored rather than reported.
        assert!(matches!(wrap_state(b"not a vault", &code), WrapState::NotAVault));
    }
}

