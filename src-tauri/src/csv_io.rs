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

/// A file that is not UTF-8 is refused rather than half-converted: `from_utf8_lossy` turns
/// `Cañón` into `Ca\u{FFFD}\u{FFFD}n` and the password is silently lost.
fn text(raw: &[u8]) -> Result<String> {
    // A NUL byte cannot appear in UTF-8 CSV text; it means the file is UTF-16 or binary. Values with
    // hidden NULs also made fills fail later, so refuse them here (the red-team report, L1).
    if raw.contains(&0) {
        return Err(Error::Import(
            "This file is not plain UTF-8 text. Save it as CSV UTF-8 and import it again.".into(),
        ));
    }
    String::from_utf8(raw.to_vec()).map_err(|_| {
        Error::Import(
            "This file is not UTF-8 text, so accents and letters would be damaged. Re-save it as \
             UTF-8 (in Excel, choose CSV UTF-8) and import it again."
                .into(),
        )
    })
}

/// Reads the rows of the file. Returns them with the number of blank rows skipped.
fn parse(bytes: &[u8]) -> Result<(Vec<Row>, usize)> {
    // Excel's "Unicode Text" export is UTF-16. Its NUL-interleaved bytes used to import as garbage,
    // sometimes with a blank extra entry; refuse it and say what to do (the red-team report, L1).
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(Error::Import(
            "This file is UTF-16, which Lados cannot read. In Excel choose CSV UTF-8, then import it again.".into(),
        ));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    // `flexible(false)`: a row whose columns do not match the header is reported, not swallowed.
    // With `flexible(true)` an unclosed quote absorbed the rest of the file into one entry.
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(detect_delimiter(bytes))
        .flexible(false)
        .from_reader(bytes);

    // The first column of each kind wins; repeated ones are kept as custom fields.
    let mut seen = HashSet::new();
    let mut columns: Vec<Column> = Vec::new();
    {
        let headers = reader.byte_headers().map_err(import_error)?;
        for raw in headers.iter() {
            let header = text(raw)?;
            let column = classify(&header);
            columns.push(match column {
                Column::Ignored | Column::Custom(_) => column,
                _ if !seen.insert(discriminant(&column)) => Column::Custom(header.trim().to_string()),
                _ => column,
            });
        }
    }
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
            let value = Zeroizing::new(text(raw)?);
            if value.trim().is_empty() {
                continue;
            }
            match column {
                // Visible fields are read back through `unsanitize`, so a round trip leaves them
                // unchanged. The password is not: it was written out verbatim.
                Column::Title => content.title = unsanitize(value.trim()).to_string(),
                Column::Username => content.username = unsanitize(value.trim()).to_string(),
                Column::Password => content.password = value.to_string(),
                Column::Url => content.url = Some(unsanitize(value.trim()).to_string()),
                Column::Notes => content.notes = Some(unsanitize(&value.replace("\r\n", "\n")).to_string()),
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
                Column::Custom(name) => {
                    let protected = looks_secret(name);
                    content.custom_fields.push(CustomField {
                        name: name.clone(),
                        value: if protected { value.to_string() } else { unsanitize(&value).to_string() },
                        protected,
                    });
                }
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

/// A spreadsheet runs a cell that begins with `=`, `+`, `-` or `@` as a formula, and `HYPERLINK` or
/// `WEBSERVICE` can then read the rest of the sheet — including the passwords exported beside it.
/// Prefixing such a value makes it plain text. Only non-secret fields are touched: changing a
/// password would corrupt it (the red-team report, M14).
fn sanitize(value: &str) -> String {
    let dangerous = matches!(value.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r'));
    if dangerous {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// Undoes [`sanitize`] when a file is read back, so a round trip does not leave a stray apostrophe.
fn unsanitize(value: &str) -> &str {
    match value.strip_prefix('\'') {
        Some(rest) if matches!(rest.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r')) => rest,
        _ => value,
    }
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
    let mut header: Vec<String> = [
        "Group", "Title", "Username", "Password", "URL", "Notes", "Expires", "Created", "Modified",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    header.extend(field_names.iter().map(|name| sanitize(name)));
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
            sanitize(&content.title),
            sanitize(&content.username),
            // The password goes out exactly as it is: prefixing it would change it on the way back.
            content.password.clone(),
            sanitize(content.url.as_deref().unwrap_or_default()),
            sanitize(content.notes.as_deref().unwrap_or_default()),
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
            // A protected custom field is a secret as well: leave it exactly as it is.
            let all_protected = content
                .custom_fields
                .iter()
                .filter(|field| field.name == *name)
                .all(|field| field.protected);
            let joined = values.join("\n");
            record.push(if all_protected { joined } else { sanitize(&joined) });
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
    fn files_without_known_columns_are_rejected() {        let mut vault = Vault::new("Test").unwrap();
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

    /// A Latin-1 export used to import silently as `Ca\u{FFFD}n`, losing the accented letters — and
    /// the password with them. It must be refused instead.
    #[test]
    fn a_file_that_is_not_utf8_is_refused_instead_of_garbled() {
        let latin1 = b"Title,Username,Password\nCa\xF1on,jos\xE9,secret\n";
        assert!(matches!(parse(latin1), Err(Error::Import(_))));
        // The same content as UTF-8 is fine.
        assert!(parse("Title,Username,Password\nCañón,josé,secret\n".as_bytes()).is_ok());
    }

    /// An unclosed quote used to absorb the rest of the file into one garbage entry.
    #[test]
    fn an_unclosed_quote_is_reported_instead_of_swallowed() {        let malformed = b"Title,Username,Password\n\"unclosed,quote,pw\nmore,rows,here\n";
        let result = parse(malformed);
        assert!(result.is_err(), "a malformed file must be reported, not imported");
        assert!(matches!(result, Err(Error::Import(_))));
    }

    /// A cell a spreadsheet would run as a formula is neutralised on export, and restored on
    /// import — but a password is written and read back exactly (the red-team report, M14).
    #[test]
    fn formula_like_text_is_neutralised_on_export_and_restored_on_import() {
        let title = "=HYPERLINK(\"http://evil\",\"x\")";
        let username = "+cmd|' /C calc'!A0";

        let mut vault = Vault::new("Test").unwrap();
        let mut entry = EntryContent {
            title: title.into(),
            username: username.into(),
            password: "=1+1".into(),
            ..Default::default()
        };
        entry.custom_fields.push(CustomField { name: "Note2".into(), value: "@SUM(1)".into(), protected: false });
        entry.custom_fields.push(CustomField { name: "PIN".into(), value: "=1+1".into(), protected: true });
        vault.add_entry(None, entry).unwrap();

        let exported = String::from_utf8(export_csv(&vault).unwrap().to_vec()).unwrap();
        assert!(exported.contains("'=HYPERLINK"), "the title is neutralised: {exported}");
        assert!(exported.contains("'+cmd"), "the username is neutralised");
        assert!(exported.contains("'@SUM(1)"), "the unprotected custom field is neutralised");
        assert!(exported.contains(",=1+1,"), "the password is left exactly as it is");

        let mut copy = Vault::new("Copy").unwrap();
        import_csv(&mut copy, exported.as_bytes(), "Backup").unwrap();
        assert_eq!(copy.entries[0].content.title, title);
        assert_eq!(copy.entries[0].content.username, username);
        assert_eq!(copy.entries[0].content.password, "=1+1");
        assert!(copy.entries[0].content.custom_fields.iter().any(|f| f.name == "Note2" && f.value == "@SUM(1)"));
    }
}
