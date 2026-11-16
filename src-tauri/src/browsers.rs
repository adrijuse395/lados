//! Detects which browsers are installed, so the extension flow can offer the add-on store when
//! the browser is present, or the browser's own download page when it is not. It can also open a
//! link in a *specific* browser, so the Firefox add-on page opens in Firefox even when another
//! browser is the system default.

use std::path::PathBuf;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// Whether each supported browser is installed on this machine.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browsers {
    pub firefox: bool,
    pub chrome: bool,
    pub edge: bool,
}

const FIREFOX_NAMES: &[&str] = &["firefox", "firefox.exe"];
const CHROME_NAMES: &[&str] = &["google-chrome", "google-chrome-stable", "chromium", "chrome", "chrome.exe"];
const EDGE_NAMES: &[&str] = &["microsoft-edge", "microsoft-edge-stable", "msedge", "msedge.exe"];

#[tauri::command]
pub fn detected_browsers() -> Browsers {
    Browsers {
        firefox: executable(FIREFOX_NAMES, firefox_paths()).is_some(),
        chrome: executable(CHROME_NAMES, chrome_paths()).is_some(),
        edge: executable(EDGE_NAMES, edge_paths()).is_some(),
    }
}

/// Opens a web address in a specific browser, falling back to the system default when that
/// browser is not installed. Only the supported browsers and http(s) links are accepted.
#[tauri::command]
pub fn open_in_browser(app: AppHandle, browser: String, url: String) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web addresses can be opened".into());
    }
    let target = match browser.as_str() {
        "firefox" => executable(FIREFOX_NAMES, firefox_paths()),
        "chrome" => executable(CHROME_NAMES, chrome_paths()),
        "edge" => executable(EDGE_NAMES, edge_paths()),
        _ => None,
    };
    if let Some(exe) = target
        && std::process::Command::new(exe).arg(url).spawn().is_ok()
    {
        return Ok(());
    }
    app.opener().open_url(url, None::<&str>).map_err(|error| error.to_string())
}

/// First existing executable named in `names`, under `paths` or anywhere on `PATH`.
fn executable(names: &[&str], paths: Vec<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = paths.into_iter().find(|path| path.is_file()) {
        return Some(path);
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn firefox_paths() -> Vec<PathBuf> {
    windows_paths(&[&["Mozilla Firefox", "firefox.exe"]])
}

#[cfg(target_os = "windows")]
fn chrome_paths() -> Vec<PathBuf> {
    windows_paths(&[
        &["Google", "Chrome", "Application", "chrome.exe"],
        &["Chromium", "Application", "chrome.exe"],
    ])
}

#[cfg(target_os = "windows")]
fn edge_paths() -> Vec<PathBuf> {
    windows_paths(&[&["Microsoft", "Edge", "Application", "msedge.exe"]])
}

/// Joins each sub-path under the usual Windows install roots: Program Files, Program Files (x86)
/// and, for per-user installs, LocalAppData.
#[cfg(target_os = "windows")]
fn windows_paths(relative: &[&[&str]]) -> Vec<PathBuf> {
    let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"];
    let mut paths = Vec::new();
    for root in roots {
        let Some(base) = std::env::var_os(root) else {
            continue;
        };
        for parts in relative {
            let mut path = PathBuf::from(&base);
            for part in *parts {
                path.push(part);
            }
            paths.push(path);
        }
    }
    paths
}

#[cfg(not(target_os = "windows"))]
fn firefox_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Firefox.app/Contents/MacOS/firefox"),
        PathBuf::from("/usr/bin/firefox"),
        PathBuf::from("/usr/local/bin/firefox"),
        PathBuf::from("/snap/bin/firefox"),
    ]
}

#[cfg(not(target_os = "windows"))]
fn chrome_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
    ]
}

#[cfg(not(target_os = "windows"))]
fn edge_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        PathBuf::from("/usr/bin/microsoft-edge"),
    ]
}
