//! The account side: the secondary auth secret, and the login/register/vault calls.
//!
//! The vault's **encryption key never leaves the device**. Only the derived auth secret
//! is exchanged, and the server stores a salted hash of it (see `docs/SYNC.md` §4).

use reqwest::Method;
use serde_json::json;

use crate::crypto::{self, SecretKey};
use crate::error::{Error, Result};

use super::http::{self, Response};

/// Default hosted server. Stored per account so self-hosting can point elsewhere.
pub const DEFAULT_SERVER: &str = "https://lados-sync.ladospwm.workers.dev";

/// The secret that authenticates the account: Argon2id over the master credentials,
/// domain-separated from the vault key. Returned as lowercase hex.
pub fn auth_secret(composite: &SecretKey) -> Result<String> {
    crypto::sync_auth_secret(composite)
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

/// Logs in with the derived auth secret. `legacy_secret` is the old derivation, sent only
/// so a pre-hardening account can authenticate once and be upgraded. Fails with invalid
/// credentials if wrong.
pub fn login(base: &str, email: &str, auth_secret: &str, legacy_secret: &str) -> Result<String> {
    let url = format!("{}/auth/login", base.trim_end_matches('/'));
    let body = json!({ "email": email, "authSecret": auth_secret, "legacyAuthSecret": legacy_secret });
    let response = http::send(Method::POST, &url, None, None, Some(&body), None)?;
    match response.status {
        200 => token_from(response),
        401 => Err(Error::InvalidCredentials),
        _ => Err(Error::System(format!("login failed ({})", response.status))),
    }
}

/// Returns a vault id for the account, creating one called `name` if none exists.
pub fn ensure_vault(base: &str, token: &str, name: &str) -> Result<String> {
    let root = base.trim_end_matches('/');

    let list = http::send(Method::GET, &format!("{root}/vaults"), Some(token), None, None, None)?;
    if list.status != 200 {
        return Err(Error::System(format!("could not list vaults ({})", list.status)));
    }
    if let Some(first) = list
        .value
        .as_ref()
        .and_then(|value| value.get("vaults"))
        .and_then(|value| value.as_array())
        .and_then(|vaults| vaults.first())
        .and_then(|vault| vault.get("id"))
        .and_then(|id| id.as_str())
    {
        return Ok(first.to_string());
    }

    let body = json!({ "name": name });
    let created = http::send(Method::POST, &format!("{root}/vaults"), Some(token), None, Some(&body), None)?;
    created
        .value
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::System(format!("could not create vault ({})", created.status)))
}
