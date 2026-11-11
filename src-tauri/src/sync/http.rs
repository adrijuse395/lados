//! HTTP transport for Lados Sync, plus a [`Remote`] implementation.
//!
//! Uses blocking `reqwest`; the caller runs it off the UI thread (see the commands).

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::Method;
use reqwest::blocking::Client;
use serde_json::Value;

use crate::error::{Error, Result};

use super::remote::{PutOutcome, Remote, RemoteVault};

/// One HTTP exchange, decoded enough for callers to react to the status.
pub struct Response {
    pub status: u16,
    pub value: Option<Value>,
}

/// Sends one request. `if_match` maps to the `If-Match` header (0 when unknown),
/// `json` sends a JSON body, `body` sends raw bytes.
pub fn send(
    method: Method,
    url: &str,
    bearer: Option<&str>,
    if_match: Option<u64>,
    json: Option<&Value>,
    body: Option<&[u8]>,
) -> Result<Response> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| Error::System(error.to_string()))?;

    let mut request = client.request(method, url);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    if let Some(version) = if_match {
        request = request.header("If-Match", version.to_string());
    }
    if let Some(value) = json {
        request = request.json(value);
    }
    if let Some(bytes) = body {
        request = request
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec());
    }

    let response = request.send().map_err(|error| Error::System(error.to_string()))?;
    let status = response.status().as_u16();
    let bytes = response.bytes().map_err(|error| Error::System(error.to_string()))?;
    let value = serde_json::from_slice(&bytes).ok();
    Ok(Response { status, value })
}

/// The sync backend for the vault engine: talks to one vault on the server.
pub struct HttpRemote {
    base: String,
    token: String,
}

impl HttpRemote {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        HttpRemote {
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }
}

impl Remote for HttpRemote {
    fn get(&self, vault_id: &str) -> Result<Option<RemoteVault>> {
        let url = format!("{}/vaults/{}", self.base, vault_id);
        let response = send(Method::GET, &url, Some(&self.token), None, None, None)?;
        match response.status {
            200 => {
                let value = response.value.ok_or_else(|| Error::invalid("unexpected response"))?;
                let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
                if version == 0 {
                    return Ok(None);
                }
                let blob = value.get("blob").and_then(Value::as_str).unwrap_or("");
                let bytes = BASE64.decode(blob).map_err(|_| Error::Corrupted)?;
                Ok(Some(RemoteVault { version, bytes }))
            }
            404 => Ok(None),
            401 => Err(Error::Locked),
            _ => Err(Error::System(format!("sync get failed ({})", response.status))),
        }
    }

    fn put(&self, vault_id: &str, base: Option<u64>, bytes: &[u8]) -> Result<PutOutcome> {
        let url = format!("{}/vaults/{}", self.base, vault_id);
        let response = send(Method::PUT, &url, Some(&self.token), Some(base.unwrap_or(0)), None, Some(bytes))?;
        match response.status {
            200 => {
                let version = response
                    .value
                    .as_ref()
                    .and_then(|value| value.get("version"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                Ok(PutOutcome::Stored { version })
            }
            409 => match self.get(vault_id)? {
                Some(current) => Ok(PutOutcome::Conflict { current }),
                None => Err(Error::System("sync conflict without a current copy".into())),
            },
            401 => Err(Error::Locked),
            402 => Err(Error::invalid("Your sync subscription is not active")),
            _ => Err(Error::System(format!("sync put failed ({})", response.status))),
        }
    }
}
