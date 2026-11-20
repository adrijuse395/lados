//! Importing, exporting and merging vault files.

use super::*;

// ----- Import, export and merge -----

#[tauri::command(async)]
pub fn import_csv(app: AppHandle, state: State<'_, AppState>, path: String, folder_id: Option<Uuid>) -> Result<ImportSummary> {
    let path = PathBuf::from(path);
    let bytes = Zeroizing::new(storage::read_file(&path)?);
    let source = source_name(&path);
    modify(&app, &state, |vault| csv_io::import_csv(vault, &bytes, &source, folder_id))
}

#[tauri::command(async)]
pub fn import_kdbx(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
    key_file: Option<String>,
    folder_id: Option<Uuid>,
) -> Result<ImportSummary> {
    let password = Zeroizing::new(password);
    state.with_vault(|_| Ok(()))?;
    let path = PathBuf::from(path);
    let bytes = Zeroizing::new(storage::read_file(&path)?);
    let key_file = read_key_file(key_file.as_deref())?;
    let database = keepass_io::open_kdbx(&bytes, &password, key_file.as_ref().map(|k| k.as_slice()))?;
    let source = source_name(&path);
    modify(&app, &state, |vault| keepass_io::import_kdbx(vault, &database, &source, folder_id))
}

/// Writes every secret in clear text: the interface warns the user before calling this, and asks
/// for the credentials that opened the vault, because this is a full copy of everything (L17).
#[tauri::command(async)]
pub fn export_csv(
    state: State<'_, AppState>,
    path: String,
    password: String,
    key_file: Option<String>,
) -> Result<()> {
    confirm_open_vault(&state, &password, key_file.as_deref())?;
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
