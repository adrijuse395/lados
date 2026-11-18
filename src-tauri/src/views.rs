//! The shapes of the data sent to the interface.
//!
//! None of them carries a password or a protected field: secrets travel one at a time,
//! only when the user explicitly reveals, copies or edits them.

use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::{CustomIcon, Entry, Folder, Icon, UrlMatchKind, Vault};
use crate::settings::{Preferences, RecentVault, SyncedVault};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// No vault selected (welcome). Prefer saying “No vault”, not “Closed”.
    Closed,
    /// A vault is selected but secrets are not in memory. UI: “Locked” / Unlock.
    Locked,
    /// Vault open in memory. UI: “Unlocked” only when status matters (tray, autofill).
    Unlocked,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub state: SessionState,
    pub vault_path: Option<String>,
    pub vault_file_name: Option<String>,
    /// Key file used last time with the selected vault, to pre-fill the unlock screen.
    pub key_file: Option<String>,
    pub preferences: Preferences,
    pub recent_vaults: Vec<RecentVaultView>,
    pub app_version: &'static str,
    /// The revision of the command surface this binary exposes. The interface carries its own and
    /// warns when the running process is older (see `commands::API_REVISION`).
    pub api_revision: u32,
    /// When the running executable was last written, in Unix milliseconds: which build am I running?
    pub built_at: u64,
    /// Set only on non-release builds (for example `"staging"`); `None` on normal builds. The
    /// interface shows it, and skips the updater so a test build cannot replace itself with a
    /// released one.
    pub channel: Option<&'static str>,
    /// The sync server this build signs in to; shown on a marked build so the target is obvious.
    pub sync_server: Option<&'static str>,
    /// Whether the open vault can also be opened with a recovery key.
    pub has_recovery_key: bool,
    /// Cloud sync state: whether the user is signed in and whether this vault syncs.
    pub sync: SyncView,
}

/// The sync state shown to the interface. Never includes the token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    /// The user is signed in to a sync account.
    pub logged_in: bool,
    pub email: Option<String>,
    pub server_url: Option<String>,
    /// The vault that is currently open is marked as synced.
    pub this_vault_synced: bool,
    /// Last successful sync of the open vault, in Unix milliseconds.
    pub last_synced_at: Option<i64>,
    /// When the account master password last changed, in Unix milliseconds.
    pub password_changed_at: Option<i64>,
    /// Entitlement the server reports: 'comp' | 'trialing' | 'active' | 'past_due' | 'canceled'
    /// | 'expired'. The server is authoritative; this only renders the interface.
    pub plan: Option<String>,
    /// End of the current paid or trial period, in Unix milliseconds.
    pub plan_valid_until: Option<i64>,
    /// Cloud vaults known to this device, grouped by the vault's own identity.
    pub cloud: Vec<CloudVaultView>,
    /// Vaults on this device that are not synced.
    pub local: Vec<LocalVaultView>,
}

/// A vault that has a cloud copy, with the local copies on this device.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVaultView {
    pub name: String,
    /// The vault's own id (immutable): the true identity of the vault.
    pub local_vault_id: String,
    pub server_vault_id: String,
    pub last_synced_at: Option<i64>,
    /// Local file paths of this vault on this device.
    pub copies: Vec<String>,
    /// At least one local copy exists on disk.
    pub present: bool,
}

/// A vault on this device that is not synced.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalVaultView {
    pub path: String,
    pub file_name: String,
    pub key_file: Option<String>,
}

/// A cloud vault as listed by the server (metadata only, no contents).
///
/// Defined here rather than in the sync module so the command surface compiles with the `sync`
/// feature off, where the sync module is absent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVaultInfo {
    pub id: String,
    pub name: String,
    /// The encrypted label; absent on vaults uploaded by older versions.
    pub name_cipher: Option<String>,
    pub vault_uuid: Option<String>,
    pub version: u64,
    pub updated_at: i64,
}

