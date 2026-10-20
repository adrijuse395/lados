//! Import from and export to CSV, the format every password manager and browser can produce.
//!
//! Importing works out what each column holds from its header, so files exported by Chrome,
//! Firefox, Bitwarden, LastPass, 1Password, KeePass, KeePassXC or Lados itself are understood
//! without asking the user to map columns.

use std::collections::{BTreeSet, HashSet};
use std::mem::discriminant;

use chrono::NaiveDate;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};
use crate::models::{CustomField, EntryContent, Vault};
use crate::views::ImportSummary;

pub const FILE_EXTENSION: &str = "csv";
const OTP_FIELD_NAME: &str = "TOTP";

/// What a column holds, worked out from its header.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Column {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Group,
    Expires,
    Totp,
    /// Bitwarden packs its custom fields in one column, one `name: value` per line.
    PackedFields,
    Ignored,
    Custom(String),
}

fn classify(header: &str) -> Column {
    let key: String = header
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    match key.as_str() {
        "" => Column::Ignored,
        "title" | "name" | "account" | "accountname" | "entry" | "item" | "itemname" | "service" => Column::Title,
        "username" | "user" | "login" | "loginname" | "loginusername" | "email" | "emailaddress" | "userid" => {
            Column::Username
        }
        "password" | "loginpassword" | "pass" | "pwd" => Column::Password,
        "url" | "uri" | "loginuri" | "loginurl" | "website" | "web" | "webaddress" | "link" => Column::Url,
        "notes" | "note" | "comments" | "comment" | "extra" | "description" => Column::Notes,
        "group" | "groupname" | "folder" | "foldername" | "grouping" | "path" | "category" | "collections" => {
            Column::Group
        }
        "expires" | "expiry" | "expiration" | "expirationdate" | "expirydate" | "expireson" => Column::Expires,
        "totp" | "logintotp" | "otp" | "otpauth" | "otpsecret" => Column::Totp,
        "fields" => Column::PackedFields,
        "favorite" | "fav" | "type" | "reprompt" | "httprealm" | "formactionorigin" | "guid" | "id" | "uuid"
        | "icon" | "created" | "modified" | "lastmodified" | "timecreated" | "timelastused"
        | "timepasswordchanged" | "creationtime" | "lastmodificationtime" | "lastaccesstime" | "lastused"
        | "passwordchanged" | "usagecount" | "archived" => Column::Ignored,
        _ => Column::Custom(header.trim().to_string()),
    }
}

/// Custom fields whose name suggests a secret are masked like passwords.
fn looks_secret(name: &str) -> bool {
    let name = name.to_lowercase();
    ["pass", "pin", "secret", "cvv", "cvc", "token", "key", "code", "otp", "seed"]
        .iter()
        .any(|word| name.contains(word))
}

/// Picks the separator that appears most often in the header line (outside quotes).
fn detect_delimiter(bytes: &[u8]) -> u8 {
    let header = bytes.split(|b| *b == b'\n').next().unwrap_or_default();
    let mut counts = [(b',', 0), (b';', 0), (b'\t', 0)];
    let mut in_quotes = false;
    for &byte in header {
        if byte == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes
            && let Some(count) = counts.iter_mut().find(|(delimiter, _)| *delimiter == byte)
        {
            count.1 += 1;
        }
    }
    counts
        .iter()
        .max_by_key(|(delimiter, count)| (*count, *delimiter == b','))
        .map(|(delimiter, _)| *delimiter)
        .unwrap_or(b',')
}

fn split_path(value: &str) -> Vec<String> {
    value
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(String::from)
        .collect()
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    let date = value.get(..10).unwrap_or(value);
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(date, "%Y/%m/%d"))
        .ok()
}

fn means_never(value: &str) -> bool {
    matches!(value.trim().to_lowercase().as_str(), "never" | "no" | "false" | "0")
}

