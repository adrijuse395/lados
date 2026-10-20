//! The shapes of the data sent to the interface.
//!
//! None of them carries a password or a protected field: secrets travel one at a time,
//! only when the user explicitly reveals, copies or edits them.

use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::{CustomIcon, Entry, Folder, Icon, Vault};
use crate::settings::{Preferences, RecentVault};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// No vault selected: the welcome screen.
    Closed,
    /// A vault is selected but its contents are not in memory.
    Locked,
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentVaultView {
    pub path: String,
    pub file_name: String,
    pub key_file: Option<String>,
    pub exists: bool,
}

impl From<&RecentVault> for RecentVaultView {
    fn from(recent: &RecentVault) -> Self {
        RecentVaultView {
            path: path_text(&recent.path),
            file_name: file_name(&recent.path),
            key_file: recent.key_file.as_deref().map(path_text),
            exists: recent.path.is_file(),
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
