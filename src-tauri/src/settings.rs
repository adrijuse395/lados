//! User preferences and the list of recently opened vaults. Stored as plain JSON in the
//! application's configuration folder: nothing here is secret.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::generator::{GeneratorOptions, MAX_LENGTH, MIN_LENGTH};
use crate::storage;

const FILE_NAME: &str = "settings.json";
const MAX_RECENT_VAULTS: usize = 8;
const MAX_AUTO_LOCK_MINUTES: u32 = 24 * 60;
const MAX_CLIPBOARD_CLEAR_SECONDS: u32 = 10 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// How the browser extension may be handed a password. A program on the same machine can read
/// anything the user can, so the default asks the user in the app: a process cannot click "Allow".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutofillReveal {
    /// Ask in the app on every fill. The default.
    #[default]
    Always,
    /// Ask once per entry, then allow that entry for the rest of the session.
    OncePerSite,
    /// Hand it over without asking. What earlier versions did.
    Never,
}

/// What the user can change in the settings dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub theme: Theme,
    /// Minutes without activity before the vault locks itself. 0 disables it.
    pub auto_lock_minutes: u32,
    /// Seconds before a copied secret is removed from the clipboard. 0 disables it.
    pub clipboard_clear_seconds: u32,
    /// Closing the window keeps Lados running in the system tray.
    pub close_to_tray: bool,
    /// Folder new vaults are created in and downloads land in. `None` = the default
    /// (`Documents/Lados`). The user can point it elsewhere.
    pub vaults_folder: Option<String>,
    /// Whether the browser extension must be confirmed in the app before a password is handed over.
    pub autofill_reveal: AutofillReveal,
    pub generator: GeneratorOptions,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            theme: Theme::System,
            auto_lock_minutes: 5,
            clipboard_clear_seconds: 20,
            close_to_tray: true,
            vaults_folder: None,
            autofill_reveal: AutofillReveal::Always,
            generator: GeneratorOptions::default(),
        }
    }
}

impl Preferences {
    /// Brings out-of-range values back to sensible limits instead of rejecting them.
    pub fn clamped(mut self) -> Self {
        self.auto_lock_minutes = self.auto_lock_minutes.min(MAX_AUTO_LOCK_MINUTES);
        self.clipboard_clear_seconds = self.clipboard_clear_seconds.min(MAX_CLIPBOARD_CLEAR_SECONDS);
        self.generator.length = self.generator.length.clamp(MIN_LENGTH, MAX_LENGTH);
        self.generator.min_lowercase = self.generator.min_lowercase.min(32);
        self.generator.min_uppercase = self.generator.min_uppercase.min(32);
        self.generator.min_digits = self.generator.min_digits.min(32);
        self.generator.min_symbols = self.generator.min_symbols.min(32);
        if self.generator.exclude_characters.len() > 256 {
            self.generator.exclude_characters = self.generator.exclude_characters.chars().take(256).collect();
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentVault {
    pub path: PathBuf,
    /// The key file used last time, so the unlock screen can suggest it.
    pub key_file: Option<PathBuf>,
}

/// A signed-in sync account (user level). Present = the user is "logged in".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncAccount {
    /// Base URL of the sync server (self-hosters may point elsewhere).
    pub server_url: String,
    pub email: String,
    /// Bearer token. Kept local; never written to the vault.
    pub token: String,
    /// When the master password last changed, in Unix milliseconds. Cached from the server so
    /// the interface can remind the user which password to use.
    pub password_changed_at: Option<i64>,
    /// Cached entitlement the server reports: 'comp' | 'trialing' | 'active' | 'past_due' |
    /// 'canceled' | 'expired'. The server stays authoritative; this only renders the interface.
    pub plan: Option<String>,
    /// End of the current paid or trial period, in Unix milliseconds.
    pub plan_valid_until: Option<i64>,
}

/// A vault that is bound to a server vault (vault level).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncedVault {
    pub path: PathBuf,
    /// The vault's id on the server.
    pub server_vault_id: String,
    /// The vault's own id, so a different vault is never merged by mistake.
    pub local_vault_id: String,
    /// Last known vault name, cached so the list can show it while the vault is locked.
    pub name: Option<String>,
    /// Last remote version this device saw.
    pub last_version: Option<u64>,
    /// Last successful sync, in Unix milliseconds.
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub preferences: Preferences,
    /// Most recent first.
    pub recent_vaults: Vec<RecentVault>,
    /// The signed-in sync account, if any.
    pub sync_account: Option<SyncAccount>,
    /// Vaults this user has marked as synced.
    pub synced_vaults: Vec<SyncedVault>,
    /// Key that encrypts vault labels for the server (base64). Local only; not security-critical.
    pub sync_label_key: Option<String>,
    /// A synced vault was recovered while signed out, so its password no longer matches the
    /// account's. The sync space says so instead of failing later with "Sync failed".
    #[serde(default)]
    pub pending_account_password: bool,
    /// Secret the browser extension must present to the autofill bridge. Local only; created on
    /// first use. It lives in this file in clear — anything running as the user can read it, which
    /// is why the *reveal* is confirmed in the app rather than trusted to this value alone.
    pub autofill_token: Option<String>,
}

impl Settings {
    /// Reads the settings, falling back to the defaults if the file is missing or unreadable.
    pub fn load(config_dir: &Path) -> Settings {
        std::fs::read(config_dir.join(FILE_NAME))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .map(|mut settings| {
                settings.preferences = settings.preferences.clamped();
                settings
            })
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        storage::write_atomic(&config_dir.join(FILE_NAME), &bytes)
    }

