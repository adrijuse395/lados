//! The application's shared state, owned by Tauri and reachable from commands, the tray and
//! the background task.
//!
//! Lock order, to rule out deadlocks: `session` before `settings`, and never the other way.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::clipboard::ClipboardState;
use crate::error::{Error, Result};
use crate::session::OpenVault;
use crate::settings::Settings;
use crate::views::{
    AppStatus, CloudVaultView, LocalVaultView, RecentVaultView, SessionState, SyncView, file_name, path_text,
};

pub enum Session {
    Closed,
    /// A vault is selected, but its contents are not in memory.
    Locked { path: PathBuf },
    Unlocked(Box<OpenVault>),
}

impl Session {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Session::Closed => None,
            Session::Locked { path } => Some(path),
            Session::Unlocked(open) => Some(open.path()),
        }
    }

    pub fn state(&self) -> SessionState {
        match self {
            Session::Closed => SessionState::Closed,
            Session::Locked { .. } => SessionState::Locked,
            Session::Unlocked(_) => SessionState::Unlocked,
        }
    }

    /// Wipes the decrypted vault from memory and keeps it selected. Returns whether it was unlocked.
    pub fn lock(&mut self) -> bool {
        match self {
            Session::Unlocked(open) => {
                let path = open.path().to_path_buf();
                *self = Session::Locked { path };
                true
            }
            _ => false,
        }
    }
}

/// A poisoned mutex only means another thread panicked while holding it; the data is still
/// usable, and refusing to continue would leave the user locked out of their vault.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct AppState {
    config_dir: PathBuf,
    session: Mutex<Session>,
    settings: Mutex<Settings>,
    last_activity: Mutex<Instant>,
    pub clipboard: ClipboardState,
    quitting: AtomicBool,
}

impl AppState {
    /// Starts on the vault list. A vault is only selected at start-up when it was opened
    /// explicitly (a double-clicked `.lados` file), never silently from the recent list.
    pub fn new(config_dir: PathBuf, opened_file: Option<PathBuf>) -> Self {
        let settings = Settings::load(&config_dir);
        AppState {
            config_dir,
            // Start on the vault list. A recent vault is only selected when it is opened
            // explicitly (a double-clicked `.lados` file), never silently at start-up.
            session: Mutex::new(match opened_file {
                Some(path) => Session::Locked { path },
                None => Session::Closed,
            }),
            settings: Mutex::new(settings),
            last_activity: Mutex::new(Instant::now()),
            clipboard: ClipboardState::default(),
            quitting: AtomicBool::new(false),
        }
    }

    pub fn session(&self) -> MutexGuard<'_, Session> {
        lock(&self.session)
    }

    pub fn settings(&self) -> MutexGuard<'_, Settings> {
        lock(&self.settings)
    }

    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut settings = self.settings();
        change(&mut settings);
        settings.save(&self.config_dir)
    }

    /// Records user activity, which postpones the automatic lock.
    pub fn touch(&self) {
        *lock(&self.last_activity) = Instant::now();
    }

    pub fn idle_time(&self) -> Duration {
        lock(&self.last_activity).elapsed()
    }

    pub fn begin_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> AppStatus {
        let (state, path) = {
            let session = self.session();
            (session.state(), session.path().map(Path::to_path_buf))
        };
        let settings = self.settings();
        AppStatus {
            state,
            vault_path: path.as_deref().map(path_text),
            vault_file_name: path.as_deref().map(file_name),
            key_file: path
                .as_deref()
                .and_then(|path| settings.key_file_for(path))
                .map(|key_file| path_text(&key_file)),
            preferences: settings.preferences.clone(),
            recent_vaults: settings
                .recent_vaults
                .iter()
                .map(|recent| RecentVaultView::new(recent, settings.synced_vault(&recent.path)))
                .collect(),
            app_version: env!("CARGO_PKG_VERSION"),
            sync: {
                let account = settings.sync_account.as_ref();
                let synced = path.as_deref().and_then(|path| settings.synced_vault(path));

                // Group bindings by the vault's own identity, so copies of one vault
                // appear as a single cloud vault with several local files.
                let mut cloud: Vec<CloudVaultView> = Vec::new();
                for entry in &settings.synced_vaults {
                    let identity = if entry.local_vault_id.is_empty() {
                        entry.server_vault_id.clone()
                    } else {
                        entry.local_vault_id.clone()
                    };
                    if let Some(group) = cloud.iter_mut().find(|group| group.local_vault_id == identity) {
                        group.copies.push(path_text(&entry.path));
                        group.present |= entry.path.is_file();
                        group.last_synced_at = group.last_synced_at.max(entry.last_synced_at);
                        // Prefer a real cached vault name over a file-name fallback.
                        if let Some(name) = &entry.name {
                            group.name = name.clone();
                        }
                    } else {
                        let fallback = entry
                            .path
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                            .unwrap_or_else(|| file_name(&entry.path));
                        cloud.push(CloudVaultView {
                            name: entry.name.clone().unwrap_or(fallback),
                            local_vault_id: identity,
                            server_vault_id: entry.server_vault_id.clone(),
                            last_synced_at: entry.last_synced_at,
                            copies: vec![path_text(&entry.path)],
                            present: entry.path.is_file(),
                        });
                    }
                }
                cloud.sort_by_key(|group| group.name.to_lowercase());

                let synced_paths: HashSet<&Path> =
                    settings.synced_vaults.iter().map(|entry| entry.path.as_path()).collect();
                let local: Vec<LocalVaultView> = settings
                    .recent_vaults
                    .iter()
                    .filter(|recent| !synced_paths.contains(recent.path.as_path()))
                    .map(|recent| LocalVaultView {
                        path: path_text(&recent.path),
                        file_name: file_name(&recent.path),
                        key_file: recent.key_file.as_deref().map(path_text),
                    })
                    .collect();

                SyncView {
                    logged_in: account.is_some(),
                    email: account.map(|account| account.email.clone()),
                    server_url: account.map(|account| account.server_url.clone()),
                    this_vault_synced: synced.is_some(),
                    last_synced_at: synced.and_then(|synced| synced.last_synced_at),
                    cloud,
                    local,
                }
            },
        }
    }

    pub fn with_vault<T>(&self, read: impl FnOnce(&OpenVault) -> Result<T>) -> Result<T> {
        match &*self.session() {
            Session::Unlocked(open) => read(open),
            Session::Locked { .. } => Err(Error::Locked),
            Session::Closed => Err(Error::NoVaultSelected),
        }
    }

    pub fn with_vault_mut<T>(&self, change: impl FnOnce(&mut OpenVault) -> Result<T>) -> Result<T> {
        match &mut *self.session() {
            Session::Unlocked(open) => change(open),
            Session::Locked { .. } => Err(Error::Locked),
            Session::Closed => Err(Error::NoVaultSelected),
        }
    }
}
