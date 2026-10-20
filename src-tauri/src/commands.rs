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
    let locked = app.state::<AppState>().session().lock();
    if locked {
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

/// Replaces the current session; an unlocked vault is wiped from memory when dropped.
fn replace_session(app: &AppHandle, state: &AppState, new: Session) {
    let previous = std::mem::replace(&mut *state.session(), new);
    if matches!(previous, Session::Unlocked(_)) {
        clipboard::clear_now(app);
    }
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

/// The interface reports keyboard and mouse activity, which postpones the automatic lock.
#[tauri::command]
pub fn touch(state: State<'_, AppState>) {
    state.touch();
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    quit_now(&app);
}

// ----- Vault session -----

#[tauri::command]
pub fn select_vault(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<AppStatus> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(Error::invalid(format!("File not found: {}", path.display())));
    }
    replace_session(&app, &state, Session::Locked { path });
    Ok(state.status())
}

#[tauri::command]
pub fn close_vault(app: AppHandle, state: State<'_, AppState>) -> AppStatus {
    replace_session(&app, &state, Session::Closed);
    state.status()
}

#[tauri::command]
pub async fn create_vault(
    app: AppHandle,
    path: String,
    name: String,
    password: String,
    key_file: Option<String>,
) -> Result<AppStatus> {
    let password = Zeroizing::new(password);
    require_password(&password)?;
    let path = with_extension(PathBuf::from(path), storage::FILE_EXTENSION);
    let composite = composite_key(&password, key_file.as_deref())?;
    let target = path.clone();
    let open = run_blocking(move || OpenVault::create(target, &name, composite, KdfParams::calibrated())).await?;

    let state = app.state::<AppState>();
    replace_session(&app, &state, Session::Unlocked(Box::new(open)));
    let _ = state.update_settings(|settings| {
        settings.remember_vault(&path, key_file_path(key_file.as_deref()).as_deref())
    });
    state.touch();
    Ok(state.status())
}

fn locked_path(app: &AppHandle) -> Result<PathBuf> {
    match &*app.state::<AppState>().session() {
        Session::Locked { path } => Ok(path.clone()),
        Session::Unlocked(_) => Err(Error::invalid("The vault is already unlocked")),
        Session::Closed => Err(Error::NoVaultSelected),
    }
}

#[tauri::command]
pub async fn unlock_vault(app: AppHandle, password: String, key_file: Option<String>) -> Result<AppStatus> {
    let password = Zeroizing::new(password);
    let path = locked_path(&app)?;
    let composite = composite_key(&password, key_file.as_deref())?;
    let target = path.clone();
    let open = run_blocking(move || OpenVault::open(target, composite)).await?;

    let state = app.state::<AppState>();
    {
        let mut session = state.session();
        // The user may have picked another vault while this one was being unlocked.
        if !matches!(&*session, Session::Locked { path: selected } if *selected == path) {
            return Err(Error::invalid("Another vault was selected in the meantime"));
        }
        *session = Session::Unlocked(Box::new(open));
    }
    let _ = state.update_settings(|settings| {
        settings.remember_vault(&path, key_file_path(key_file.as_deref()).as_deref())
    });
    state.touch();
    Ok(state.status())
}

#[tauri::command]
pub fn lock_vault(app: AppHandle) -> AppStatus {
    lock_now(&app, LockReason::User);
    app.state::<AppState>().status()
}

/// Creates a new random key file and returns its path, or `None` if the user cancelled.
#[tauri::command]
pub async fn generate_key_file(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    let suggested = Some("Lados key.keyx".to_string());
    let Some(path) = save_dialog(app, window, FileKind::KeyFile, "Save the new key file".into(), suggested).await? else {
        return Ok(None);
    };
    if path.exists() {
        return Err(Error::invalid(
            "A file with that name already exists. Key files are never overwritten, \
             because a vault protected with the old one could no longer be opened",
        ));
    }
    let contents = crypto::new_key_file()?;
    storage::write_atomic(&path, contents.as_bytes())?;
    Ok(Some(views::path_text(&path)))
}

#[tauri::command]
pub async fn change_master_key(app: AppHandle, password: String, key_file: Option<String>) -> Result<AppStatus> {
    let password = Zeroizing::new(password);
    require_password(&password)?;
    let composite = composite_key(&password, key_file.as_deref())?;
    let for_key = composite.clone();
    let key = run_blocking(move || VaultKey::generate(&for_key, KdfParams::calibrated())).await?;

    let state = app.state::<AppState>();
    state.touch();
    let (previous_path, path, outcome) = state.with_vault_mut(|open| {
        let previous_path = open.path().to_path_buf();
        let outcome = open.change_key(composite, key)?;
        Ok((previous_path, open.path().to_path_buf(), outcome))
    })?;
    report_save(&app, &state, &previous_path, &outcome);
    let _ = state.update_settings(|settings| {
        settings.remember_vault(&path, key_file_path(key_file.as_deref()).as_deref())
    });
    Ok(state.status())
}

#[tauri::command]
pub fn get_vault_view(state: State<'_, AppState>) -> Result<VaultView> {
    state.with_vault(|open| Ok(VaultView::new(open.vault(), open.path())))
}

#[tauri::command(async)]
pub fn rename_vault(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<()> {
    modify(&app, &state, |vault| vault.rename(&name))
}

// ----- File dialogs -----

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Vault,
    KeyFile,
    Csv,
    Kdbx,
    Image,
}

impl FileKind {
    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            FileKind::Vault => ("Lados vault", &[storage::FILE_EXTENSION]),
            FileKind::KeyFile => ("Key file", &["keyx", "key"]),
            FileKind::Csv => ("CSV file", &[csv_io::FILE_EXTENSION]),
            FileKind::Kdbx => ("KeePass database", &[keepass_io::FILE_EXTENSION]),
            FileKind::Image => ("Image", &["png", "jpg", "jpeg", "gif", "webp", "ico"]),
        }
    }
}

