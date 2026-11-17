//! The recovery key: creating one for the open vault, and opening a vault with one.
//!
//! The server and the interface never see the code; it exists only in the file the user saves.
//! See [`crate::recovery`] for the code itself and [`crate::storage`] for the vault format.

use super::*;

use crate::recovery;

/// Creates a recovery key for the open vault and writes the file the user keeps, returning its
/// path. The vault is re-encrypted so the code can open it; this replaces any earlier one.
#[tauri::command]
pub async fn create_recovery_key(app: AppHandle, path: String) -> Result<String> {
    // Refuse an existing file *before* touching the vault: re-keying it and then failing to hand
    // out the code would leave a recovery wrap whose key nobody has.
    let path = PathBuf::from(path);
    if path.exists() {
        return Err(Error::invalid(
            "A file with that name already exists. Choose another name, so no earlier recovery file is overwritten",
        ));
    }

    let state = app.state::<AppState>();
    let (text, code) = recovery::generate()?;
    let current = state.with_vault(|open| Ok(open.key().clone()))?;
    let composite = state.with_vault(|open| Ok(open.composite()))?;
    let vault_name = state.with_vault(|open| Ok(views::file_name(open.path())))?;

    // Adding the wrap changes the secret the vault is saved with, so it is written in one step.
    let for_key = composite.clone();
    let key = run_blocking(move || current.with_recovery(&for_key, &code)).await?;
    state.with_vault_mut(|open| open.change_key(composite, key))?;

    storage::write_atomic(&path, recovery::file_contents(&vault_name, &text).as_bytes())?;
    Ok(views::path_text(&path))
}

/// Opens the selected vault with a recovery key instead of the master password, then re-encrypts
/// it under a new master password. The same recovery key keeps working afterwards.
#[tauri::command]
pub async fn recover_vault(
    app: AppHandle,
    recovery_file: String,
    new_password: String,
    new_key_file: Option<String>,
) -> Result<AppStatus> {
    let password = Zeroizing::new(new_password);
    require_password(&password)?;
    let new_composite = composite_key(&password, new_key_file.as_deref())?;

    let text = Zeroizing::new(String::from_utf8_lossy(&storage::read_file(Path::new(&recovery_file))?).into_owned());
    let code = recovery::parse_code(&text).ok_or_else(|| Error::invalid("That file does not hold a Lados recovery key"))?;

    let path = locked_path(&app)?;
    let target = path.clone();
    let composite = new_composite.clone();
    let open = run_blocking(move || {
        let bytes = storage::read_file(&target)?;
        let header = storage::Header::parse(&bytes)?;
        // A vault with no recovery wrap is not damaged — it simply has no recovery key: the key may
        // have been made for a different vault, or for a copy from before the key existed. Saying so
        // is far more useful than "the vault file is damaged".
        if !header.has_recovery() {
            return Err(Error::invalid(
                "This vault has no recovery key. The file you chose was made for a different vault, or for a \
                 copy of this one from before the key existed.",
            ));
        }
        let recovered = storage::key_from_recovery(&header, &bytes, &code)?;
        let vault = storage::decrypt_vault(&bytes, &recovered)?;
        // Save it under the new master password, keeping the recovery wrap so the same file opens
        // the vault again next time.
        let key = recovered.rewrap(&composite, KdfParams::calibrated())?;
        storage::backup(&target)?;
        storage::write_atomic(&target, &storage::encrypt_vault(&vault, &key)?)?;
        OpenVault::open(target, composite)
    })
    .await
    .map_err(|error| match error {
        Error::InvalidCredentials => Error::invalid("That recovery key does not open this vault"),
        other => other,
    })?;

    let state = app.state::<AppState>();
    {
        let mut session = state.session();
        // The user may have picked another vault while this one was being recovered.
        if !matches!(&*session, Session::Locked { path: selected } if *selected == path) {
            return Err(Error::invalid("Another vault was selected in the meantime"));
        }
        *session = Session::Unlocked(Box::new(open));
    }
    let _ = state.update_settings(|settings| {
        settings.remember_vault(&path, key_file_path(new_key_file.as_deref()).as_deref())
    });
    state.touch();
    Ok(state.status())
}