fn parse_packed_fields(text: &str, fields: &mut Vec<CustomField>) {
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (name, value) = line.split_once(": ").unwrap_or(("Field", line));
        fields.push(CustomField {
            name: name.trim().to_string(),
            value: value.to_string(),
            protected: looks_secret(name),
        });
    }
}

struct Row {
    path: Vec<String>,
    content: EntryContent,
}

fn import_error(error: csv::Error) -> Error {
    Error::Import(error.to_string())
}

/// Reads the rows of the file. Returns them with the number of blank rows skipped.
fn parse(bytes: &[u8]) -> Result<(Vec<Row>, usize)> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(detect_delimiter(bytes))
        .flexible(true)
        .from_reader(bytes);

    // The first column of each kind wins; repeated ones are kept as custom fields.
    let mut seen = HashSet::new();
    let columns: Vec<Column> = reader
        .byte_headers()
        .map_err(import_error)?
        .iter()
        .map(|raw| {
            let header = String::from_utf8_lossy(raw);
            let column = classify(&header);
            match column {
                Column::Ignored | Column::Custom(_) => column,
                _ if !seen.insert(discriminant(&column)) => Column::Custom(header.trim().to_string()),
                _ => column,
            }
        })
        .collect();
    let recognizable = columns
        .iter()
        .any(|c| matches!(c, Column::Title | Column::Username | Column::Password | Column::Url));
    if !recognizable {
        return Err(Error::Import(
            "No known columns were found. The first line must name the columns, for example: \
             Title, Username, Password, URL"
                .into(),
        ));
    }

    let mut rows = Vec::new();
    let mut skipped = 0;
    for record in reader.byte_records() {
        let record = record.map_err(import_error)?;
        let mut content = EntryContent::default();
        let mut path = Vec::new();
        for (column, raw) in columns.iter().zip(record.iter()) {
            let value = Zeroizing::new(String::from_utf8_lossy(raw).into_owned());
            if value.trim().is_empty() {
                continue;
            }
            match column {
                Column::Title => content.title = value.trim().to_string(),
                Column::Username => content.username = value.trim().to_string(),
                Column::Password => content.password = value.to_string(),
                Column::Url => content.url = Some(value.trim().to_string()),
                Column::Notes => content.notes = Some(value.replace("\r\n", "\n")),
                Column::Group => path = split_path(&value),
                Column::Expires => match parse_date(&value) {
                    Some(date) => content.expiration_date = Some(date),
                    None if means_never(&value) => {}
                    None => content.custom_fields.push(CustomField {
                        name: "Expires".into(),
                        value: value.to_string(),
                        protected: false,
                    }),
                },
                Column::Totp => content.custom_fields.push(CustomField {
                    name: OTP_FIELD_NAME.into(),
                    value: value.trim().to_string(),
                    protected: true,
                }),
                Column::PackedFields => parse_packed_fields(&value, &mut content.custom_fields),
                Column::Ignored => {}
                Column::Custom(name) => content.custom_fields.push(CustomField {
                    name: name.clone(),
                    value: value.to_string(),
                    protected: looks_secret(name),
                }),
            }
        }
        if content == EntryContent::default() {
            skipped += 1;
            continue;
        }
        rows.push(Row {
            path,
            content: content.with_fallback_title().normalized()?,
        });
    }

    // KeePassXC writes every group path starting with its root group, "Root".
    let all_under_root = !rows.is_empty()
        && rows
            .iter()
            .all(|row| row.path.first().is_some_and(|first| first.eq_ignore_ascii_case("root")));
    if all_under_root {
        for row in &mut rows {
            row.path.remove(0);
        }
    }
    Ok((rows, skipped))
}

