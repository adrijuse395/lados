//! Import from and export to KeePass databases (`.kdbx`), so moving to or from Lados keeps
//! folders, icons, history, expiration dates and custom fields.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, NaiveDateTime, Utc};
use keepass::config::KdfConfig;
use keepass::db::{
    CustomIconId, EntryId, EntryMut, EntryRef, GroupId, GroupMut, GroupRef, History, Icon as KeePassIcon, Value,
    fields,
};
use keepass::error::{DatabaseKeyError, DatabaseOpenError};
use keepass::{Database, DatabaseKey};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::models::{self, CustomField, EntryContent, EntryRevision, Folder, Icon, Vault};
use crate::views::ImportSummary;

pub const FILE_EXTENSION: &str = "kdbx";
const OTP_FIELD_NAME: &str = "TOTP";
const TAGS_FIELD_NAME: &str = "Tags";
const DEFAULT_ENTRY_ICON: usize = 0;
const DEFAULT_GROUP_ICON: usize = 48;

/// KeePass numbers its built-in icons while Lados names them. Several KeePass icons share a name.
const ICONS_FROM_KEEPASS: &[(usize, &str)] = &[
    (1, "globe"), (3, "server"), (5, "chat"), (7, "note"), (8, "globe"), (9, "user"), (11, "camera"),
    (13, "key"), (16, "star"), (18, "monitor"), (19, "mail"), (20, "settings"), (25, "mail"),
    (26, "database"), (27, "database"), (29, "terminal"), (30, "terminal"), (34, "settings"),
    (35, "globe"), (37, "bank"), (44, "note"), (51, "lock"), (52, "lock"), (56, "book"), (58, "user"),
    (59, "tool"), (60, "home"), (61, "star"), (66, "money"), (67, "shield"), (68, "phone"),
];

const ICONS_TO_KEEPASS: &[(&str, usize)] = &[
    ("key", 0), ("globe", 1), ("server", 3), ("chat", 5), ("note", 7), ("user", 9), ("camera", 11),
    ("monitor", 18), ("mail", 19), ("settings", 20), ("database", 26), ("terminal", 30), ("bank", 37),
    ("folder", 48), ("lock", 51), ("book", 56), ("tool", 59), ("home", 60), ("star", 61),
    ("money", 66), ("shield", 67), ("phone", 68),
];

fn database_key(password: &str, key_file: Option<&[u8]>) -> Result<DatabaseKey> {
    if password.is_empty() && key_file.is_none() {
        return Err(Error::invalid("Enter a password or choose a key file"));
    }
    let mut key = DatabaseKey::new();
    if !password.is_empty() {
        key = key.with_password(password);
    }
    if let Some(bytes) = key_file {
        key = key
            .with_keyfile(&mut Cursor::new(bytes))
            .map_err(|error| Error::invalid(format!("The key file could not be read: {error}")))?;
    }
    Ok(key)
}

fn to_utc(time: Option<NaiveDateTime>) -> DateTime<Utc> {
    time.map(|t| t.and_utc()).unwrap_or_else(Utc::now)
}

// ----- Import -----

/// Decrypts a KeePass database (KDBX 3 or 4, or the old KDB format). This is the slow step,
/// because of the key derivation, so it is kept apart from the import itself.
pub fn open_kdbx(bytes: &[u8], password: &str, key_file: Option<&[u8]>) -> Result<Database> {
    Database::parse(bytes, database_key(password, key_file)?).map_err(|error| match error {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => Error::InvalidCredentials,
        other => Error::Import(format!("The KeePass file could not be read ({other})")),
    })
}

/// Adds the content of a KeePass database to the vault, inside a new top-level folder named
/// after the file. The recycle bin is left out.
pub fn import_kdbx(vault: &mut Vault, database: &Database, source_name: &str) -> Result<ImportSummary> {
    let folders_before = vault.folders.len();
    let root = vault.find_or_add_folder(None, source_name, None)?;
    let mut importer = Importer {
        database,
        icons: HashMap::new(),
        summary: ImportSummary {
            folder_name: source_name.to_string(),
            ..Default::default()
        },
    };
    importer.import_group(vault, database.root(), root)?;
    let mut summary = importer.summary;
    summary.folders_created = vault.folders.len() - folders_before;
    Ok(summary)
}