async fn open_dialog(app: AppHandle, window: WebviewWindow, kind: FileKind, title: String) -> Result<Option<PathBuf>> {
    run_blocking(move || {
        let (name, extensions) = kind.filter();
        let mut dialog = app.dialog().file().set_title(title).set_parent(&window).add_filter(name, extensions);
        if matches!(kind, FileKind::KeyFile) {
            // Any file can act as a key file.
            dialog = dialog.add_filter("All files", &["*"]);
        }
        Ok(dialog.blocking_pick_file().and_then(|path| path.into_path().ok()))
    })
    .await
}

async fn save_dialog(
    app: AppHandle,
    window: WebviewWindow,
    kind: FileKind,
    title: String,
    suggested_name: Option<String>,
) -> Result<Option<PathBuf>> {
    run_blocking(move || {
        let (name, extensions) = kind.filter();
        let mut dialog = app.dialog().file().set_title(title).set_parent(&window).add_filter(name, extensions);
        if let Some(file_name) = suggested_name {
            dialog = dialog.set_file_name(file_name);
        }
        Ok(dialog
            .blocking_save_file()
            .and_then(|path| path.into_path().ok())
            .map(|path| with_extension(path, extensions[0])))
    })
    .await
}

#[tauri::command]
pub async fn pick_open_path(
    app: AppHandle,
    window: WebviewWindow,
    kind: FileKind,
    title: String,
) -> Result<Option<String>> {
    Ok(open_dialog(app, window, kind, title).await?.map(|path| views::path_text(&path)))
}

#[tauri::command]
pub async fn pick_save_path(
    app: AppHandle,
    window: WebviewWindow,
    kind: FileKind,
    title: String,
    suggested_name: Option<String>,
) -> Result<Option<String>> {
    Ok(save_dialog(app, window, kind, title, suggested_name)
        .await?
        .map(|path| views::path_text(&path)))
}

// ----- Entries -----

#[tauri::command]
pub fn search_entries(state: State<'_, AppState>, query: String) -> Result<Vec<Uuid>> {
    state.with_vault(|open| Ok(open.vault().search(&query)))
}

#[tauri::command]
pub fn get_entry(state: State<'_, AppState>, id: Uuid) -> Result<EntryDetails> {
    state.with_vault(|open| Ok(EntryDetails::new(open.vault().entry(id)?, open.vault())))
}

/// The full content, secrets included, for the edit dialog.
#[tauri::command]
pub fn get_entry_for_edit(state: State<'_, AppState>, id: Uuid) -> Result<EntryContent> {
    state.touch();
    state.with_vault(|open| Ok(open.vault().entry(id)?.content.clone()))
}

#[tauri::command(async)]
pub fn create_entry(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_id: Option<Uuid>,
    content: EntryContent,
) -> Result<Uuid> {
    modify(&app, &state, |vault| vault.add_entry(folder_id, content))
}

#[tauri::command(async)]
pub fn update_entry(app: AppHandle, state: State<'_, AppState>, id: Uuid, content: EntryContent) -> Result<()> {
    modify(&app, &state, |vault| vault.update_entry(id, content).map(|_| ()))
}

#[tauri::command(async)]
pub fn duplicate_entry(app: AppHandle, state: State<'_, AppState>, id: Uuid) -> Result<Uuid> {
    modify(&app, &state, |vault| vault.duplicate_entry(id))
}

#[tauri::command(async)]
pub fn move_entry(app: AppHandle, state: State<'_, AppState>, id: Uuid, folder_id: Option<Uuid>) -> Result<()> {
    modify(&app, &state, |vault| vault.move_entry(id, folder_id))
}

