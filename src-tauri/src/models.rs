use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::{Error, Result};

/// Version of the vault's internal structure, so future releases can migrate old files.
pub const SCHEMA_VERSION: u8 = 1;

/// How many previous versions of each entry are kept.
pub const MAX_HISTORY: usize = 10;

/// Icon shown next to an entry or a folder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Icon {
    /// One of the icons bundled with the interface, referenced by name.
    Builtin(String),
    /// An image stored inside the vault, so it travels with the file.
    Custom(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIcon {
    #[zeroize(skip)]
    pub id: Uuid,
    pub name: String,
    pub mime: String,
    /// Image bytes encoded as base64.
    pub data: String,
}

/// Extra information attached to an entry, such as a PIN or a security answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct CustomField {
    pub name: String,
    pub value: String,
    /// Protected fields are masked in the interface, like passwords.
    pub protected: bool,
}

/// Everything the user can edit on an entry. Previous versions are kept in its history.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase", default)]
pub struct EntryContent {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: Option<String>,
    pub notes: Option<String>,
    #[zeroize(skip)]
    pub expiration_date: Option<NaiveDate>,
    #[zeroize(skip)]
    pub icon: Option<Icon>,
    pub custom_fields: Vec<CustomField>,
}

/// A previous version of an entry's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRevision {
    #[zeroize(skip)]
    pub modified_at: DateTime<Utc>,
    pub content: EntryContent,
}

/// A saved account: the content plus the metadata needed to merge vaults safely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[zeroize(skip)]
    pub id: Uuid,
    /// `None` means the entry is not inside any folder.
    #[zeroize(skip)]
    pub folder_id: Option<Uuid>,
    pub content: EntryContent,
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>,
    /// Last time the content changed.
    #[zeroize(skip)]
    pub modified_at: DateTime<Utc>,
    /// Last time the entry moved to another folder. Tracked apart from the content so
    /// that moving an entry in one copy and editing it in another can both survive a merge.
    #[zeroize(skip)]
    pub location_changed_at: DateTime<Utc>,
    /// Previous contents, newest first.
    pub history: Vec<EntryRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    #[zeroize(skip)]
    pub id: Uuid,
    /// `None` means it is a top-level folder.
    #[zeroize(skip)]
    pub parent_id: Option<Uuid>,
    pub name: String,
    #[zeroize(skip)]
    pub icon: Option<Icon>,
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>,
    #[zeroize(skip)]
    pub modified_at: DateTime<Utc>,
    #[zeroize(skip)]
    pub location_changed_at: DateTime<Utc>,
}

/// Remembers that an entry or folder was deleted, so a merge does not bring it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedObject {
    pub id: Uuid,
    pub deleted_at: DateTime<Utc>,
}

/// The whole database. This is exactly what gets encrypted into the `.lados` file.
///
/// Folders and entries are stored in flat lists that reference their parent by id:
/// moving, searching and merging become simple lookups instead of tree walks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase")]
pub struct Vault {
    pub version: u8,
    #[zeroize(skip)]
    pub id: Uuid,
    pub name: String,
    #[zeroize(skip)]
    pub name_changed_at: DateTime<Utc>,
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>,
    pub folders: Vec<Folder>,
    pub entries: Vec<Entry>,
    pub custom_icons: Vec<CustomIcon>,
    #[zeroize(skip)]
    pub deleted_objects: Vec<DeletedObject>,
}

/// The current time, but never earlier than `previous`.
///
/// If the system clock goes backwards, a new change must still look newer than the last one,
/// otherwise a merge would discard it.
pub fn timestamp_after(previous: DateTime<Utc>) -> DateTime<Utc> {
    let now = Utc::now();
    if now > previous {
        now
    } else {
        previous + TimeDelta::microseconds(1)
    }
}

fn clean_name(name: &str, what: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid(format!("The {what} name cannot be empty")));
    }
    Ok(name.to_string())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Recognizes the image formats accepted as custom icons by their first bytes.
/// SVG is deliberately not accepted: it can contain scripts.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else {
        None
    }
}

/// The host name of a web address: `https://mail.example.com/inbox` gives `mail.example.com`.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