struct Importer<'a> {
    database: &'a Database,
    /// KeePass icon ids already copied into the vault.
    icons: HashMap<CustomIconId, Uuid>,
    summary: ImportSummary,
}

impl Importer<'_> {
    fn import_group(&mut self, vault: &mut Vault, group: GroupRef<'_>, folder_id: Uuid) -> Result<()> {
        for entry in group.entries() {
            self.import_entry(vault, &entry, folder_id)?;
        }
        let recycle_bin = self.database.meta.recyclebin_uuid;
        for child in group.groups() {
            if Some(child.id().uuid()) == recycle_bin {
                continue;
            }
            let icon = self.icon(vault, child.icon());
            let name = if child.name.trim().is_empty() { "Unnamed group" } else { child.name.as_str() };
            let child_folder = vault.find_or_add_folder(Some(folder_id), name, icon)?;
            self.import_group(vault, child, child_folder)?;
        }
        Ok(())
    }

    fn import_entry(&mut self, vault: &mut Vault, entry: &EntryRef<'_>, folder_id: Uuid) -> Result<()> {
        self.summary.attachments_skipped += entry.attachments().count();
        let mut content = content_from(entry);
        content.icon = self.icon(vault, entry.icon());
        let content = content.with_fallback_title().normalized()?;
        if vault.has_identical_entry(Some(folder_id), &content) {
            self.summary.duplicates_skipped += 1;
            return Ok(());
        }

        let history = entry
            .history
            .iter()
            .flat_map(|history| history.get_entries())
            .filter_map(|old| {
                let mut previous = content_from(old);
                previous.icon = content.icon.clone();
                Some(EntryRevision {
                    modified_at: to_utc(old.times.last_modification),
                    content: previous.with_fallback_title().normalized().ok()?,
                })
            })
            .collect();
        vault.add_imported_entry(
            Some(folder_id),
            content,
            to_utc(entry.times.creation),
            to_utc(entry.times.last_modification),
            history,
        )?;
        self.summary.entries_imported += 1;
        Ok(())
    }

    /// KeePass' default icons (the key for entries, the folder for groups) are not in the table,
    /// so they become `None`: Lados' own defaults.
    fn icon(&mut self, vault: &mut Vault, icon: Option<&KeePassIcon>) -> Option<Icon> {
        match icon? {
            KeePassIcon::BuiltIn(index) => ICONS_FROM_KEEPASS
                .iter()
                .find(|(i, _)| i == index)
                .map(|(_, name)| Icon::Builtin(name.to_string())),
            KeePassIcon::Custom(id) => {
                if let Some(known) = self.icons.get(id) {
                    return Some(Icon::Custom(*known));
                }
                let custom = self.database.custom_icon(*id)?;
                let mime = models::image_mime(&custom.data)?;
                let name = custom.name.clone().unwrap_or_else(|| "KeePass icon".into());
                let uuid = vault.add_custom_icon(&name, mime, BASE64.encode(&custom.data));
                self.icons.insert(*id, uuid);
                Some(Icon::Custom(uuid))
            }
        }
    }
}

fn content_from(entry: &keepass::db::Entry) -> EntryContent {
    let text = |key: &str| entry.get(key).unwrap_or_default().to_string();
    let mut content = EntryContent {
        title: text(fields::TITLE),
        username: text(fields::USERNAME),
        password: text(fields::PASSWORD),
        url: entry.get(fields::URL).map(str::to_string),
        notes: entry.get(fields::NOTES).map(|notes| notes.replace("\r\n", "\n")),
        ..Default::default()
    };
    if entry.times.expires == Some(true) {
        content.expiration_date = entry.times.expiry.map(|expiry| expiry.date());
    }

    let mut names: Vec<&String> = entry
        .fields
        .keys()
        .filter(|name| !fields::KNOWN_FIELDS.contains(&name.as_str()))
        .collect();
    names.sort();
    for name in names {
        let value = &entry.fields[name];
        if value.get().is_empty() {
            continue;
        }
        let (name, protected) = if name == fields::OTP {
            (OTP_FIELD_NAME.to_string(), true)
        } else {
            (name.clone(), value.is_protected())
        };
        content.custom_fields.push(CustomField {
            name,
            value: value.get().clone(),
            protected,
        });
    }
    if !entry.tags.is_empty() {
        content.custom_fields.push(CustomField {
            name: TAGS_FIELD_NAME.into(),
            value: entry.tags.join(", "),
            protected: false,
        });
    }
    content
}

