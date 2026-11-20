//! The only door between the interface and the Rust core.
//!
//! Every `#[tauri::command]` below can be called from the interface with `invoke`. Commands
//! check their input, delegate to the core modules and answer with the shapes in `views.rs`.
//! Secrets leave Rust one at a time, only when the user reveals, copies or edits them.
//!
//! Commands that touch the disk, open dialogs or derive keys run off the main thread
//! (`async`), so the window never freezes.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::clipboard;
use crate::crypto::{self, KdfParams, SecretKey};
use crate::csv_io;
use crate::error::{Error, Result};
use crate::generator::{self, GeneratorOptions};
use crate::keepass_io;
use crate::merge::MergeReport;
use crate::models::{self, EntryContent, Icon, Vault};
use crate::session::{self, OpenVault, SaveOutcome};
use crate::settings::Preferences;
use crate::state::{AppState, Session};
use crate::storage::{self, VaultKey};
use crate::views::{self, AppStatus, EntryDetails, ImportSummary, RevisionSummary, VaultView};

const MAX_ICON_BYTES: usize = 512 * 1024;

// The command surface is split by what it is about. Each module below re-uses the imports and the
// helpers declared here with `use super::*`, so this file stays the shared base of the whole
// surface - and no single file grows without bound.

/// Bumped whenever a command's arguments or results change shape.
///
/// The interface carries the same number (`API_REVISION` in `src/types.ts`) and compares the two.
/// Without it, a running process older than its interface fails with a confusing message on the
/// first new call — which is what happens in development, where the dev server reloads the
/// interface instantly while the app process only changes when it is restarted.
pub const API_REVISION: u32 = 6;

pub mod account;
pub mod autofill;
pub mod billing;
pub mod entries;
pub mod recovery;
pub mod sync;
pub mod transfer;
pub mod vault;

// ----- Actions shared with the tray and the background task -----

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    User,
    Inactivity,
    Tray,
}

/// Wipes the decrypted vault from memory and removes any copied secret from the clipboard.
pub fn lock_now(app: &AppHandle, reason: LockReason) -> bool {
    let state = app.state::<AppState>();
    // Never throw away a vault whose file has gone: write it back first, or locking would lose the
    // only copy (the red-team report, M8).
    save_current_if_missing(app, &state);
    let locked = state.session().lock();
    if locked {
        // A locked vault must not leave an "allow this fill?" prompt answerable, nor an undo
        // holding decrypted rows from a vault that is no longer open.
        state.cancel_reveals();
        state.clear_delete();
        clipboard::clear_now(app);
        let _ = app.emit("vault-locked", reason);
    }
    locked
}

pub fn quit_now(app: &AppHandle) {
    cleanup(app);
    app.exit(0);
}

/// Leaves no secret behind, however the application ends.
pub fn cleanup(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.begin_quit();
    clipboard::clear_now(app);
    state.session().lock();
    state.cancel_reveals();
    state.clear_delete();
}

/// A conflict copy joins the recent vaults, with the same key file as the original.
pub fn remember_copy(state: &AppState, original: &Path, copy: &Path) {
    let key_file = state.settings().key_file_for(original);
    let _ = state.update_settings(|settings| settings.remember_vault(copy, key_file.as_deref()));
}

// ----- Helpers -----

fn report_save(app: &AppHandle, state: &AppState, previous_path: &Path, outcome: &SaveOutcome) {
    if let Some(report) = &outcome.merged {
        let _ = app.emit("vault-changed", report);
    }
    if let Some(copy) = &outcome.conflict_copy {
        remember_copy(state, previous_path, copy);
        let _ = app.emit("vault-diverged", views::path_text(copy));
    }
    if outcome.missing {
        // The file had been deleted or moved; **this save wrote it again** at the same path. A
        // distinct event from the poll's "the file is gone" (the red-team report, M8), because this
        // one is true: a save really happened.
        let _ = app.emit("vault-recreated", views::path_text(previous_path));
    }
}

/// Applies a change to the unlocked vault and saves it.
fn modify<T>(app: &AppHandle, state: &AppState, change: impl FnOnce(&mut Vault) -> Result<T>) -> Result<T> {
    state.touch();
    let (previous_path, value, outcome) = state.with_vault_mut(|open| {
        let previous_path = open.path().to_path_buf();
        let (value, outcome) = open.modify(change)?;
        Ok((previous_path, value, outcome))
    })?;
    report_save(app, state, &previous_path, &outcome);
    Ok(value)
}

/// Writes the open vault back to disk when its file is no longer there. Called before a lock, so a
/// vault stripped from disk by another program is not also lost from memory (the report, M8).
pub fn save_current_if_missing(app: &AppHandle, state: &AppState) {
    let missing = state.with_vault(|open| Ok(!open.path().is_file())).unwrap_or(false);
    if missing {
        let _ = modify(app, state, |_| Ok(()));
    }
}