impl EntryContent {
    /// Trims what should never carry surrounding spaces and rejects incomplete data.
    /// The password is kept exactly as typed: spaces can be part of it.
    pub fn normalized(mut self) -> Result<Self> {
        self.title = self.title.trim().to_string();
        if self.title.is_empty() {
            return Err(Error::invalid("The title cannot be empty"));
        }
        self.username = self.username.trim().to_string();
        self.url = non_empty(self.url).map(|url| url.trim().to_string());
        self.notes = non_empty(self.notes);

        let mut fields = Vec::with_capacity(self.custom_fields.len());
        for mut field in std::mem::take(&mut self.custom_fields) {
            field.name = field.name.trim().to_string();
            if field.name.is_empty() && field.value.is_empty() {
                continue;
            }
            if field.name.is_empty() {
                return Err(Error::invalid("Every custom field needs a name"));
            }
            fields.push(field);
        }
        self.custom_fields = fields;
        Ok(self)
    }

    /// Imported entries may come without a title: the website or the username stand in for it.
    pub fn with_fallback_title(mut self) -> Self {
        if self.title.trim().is_empty() {
            let username = self.username.trim();
            self.title = self
                .url
                .as_deref()
                .and_then(host_of)
                .or_else(|| (!username.is_empty()).then(|| username.to_string()))
                .unwrap_or_else(|| "Untitled entry".to_string());
        }
        self
    }

    /// Text used by the search box. Protected values are never searchable.
    fn searchable_text(&self) -> String {
        let mut text = format!("{}\n{}\n", self.title, self.username);
        for value in [&self.url, &self.notes].into_iter().flatten() {
            text.push_str(value);
            text.push('\n');
        }
        for field in &self.custom_fields {
            text.push_str(&field.name);
            text.push('\n');
            if !field.protected {
                text.push_str(&field.value);
                text.push('\n');
            }
        }
        text.to_lowercase()
    }
}

impl Entry {
    pub fn new(folder_id: Option<Uuid>, content: EntryContent) -> Self {
        let now = Utc::now();
        Entry {
            id: Uuid::new_v4(),
            folder_id,
            content,
            created_at: now,
            modified_at: now,
            location_changed_at: now,
            history: Vec::new(),
        }
    }

    pub fn last_change(&self) -> DateTime<Utc> {
        self.modified_at.max(self.location_changed_at)
    }
}

impl Folder {
    pub fn new(parent_id: Option<Uuid>, name: String, icon: Option<Icon>) -> Self {
        let now = Utc::now();
        Folder {
            id: Uuid::new_v4(),
            parent_id,
            name,
            icon,
            created_at: now,
            modified_at: now,
            location_changed_at: now,
        }
    }

    pub fn last_change(&self) -> DateTime<Utc> {
        self.modified_at.max(self.location_changed_at)
    }
}

impl Vault {
    pub fn new(name: &str) -> Result<Self> {
        let now = Utc::now();
        Ok(Vault {
            version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: clean_name(name, "vault")?,
            name_changed_at: now,
            created_at: now,
            folders: Vec::new(),
            entries: Vec::new(),
            custom_icons: Vec::new(),
            deleted_objects: Vec::new(),
        })
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        let name = clean_name(name, "vault")?;
        if name != self.name {
            self.name = name;
            self.name_changed_at = timestamp_after(self.name_changed_at);
        }
        Ok(())
    }

    // ----- Lookups -----

    pub fn entry(&self, id: Uuid) -> Result<&Entry> {
        self.entries.iter().find(|e| e.id == id).ok_or(Error::NotFound("entry"))
    }

