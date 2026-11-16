//! Detects which browsers are installed, so the extension flow can offer the add-on store when
//! the browser is present, or the browser's own download page when it is not.

use std::path::PathBuf;

use serde::Serialize;

/// Whether each supported browser is installed on this machine.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browsers {
    pub firefox: bool,
    pub chrome: bool,
    pub edge: bool,
}

#[tauri::command]
pub fn detected_browsers() -> Browsers {
    Browsers {
        firefox: installed(&["firefox", "firefox.exe"], firefox_paths()),
        chrome: installed(
            &["google-chrome", "google-chrome-stable", "chromium", "chrome", "chrome.exe"],
            chrome_paths(),
        ),
        edge: installed(
            &["microsoft-edge", "microsoft-edge-stable", "msedge", "msedge.exe"],
            edge_paths(),
        ),
    }
}

/// True when an executable named in `names` exists under `paths` or anywhere on `PATH`.
fn installed(names: &[&str], paths: Vec<PathBuf>) -> bool {
    if paths.iter().any(|path| path.is_file()) {
        return true;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| names.iter().any(|name| dir.join(name).is_file()))
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
        PathBuf::from("/Applications/Firefox.app"),
        PathBuf::from("/usr/bin/firefox"),
        PathBuf::from("/usr/local/bin/firefox"),
        PathBuf::from("/snap/bin/firefox"),
    ]
}

#[cfg(not(target_os = "windows"))]
fn chrome_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Google Chrome.app"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
    ]
}

#[cfg(not(target_os = "windows"))]
fn edge_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Microsoft Edge.app"),
        PathBuf::from("/usr/bin/microsoft-edge"),
    ]
}