/// Writes the open vault back to its file, recreating it when it had been removed. The user asks for
/// this from the "the vault file is gone" prompt, so nothing is lost (the red-team report, M8).
#[tauri::command(async)]
pub fn save_vault_again(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    modify(&app, &state, |_| Ok(()))
}

/// Replaces the current session; an unlocked vault is wiped from memory when dropped.
fn replace_session(app: &AppHandle, state: &AppState, new: Session) {
    let previous = std::mem::replace(&mut *state.session(), new);
    if matches!(previous, Session::Unlocked(_)) {
        clipboard::clear_now(app);
    }
    // A pending undo belongs to the vault that was open; a new one must not inherit it.
    state.clear_delete();
}

fn key_file_path(key_file: Option<&str>) -> Option<PathBuf> {
    key_file.map(str::trim).filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn read_key_file(key_file: Option<&str>) -> Result<Option<Zeroizing<Vec<u8>>>> {
    key_file_path(key_file)
        .map(|path| storage::read_key_file(&path))
        .transpose()
}

fn composite_key(password: &str, key_file: Option<&str>) -> Result<SecretKey> {
    let key_file = read_key_file(key_file)?;
    Ok(crypto::composite_key(password, key_file.as_ref().map(|bytes| bytes.as_slice())))
}

fn require_password(password: &str) -> Result<()> {
    if password.is_empty() {
        return Err(Error::invalid("The master password cannot be empty"));
    }
    Ok(())
}

/// Confirms that the caller knows the credentials that opened the current vault.
///
/// Used before an action that cannot be undone — writing every secret out in clear, changing the
/// master key — so that someone who walks up to an unlocked machine cannot do it. A vault opened
/// with a key file and no password is accepted, as long as the key file is given.
fn confirm_open_vault(state: &AppState, password: &str, key_file: Option<&str>) -> Result<()> {
    if password.is_empty() && key_file.is_none() {
        return Err(Error::invalid("Enter the password, or choose the key file that opened this vault"));
    }
    let password = Zeroizing::new(password.to_owned());
    let composite = composite_key(&password, key_file)?;
    let open = state.with_vault(|open| Ok(open.composite()))?;
    if crypto::secret_to_base64(&composite) != crypto::secret_to_base64(&open) {
        return Err(Error::invalid("That is not what opened this vault"));
    }
    Ok(())
}

/// Runs slow work (key derivation, file dialogs) on a thread made for blocking tasks.
async fn run_blocking<T: Send + 'static>(task: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| Error::System(error.to_string()))?
}

fn with_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension)) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Like [`with_extension`], but keeps whatever extension the user typed: only a name with none
/// gets the default. Used where forcing the extension would rename the user's own file.
fn with_default_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    with_extension(path, extension)
}

fn safe_vault_file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "Vault".into()
    } else {
        cleaned
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Vault,
    KeyFile,
    Csv,
    Kdbx,
    Image,
    /// The text file a recovery key is written to.
    Recovery,
}

/// Where `.lados` vaults live.
///
/// On mobile this is the shared `Documents/Lados` folder so vaults survive app
/// uninstall/reinstall. App-private storage is wiped by Android on uninstall, which
/// is unacceptable for a password manager.
fn vaults_directory(app: &AppHandle) -> Result<PathBuf> {
    #[cfg(mobile)]
    {
        let dir = persistent_mobile_vaults_dir(app)?;
        migrate_legacy_mobile_vaults(app, &dir);
        return Ok(dir);
    }
    #[cfg(not(mobile))]
    {
        // A folder the user can see, back up and put in a cloud drive — not app-private storage.
        let custom = app.state::<AppState>().settings().preferences.vaults_folder.clone();
        let dir = match custom {
            Some(folder) if !folder.trim().is_empty() => PathBuf::from(folder.trim()),
            _ => default_vaults_directory(app)?,
        };
        std::fs::create_dir_all(&dir).map_err(|error| Error::System(error.to_string()))?;
        Ok(dir)
    }
}

/// The standard vaults folder when the user has not chosen another one.
#[cfg(not(mobile))]
fn default_vaults_directory(app: &AppHandle) -> Result<PathBuf> {
    match app.path().document_dir() {
        Ok(documents) => Ok(documents.join("Lados")),
        Err(_) => {
            let base = app
                .path()
                .app_data_dir()
                .map_err(|error| Error::System(error.to_string()))?;
            Ok(base.join("vaults"))
        }
    }
}

