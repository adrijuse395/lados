//! Copying to the system clipboard. Secrets are kept out of the Windows clipboard history and
//! cloud sync, and removed after a while, but only if the user has not copied something else.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroizing;

use crate::crypto::sha256;
use crate::error::{Error, Result};
use crate::state::{AppState, lock};

#[derive(Default)]
pub struct ClipboardState {
    copied: Mutex<Option<CopiedSecret>>,
}

/// The last secret copied, remembered by its hash so it can be recognized without storing it.
struct CopiedSecret {
    fingerprint: [u8; 32],
    clear_at: Option<Instant>,
}

fn clipboard_error(error: tauri_plugin_clipboard_manager::Error) -> Error {
    Error::System(format!("The clipboard is not available ({error})"))
}

pub fn copy_secret(app: &AppHandle, text: &str) -> Result<()> {
    app.clipboard().write_secret(text).map_err(clipboard_error)?;
    let state = app.state::<AppState>();
    let seconds = state.settings().preferences.clipboard_clear_seconds;
    *lock(&state.clipboard.copied) = Some(CopiedSecret {
        fingerprint: sha256(text.as_bytes()),
        clear_at: (seconds > 0).then(|| Instant::now() + Duration::from_secs(seconds.into())),
    });
    Ok(())
}

/// Copies information that is not secret, such as a username or a web address.
pub fn copy_plain(app: &AppHandle, text: &str) -> Result<()> {
    app.clipboard().write_text(text).map_err(clipboard_error)
}

/// Called every second by the background task.
pub fn clear_if_due(app: &AppHandle) {
    let state = app.state::<AppState>();
    let due = lock(&state.clipboard.copied)
        .as_ref()
        .and_then(|copied| copied.clear_at)
        .is_some_and(|clear_at| Instant::now() >= clear_at);
    if due {
        clear_now(app);
    }
}

/// Removes the last copied secret from the clipboard, if it is still there.
pub fn clear_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(copied) = lock(&state.clipboard.copied).take() else {
        return;
    };
    let still_there = app
        .clipboard()
        .read_text()
        .map(|text| sha256(Zeroizing::new(text).as_bytes()) == copied.fingerprint)
        .unwrap_or(false);
    if still_there && app.clipboard().clear().is_ok() {
        let _ = app.emit("clipboard-cleared", ());
    }
}
