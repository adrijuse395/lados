//! Localhost autofill bridge for the browser extension (desktop only).
//! Listens on 127.0.0.1:18765 — never binds to a public interface.
//!
//! Two gates, because the caller is a *browser*:
//! - **Pairing** — the extension must present a secret this app generated. It keeps web pages,
//!   other extensions and scripts that merely knock on the port out.
//! - **Consent** — a password is handed over only after the user allows it *in the app*. A local
//!   program can read the secret above (it is on disk); it cannot click "Allow".

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::settings::AutofillReveal;
use crate::state::{AppState, Session};
use crate::views::UrlMatchHit;

pub const PORT: u16 = 18765;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    state: &'static str,
    bridge: &'static str,
}

/// A candidate for the picker. Deliberately carries **no password**.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryBody {
    id: String,
    title: String,
    username: String,
    url: Option<String>,
    match_kind: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntriesBody {
    entries: Vec<EntryBody>,
}

/// The secrets, handed over one entry at a time and only when the user fills.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevealBody {
    username: String,
    password: String,
}

/// Starts the loopback HTTP bridge in a background thread.
pub fn start(app: AppHandle) {
    thread::Builder::new()
        .name("lados-autofill-http".into())
        .spawn(move || {
            let bind = format!("127.0.0.1:{PORT}");
            let listener = match TcpListener::bind(&bind) {
                Ok(listener) => listener,
                Err(error) => {
                    eprintln!("Lados autofill bridge could not bind {bind}: {error}");
                    return;
                }
            };
            let _ = listener.set_nonblocking(false);
            let app = Arc::new(app);
            eprintln!("Lados autofill bridge listening on {bind}");
            for stream in listener.incoming().flatten() {
                let app = Arc::clone(&app);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                thread::spawn(move || handle_client(app, stream));
            }
        })
        .ok();
}

