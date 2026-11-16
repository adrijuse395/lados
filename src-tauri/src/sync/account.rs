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
    match created.status {
        401 => Err(Error::SessionExpired),
        _ => created
            .value
            .as_ref()
            .and_then(|value| value.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_string)
            .ok_or_else(|| Error::System(format!("could not create vault ({})", created.status))),
    }
}

/// Replaces the encrypted label of an existing cloud vault. Used when the label is re-sealed
/// (after a master-password change) or when a pre-encryption vault gets its name encrypted.
pub fn set_vault_label(base: &str, token: &str, vault_id: &str, name_cipher: &str) -> Result<()> {
    let url = format!("{}/vaults/{}", base.trim_end_matches('/'), vault_id);
    let body = json!({ "nameCipher": name_cipher });
    let response = http::send(Method::PATCH, &url, Some(token), None, Some(&body), None)?;
    match response.status {
        200 => Ok(()),
        401 => Err(Error::SessionExpired),
        _ => Err(Error::System(format!("could not update the vault label ({})", response.status))),
    }
}

/// The account as the server sees it (never includes anything secret).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub plan: String,
    /// End of the current paid (or trial) period, in Unix milliseconds. `None` if there is none.
    pub plan_valid_until: Option<i64>,
    /// When the master password last changed, in Unix milliseconds. `None` if it never did.
    pub password_changed_at: Option<i64>,
}

/// Reads the account: its plan, the current period end, and when the master password last changed.
pub fn me(base: &str, token: &str) -> Result<AccountInfo> {
    let url = format!("{}/me", base.trim_end_matches('/'));
    let response = http::send(Method::GET, &url, Some(token), None, None, None)?;
    match response.status {
        200 => {
            let value = response.value.unwrap_or(serde_json::Value::Null);
            Ok(AccountInfo {
                plan: value.get("plan").and_then(serde_json::Value::as_str).unwrap_or("active").to_string(),
                plan_valid_until: value.get("planValidUntil").and_then(serde_json::Value::as_i64),
                password_changed_at: value.get("passwordChangedAt").and_then(serde_json::Value::as_i64),
            })
        }
        401 => Err(Error::SessionExpired),
        _ => Err(Error::System(format!("could not read the account ({})", response.status))),
    }
}

/// Starts a subscription checkout and returns the hosted checkout URL to open in a browser.
pub fn billing_checkout(base: &str, token: &str) -> Result<String> {
    let url = format!("{}/billing/checkout", base.trim_end_matches('/'));
    let response = http::send(Method::POST, &url, Some(token), None, None, None)?;
    match response.status {
        200 => response
            .value
            .as_ref()
            .and_then(|value| value.get("url"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::System("the checkout response had no URL".into())),
        401 => Err(Error::SessionExpired),
        409 => {
            let code = response
                .value
                .as_ref()
                .and_then(|value| value.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            Err(Error::invalid(match code {
                "already_subscribed" => "You already have a subscription — resume it to keep it going.",
                "not_billable" => "This account already has free access.",
                _ => "This account cannot be charged.",
            }))
        }
        503 => Err(Error::invalid("Subscriptions are not available yet")),
        _ => Err(Error::System(format!("could not start the checkout ({})", response.status))),
    }
}

/// Cancels the subscription at the end of the paid period (access continues until then).
pub fn billing_cancel(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/billing/cancel", base.trim_end_matches('/'));
    let response = http::send(Method::POST, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        400 => Err(Error::invalid("There is no subscription to cancel")),
        401 => Err(Error::SessionExpired),
        409 => Err(Error::invalid("This subscription already has a pending change — reopen the sync space to see it.")),
        503 => Err(Error::invalid("Subscriptions are not available yet")),
        _ => Err(Error::System(format!("could not cancel the subscription ({})", response.status))),
    }
}

/// Undoes a scheduled cancellation, so the subscription keeps renewing.
pub fn billing_resume(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/billing/resume", base.trim_end_matches('/'));
    let response = http::send(Method::POST, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        400 => Err(Error::invalid("There is no subscription to resume")),
        401 => Err(Error::SessionExpired),
        503 => Err(Error::invalid("Subscriptions are not available yet")),
        _ => Err(Error::System(format!("could not resume the subscription ({})", response.status))),
    }
}

/// Deletes the account and every vault it owns on the server.
pub fn delete_account(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/account", base.trim_end_matches('/'));
    let response = http::send(Method::DELETE, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        401 => Err(Error::SessionExpired),
        502 => Err(Error::invalid(
            "We could not stop your subscription, so your account was not deleted. Please try again in a moment.",
        )),
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
    if response.status == 401 {
        return Err(Error::SessionExpired);
    }
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
        401 => Err(Error::SessionExpired),
        _ => Err(Error::System(format!("could not delete the vault ({})", response.status))),
    }
}

/// Revokes every session of the account except this device's ("sign out everywhere").
pub fn sign_out_everywhere(base: &str, token: &str) -> Result<()> {
    let url = format!("{}/sessions", base.trim_end_matches('/'));
    let response = http::send(Method::DELETE, &url, Some(token), None, None, None)?;
    match response.status {
        200 => Ok(()),
        401 => Err(Error::SessionExpired),
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