#[cfg(mobile)]
fn persistent_mobile_vaults_dir(app: &AppHandle) -> Result<PathBuf> {
    // home_dir on Android is the shared external storage root (e.g. /storage/emulated/0).
    let home = app
        .path()
        .home_dir()
        .map_err(|error| Error::System(error.to_string()))?;
    let dir = home.join("Documents").join("Lados");
    std::fs::create_dir_all(&dir).map_err(|error| {
        Error::System(format!(
            "Could not create Documents/Lados. Allow Lados to manage files on this device, then try again. ({error})"
        ))
    })?;
    // Prove we can write; otherwise the user still needs the storage permission.
    let probe = dir.join(".lados-write-check");
    std::fs::write(&probe, b"ok").map_err(|error| {
        Error::System(format!(
            "Documents/Lados is not writable yet. Open Android settings and allow Lados “All files access”, then return to the app. ({error})"
        ))
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(dir)
}

/// Older builds stored vaults under app-private storage (deleted on uninstall).
/// Copy any survivors into Documents/Lados once.
#[cfg(mobile)]
fn migrate_legacy_mobile_vaults(app: &AppHandle, dest: &Path) {
    let mut sources = Vec::new();
    if let Ok(app_data) = app.path().app_data_dir() {
        sources.push(app_data.join("vaults"));
    }
    // Tauri's document_dir is app-specific external storage (also wiped on uninstall).
    if let Ok(docs) = app.path().document_dir() {
        sources.push(docs.clone());
        sources.push(docs.join("Lados"));
    }
    for source in sources {
        let Ok(entries) = std::fs::read_dir(&source) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_vault = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(storage::FILE_EXTENSION))
                && path.is_file();
            if !is_vault {
                continue;
            }
            let Some(name) = path.file_name() else {
                continue;
            };
            let target = dest.join(name);
            if target.exists() {
                continue;
            }
            let _ = std::fs::copy(&path, &target);
        }
    }
}

#[cfg(mobile)]
fn imports_directory(app: &AppHandle, kind: FileKind) -> Result<PathBuf> {
    if matches!(kind, FileKind::Vault) {
        return vaults_directory(app);
    }
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| Error::System(error.to_string()))?;
    let name = match kind {
        FileKind::KeyFile => "keys",
        FileKind::Csv | FileKind::Kdbx => "imports",
        FileKind::Image => "icons",
        FileKind::Vault => unreachable!("vaults use vaults_directory"),
    };
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).map_err(|error| Error::System(error.to_string()))?;
    Ok(dir)
}

#[tauri::command]
pub fn is_mobile_shell() -> bool {
    cfg!(mobile)
}

#[tauri::command]
pub fn suggested_vault_path(app: AppHandle, name: String) -> Result<String> {
    let path = vaults_directory(&app)?.join(format!(
        "{}.{}",
        safe_vault_file_stem(&name),
        storage::FILE_EXTENSION
    ));
    Ok(views::path_text(&path))
}

/// The folder new vaults are created in and downloads land in.
#[tauri::command]
pub fn vaults_folder(app: AppHandle) -> Result<String> {
    Ok(views::path_text(&vaults_directory(&app)?))
}

/// Vaults already stored in the app folder (created or imported on this device).
#[tauri::command]
pub fn list_device_vaults(app: AppHandle) -> Result<Vec<views::RecentVaultView>> {
    let dir = vaults_directory(&app)?;
    let mut vaults = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(vaults),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_vault = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(storage::FILE_EXTENSION))
            && path.is_file();
        if !is_vault {
            continue;
        }
        vaults.push(views::RecentVaultView {
            path: views::path_text(&path),
            file_name: views::file_name(&path),
            key_file: None,
            exists: true,
            synced: false,
            last_synced_at: None,
        });
    }
    vaults.sort_by_key(|a| a.file_name.to_lowercase());
    Ok(vaults)
}

/// The file name without extension, used to name the folder that receives an import.
fn source_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Imported".into())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Only web pages are opened: a stored `file:` or `javascript:` link could run anything.
fn web_address(url: &str) -> Result<String> {
    let url = url.trim();
    let lower = url.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Ok(url.to_string());
    }
    let has_scheme = url.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphabetic())
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    if has_scheme || url.is_empty() {
        return Err(Error::invalid("Only web addresses (http or https) can be opened"));
    }
    Ok(format!("https://{url}"))
}

// ----- Account and session helpers, shared by the sync and account commands -----

/// The label key used to encrypt vault names for the server, if it is remembered.
#[cfg(feature = "sync")]
fn stored_label_key(state: &AppState) -> Option<SecretKey> {
    state
        .settings()
        .sync_label_key
        .as_ref()
        .and_then(|value| crypto::secret_from_base64(value))
}