// ----- Export -----

/// Writes the vault as a KDBX 4 database protected with Argon2id.
pub fn export_kdbx(vault: &Vault, password: &str, key_file: Option<&[u8]>) -> Result<Zeroizing<Vec<u8>>> {
    let key = database_key(password, key_file)?;
    let mut database = to_database(vault)?;
    database.config.kdf_config = match database.config.kdf_config.clone() {
        KdfConfig::Argon2 { version, .. } | KdfConfig::Argon2id { version, .. } => KdfConfig::Argon2id {
            iterations: 3,
            memory: 64 * 1024 * 1024,
            parallelism: 4,
            version,
        },
        other => other,
    };
    save(&database, key)
}

fn save(database: &Database, key: DatabaseKey) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    database
        .save(&mut *bytes, key)
        .map_err(|error| Error::Export(error.to_string()))?;
    Ok(bytes)
}

fn missing_group() -> Error {
    Error::Export("The folder structure could not be rebuilt".into())
}

fn to_database(vault: &Vault) -> Result<Database> {
    let mut database = Database::new();
    database.meta.database_name = Some(vault.name.clone());
    database.root_mut().name = vault.name.clone();
    let root = database.root().id();

    let mut groups: HashMap<Uuid, GroupId> = HashMap::new();
    let mut icons: HashMap<Uuid, CustomIconId> = HashMap::new();

    for folder in folders_top_down(vault) {
        let parent = folder.parent_id.and_then(|p| groups.get(&p).copied()).unwrap_or(root);
        let mut parent_group = database.group_mut(parent).ok_or_else(missing_group)?;
        let mut group = parent_group
            .add_group_with_id(GroupId::from_uuid(folder.id))
            .map_err(|_| missing_group())?;
        group.name = folder.name.clone();
        group.times.creation = Some(folder.created_at.naive_utc());
        group.times.last_modification = Some(folder.modified_at.naive_utc());
        group.times.location_changed = Some(folder.location_changed_at.naive_utc());
        set_group_icon(&mut group, folder.icon.as_ref(), vault, &mut icons);
        groups.insert(folder.id, group.id());
    }

    for entry in &vault.entries {
        let group_id = entry.folder_id.and_then(|f| groups.get(&f).copied()).unwrap_or(root);
        let mut group = database.group_mut(group_id).ok_or_else(missing_group)?;
        let mut target = group
            .add_entry_with_id(EntryId::from_uuid(entry.id))
            .map_err(|_| missing_group())?;
        write_content(&mut target, &entry.content);
        target.times.creation = Some(entry.created_at.naive_utc());
        target.times.last_modification = Some(entry.modified_at.naive_utc());
        target.times.location_changed = Some(entry.location_changed_at.naive_utc());
        set_entry_icon(&mut target, entry.content.icon.as_ref(), vault, &mut icons);

        // KeePass keeps the newest version first and `add_entry` prepends, so go oldest first.
        let mut history = History::default();
        for revision in entry.history.iter().rev() {
            let mut old = (*target).clone();
            old.fields.clear();
            old.tags.clear();
            write_content(&mut old, &revision.content);
            old.times.last_modification = Some(revision.modified_at.naive_utc());
            history.add_entry(old);
        }
        target.history = Some(history);
    }
    Ok(database)
}

/// Parents before children, so every group can be created inside an existing one.
fn folders_top_down(vault: &Vault) -> Vec<&Folder> {
    let mut ordered = Vec::with_capacity(vault.folders.len());
    let mut pending = VecDeque::from([None]);
    while let Some(parent) = pending.pop_front() {
        for folder in vault.folders.iter().filter(|f| f.parent_id == parent) {
            ordered.push(folder);
            pending.push_back(Some(folder.id));
        }
    }
    ordered
}

