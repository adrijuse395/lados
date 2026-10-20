//! The application's shared state, owned by Tauri and reachable from commands, the tray and
//! the background task.
//!
//! Lock order, to rule out deadlocks: `session` before `settings`, and never the other way.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::clipboard::ClipboardState;
use crate::error::{Error, Result};
use crate::session::OpenVault;
use crate::settings::Settings;
use crate::views::{AppStatus, RecentVaultView, SessionState, file_name, path_text};

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
    /// Starts with the vault passed on the command line (a double-clicked `.lados` file),
    /// or else the most recent one that still exists.
    pub fn new(config_dir: PathBuf, opened_file: Option<PathBuf>) -> Self {
        let settings = Settings::load(&config_dir);
        let selected = opened_file.or_else(|| {
            settings
                .recent_vaults
                .iter()
                .map(|recent| recent.path.clone())
                .find(|path| path.is_file())
        });
        AppState {
            config_dir,
            session: Mutex::new(match selected {
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
            recent_vaults: settings.recent_vaults.iter().map(RecentVaultView::from).collect(),
            app_version: env!("CARGO_PKG_VERSION"),
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