    fn entry_mut(&mut self, id: Uuid) -> Result<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id).ok_or(Error::NotFound("entry"))
    }

    pub fn folder(&self, id: Uuid) -> Result<&Folder> {
        self.folders.iter().find(|f| f.id == id).ok_or(Error::NotFound("folder"))
    }

    fn folder_mut(&mut self, id: Uuid) -> Result<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id).ok_or(Error::NotFound("folder"))
    }

    fn ensure_folder_exists(&self, id: Option<Uuid>) -> Result<()> {
        match id {
            Some(id) => self.folder(id).map(|_| ()),
            None => Ok(()),
        }
    }

    /// The folder itself plus every folder nested inside it, at any depth.
    pub fn subtree(&self, id: Uuid) -> Vec<Uuid> {
        let mut result = vec![id];
        let mut index = 0;
        while index < result.len() {
            let current = result[index];
            result.extend(
                self.folders
                    .iter()
                    .filter(|f| f.parent_id == Some(current))
                    .map(|f| f.id),
            );
            index += 1;
        }
        result
    }

    /// Names from the top level down to the given folder, e.g. `["Work", "Servers"]`.
    pub fn folder_path(&self, id: Option<Uuid>) -> Vec<String> {
        let mut path = Vec::new();
        let mut current = id;
        // The guard protects against a corrupted file that contains a cycle.
        while let Some(folder) = current.and_then(|id| self.folder(id).ok()) {
            if path.len() > self.folders.len() {
                break;
            }
            path.push(folder.name.clone());
            current = folder.parent_id;
        }
        path.reverse();
        path
    }

    /// Returns the folder at `path` below `parent_id`, creating the missing levels.
    pub fn ensure_folder_path(&mut self, parent_id: Option<Uuid>, path: &[String]) -> Result<Option<Uuid>> {
        let mut current = parent_id;
        for name in path {
            current = Some(self.find_or_add_folder(current, name, None)?);
        }
        Ok(current)
    }

    /// Returns the folder called `name` (ignoring case) inside `parent_id`, creating it if needed.
    pub fn find_or_add_folder(&mut self, parent_id: Option<Uuid>, name: &str, icon: Option<Icon>) -> Result<Uuid> {
        let wanted = clean_name(name, "folder")?.to_lowercase();
        let existing = self
            .folders
            .iter()
            .find(|f| f.parent_id == parent_id && f.name.to_lowercase() == wanted);
        match existing {
            Some(folder) => Ok(folder.id),
            None => self.add_folder(parent_id, name, icon),
        }
    }

    // ----- Folders -----

    pub fn add_folder(&mut self, parent_id: Option<Uuid>, name: &str, icon: Option<Icon>) -> Result<Uuid> {
        self.ensure_folder_exists(parent_id)?;
        let folder = Folder::new(parent_id, clean_name(name, "folder")?, icon);
        let id = folder.id;
        self.folders.push(folder);
        Ok(id)
    }

    pub fn update_folder(&mut self, id: Uuid, name: &str, icon: Option<Icon>) -> Result<()> {
        let name = clean_name(name, "folder")?;
        let folder = self.folder_mut(id)?;
        if folder.name != name || folder.icon != icon {
            folder.name = name;
            folder.icon = icon;
            folder.modified_at = timestamp_after(folder.modified_at);
        }
        Ok(())
    }

    pub fn move_folder(&mut self, id: Uuid, parent_id: Option<Uuid>) -> Result<()> {
        self.ensure_folder_exists(parent_id)?;
        if let Some(parent) = parent_id
            && self.subtree(id).contains(&parent)
        {
            return Err(Error::invalid("A folder cannot be moved inside itself"));
        }
        let folder = self.folder_mut(id)?;
        if folder.parent_id != parent_id {
            folder.parent_id = parent_id;
            folder.location_changed_at = timestamp_after(folder.location_changed_at);
        }
        Ok(())
    }

    /// Deletes the folder, its subfolders and every entry inside them.
    pub fn delete_folder(&mut self, id: Uuid) -> Result<()> {
        self.folder(id)?;
        let doomed = self.subtree(id);

        let mut deleted = Vec::new();
        self.entries.retain(|entry| {
            let inside = entry.folder_id.is_some_and(|f| doomed.contains(&f));
            if inside {
                deleted.push((entry.id, entry.last_change()));
            }
            !inside
        });
        self.folders.retain(|folder| {
            let inside = doomed.contains(&folder.id);
            if inside {
                deleted.push((folder.id, folder.last_change()));
            }
            !inside
        });
        for (id, last_change) in deleted {
            self.record_deletion(id, last_change);
        }
        Ok(())
    }

    // ----- Entries -----

    pub fn add_entry(&mut self, folder_id: Option<Uuid>, content: EntryContent) -> Result<Uuid> {
        self.ensure_folder_exists(folder_id)?;
        let entry = Entry::new(folder_id, content.normalized()?);
        let id = entry.id;
        self.entries.push(entry);
        Ok(id)
    }

    /// Adds an entry brought from another program, keeping its dates and previous versions.
    pub fn add_imported_entry(
        &mut self,
        folder_id: Option<Uuid>,
        content: EntryContent,
        created_at: DateTime<Utc>,
        modified_at: DateTime<Utc>,
        mut history: Vec<EntryRevision>,
    ) -> Result<Uuid> {
        self.ensure_folder_exists(folder_id)?;
        let mut entry = Entry::new(folder_id, content.normalized()?);
        entry.created_at = created_at.min(modified_at);
        entry.modified_at = modified_at;
        history.retain(|revision| revision.modified_at <= modified_at && revision.content != entry.content);
        history.sort_by_key(|revision| std::cmp::Reverse(revision.modified_at));
        history.truncate(MAX_HISTORY);
        entry.history = history;
        let id = entry.id;
        self.entries.push(entry);
        Ok(id)
    }

    /// Replaces the content of an entry, keeping the previous version in its history.
    /// Returns `false` when nothing changed.
    pub fn update_entry(&mut self, id: Uuid, content: EntryContent) -> Result<bool> {
        let content = content.normalized()?;
        let entry = self.entry_mut(id)?;
        if entry.content == content {
            return Ok(false);
        }
        let previous = EntryRevision {
            modified_at: entry.modified_at,
            content: std::mem::replace(&mut entry.content, content),
        };
        entry.history.insert(0, previous);
        entry.history.truncate(MAX_HISTORY);
        entry.modified_at = timestamp_after(entry.modified_at);
        Ok(true)
    }

    pub fn move_entry(&mut self, id: Uuid, folder_id: Option<Uuid>) -> Result<()> {
        self.ensure_folder_exists(folder_id)?;
        let entry = self.entry_mut(id)?;
        if entry.folder_id != folder_id {
            entry.folder_id = folder_id;
            entry.location_changed_at = timestamp_after(entry.location_changed_at);
        }
        Ok(())
    }

    pub fn delete_entry(&mut self, id: Uuid) -> Result<()> {
        let index = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(Error::NotFound("entry"))?;
        let entry = self.entries.remove(index);
        self.record_deletion(entry.id, entry.last_change());
        Ok(())
    }

    pub fn duplicate_entry(&mut self, id: Uuid) -> Result<Uuid> {
        let source = self.entry(id)?;
        let mut content = source.content.clone();
        content.title = format!("{} (copy)", content.title);
        let copy = Entry::new(source.folder_id, content);
        let copy_id = copy.id;
        self.entries.push(copy);
        Ok(copy_id)
    }

    /// Brings back a previous version. The current one goes to the history, so it can be undone.
    pub fn restore_revision(&mut self, id: Uuid, index: usize) -> Result<()> {
        let content = self
            .entry(id)?
            .history
            .get(index)
            .map(|revision| revision.content.clone())
            .ok_or(Error::NotFound("version"))?;
        self.update_entry(id, content)?;
        Ok(())
    }

    /// Lets imports skip entries that are already in the vault, so importing twice is harmless.
    pub fn has_identical_entry(&self, folder_id: Option<Uuid>, content: &EntryContent) -> bool {
        self.entries
            .iter()
            .any(|e| e.folder_id == folder_id && e.content == *content)
    }

    /// Ids of the entries that contain every word of `query`, ignoring case.
    pub fn search(&self, query: &str) -> Vec<Uuid> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|entry| {
                let text = entry.content.searchable_text();
                terms.iter().all(|term| text.contains(term))
            })
            .map(|entry| entry.id)
            .collect()
    }

    // ----- Icons -----

    /// Stores an image inside the vault. Adding the same image twice reuses the first copy.
    pub fn add_custom_icon(&mut self, name: &str, mime: &str, data: String) -> Uuid {
        if let Some(existing) = self.custom_icons.iter().find(|icon| icon.data == data) {
            return existing.id;
        }
        let id = Uuid::new_v4();
        self.custom_icons.push(CustomIcon {
            id,
            name: name.to_string(),
            mime: mime.to_string(),
            data,
        });
        id
    }

    fn record_deletion(&mut self, id: Uuid, last_change: DateTime<Utc>) {
        let deleted_at = timestamp_after(last_change);
        match self.deleted_objects.iter_mut().find(|d| d.id == id) {
            Some(existing) => existing.deleted_at = existing.deleted_at.max(deleted_at),
            None => self.deleted_objects.push(DeletedObject { id, deleted_at }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(title: &str) -> EntryContent {
        EntryContent {
            title: title.into(),
            username: "adria".into(),
            password: "secret".into(),
            ..Default::default()
        }
    }

    #[test]
    fn entries_are_validated_and_normalized() {
        let mut vault = Vault::new("Test").unwrap();
        assert!(vault.add_entry(None, content("   ")).is_err());

        let mut raw = content("  Mail  ");
        raw.url = Some("   ".into());
        raw.password = " spaced ".into();
        let id = vault.add_entry(None, raw).unwrap();
        let entry = vault.entry(id).unwrap();
        assert_eq!(entry.content.title, "Mail");
        assert_eq!(entry.content.url, None);
        assert_eq!(entry.content.password, " spaced ");
    }

    #[test]
    fn updates_keep_previous_versions() {
        let mut vault = Vault::new("Test").unwrap();
        let id = vault.add_entry(None, content("Mail")).unwrap();

        assert!(!vault.update_entry(id, content("Mail")).unwrap());
        assert!(vault.entry(id).unwrap().history.is_empty());

        for i in 0..MAX_HISTORY + 3 {
            let mut changed = content("Mail");
            changed.password = format!("password-{i}");
            assert!(vault.update_entry(id, changed).unwrap());
        }
        let entry = vault.entry(id).unwrap();
        assert_eq!(entry.history.len(), MAX_HISTORY);
        assert_eq!(entry.content.password, format!("password-{}", MAX_HISTORY + 2));
        assert_eq!(entry.history[0].content.password, format!("password-{}", MAX_HISTORY + 1));
        assert!(entry.modified_at > entry.history[0].modified_at);
    }

    #[test]
    fn restoring_a_revision_can_be_undone() {
        let mut vault = Vault::new("Test").unwrap();
        let id = vault.add_entry(None, content("Mail")).unwrap();
        let mut changed = content("Mail");
        changed.password = "new".into();
        vault.update_entry(id, changed).unwrap();

        vault.restore_revision(id, 0).unwrap();
        let entry = vault.entry(id).unwrap();
        assert_eq!(entry.content.password, "secret");
        assert_eq!(entry.history[0].content.password, "new");
    }

    #[test]
    fn folders_cannot_be_moved_inside_themselves() {
        let mut vault = Vault::new("Test").unwrap();
        let work = vault.add_folder(None, "Work", None).unwrap();
        let servers = vault.add_folder(Some(work), "Servers", None).unwrap();

        assert!(vault.move_folder(work, Some(servers)).is_err());
        assert!(vault.move_folder(work, Some(work)).is_err());
        vault.move_folder(servers, None).unwrap();
        assert_eq!(vault.folder(servers).unwrap().parent_id, None);
    }

    #[test]
    fn deleting_a_folder_removes_its_content_and_remembers_it() {
        let mut vault = Vault::new("Test").unwrap();
        let work = vault.add_folder(None, "Work", None).unwrap();
        let servers = vault.add_folder(Some(work), "Servers", None).unwrap();
        let personal = vault.add_folder(None, "Personal", None).unwrap();
        let inside = vault.add_entry(Some(servers), content("SSH")).unwrap();
        let outside = vault.add_entry(Some(personal), content("Bank")).unwrap();

        vault.delete_folder(work).unwrap();

        assert!(vault.entry(inside).is_err());
        assert!(vault.entry(outside).is_ok());
        assert!(vault.folder(servers).is_err());
        let deleted: Vec<Uuid> = vault.deleted_objects.iter().map(|d| d.id).collect();
        assert!(deleted.contains(&work) && deleted.contains(&servers) && deleted.contains(&inside));
    }

    #[test]
    fn folder_paths_are_created_once_and_reused() {
        let mut vault = Vault::new("Test").unwrap();
        let path = vec!["Work".to_string(), "Servers".to_string()];
        let first = vault.ensure_folder_path(None, &path).unwrap();
        let second = vault
            .ensure_folder_path(None, &["work".to_string(), "Servers".to_string()])
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(vault.folders.len(), 2);
        assert_eq!(vault.folder_path(first), path);
    }

    #[test]
    fn search_matches_every_word_but_never_protected_values() {
        let mut vault = Vault::new("Test").unwrap();
        let mut bank = content("Bank");
        bank.notes = Some("Branch in Barcelona".into());
        bank.custom_fields = vec![CustomField {
            name: "PIN".into(),
            value: "4321".into(),
            protected: true,
        }];
        let bank_id = vault.add_entry(None, bank).unwrap();
        vault.add_entry(None, content("Mail")).unwrap();

        assert_eq!(vault.search("bank barcelona"), vec![bank_id]);
        assert_eq!(vault.search("pin"), vec![bank_id]);
        assert!(vault.search("4321").is_empty());
        assert!(vault.search("secret").is_empty());
        assert_eq!(vault.search("ADRIA").len(), 2);
    }

    #[test]
    fn identical_custom_icons_are_stored_once() {
        let mut vault = Vault::new("Test").unwrap();
        let first = vault.add_custom_icon("a.png", "image/png", "AAAA".into());
        let second = vault.add_custom_icon("b.png", "image/png", "AAAA".into());
        assert_eq!(first, second);
        assert_eq!(vault.custom_icons.len(), 1);
    }
}