#[cfg(feature = "sync")]
impl From<crate::sync::CloudVault> for CloudVaultInfo {
    fn from(vault: crate::sync::CloudVault) -> Self {
        CloudVaultInfo {
            id: vault.id,
            name: vault.name,
            name_cipher: vault.name_cipher,
            vault_uuid: vault.vault_uuid,
            version: vault.version,
            updated_at: vault.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentVaultView {
    pub path: String,
    pub file_name: String,
    pub key_file: Option<String>,
    pub exists: bool,
    /// This vault is set up for cloud sync.
    pub synced: bool,
    /// Last successful sync, in Unix milliseconds.
    pub last_synced_at: Option<i64>,
}

impl RecentVaultView {
    pub fn new(recent: &RecentVault, synced: Option<&SyncedVault>) -> Self {
        RecentVaultView {
            path: path_text(&recent.path),
            file_name: file_name(&recent.path),
            key_file: recent.key_file.as_deref().map(path_text),
            exists: recent.path.is_file(),
            synced: synced.is_some(),
            last_synced_at: synced.and_then(|synced| synced.last_synced_at),
        }
    }
}

pub fn path_text(path: &Path) -> String {
    path.display().to_string()
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_text(path))
}

/// Everything the main window needs to draw the vault.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    pub name: String,
    pub path: String,
    pub file_name: String,
    pub folders: Vec<FolderView>,
    pub entries: Vec<EntrySummary>,
    pub custom_icons: Vec<CustomIconView>,
}

impl VaultView {
    pub fn new(vault: &Vault, path: &Path) -> Self {
        VaultView {
            name: vault.name.clone(),
            path: path_text(path),
            file_name: file_name(path),
            folders: vault.folders.iter().map(FolderView::from).collect(),
            entries: vault.entries.iter().map(EntrySummary::from).collect(),
            custom_icons: vault.custom_icons.iter().map(CustomIconView::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub icon: Option<Icon>,
}

impl From<&Folder> for FolderView {
    fn from(folder: &Folder) -> Self {
        FolderView {
            id: folder.id,
            parent_id: folder.parent_id,
            name: folder.name.clone(),
            icon: folder.icon.clone(),
        }
    }
}

/// One row of the entry list.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: Uuid,
    pub folder_id: Option<Uuid>,
    pub title: String,
    pub username: String,
    pub url: Option<String>,
    pub icon: Option<Icon>,
    pub expiration_date: Option<NaiveDate>,
    pub modified_at: DateTime<Utc>,
    pub has_password: bool,
}

impl From<&Entry> for EntrySummary {
    fn from(entry: &Entry) -> Self {
        EntrySummary {
            id: entry.id,
            folder_id: entry.folder_id,
            title: entry.content.title.clone(),
            username: entry.content.username.clone(),
            url: entry.content.url.clone(),
            icon: entry.content.icon.clone(),
            expiration_date: entry.content.expiration_date,
            modified_at: entry.modified_at,
            has_password: !entry.content.password.is_empty(),
        }
    }
}

/// One autofill candidate for a page URL (no secrets).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlMatchHit {
    pub entry: EntrySummary,
    /// `exactHost` | `parentHost`
    pub match_kind: &'static str,
}

impl UrlMatchHit {
    pub fn new(entry: &Entry, kind: UrlMatchKind) -> Self {
        UrlMatchHit {
            entry: EntrySummary::from(entry),
            match_kind: match kind {
                UrlMatchKind::ExactHost => "exactHost",
                UrlMatchKind::ParentHost => "parentHost",
            },
        }
    }
}

/// Unlocked-session payload for Android Autofill (passwords included; RAM only).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutofillPayloadEntry {
    pub id: Uuid,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: Option<String>,
}

/// The details panel. Protected values are replaced by `None` until the user reveals them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetails {
    pub id: Uuid,
    pub folder_id: Option<Uuid>,
    pub folder_path: Vec<String>,
    pub title: String,
    pub username: String,
    pub has_password: bool,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub expiration_date: Option<NaiveDate>,
    pub icon: Option<Icon>,
    pub custom_fields: Vec<CustomFieldView>,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub history_count: usize,
}

impl EntryDetails {
    pub fn new(entry: &Entry, vault: &Vault) -> Self {
        let content = &entry.content;
        EntryDetails {
            id: entry.id,
            folder_id: entry.folder_id,
            folder_path: vault.folder_path(entry.folder_id),
            title: content.title.clone(),
            username: content.username.clone(),
            has_password: !content.password.is_empty(),
            url: content.url.clone(),
            notes: content.notes.clone(),
            expiration_date: content.expiration_date,
            icon: content.icon.clone(),
            custom_fields: content
                .custom_fields
                .iter()
                .map(|field| CustomFieldView {
                    name: field.name.clone(),
                    value: (!field.protected).then(|| field.value.clone()),
                    protected: field.protected,
                })
                .collect(),
            created_at: entry.created_at,
            modified_at: entry.modified_at,
            history_count: entry.history.len(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldView {
    pub name: String,
    pub value: Option<String>,
    pub protected: bool,
}

/// One previous version in the history dialog.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSummary {
    pub index: usize,
    pub modified_at: DateTime<Utc>,
    pub title: String,
    pub username: String,
    pub url: Option<String>,
    /// Whether that version's password is different from the current one.
    pub password_differs: bool,
}

pub fn revisions(entry: &Entry) -> Vec<RevisionSummary> {
    entry
        .history
        .iter()
        .enumerate()
        .map(|(index, revision)| RevisionSummary {
            index,
            modified_at: revision.modified_at,
            title: revision.content.title.clone(),
            username: revision.content.username.clone(),
            url: revision.content.url.clone(),
            password_differs: revision.content.password != entry.content.password,
        })
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIconView {
    pub id: Uuid,
    pub mime: String,
    pub data: String,
}

impl From<&CustomIcon> for CustomIconView {
    fn from(icon: &CustomIcon) -> Self {
        CustomIconView {
            id: icon.id,
            mime: icon.mime.clone(),
            data: icon.data.clone(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// Top-level folder that received the imported entries.
    pub folder_name: String,
    pub entries_imported: usize,
    pub folders_created: usize,
    pub duplicates_skipped: usize,
    pub rows_skipped: usize,
    /// KeePass attachments are not supported, so they are counted and reported.
    pub attachments_skipped: usize,
}
