//! The account side: the secondary auth secret, and the login/register/vault calls.
//!
//! The vault's **encryption key never leaves the device**. Only the derived auth secret
//! is exchanged, and the server stores a salted hash of it (see `docs/SYNC.md` §4).

use reqwest::Method;
use serde::Serialize;
use serde_json::json;

use crate::crypto::{self, SecretKey};
use crate::error::{Error, Result};

use super::http::{self, Response};

/// Default hosted server. Stored per account so self-hosting can point elsewhere.
pub const DEFAULT_SERVER: &str = "https://lados-sync.ladospwm.workers.dev";

/// The secret that authenticates the account: Argon2id over the master credentials, salted
/// per account (email), domain-separated from the vault key. Returned as lowercase hex.
pub fn auth_secret(composite: &SecretKey, account: &str) -> Result<String> {
    crypto::sync_auth_secret(composite, account)
}

/// The previous auth secret (one global salt). Sent on login only, so accounts created
/// before the per-account salt can authenticate once and be upgraded.
pub fn previous_auth_secret(composite: &SecretKey) -> Result<String> {
    crypto::previous_auth_secret(composite)
}

/// The pre-hardening derivation (a single fast hash of the composite key). Kept only so
/// accounts created before the Argon2 verifier can sign in once and be upgraded.
pub fn legacy_auth_secret(composite: &SecretKey) -> String {
    let mut buffer = Vec::with_capacity(19 + 32);
    buffer.extend_from_slice(b"lados-sync-auth-v1");
    buffer.extend_from_slice(&**composite);
    crypto::sha256(&buffer)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn token_from(response: Response) -> Result<String> {
    response
        .value
        .as_ref()
        .and_then(|value| value.get("token"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::System(format!("unexpected auth response ({})", response.status)))
}

/// Registers a new account. Fails if the email is already taken.
pub fn register(base: &str, email: &str, auth_secret: &str) -> Result<String> {
    let url = format!("{}/auth/register", base.trim_end_matches('/'));
    let body = json!({ "email": email, "authSecret": auth_secret });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => token_from(response),
        409 => Err(Error::invalid("That email is already registered")),
        _ => Err(Error::System(format!("register failed ({})", response.status))),
    }
}

/// Logs in with the derived auth secret. `previous_secret` and `legacy_secret` are the
/// older derivations, sent only so a pre-existing account can authenticate once and be
/// upgraded. Fails with invalid credentials if wrong.
pub fn login(base: &str, email: &str, auth_secret: &str, previous_secret: &str, legacy_secret: &str) -> Result<String> {
    let url = format!("{}/auth/login", base.trim_end_matches('/'));
    let body = json!({
        "email": email,
        "authSecret": auth_secret,
        "previousAuthSecret": previous_secret,
        "legacyAuthSecret": legacy_secret,
    });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => token_from(response),
        401 => Err(Error::InvalidCredentials),
        _ => Err(Error::System(format!("login failed ({})", response.status))),
    }
}

/// Returns the server vault for this vault identity, creating it if the account has none.
/// The label is sent only as ciphertext, so the server never learns the vault's name.
pub fn ensure_vault(base: &str, token: &str, vault_uuid: &str, name_cipher: &str) -> Result<String> {
    let root = base.trim_end_matches('/');
    let body = json!({ "vaultUuid": vault_uuid, "nameCipher": name_cipher });
    let created = http::send(Method::POST, &format!("{root}/vaults"), Some(token), None, Some(&body), None)?;
    created
        .value
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::System(format!("could not create vault ({})", created.status)))
}

/// Deletes the account and every vault it owns on the server.
pub fn delete_account(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/account", base.trim_end_matches('/'));
    let response = http::send(Method::DELETE, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        _ => Err(Error::System(format!("could not delete the account ({})", response.status))),
    }
}

/// A cloud vault as listed by the server (metadata only, no contents).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVault {
    pub id: String,
    pub name: String,
    pub name_cipher: Option<String>,
    pub vault_uuid: Option<String>,
    pub version: u64,
    pub updated_at: i64,
}