/// Adds the entries of a CSV file to the vault, inside a new top-level folder named after
/// the file. Entries already present in the same place are skipped, so importing twice is safe.
pub fn import_csv(vault: &mut Vault, bytes: &[u8], source_name: &str) -> Result<ImportSummary> {
    let (rows, rows_skipped) = parse(bytes)?;
    if rows.is_empty() {
        return Err(Error::Import("The file does not contain any entry".into()));
    }

    let folders_before = vault.folders.len();
    let root = vault.find_or_add_folder(None, source_name, None)?;
    let mut summary = ImportSummary {
        folder_name: source_name.to_string(),
        rows_skipped,
        ..Default::default()
    };
    for row in rows {
        let folder_id = vault.ensure_folder_path(Some(root), &row.path)?;
        if vault.has_identical_entry(folder_id, &row.content) {
            summary.duplicates_skipped += 1;
            continue;
        }
        vault.add_entry(folder_id, row.content)?;
        summary.entries_imported += 1;
    }
    summary.folders_created = vault.folders.len() - folders_before;
    Ok(summary)
}

/// Writes every entry, with its folder path, in clear text.
pub fn export_csv(vault: &Vault) -> Result<Zeroizing<Vec<u8>>> {
    let export_error = |error: csv::Error| Error::Export(error.to_string());
    let field_names: BTreeSet<&str> = vault
        .entries
        .iter()
        .flat_map(|entry| entry.content.custom_fields.iter().map(|field| field.name.as_str()))
        .collect();

    let mut writer = csv::Writer::from_writer(Vec::new());
    let mut header = vec![
        "Group", "Title", "Username", "Password", "URL", "Notes", "Expires", "Created", "Modified",
    ];
    header.extend(field_names.iter().copied());
    writer.write_record(&header).map_err(export_error)?;

    let mut entries: Vec<(String, &crate::models::Entry)> = vault
        .entries
        .iter()
        .map(|entry| (vault.folder_path(entry.folder_id).join("/"), entry))
        .collect();
    entries.sort_by_cached_key(|(path, entry)| (path.to_lowercase(), entry.content.title.to_lowercase()));

    for (path, entry) in entries {
        let content = &entry.content;
        let mut record = vec![
            path,
            content.title.clone(),
            content.username.clone(),
            content.password.clone(),
            content.url.clone().unwrap_or_default(),
            content.notes.clone().unwrap_or_default(),
            content.expiration_date.map(|date| date.to_string()).unwrap_or_default(),
            entry.created_at.to_rfc3339(),
            entry.modified_at.to_rfc3339(),
        ];
        for name in &field_names {
            let values: Vec<&str> = content
                .custom_fields
                .iter()
                .filter(|field| field.name == *name)
                .map(|field| field.value.as_str())
                .collect();
            record.push(values.join("\n"));
        }
        let written = writer.write_record(&record);
        record.zeroize();
        written.map_err(export_error)?;
    }
    let bytes = writer.into_inner().map_err(|error| Error::Export(error.to_string()))?;
    Ok(Zeroizing::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Icon;

    fn import(text: &str) -> (Vault, ImportSummary) {
        let mut vault = Vault::new("Test").unwrap();
        let summary = import_csv(&mut vault, text.as_bytes(), "Import").unwrap();
        (vault, summary)
    }

    fn only_entry(vault: &Vault) -> &EntryContent {
        assert_eq!(vault.entries.len(), 1);
        &vault.entries[0].content
    }

    #[test]
    fn chrome_exports_are_understood() {
        let (vault, summary) = import(
            "name,url,username,password,note\n\
             mail.example.com,https://mail.example.com/login,adria,p4ss word ,\n",
        );
        let entry = only_entry(&vault);
        assert_eq!(entry.title, "mail.example.com");
        assert_eq!(entry.password, "p4ss word ");
        assert_eq!(entry.url.as_deref(), Some("https://mail.example.com/login"));
        assert_eq!(summary.entries_imported, 1);
    }

    #[test]
    fn bitwarden_exports_keep_folders_custom_fields_and_totp() {
        let (vault, _) = import(
            "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
             Work/Servers,1,login,SSH,\"line 1\nline 2\",\"PIN: 1234\nRegion: eu\",0,,root,s3cret,JBSWY3DP\n",
        );
        let entry = only_entry(&vault);
        assert_eq!(entry.notes.as_deref(), Some("line 1\nline 2"));
        let fields: Vec<(&str, &str, bool)> = entry
            .custom_fields
            .iter()
            .map(|f| (f.name.as_str(), f.value.as_str(), f.protected))
            .collect();
        assert_eq!(
            fields,
            vec![("PIN", "1234", true), ("Region", "eu", false), ("TOTP", "JBSWY3DP", true)]
        );
        let folder = vault.entries[0].folder_id;
        assert_eq!(vault.folder_path(folder), vec!["Import", "Work", "Servers"]);
    }

    #[test]
    fn semicolons_byte_order_marks_and_keepassxc_root_groups_are_handled() {
        let (vault, _) = import(
            "\u{feff}\"Group\";\"Title\";\"Username\";\"Password\";\"URL\";\"Notes\";\"TOTP\";\"Icon\";\"Last Modified\";\"Created\"\n\
             \"Root/Internet\";\"Forum\";\"adria\";\"pw\";\"\";\"\";\"\";\"0\";\"2024-01-01\";\"2024-01-01\"\n",
        );
        let entry = &vault.entries[0];
        assert_eq!(entry.content.title, "Forum");
        assert!(entry.content.custom_fields.is_empty());
        assert_eq!(vault.folder_path(entry.folder_id), vec!["Import", "Internet"]);
    }

    #[test]
    fn rows_without_title_get_one_and_blank_rows_are_skipped() {
        let (vault, summary) = import(
            "url,username,password\n\
             https://shop.example.org/cart,adria,pw\n\
             ,,\n",
        );
        assert_eq!(only_entry(&vault).title, "shop.example.org");
        assert_eq!(summary.rows_skipped, 1);
    }

    #[test]
    fn files_without_known_columns_are_rejected() {
        let mut vault = Vault::new("Test").unwrap();
        let result = import_csv(&mut vault, b"foo,bar\n1,2\n", "Import");
        assert!(matches!(result, Err(Error::Import(_))));
    }

    #[test]
    fn exporting_and_importing_again_gives_the_same_entries() {
        let mut vault = Vault::new("Test").unwrap();
        let work = vault.add_folder(None, "Work", None).unwrap();
        let servers = vault.add_folder(Some(work), "Servers", None).unwrap();
        let content = EntryContent {
            title: "SSH, \"prod\"".into(),
            username: "root".into(),
            password: "a,b;c\"d".into(),
            url: Some("ssh://prod.example.com".into()),
            notes: Some("first line\nsecond line".into()),
            expiration_date: NaiveDate::from_ymd_opt(2030, 1, 31),
            icon: Some(Icon::Builtin("server".into())),
            custom_fields: vec![CustomField {
                name: "PIN".into(),
                value: "0000".into(),
                protected: true,
            }],
        };
        vault.add_entry(Some(servers), content.clone()).unwrap();
        vault
            .add_entry(None, EntryContent { title: "Bank".into(), ..Default::default() })
            .unwrap();

        let exported = export_csv(&vault).unwrap();
        let mut copy = Vault::new("Copy").unwrap();
        let summary = import_csv(&mut copy, &exported, "Backup").unwrap();
        assert_eq!(summary.entries_imported, 2);

        let ssh = copy.entries.iter().find(|e| e.content.title.starts_with("SSH")).unwrap();
        let expected = EntryContent { icon: None, ..content };
        assert_eq!(ssh.content, expected);
        assert_eq!(copy.folder_path(ssh.folder_id), vec!["Backup", "Work", "Servers"]);

        let again = import_csv(&mut copy, &exported, "Backup").unwrap();
        assert_eq!(again.entries_imported, 0);
        assert_eq!(again.duplicates_skipped, 2);
    }
}
