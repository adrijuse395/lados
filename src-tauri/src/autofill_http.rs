//! Localhost autofill bridge for the Chromium sideload extension (desktop only).
//! Listens on 127.0.0.1:18765 — never binds to a public interface.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

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
        respond(&mut stream, 405, r#"{"error":"method not allowed"}"#);
        return;
    }

    // Defence in depth: a web page carries its own http(s) origin, which we refuse.
    // A browser extension's Origin is `moz-extension://…` / `chrome-extension://…`.
    // The extension does not rely on CORS at all (it has host permissions), and we
    // never send `Access-Control-Allow-Origin`, so web pages cannot read responses.
    if let Some(origin) = header_value(&request, "origin") {
        let origin = origin.to_ascii_lowercase();
        if origin.starts_with("http://") || origin.starts_with("https://") {
            respond(&mut stream, 403, r#"{"error":"forbidden"}"#);
            return;
        }
    }

    if method == "OPTIONS" {
        respond(&mut stream, 204, "");
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
            respond(&mut stream, 200, &body);
        }
        "/v1/entries" => {
            let page_url = query_param(query, "url").unwrap_or_default();
            match entries_for(&app, &page_url) {
                Ok(body) => respond(&mut stream, 200, &body),
                Err(code) => respond(&mut stream, code, r#"{"error":"unavailable"}"#),
            }
        }
        "/v1/reveal" => {
            let id = query_param(query, "id").unwrap_or_default();
            match reveal_for(&app, &id) {
                Ok(body) => respond(&mut stream, 200, &body),
                Err(code) => respond(&mut stream, code, r#"{"error":"unavailable"}"#),
            }
        }
        _ => respond(&mut stream, 404, r#"{"error":"not found"}"#),
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
    state.touch();
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

/// Hands over one entry's secrets, and only while the vault is unlocked.
/// The extension calls this at the moment the user chooses to fill.
fn reveal_for(app: &AppHandle, id: &str) -> Result<String, u16> {
    let id = Uuid::parse_str(id).map_err(|_| 404_u16)?;
    let state = app.try_state::<AppState>().ok_or(503_u16)?;
    state.touch();
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

/// Case-insensitive lookup of a single request header.
fn header_value(request: &str, name: &str) -> Option<String> {
    for line in request.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value.trim().to_string());
            }
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

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
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
    // No `Access-Control-Allow-Origin`: web pages cannot read these responses.
    // The browser extension reaches the bridge through its host permission instead.
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if !body.is_empty() {
        let _ = stream.write_all(body.as_bytes());
    }
    let _ = stream.flush();
}