fn write_content(entry: &mut keepass::db::Entry, content: &EntryContent) {
    entry.set_unprotected(fields::TITLE, content.title.clone());
    entry.set_unprotected(fields::USERNAME, content.username.clone());
    entry.set_protected(fields::PASSWORD, content.password.clone());
    if let Some(url) = &content.url {
        entry.set_unprotected(fields::URL, url.clone());
    }
    if let Some(notes) = &content.notes {
        entry.set_unprotected(fields::NOTES, notes.clone());
    }
    for field in &content.custom_fields {
        if field.name == TAGS_FIELD_NAME && !field.protected {
            entry.tags = field
                .value
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(String::from)
                .collect();
            continue;
        }
        let name = if field.name == OTP_FIELD_NAME {
            fields::OTP.to_string()
        } else {
            free_field_name(entry, &field.name)
        };
        let value = if field.protected {
            Value::protected(field.value.clone())
        } else {
            Value::unprotected(field.value.clone())
        };
        entry.set(name, value);
    }
    match content.expiration_date {
        Some(date) => {
            entry.times.expires = Some(true);
            entry.times.expiry = date.and_hms_opt(12, 0, 0);
        }
        None => {
            entry.times.expires = Some(false);
            entry.times.expiry = None;
        }
    }
}

/// KeePass stores fields by name, so repeated names or names used by the standard fields
/// get a numbered suffix instead of overwriting each other.
fn free_field_name(entry: &keepass::db::Entry, name: &str) -> String {
    let taken = |candidate: &str| {
        fields::KNOWN_FIELDS.contains(&candidate) || candidate == fields::OTP || entry.fields.contains_key(candidate)
    };
    if !taken(name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_string())
}

/// The image bytes of a custom icon, or `None` if the icon is missing or damaged.
fn custom_icon_data(vault: &Vault, id: Uuid) -> Option<(String, Vec<u8>)> {
    let icon = vault.custom_icons.iter().find(|icon| icon.id == id)?;
    let data = BASE64.decode(&icon.data).ok()?;
    Some((icon.name.clone(), data))
}

fn builtin_index(name: &str) -> Option<usize> {
    ICONS_TO_KEEPASS.iter().find(|(n, _)| *n == name).map(|(_, index)| *index)
}

fn set_group_icon(group: &mut GroupMut<'_>, icon: Option<&Icon>, vault: &Vault, known: &mut HashMap<Uuid, CustomIconId>) {
    match icon {
        Some(Icon::Custom(id)) => {
            if let Some(existing) = known.get(id) {
                if group.set_icon_custom(*existing).is_ok() {
                    return;
                }
            } else if let Some((name, data)) = custom_icon_data(vault, *id) {
                let mut created = group.set_icon_custom_new(data);
                created.name = Some(name);
                known.insert(*id, created.id());
                return;
            }
            group.set_icon_builtin(DEFAULT_GROUP_ICON);
        }
        Some(Icon::Builtin(name)) => group.set_icon_builtin(builtin_index(name).unwrap_or(DEFAULT_GROUP_ICON)),
        None => group.set_icon_builtin(DEFAULT_GROUP_ICON),
    }
}