#[tauri::command(async)]
pub fn delete_entry(app: AppHandle, state: State<'_, AppState>, id: Uuid) -> Result<()> {
    modify(&app, &state, |vault| vault.delete_entry(id))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", content = "index", rename_all = "camelCase")]
pub enum EntryField {
    Username,
    Password,
    Url,
    Notes,
    Custom(usize),
}

/// The value of a field and whether it is a secret.
fn field_value(content: &EntryContent, field: EntryField) -> Result<(Zeroizing<String>, bool)> {
    let (value, secret) = match field {
        EntryField::Username => (content.username.clone(), false),
        EntryField::Password => (content.password.clone(), true),
        EntryField::Url => (content.url.clone().unwrap_or_default(), false),
        EntryField::Notes => (content.notes.clone().unwrap_or_default(), false),
        EntryField::Custom(index) => {
            let field = content.custom_fields.get(index).ok_or(Error::NotFound("field"))?;
            (field.value.clone(), field.protected)
        }
    };
    Ok((Zeroizing::new(value), secret))
}

#[tauri::command]
pub fn reveal_field(state: State<'_, AppState>, id: Uuid, field: EntryField) -> Result<String> {
    state.touch();
    state.with_vault(|open| {
        let (value, _) = field_value(&open.vault().entry(id)?.content, field)?;
        Ok(value.to_string())
    })
}

/// Secrets are copied so that they stay out of the clipboard history and are cleared later.
#[tauri::command]
pub fn copy_field(app: AppHandle, state: State<'_, AppState>, id: Uuid, field: EntryField) -> Result<()> {
    state.touch();
    let (value, secret) = state.with_vault(|open| field_value(&open.vault().entry(id)?.content, field))?;
    if value.is_empty() {
        return Err(Error::invalid("This field is empty"));
    }
    if secret {
        clipboard::copy_secret(&app, &value)
    } else {
        clipboard::copy_plain(&app, &value)
    }
}

#[tauri::command]
pub fn get_entry_history(state: State<'_, AppState>, id: Uuid) -> Result<Vec<RevisionSummary>> {
    state.with_vault(|open| Ok(views::revisions(open.vault().entry(id)?)))
}

#[tauri::command(async)]
pub fn restore_revision(app: AppHandle, state: State<'_, AppState>, id: Uuid, index: usize) -> Result<()> {
    modify(&app, &state, |vault| vault.restore_revision(id, index))
}

#[tauri::command]
pub fn copy_revision_password(app: AppHandle, state: State<'_, AppState>, id: Uuid, index: usize) -> Result<()> {
    state.touch();
    let password = state.with_vault(|open| {
        let revision = open.vault().entry(id)?.history.get(index).ok_or(Error::NotFound("version"))?;
        Ok(Zeroizing::new(revision.content.password.clone()))
    })?;
    if password.is_empty() {
        return Err(Error::invalid("That version has no password"));
    }
    clipboard::copy_secret(&app, &password)
}

#[tauri::command]
pub fn open_entry_url(app: AppHandle, state: State<'_, AppState>, id: Uuid) -> Result<()> {
    state.touch();
    let url = state.with_vault(|open| {
        open.vault()
            .entry(id)?
            .content
            .url
            .clone()
            .ok_or_else(|| Error::invalid("This entry has no web address"))
    })?;
    app.opener()
        .open_url(web_address(&url)?, None::<&str>)
        .map_err(|error| Error::System(error.to_string()))
}

// ----- Folders and icons -----

#[tauri::command(async)]
pub fn create_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_id: Option<Uuid>,
    name: String,
    icon: Option<Icon>,
) -> Result<Uuid> {
    modify(&app, &state, |vault| vault.add_folder(parent_id, &name, icon))
}

#[tauri::command(async)]
pub fn update_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Uuid,
    name: String,
    icon: Option<Icon>,
) -> Result<()> {
    modify(&app, &state, |vault| vault.update_folder(id, &name, icon))
}

#[tauri::command(async)]
pub fn move_folder(app: AppHandle, state: State<'_, AppState>, id: Uuid, parent_id: Option<Uuid>) -> Result<()> {
    modify(&app, &state, |vault| vault.move_folder(id, parent_id))
}

#[tauri::command(async)]
pub fn delete_folder(app: AppHandle, state: State<'_, AppState>, id: Uuid) -> Result<()> {
    modify(&app, &state, |vault| vault.delete_folder(id))
}

