//! Pairing and consent for the browser autofill bridge.
//!
//! Two things keep the loopback bridge from being a way to read passwords:
//!
//! 1. **Pairing** — the extension must present a secret this app generated (`autofill_token`). It
//!    stops web pages, other extensions and scripts that merely knock on the port.
//! 2. **Consent** — handing over a *password* is confirmed by the user in the app, because a process
//!    running as the user can read the secret above; it cannot click "Allow".

use super::*;

/// The pairing code the browser extension is given once. Created on first use, and shown in
/// Settings. Pass `reset` to replace it — every paired extension must then be paired again.
#[tauri::command]
pub fn autofill_pairing_token(state: State<'_, AppState>, reset: Option<bool>) -> Result<String> {
    if reset.unwrap_or(false) {
        return state.reset_autofill_token();
    }
    state.autofill_token()
}

/// Answers the reveal request the user was shown. Fails when it already expired or was answered.
#[tauri::command]
pub fn autofill_answer(state: State<'_, AppState>, id: String, allow: bool) -> Result<()> {
    let id = Uuid::parse_str(&id).map_err(|_| Error::invalid("Unknown autofill request"))?;
    if !state.answer_reveal(id, allow) {
        return Err(Error::invalid("That autofill request has already expired"));
    }
    Ok(())
}