/// Lists the signed-in account's cloud vaults.
pub fn list_vaults(base: &str, token: &str) -> Result<Vec<CloudVault>> {
    let url = format!("{}/vaults", base.trim_end_matches('/'));
    let response = http::send(Method::GET, &url, Some(token), None, None, None)?;
    if response.status != 200 {
        return Err(Error::System(format!("could not list vaults ({})", response.status)));
    }
    let vaults = response
        .value
        .as_ref()
        .and_then(|value| value.get("vaults"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(vaults
        .iter()
        .map(|vault| CloudVault {
            id: vault.get("id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
            name: vault.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
            name_cipher: vault.get("name_cipher").and_then(serde_json::Value::as_str).map(str::to_string),
            vault_uuid: vault.get("vault_uuid").and_then(serde_json::Value::as_str).map(str::to_string),
            version: vault.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0),
            updated_at: vault.get("updated_at").and_then(serde_json::Value::as_i64).unwrap_or(0),
        })
        .collect())
}

/// Deletes a cloud vault (and its stored blobs) from the server.
pub fn delete_vault(base: &str, token: &str, server_vault_id: &str) -> Result<()> {
    let url = format!("{}/vaults/{}", base.trim_end_matches('/'), server_vault_id);
    let response = http::send(Method::DELETE, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        _ => Err(Error::System(format!("could not delete the vault ({})", response.status))),
    }
}

/// Revokes every session of the account except this device's ("sign out everywhere").
pub fn sign_out_everywhere(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/sessions", base.trim_end_matches('/'));
    let response = http::send(Method::DELETE, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        _ => Err(Error::System(format!("could not sign out other devices ({})", response.status))),
    }
}

fn string_at(response: &Response, key: &str) -> Result<String> {
    response
        .value
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::System(format!("unexpected auth response ({})", response.status)))
}

// ----- OPAQUE auth -----

pub fn opaque_register_start(base: &str, email: &str, registration_request: &str) -> Result<String> {
    let url = format!("{}/auth/register/start", base.trim_end_matches('/'));
    let body = json!({ "email": email, "registrationRequest": registration_request });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => string_at(&response, "registrationResponse"),
        409 => Err(Error::invalid("That email is already registered")),
        _ => Err(Error::System(format!("register failed ({})", response.status))),
    }
}

pub fn opaque_register_finish(base: &str, email: &str, registration_record: &str) -> Result<String> {
    let url = format!("{}/auth/register/finish", base.trim_end_matches('/'));
    let body = json!({ "email": email, "registrationRecord": registration_record });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => token_from(response),
        409 => Err(Error::invalid("That email is already registered")),
        _ => Err(Error::System(format!("register failed ({})", response.status))),
    }
}

/// Returns `(loginId, loginResponse)`.
pub fn opaque_login_start(base: &str, email: &str, start_login_request: &str) -> Result<(String, String)> {
    let url = format!("{}/auth/login/start", base.trim_end_matches('/'));
    let body = json!({ "email": email, "startLoginRequest": start_login_request });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    if response.status != 200 {
        return Err(Error::System(format!("login failed ({})", response.status)));
    }
    Ok((string_at(&response, "loginId")?, string_at(&response, "loginResponse")?))
}

pub fn opaque_login_finish(base: &str, login_id: &str, finish_login_request: &str) -> Result<String> {
    let url = format!("{}/auth/login/finish", base.trim_end_matches('/'));
    let body = json!({ "loginId": login_id, "finishLoginRequest": finish_login_request });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => token_from(response),
        401 => Err(Error::InvalidCredentials),
        _ => Err(Error::System(format!("login failed ({})", response.status))),
    }
}

pub fn opaque_upgrade_start(base: &str, token: &str, registration_request: &str) -> Result<String> {
    let url = format!("{}/auth/upgrade/start", base.trim_end_matches('/'));
    let body = json!({ "registrationRequest": registration_request });
    let response = http::send(Method::POST, &url, Some(token), None, Some(&body), None)?;
    if response.status != 200 {
        return Err(Error::System(format!("upgrade failed ({})", response.status)));
    }
    string_at(&response, "registrationResponse")
}

pub fn opaque_upgrade_finish(base: &str, token: &str, registration_record: &str) -> Result<()> {
    let url = format!("{}/auth/upgrade/finish", base.trim_end_matches('/'));
    let body = json!({ "registrationRecord": registration_record });
    let response = http::send(Method::POST, &url, Some(token), None, Some(&body), None)?;
    match response.status {
        200 => Ok(()),
        _ => Err(Error::System(format!("upgrade failed ({})", response.status))),
    }
}
