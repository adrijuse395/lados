//! Opening, creating, locking and re-keying a vault, plus the file dialogs that go with it.

use super::*;

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
    // A signed-in sync user has one master password — the account's — and every synced vault
    // must share it. Re-keying a single vault here would break that link, so it is refused; the
    // account-wide change lives in the Sync settings.
    let state = app.state::<AppState>();
    if state.settings().sync_account.is_some() {
        return Err(Error::invalid(
            "You're signed in to Lados Sync, where your master password is your account password. \
             Change it from Sync settings so every synced vault stays in step.",
        ));
    }

    let password = Zeroizing::new(password);
    require_password(&password)?;
    let composite = composite_key(&password, key_file.as_deref())?;
    let current_key = match &*state.session() {
        Session::Unlocked(open) => Some(open.key().clone()),
        _ => None,
    };
    let for_key = composite.clone();
    let key = run_blocking(move || match current_key {
        Some(current) => current.rewrap(&for_key, KdfParams::calibrated()),
        None => VaultKey::generate(&for_key, KdfParams::calibrated()),
    })
    .await?;

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

    #[cfg(mobile)]
    fn default_file_name(self) -> String {
        match self {
            FileKind::Vault => format!("imported.{}", storage::FILE_EXTENSION),
            FileKind::KeyFile => "key.keyx".into(),
            FileKind::Csv => format!("import.{}", csv_io::FILE_EXTENSION),
            FileKind::Kdbx => format!("import.{}", keepass_io::FILE_EXTENSION),
            FileKind::Image => "icon.png".into(),
        }
    }
}

#[cfg(mobile)]
fn file_name_from_pick(picked: &tauri_plugin_fs::FilePath, kind: FileKind) -> String {
    let fallback = kind.default_file_name();
    match picked {
        tauri_plugin_fs::FilePath::Path(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback),
        tauri_plugin_fs::FilePath::Url(url) => {
            // Android document URIs often end with an encoded name like ...%2FVault.lados
            let raw = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or("");
            let decoded = percent_decode(raw);
            let candidate = decoded
                .rsplit(['/', '\\', ':'])
                .next()
                .unwrap_or(&decoded)
                .trim();
            if candidate.is_empty() || candidate == "document" || candidate.chars().all(|c| c.is_ascii_digit()) {
                fallback
            } else {
                candidate.to_string()
            }
        }
    }
}

#[cfg(mobile)]
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(mobile)]
fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(mobile)]
fn unique_destination(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let extension = path.extension().map(|s| s.to_string_lossy().into_owned());
    for index in 2..10_000 {
        let name = match &extension {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.tmp", Uuid::new_v4()))
}

/// Turns a mobile picker result (often a content:// URI) into a normal file under app storage.
#[cfg(mobile)]
fn materialize_picked_file(
    app: &AppHandle,
    picked: tauri_plugin_fs::FilePath,
    kind: FileKind,
) -> Result<PathBuf> {
    let dest_dir = imports_directory(app, kind)?;
    let mut file_name = file_name_from_pick(&picked, kind);
    if matches!(kind, FileKind::Vault)
        && !file_name
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", storage::FILE_EXTENSION))
    {
        file_name = format!("{file_name}.{}", storage::FILE_EXTENSION);
    }
    let destination = unique_destination(&dest_dir, &file_name);

    if let Ok(path) = picked.clone().into_path()
        && path.is_file()
    {
        if path.starts_with(&dest_dir) {
            return Ok(path);
        }
        std::fs::copy(&path, &destination).map_err(|error| Error::System(error.to_string()))?;
        return Ok(destination);
    }

    // Android: content:// cannot be opened with std::fs — copy via the FS plugin.
    let bytes = tauri_plugin_fs::FsExt::fs(app)
        .read(picked)
        .map_err(|error| {
            Error::System(format!(
                "Could not read the selected file. Try picking it again from Files or Downloads. ({error})"
            ))
        })?;
    if bytes.is_empty() {
        return Err(Error::invalid("The selected file is empty"));
    }
    std::fs::write(&destination, &bytes).map_err(|error| Error::System(error.to_string()))?;
    Ok(destination)
}

/// The file picker. Shared with the entries module, which picks icon images with it.
pub(crate) async fn open_dialog(app: AppHandle, window: WebviewWindow, kind: FileKind, title: String) -> Result<Option<PathBuf>> {
    let start_dir = match kind {
        FileKind::Vault => vaults_directory(&app).ok(),
        _ => None,
    };
    let picked = run_blocking({
        let app = app.clone();
        move || {
            let (name, extensions) = kind.filter();
            let dialog = app.dialog().file().set_title(title);
            #[cfg(desktop)]
            let dialog = dialog.set_parent(&window);
            #[cfg(mobile)]
            let _ = &window;
            let mut dialog = dialog.add_filter(name, extensions);
            // Prefer the standard vaults folder so open/save start in the same place.
            // (On Android the system picker often ignores this; the mobile UI lists that folder first.)
            if let Some(dir) = start_dir {
                dialog = dialog.set_directory(dir);
            }
            // On Android, unknown extensions like `.lados` often have no MIME mapping, so the
            // filtered picker shows nothing usable. Offer a catch-all MIME as well.
            #[cfg(mobile)]
            {
                dialog = dialog.add_filter("All files", &["*/*"]);
            }
            #[cfg(desktop)]
            if matches!(kind, FileKind::KeyFile) {
                // Any file can act as a key file.
                dialog = dialog.add_filter("All files", &["*"]);
            }
            Ok(dialog.blocking_pick_file())
        }
    })
    .await?;

    let Some(picked) = picked else {
        return Ok(None);
    };

    #[cfg(desktop)]
    {
        let path = picked.into_path().map_err(|_| Error::invalid("Could not use the selected file"))?;
        if !path.is_file() {
            return Err(Error::invalid(format!("File not found: {}", path.display())));
        }
        Ok(Some(path))
    }

    #[cfg(mobile)]
    {
        Ok(Some(materialize_picked_file(&app, picked, kind)?))
    }
}

async fn save_dialog(
    app: AppHandle,
    window: WebviewWindow,
    kind: FileKind,
    title: String,
    suggested_name: Option<String>,
) -> Result<Option<PathBuf>> {
    let start_dir = match kind {
        FileKind::Vault => vaults_directory(&app).ok(),
        _ => None,
    };
    run_blocking(move || {
        let (name, extensions) = kind.filter();
        let dialog = app.dialog().file().set_title(title);
        #[cfg(desktop)]
        let dialog = dialog.set_parent(&window);
        #[cfg(mobile)]
        let _ = &window;
        let mut dialog = dialog.add_filter(name, extensions);
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
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

/// Lets the user pick a folder (used when saving a downloaded vault).
#[tauri::command]
pub async fn pick_folder(app: AppHandle, window: WebviewWindow, title: String) -> Result<Option<String>> {
    run_blocking(move || {
        let dialog = app.dialog().file().set_title(title);
        #[cfg(desktop)]
        let dialog = dialog.set_parent(&window);
        #[cfg(mobile)]
        let _ = &window;
        let mut dialog = dialog;
        if let Ok(dir) = vaults_directory(&app) {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog
            .blocking_pick_folder()
            .and_then(|path| path.into_path().ok())
            .map(|path| views::path_text(&path)))
    })
    .await
}