    pub fn remember_vault(&mut self, path: &Path, key_file: Option<&Path>) {
        self.forget_vault(path);
        self.recent_vaults.insert(
            0,
            RecentVault {
                path: path.to_path_buf(),
                key_file: key_file.map(Path::to_path_buf),
            },
        );
        self.recent_vaults.truncate(MAX_RECENT_VAULTS);
    }

    pub fn forget_vault(&mut self, path: &Path) {
        self.recent_vaults.retain(|recent| recent.path != path);
    }

    pub fn key_file_for(&self, path: &Path) -> Option<PathBuf> {
        self.recent_vaults
            .iter()
            .find(|recent| recent.path == path)
            .and_then(|recent| recent.key_file.clone())
    }

    pub fn synced_vault(&self, path: &Path) -> Option<&SyncedVault> {
        self.synced_vaults.iter().find(|synced| synced.path == path)
    }

    pub fn set_synced_vault(&mut self, synced: SyncedVault) {
        self.synced_vaults.retain(|existing| existing.path != synced.path);
        self.synced_vaults.push(synced);
    }

    pub fn forget_synced_vault(&mut self, path: &Path) {
        self.synced_vaults.retain(|synced| synced.path != path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip_and_bad_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());

        let mut settings = Settings::default();
        settings.preferences.theme = Theme::Dark;
        settings.remember_vault(Path::new("C:/a.lados"), Some(Path::new("C:/a.keyx")));
        settings.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), settings);

        std::fs::write(dir.path().join(FILE_NAME), b"{ not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    #[test]
    fn recent_vaults_are_unique_and_limited() {
        let mut settings = Settings::default();
        for i in 0..12 {
            settings.remember_vault(Path::new(&format!("C:/vault-{i}.lados")), None);
        }
        settings.remember_vault(Path::new("C:/vault-5.lados"), Some(Path::new("C:/k.keyx")));
        assert_eq!(settings.recent_vaults.len(), MAX_RECENT_VAULTS);
        assert_eq!(settings.recent_vaults[0].path, Path::new("C:/vault-5.lados"));
        assert_eq!(
            settings.key_file_for(Path::new("C:/vault-5.lados")),
            Some(PathBuf::from("C:/k.keyx"))
        );
        let fives = settings
            .recent_vaults
            .iter()
            .filter(|r| r.path == Path::new("C:/vault-5.lados"))
            .count();
        assert_eq!(fives, 1);
    }

    #[test]
    fn preferences_are_clamped() {
        let mut preferences = Preferences {
            auto_lock_minutes: 99_999,
            ..Default::default()
        };
        preferences.generator.length = 1;
        let preferences = preferences.clamped();
        assert_eq!(preferences.auto_lock_minutes, MAX_AUTO_LOCK_MINUTES);
        assert_eq!(preferences.generator.length, MIN_LENGTH);
    }
}
