//! Entries, folders, icons, the password generator and the clipboard.

use super::*;

// ----- Entries -----

#[tauri::command]
pub fn search_entries(state: State<'_, AppState>, query: String) -> Result<Vec<Uuid>> {
    state.with_vault(|open| Ok(open.vault().search(&query)))
}

/// Entries that match a page URL for autofill, ranked best-first. Requires an unlocked vault.
#[tauri::command]
pub fn find_entries_for_url(state: State<'_, AppState>, url: String) -> Result<Vec<views::UrlMatchHit>> {
    state.with_vault(|open| {
        let vault = open.vault();
        Ok(vault
            .entries_for_url(&url)
            .into_iter()
            .filter_map(|(id, kind)| vault.entry(id).ok().map(|entry| views::UrlMatchHit::new(entry, kind)))
            .collect())
    })
}

/// Full credentials for the Android autofill cache (unlocked session only; never written to disk).
#[tauri::command]
pub fn get_autofill_payload(state: State<'_, AppState>) -> Result<Vec<views::AutofillPayloadEntry>> {
    state.touch();
    state.with_vault(|open| {
        Ok(open
            .vault()
            .entries
            .iter()
            .map(|entry| views::AutofillPayloadEntry {
                id: entry.id,
                title: entry.content.title.clone(),
                username: entry.content.username.clone(),
                password: entry.content.password.clone(),
                url: entry.content.url.clone(),
            })
            .collect())
    })
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

/// Opens a web address in the user's default browser. Used by the interface to
/// send the user to the Firefox add-on page (a webview cannot open external URLs itself).
#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> Result<()> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(Error::invalid("Only web addresses can be opened"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| Error::System(error.to_string()))
}

/// Whether Lados is set to start when the user signs in (desktop only).
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch()
            .is_enabled()
            .map_err(|error| Error::System(error.to_string()))
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Ok(false)
    }
}

/// Enables or disables starting Lados at login (desktop only).
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<()> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        let result = if enabled { manager.enable() } else { manager.disable() };
        result.map_err(|error| Error::System(error.to_string()))
    }
    #[cfg(mobile)]
    {
        let _ = (app, enabled);
        Err(Error::invalid("Starting at login is only available on the desktop app"))
    }
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
    let Some(path) = vault::open_dialog(app.clone(), window, FileKind::Image, "Choose an icon".into()).await? else {
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