/// The label key, derived from the master credentials when it is not remembered yet.
#[cfg(feature = "sync")]
fn label_key_for(state: &AppState, composite: &SecretKey, email: &str) -> Result<SecretKey> {
    if let Some(key) = stored_label_key(state) {
        return Ok(key);
    }
    let key = crypto::label_key(composite, email)?;
    let encoded = crypto::secret_to_base64(&key);
    state.update_settings(|settings| settings.sync_label_key = Some(encoded))?;
    Ok(key)
}

/// Runs an OPAQUE login. Fails without detail when the account has no OPAQUE record yet, or
/// when the password is wrong.
#[cfg(feature = "sync")]
fn opaque_login(base: &str, email: &str, secret: &[u8]) -> Result<String> {
    let (state, request) = crate::sync::login_start(secret)?;
    let (login_id, response) = crate::sync::opaque_login_start(base, email, &request)?;
    let finish = crate::sync::login_finish(secret, &state, &response)?;
    crate::sync::opaque_login_finish(base, &login_id, &finish)
}

/// Attaches an OPAQUE record to an already-authenticated account, so the next sign-in uses OPAQUE.
#[cfg(feature = "sync")]
fn upgrade_to_opaque(base: &str, token: &str, secret: &[u8]) -> Result<()> {
    let (state, request) = crate::sync::registration_start(secret)?;
    let response = crate::sync::opaque_upgrade_start(base, token, &request)?;
    let record = crate::sync::registration_finish(secret, &state, &response)?;
    crate::sync::opaque_upgrade_finish(base, token, &record)
}

/// Whether `composite` is the account's master password, verified with an OPAQUE login. Nothing
/// secret has to be remembered between sessions.
#[cfg(feature = "sync")]
fn is_the_account_password(base: &str, email: &str, composite: &SecretKey) -> bool {
    let Ok(auth) = crate::sync::auth_secret(composite, email) else {
        return false;
    };
    opaque_login(base, email, auth.as_bytes()).is_ok()
}

/// Sends one step of a coordinated master-password change to the interface.
#[cfg(feature = "sync")]
fn emit_password_change(app: &AppHandle, step: &str, index: usize, total: usize, label: &str) {
    let _ = app.emit(
        "master-password-progress",
        serde_json::json!({ "step": step, "index": index, "total": total, "label": label }),
    );
}

/// The file of the vault that is selected but still locked: what unlocking acts on.
fn locked_path(app: &AppHandle) -> Result<PathBuf> {
    match &*app.state::<AppState>().session() {
        Session::Locked { path } => Ok(path.clone()),
        Session::Unlocked(_) => Err(Error::invalid("The vault is already unlocked")),
        Session::Closed => Err(Error::NoVaultSelected),
    }
}

// ----- Application -----

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> AppStatus {
    state.status()
}

#[tauri::command(async)]
pub fn update_preferences(state: State<'_, AppState>, preferences: Preferences) -> Result<AppStatus> {
    state.update_settings(|settings| settings.preferences = preferences.clamped())?;
    Ok(state.status())
}

#[tauri::command(async)]
pub fn forget_recent_vault(state: State<'_, AppState>, path: String) -> Result<AppStatus> {
    state.update_settings(|settings| settings.forget_vault(Path::new(&path)))?;
    Ok(state.status())
}

/// Removes a vault from the list. On mobile this also deletes the `.lados` file from
/// `Documents/Lados` (only paths under that folder are accepted). On desktop it only
/// forgets the recent entry — the file on disk is left alone.
#[tauri::command(async)]
pub fn remove_listed_vault(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<AppStatus> {
    let path = PathBuf::from(path);
    #[cfg(mobile)]
    {
        let vaults_dir = vaults_directory(&app)?;
        if !path_is_inside_dir(&path, &vaults_dir) {
            return Err(Error::invalid(
                "Only vaults stored in Documents › Lados can be deleted from this list",
            ));
        }
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|error| Error::System(error.to_string()))?;
        }
    }
    #[cfg(desktop)]
    let _ = &app;
    state.update_settings(|settings| settings.forget_vault(&path))?;
    Ok(state.status())
}

#[cfg(mobile)]
fn path_is_inside_dir(path: &Path, dir: &Path) -> bool {
    let canonical_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if let Ok(canonical_path) = std::fs::canonicalize(path) {
        return canonical_path.starts_with(&canonical_dir);
    }
    path.parent()
        .map(|parent| {
            std::fs::canonicalize(parent)
                .map(|parent| parent == canonical_dir)
                .unwrap_or_else(|_| parent == dir)
        })
        .unwrap_or(false)
}

/// The interface reports keyboard and mouse activity, which postpones the automatic lock.
#[tauri::command]
pub fn touch(state: State<'_, AppState>) {
    state.touch();
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    quit_now(&app);
}