fn set_entry_icon(entry: &mut EntryMut<'_>, icon: Option<&Icon>, vault: &Vault, known: &mut HashMap<Uuid, CustomIconId>) {
    match icon {
        Some(Icon::Custom(id)) => {
            if let Some(existing) = known.get(id) {
                if entry.set_icon_custom(*existing).is_ok() {
                    return;
                }
            } else if let Some((name, data)) = custom_icon_data(vault, *id) {
                let mut created = entry.set_icon_custom_new(data);
                created.name = Some(name);
                known.insert(*id, created.id());
                return;
            }
            entry.set_icon_builtin(DEFAULT_ENTRY_ICON);
        }
        Some(Icon::Builtin(name)) => entry.set_icon_builtin(builtin_index(name).unwrap_or(DEFAULT_ENTRY_ICON)),
        None => entry.set_icon_builtin(DEFAULT_ENTRY_ICON),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// Light key derivation so the tests stay fast.
    fn save_quickly(vault: &Vault, key: DatabaseKey) -> Zeroizing<Vec<u8>> {
        let mut database = to_database(vault).unwrap();
        database.config.kdf_config = match database.config.kdf_config.clone() {
            KdfConfig::Argon2 { version, .. } => KdfConfig::Argon2id {
                iterations: 1,
                memory: 1024 * 1024,
                parallelism: 1,
                version,
            },
            other => other,
        };
        save(&database, key).unwrap()
    }

    fn sample_vault() -> Vault {
        let mut vault = Vault::new("Personal").unwrap();
        let work = vault.add_folder(None, "Work", Some(Icon::Builtin("bank".into()))).unwrap();
        let icon = vault.add_custom_icon("dot.png", "image/png", BASE64.encode(b"\x89PNG\r\n\x1a\nfake"));
        let id = vault
            .add_entry(
                Some(work),
                EntryContent {
                    title: "Mail".into(),
                    username: "adria".into(),
                    password: "old".into(),
                    url: Some("https://mail.example.com".into()),
                    notes: Some("notes".into()),
                    expiration_date: NaiveDate::from_ymd_opt(2031, 5, 4),
                    icon: Some(Icon::Custom(icon)),
                    custom_fields: vec![
                        CustomField { name: "PIN".into(), value: "1234".into(), protected: true },
                        CustomField { name: "TOTP".into(), value: "otpauth://totp/x".into(), protected: true },
                        CustomField { name: "Title".into(), value: "clash".into(), protected: false },
                    ],
                },
            )
            .unwrap();
        let mut newer = vault.entry(id).unwrap().content.clone();
        newer.password = "new".into();
        vault.update_entry(id, newer).unwrap();
        vault
    }

    #[test]
    fn vaults_survive_a_trip_through_keepass() {
        let vault = sample_vault();
        let bytes = save_quickly(&vault, DatabaseKey::new().with_password("export"));

        let database = open_kdbx(&bytes, "export", None).unwrap();
        let mut copy = Vault::new("Copy").unwrap();
        let summary = import_kdbx(&mut copy, &database, "Personal").unwrap();
        assert_eq!(summary.entries_imported, 1);

        let entry = &copy.entries[0];
        let original = &vault.entries[0];
        assert_eq!(copy.folder_path(entry.folder_id), vec!["Personal", "Work"]);
        assert_eq!(entry.content.password, "new");
        assert_eq!(entry.content.expiration_date, original.content.expiration_date);
        assert_eq!(entry.history.len(), 1);
        assert_eq!(entry.history[0].content.password, "old");

        let fields: Vec<(&str, &str, bool)> = entry
            .content
            .custom_fields
            .iter()
            .map(|f| (f.name.as_str(), f.value.as_str(), f.protected))
            .collect();
        assert_eq!(
            fields,
            vec![("PIN", "1234", true), ("Title (2)", "clash", false), ("TOTP", "otpauth://totp/x", true)]
        );
        assert!(matches!(entry.content.icon, Some(Icon::Custom(_))));
        assert_eq!(copy.custom_icons.len(), 1);
        let work = copy.folders.iter().find(|f| f.name == "Work").unwrap();
        assert_eq!(work.icon, Some(Icon::Builtin("bank".into())));

        let again = import_kdbx(&mut copy, &database, "Personal").unwrap();
        assert_eq!(again.duplicates_skipped, 1);
    }

    #[test]
    fn key_files_generated_by_lados_work_with_keepass() {
        let vault = sample_vault();
        let key_file = crate::crypto::new_key_file().unwrap();
        let key = DatabaseKey::new()
            .with_password("pw")
            .with_keyfile(&mut Cursor::new(key_file.as_bytes()))
            .unwrap();
        let bytes = save_quickly(&vault, key);

        assert!(open_kdbx(&bytes, "pw", Some(key_file.as_bytes())).is_ok());
        assert!(matches!(open_kdbx(&bytes, "pw", None), Err(Error::InvalidCredentials)));
    }
}
