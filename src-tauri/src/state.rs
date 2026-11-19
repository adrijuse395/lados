//! The application's shared state, owned by Tauri and reachable from commands, the tray and
//! the background task.
//!
//! Lock order, to rule out deadlocks: `session` before `settings`, and never the other way.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::clipboard::ClipboardState;
use crate::crypto;
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

/// Compares two byte strings without leaking where they first differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    difference == 0
}

/// When the running executable was last written, in Unix milliseconds: "which build am I running?"
/// without having to inspect the binary by hand.
fn built_at() -> u64 {
    std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Set at build time to mark a non-release build (`LADOS_CHANNEL=staging`). The interface shows it
/// and skips the updater, so a test build can never be mistaken for — or silently replaced by — a
/// released one.
pub const BUILD_CHANNEL: Option<&str> = option_env!("LADOS_CHANNEL");

/// The sync server this build signs in to, for the interface to show on a marked build.
#[cfg(feature = "sync")]
fn sync_server() -> Option<&'static str> {
    Some(crate::sync::DEFAULT_SERVER)
}

#[cfg(not(feature = "sync"))]
fn sync_server() -> Option<&'static str> {
    None
}

pub struct AppState {
    config_dir: PathBuf,
    session: Mutex<Session>,
    settings: Mutex<Settings>,
    last_activity: Mutex<Instant>,
    pub clipboard: ClipboardState,
    quitting: AtomicBool,
    /// Autofill reveal requests still waiting for the user's answer in the interface.
    pending_reveals: Mutex<HashMap<Uuid, mpsc::Sender<bool>>>,
    /// Entries the user already allowed this session, for the "once per entry" mode.
    allowed_reveals: Mutex<HashSet<(String, String)>>,
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
            pending_reveals: Mutex::new(HashMap::new()),
            allowed_reveals: Mutex::new(HashSet::new()),
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

    // ----- Autofill pairing and consent -----

    /// The secret the browser extension must present to the autofill bridge. Created on first use
    /// and kept locally. It is stored in clear, so a program running as the user can read it — which
    /// is why handing over a *password* is confirmed in the app instead of trusted to this alone.
    pub fn autofill_token(&self) -> Result<String> {
        if let Some(token) = self.settings().autofill_token.clone() {
            return Ok(token);
        }
        let generated = {
            let key = Zeroizing::new(crypto::random_bytes::<32>()?);
            key.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        };
        let mut token = generated.clone();
        self.update_settings(|settings| {
            token = settings.autofill_token.get_or_insert(generated).clone();
        })?;
        Ok(token)
    }

    /// Replaces the pairing secret, so an old code stops working and the extension must pair again.
    pub fn reset_autofill_token(&self) -> Result<String> {
        self.update_settings(|settings| settings.autofill_token = None)?;
        self.autofill_token()
    }

    /// Whether the extension may talk to the bridge at all.
    pub fn autofill_paired(&self, given: Option<&str>) -> bool {
        let Some(expected) = self.settings().autofill_token.clone() else {
            return false;
        };
        match given {
            Some(given) => constant_time_eq(given.as_bytes(), expected.as_bytes()),
            None => false,
        }
    }

    /// Registers a reveal request; the interface answers it through `answer_reveal`.
    pub fn register_reveal(&self) -> (Uuid, mpsc::Receiver<bool>) {
        let (sender, receiver) = mpsc::channel();
        let id = Uuid::new_v4();
        lock(&self.pending_reveals).insert(id, sender);
        (id, receiver)
    }

    /// Answers a pending request. False when it already expired, was answered, or never existed.
    pub fn answer_reveal(&self, id: Uuid, allow: bool) -> bool {
        match lock(&self.pending_reveals).remove(&id) {
            Some(sender) => sender.send(allow).is_ok(),
            None => false,
        }
    }

    /// Drops every pending request and grant, so a locked vault cannot leave a prompt answerable.
    pub fn cancel_reveals(&self) {
        lock(&self.pending_reveals).clear();
        lock(&self.allowed_reveals).clear();
    }

    pub fn allow_reveal_entry(&self, vault_id: &str, entry_id: &str) {
        lock(&self.allowed_reveals).insert((vault_id.to_owned(), entry_id.to_owned()));
    }

    pub fn reveal_entry_allowed(&self, vault_id: &str, entry_id: &str) -> bool {
        lock(&self.allowed_reveals).contains(&(vault_id.to_owned(), entry_id.to_owned()))
    }

    pub fn status(&self) -> AppStatus {
        let (state, path, has_recovery_key) = {
            let session = self.session();
            let has_recovery_key = match &*session {
                Session::Unlocked(open) => open.key().has_recovery(),
                _ => false,
            };
            (session.state(), session.path().map(Path::to_path_buf), has_recovery_key)
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
            api_revision: crate::commands::API_REVISION,
            built_at: built_at(),
            channel: BUILD_CHANNEL,
            sync_server: sync_server(),
            has_recovery_key,
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
                    password_changed_at: account.and_then(|account| account.password_changed_at),
                    plan: account.and_then(|account| account.plan.clone()),
                    plan_valid_until: account.and_then(|account| account.plan_valid_until),
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

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn the_pairing_code_is_compared_without_an_early_exit() {
        assert!(constant_time_eq(b"a1b2c3", b"a1b2c3"));
        assert!(!constant_time_eq(b"a1b2c3", b"a1b2c4"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }
}