fn handle_client(app: Arc<AppHandle>, mut stream: TcpStream) {
    let mut buf = [0_u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    if method != "GET" && method != "OPTIONS" {
        respond(&mut stream, 405, r#"{"error":"method not allowed"}"#, None);
        return;
    }

    // **Only a browser extension may call the bridge.** A web page carries its own `http(s)://`
    // origin, a sandboxed frame carries the literal `null`, and a request with no `Origin` at all
    // is neither — all three are refused. What this cannot stop is a *local process*, which can
    // forge any header; that is what the per-install pairing secret is for.
    let origin = match header_value(&request, "origin") {
        Some(origin) if allowed_origin(&origin) => origin,
        _ => {
            respond(&mut stream, 403, r#"{"error":"forbidden"}"#, None);
            return;
        }
    };

    // DNS rebinding: the `Host` is the name the caller used. A rebinding page reaches this socket
    // with an attacker-controlled Host, so only our own loopback host is accepted.
    if !host_is_loopback(&request) {
        respond(&mut stream, 403, r#"{"error":"forbidden"}"#, None);
        return;
    }

    // ...and it must know the pairing code. A web page, another extension or a script that merely
    // knocks on the port is refused here. A local program that *reads* the code (it is stored as
    // plain text) still cannot obtain a password: that is confirmed by the user — see `reveal_for`.
    let paired = app.try_state::<AppState>().is_some_and(|state| {
        state.autofill_paired(header_value(&request, "x-lados-token").as_deref())
    });
    if !paired {
        respond(&mut stream, 403, r#"{"error":"not paired"}"#, Some(origin.as_str()));
        return;
    }

    let cors = Some(origin.as_str());

    if method == "OPTIONS" {
        respond(&mut stream, 204, "", cors);
        return;
    }

    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    match route {
        "/v1/status" => {
            let state = status_label(&app);
            let body = serde_json::to_string(&StatusBody {
                state,
                bridge: "lados-autofill",
            })
            .unwrap_or_else(|_| r#"{"state":"unknown"}"#.into());
            respond(&mut stream, 200, &body, cors);
        }
        "/v1/entries" => {
            let page_url = query_param(query, "url").unwrap_or_default();
            match entries_for(&app, &page_url) {
                Ok(body) => respond(&mut stream, 200, &body, cors),
                Err(code) => respond(&mut stream, code, r#"{"error":"unavailable"}"#, cors),
            }
        }
        "/v1/reveal" => {
            let id = query_param(query, "id").unwrap_or_default();
            match reveal_for(&app, &id) {
                Ok(body) => respond(&mut stream, 200, &body, cors),
                Err(code) => respond(&mut stream, code, r#"{"error":"unavailable"}"#, cors),
            }
        }
        _ => respond(&mut stream, 404, r#"{"error":"not found"}"#, cors),
    }
}

fn status_label(app: &AppHandle) -> &'static str {
    match app.try_state::<AppState>() {
        Some(state) => match &*state.session() {
            Session::Closed => "closed",
            Session::Locked { .. } => "locked",
            Session::Unlocked(_) => "unlocked",
        },
        None => "no-state",
    }
}

fn entries_for(app: &AppHandle, page_url: &str) -> Result<String, u16> {
    let state = app.try_state::<AppState>().ok_or(503_u16)?;
    let payload: Vec<EntryBody> = state
        .with_vault(|open| {
            let vault = open.vault();
            Ok(vault
                .entries_for_url(page_url)
                .into_iter()
                .filter_map(|(id, kind)| {
                    let entry = vault.entry(id).ok()?;
                    let hit = UrlMatchHit::new(entry, kind);
                    Some(EntryBody {
                        id: entry.id.to_string(),
                        title: entry.content.title.clone(),
                        username: entry.content.username.clone(),
                        url: entry.content.url.clone(),
                        match_kind: hit.match_kind,
                    })
                })
                .collect())
        })
        .map_err(|_| 401_u16)?;

    serde_json::to_string(&EntriesBody { entries: payload }).map_err(|_| 500_u16)
}

/// Hands over one entry's secrets, and only while the vault is unlocked. Depending on the
/// preference the user is asked in the app first: a program on this machine can read the pairing
/// code, but it cannot click "Allow".
fn reveal_for(app: &AppHandle, id: &str) -> Result<String, u16> {
    let id = Uuid::parse_str(id).map_err(|_| 404_u16)?;
    let state = app.try_state::<AppState>().ok_or(503_u16)?;

    // What is being asked for, so the prompt can name it. Carries no secret.
    let (vault_id, title, username, url) = state
        .with_vault(|open| {
            let entry = open.vault().entry(id)?;
            Ok((
                open.vault().id.to_string(),
                entry.content.title.clone(),
                entry.content.username.clone(),
                entry.content.url.clone(),
            ))
        })
        .map_err(|_| 401_u16)?;

    let entry_id = id.to_string();
    let mode = state.settings().preferences.autofill_reveal;
    let already_allowed = mode == AutofillReveal::OncePerSite && state.reveal_entry_allowed(&vault_id, &entry_id);

    if mode != AutofillReveal::Never && !already_allowed {
        let (request_id, answer) = state.register_reveal();
        let _ = app.emit(
            "autofill-reveal",
            serde_json::json!({
                "id": request_id.to_string(),
                "title": title,
                "username": username,
                "url": url,
            }),
        );
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        match answer.recv_timeout(Duration::from_secs(30)) {
            Ok(true) => {
                if mode == AutofillReveal::OncePerSite {
                    state.allow_reveal_entry(&vault_id, &entry_id);
                }
            }
            // Denied, unanswered, or the vault locked while the prompt was up.
            _ => return Err(403_u16),
        }
    }

    // Read it now, after the decision: the vault may have locked or changed in the meantime.
    let body = state
        .with_vault(|open| {
            let entry = open.vault().entry(id)?;
            Ok(RevealBody {
                username: entry.content.username.clone(),
                password: entry.content.password.clone(),
            })
        })
        .map_err(|_| 401_u16)?;

    serde_json::to_string(&body).map_err(|_| 500_u16)
}

/// Which origins may call the bridge: a browser extension, and nothing else. A web page carries an
/// `http(s)://` origin and a sandboxed frame carries the literal `null`, so neither ever matches.
fn allowed_origin(origin: &str) -> bool {
    let lower = origin.trim().to_ascii_lowercase();
    lower.starts_with("chrome-extension://") || lower.starts_with("moz-extension://")
}

/// Whether the request was addressed to loopback by name and not to an attacker's hostname that
/// resolves here (DNS rebinding).
fn host_is_loopback(request: &str) -> bool {
    let Some(host) = header_value(request, "host") else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    host == format!("127.0.0.1:{PORT}")
        || host == format!("localhost:{PORT}")
        || host == format!("[::1]:{PORT}")
}

/// Case-insensitive lookup of a single request header.
fn header_value(request: &str, name: &str) -> Option<String> {
    for line in request.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case(name)
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        if k != key {
            continue;
        }
        let v = parts.next().unwrap_or("");
        return Some(percent_decode(v));
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &input[i + 1..i + 3];
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(stream: &mut TcpStream, status: u16, body: &str, allow_origin: Option<&str>) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n",
        body.len()
    );
    // Echo only the calling extension's own origin (never a wildcard, never a web page's), plus
    // the Private Network Access grant that some browsers require for loopback requests.
    if let Some(origin) = allow_origin {
        header.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\n\
             Access-Control-Allow-Methods: GET, OPTIONS\r\n\
             Access-Control-Allow-Headers: *\r\n\
             Access-Control-Allow-Private-Network: true\r\n\
             Vary: Origin\r\n"
        ));
    }
    header.push_str("\r\n");
    let _ = stream.write_all(header.as_bytes());
    if !body.is_empty() {
        let _ = stream.write_all(body.as_bytes());
    }
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_browser_extension_may_call_the_bridge() {
        assert!(allowed_origin("chrome-extension://abcdefghijklmnop"));
        assert!(allowed_origin("moz-extension://0f1e2d3c-4b5a"));
        assert!(allowed_origin("CHROME-EXTENSION://abc"));
        // A web page, a sandboxed frame (`null`), a file:// page and a bare origin: all refused.
        assert!(!allowed_origin("https://example.com"));
        assert!(!allowed_origin("http://127.0.0.1:8099"));
        assert!(!allowed_origin("null"));
        assert!(!allowed_origin("file://"));
        assert!(!allowed_origin(""));
    }

    #[test]
    fn the_request_must_be_addressed_to_loopback() {
        assert!(host_is_loopback("GET / HTTP/1.1\r\nHost: 127.0.0.1:18765\r\n\r\n"));
        assert!(host_is_loopback("GET / HTTP/1.1\r\nhost: localhost:18765\r\n\r\n"));
        // DNS rebinding arrives with the attacker's own hostname.
        assert!(!host_is_loopback("GET / HTTP/1.1\r\nHost: evil.example.com\r\n\r\n"));
        assert!(!host_is_loopback("GET / HTTP/1.1\r\n\r\n"));
    }
}