/// Lets the user pick an image and stores it inside the vault. Returns its id, or `None` if cancelled.
#[tauri::command]
pub async fn add_custom_icon(app: AppHandle, window: WebviewWindow) -> Result<Option<Uuid>> {
    let Some(path) = open_dialog(app.clone(), window, FileKind::Image, "Choose an icon".into()).await? else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path)?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(Error::invalid("The image is too large (maximum 512 KB)"));
    }
    let mime = models::image_mime(&bytes).ok_or_else(|| Error::invalid("Choose a PNG, JPEG, GIF, WebP or ICO image"))?;
    let name = views::file_name(&path);
    let data = BASE64.encode(&bytes);
    let state = app.state::<AppState>();
    modify(&app, &state, |vault| Ok(vault.add_custom_icon(&name, mime, data))).map(Some)
}

// ----- Generator and clipboard -----

#[tauri::command]
pub fn generate_password(options: GeneratorOptions) -> Result<String> {
    generator::generate(&options)
}

/// Copies a secret that is not stored yet, such as a freshly generated password.
#[tauri::command]
pub fn copy_secret_text(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<()> {
    state.touch();
    let text = Zeroizing::new(text);
    clipboard::copy_secret(&app, &text)
}

// ----- Import, export and merge -----

#[tauri::command(async)]
pub fn import_csv(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<ImportSummary> {
    let path = PathBuf::from(path);
    let bytes = Zeroizing::new(storage::read_file(&path)?);
    let source = source_name(&path);
    modify(&app, &state, |vault| csv_io::import_csv(vault, &bytes, &source))
}

#[tauri::command(async)]
pub fn import_kdbx(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
    key_file: Option<String>,
) -> Result<ImportSummary> {
    let password = Zeroizing::new(password);
    state.with_vault(|_| Ok(()))?;
    let path = PathBuf::from(path);
    let bytes = Zeroizing::new(storage::read_file(&path)?);
    let key_file = read_key_file(key_file.as_deref())?;
    let database = keepass_io::open_kdbx(&bytes, &password, key_file.as_ref().map(|k| k.as_slice()))?;
    let source = source_name(&path);
    modify(&app, &state, |vault| keepass_io::import_kdbx(vault, &database, &source))
}

/// Writes every secret in clear text: the interface warns the user before calling this.
#[tauri::command(async)]
pub fn export_csv(state: State<'_, AppState>, path: String) -> Result<()> {
    state.touch();
    let bytes = state.with_vault(|open| csv_io::export_csv(open.vault()))?;
    storage::write_atomic(&with_extension(PathBuf::from(path), csv_io::FILE_EXTENSION), &bytes)
}

#[tauri::command(async)]
pub fn export_kdbx(
    state: State<'_, AppState>,
    path: String,
    password: String,
    key_file: Option<String>,
) -> Result<()> {
    state.touch();
    let password = Zeroizing::new(password);
    let key_file = read_key_file(key_file.as_deref())?;
    // A snapshot, so the slow key derivation does not keep the vault busy.
    let snapshot = state.with_vault(|open| Ok(Zeroizing::new(open.vault().clone())))?;
    let bytes = keepass_io::export_kdbx(&snapshot, &password, key_file.as_ref().map(|k| k.as_slice()))?;
    storage::write_atomic(&with_extension(PathBuf::from(path), keepass_io::FILE_EXTENSION), &bytes)
}

/// Merges another Lados vault into the open one. Without credentials, the open vault's own
/// are tried first; the interface asks for others if they fail.
#[tauri::command(async)]
pub fn merge_vault_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
    key_file: Option<String>,
) -> Result<MergeReport> {
    state.touch();
    let path = PathBuf::from(path);
    let (own_path, own_composite) = state.with_vault(|open| Ok((open.path().to_path_buf(), open.composite())))?;
    if same_file(&path, &own_path) {
        return Err(Error::invalid("That is the vault that is already open"));
    }
    let composite = match password {
        Some(password) => composite_key(&Zeroizing::new(password), key_file.as_deref())?,
        None => own_composite,
    };
    let other = session::read_other(&path, &composite)?;
    let (previous_path, report, outcome) = state.with_vault_mut(|open| {
        let previous_path = open.path().to_path_buf();
        let (report, outcome) = open.merge_from(&other)?;
        Ok((previous_path, report, outcome))
    })?;
    report_save(&app, &state, &previous_path, &outcome);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_opened() {
        assert_eq!(web_address("https://example.com").unwrap(), "https://example.com");
        assert_eq!(web_address("example.com/login").unwrap(), "https://example.com/login");
        assert_eq!(web_address("localhost:8080").unwrap(), "https://localhost:8080");
        assert!(web_address("javascript:alert(1)").is_err());
        assert!(web_address("file:///C:/Windows/System32/calc.exe").is_err());
        assert!(web_address("  ").is_err());
    }

    #[test]
    fn extensions_are_added_only_when_missing() {
        assert_eq!(with_extension(PathBuf::from("a/vault"), "lados"), PathBuf::from("a/vault.lados"));
        assert_eq!(with_extension(PathBuf::from("a/vault.LADOS"), "lados"), PathBuf::from("a/vault.LADOS"));
    }
}
