//! An unlocked vault: the decrypted data plus everything needed to save it safely and to stay
//! in sync with other Lados windows that have the same file open.
//!
//! Every change is saved immediately. Before writing, the file is read again: if another window
//! saved something in the meantime, both versions are merged first, so neither loses work.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{KdfParams, SecretKey, sha256};
use crate::error::{Error, Result};
use crate::merge::{self, MergeReport};
use crate::models::Vault;
use crate::storage::{self, FileStamp, Header, VaultKey};

/// Something that happened while saving that the user should know about.
#[derive(Debug, Default)]
pub struct SaveOutcome {
    /// Changes saved by another Lados window were merged in first.
    pub merged: Option<MergeReport>,
    /// The file had been replaced by one this window cannot read (another master key, a newer
    /// version of Lados...). It was left untouched and this window's vault was saved next to it.
    pub conflict_copy: Option<PathBuf>,
}

pub enum SyncOutcome {
    Unchanged,
    /// Changes saved by another window were merged into this one.
    Merged { report: MergeReport, local_changed: bool },
    /// The file can no longer be read: this window now works on the copy at this path.
    Diverged(PathBuf),
}

/// How the file differs from what this window last read or wrote.
enum DiskChange {
    None,
    /// Same content with a new timestamp.
    Touched(Option<FileStamp>),
    Readable {
        remote: Box<Zeroizing<Vault>>,
        hash: [u8; 32],
        stamp: Option<FileStamp>,
        /// Another window re-encrypted the file with the same credentials but a new salt or
        /// cost. This window must adopt that key, or its next save would undo the change.
        new_key: Option<VaultKey>,
    },
    Unreadable,
}

fn replace(target: &mut Vault, new: Vault) {
    let mut old = std::mem::replace(target, new);
    old.zeroize();
}

/// The file side of an open vault: where it lives, how to encrypt it and what it last contained.
struct VaultFile {
    path: PathBuf,
    composite: SecretKey,
    key: VaultKey,
    disk_hash: [u8; 32],
    disk_stamp: Option<FileStamp>,
    /// A copy of the original file is kept once per session, before the first save.
    backup_done: bool,
}

impl VaultFile {
    fn new(path: PathBuf, composite: SecretKey, key: VaultKey, bytes: &[u8], backup_done: bool) -> Self {
        let mut file = VaultFile {
            path,
            composite,
            key,
            disk_hash: [0; 32],
            disk_stamp: None,
            backup_done,
        };
        file.remember(bytes);
        file
    }

    fn remember(&mut self, bytes: &[u8]) {
        self.disk_hash = sha256(bytes);
        self.disk_stamp = storage::file_stamp(&self.path);
    }

    /// `trust_stamp` skips reading the file when its size and date have not changed. Polling
    /// uses it to stay cheap; saving does not, because some file systems (FAT32 on USB drives)
    /// only record the time with a two-second precision.
    fn check_disk(&self, trust_stamp: bool) -> Result<DiskChange> {
        let stamp = storage::file_stamp(&self.path);
        // A missing file is simply written again by the next save.
        if stamp.is_none() || (trust_stamp && stamp == self.disk_stamp) {
            return Ok(DiskChange::None);
        }
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(DiskChange::None),
            Err(error) => return Err(error.into()),
        };
        let hash = sha256(&bytes);
        if hash == self.disk_hash {
            return Ok(DiskChange::Touched(stamp));
        }

        let Ok(header) = Header::parse(&bytes) else {
            return Ok(DiskChange::Unreadable);
        };
        let new_key = if self.key.matches(&header) {
            None
        } else {
            match VaultKey::for_header(&self.composite, &header) {
                Ok(key) => Some(key),
                Err(Error::Corrupted) => return Ok(DiskChange::Unreadable),
                Err(other) => return Err(other),
            }
        };
        match storage::decrypt_vault(&bytes, new_key.as_ref().unwrap_or(&self.key)) {
            Ok(remote) => Ok(DiskChange::Readable {
                remote: Box::new(Zeroizing::new(remote)),
                hash,
                stamp,
                new_key,
            }),
            Err(Error::InvalidCredentials | Error::Corrupted | Error::NotAVault | Error::UnsupportedVersion(_)) => {
                Ok(DiskChange::Unreadable)
            }
            Err(other) => Err(other),
        }
    }

    /// Writes `vault`, merging into it first whatever another window saved since we last looked.
    /// With `new_key`, the file is re-encrypted with it. Nothing changes in `self` unless the
    /// write succeeds.
    fn save(&mut self, vault: &mut Vault, new_key: Option<(SecretKey, VaultKey)>) -> Result<SaveOutcome> {
        let mut outcome = SaveOutcome::default();
        let mut target = self.path.clone();
        let mut adopted = None;
        match self.check_disk(false)? {
            DiskChange::None | DiskChange::Touched(_) => {}
            DiskChange::Readable { remote, new_key: disk_key, .. } => {
                let merged = merge::merge(vault, &remote);
                if merged.local_changed {
                    outcome.merged = Some(merged.report);
                }
                replace(vault, merged.vault);
                adopted = disk_key;
            }
            DiskChange::Unreadable => target = storage::conflict_copy_path(&self.path),
        }

        let key = new_key.as_ref().map(|(_, key)| key).or(adopted.as_ref()).unwrap_or(&self.key);
        let bytes = storage::encrypt_vault(vault, key)?;
        let diverged = target != self.path;
        if !self.backup_done && !diverged {
            storage::backup(&self.path)?;
        }
        storage::write_atomic(&target, &bytes)?;

        if diverged {
            outcome.conflict_copy = Some(target.clone());
            self.path = target;
        }
        self.backup_done = true;
        if let Some((composite, key)) = new_key {
            self.composite = composite;
            self.key = key;
        } else if let Some(key) = adopted {
            self.key = key;
        }
        self.remember(&bytes);
        Ok(outcome)
    }
}

pub struct OpenVault {
    vault: Vault,
    file: VaultFile,
}

impl Drop for OpenVault {
    fn drop(&mut self) {
        self.vault.zeroize();
    }
}

impl OpenVault {
    /// Creates a new, empty vault file. Slow: derives the key.
    pub fn create(path: PathBuf, name: &str, composite: SecretKey, kdf: KdfParams) -> Result<Self> {
        let vault = Vault::new(name)?;
        let key = VaultKey::generate(&composite, kdf)?;
        let bytes = storage::encrypt_vault(&vault, &key)?;
        storage::write_atomic(&path, &bytes)?;
        Ok(OpenVault {
            vault,
            file: VaultFile::new(path, composite, key, &bytes, true),
        })
    }

    /// Opens an existing vault. Slow: derives the key.
    pub fn open(path: PathBuf, composite: SecretKey) -> Result<Self> {
        let bytes = storage::read_file(&path)?;
        let header = Header::parse(&bytes)?;
        let key = VaultKey::for_header(&composite, &header)?;
        let vault = storage::decrypt_vault(&bytes, &key)?;
        Ok(OpenVault {
            vault,
            file: VaultFile::new(path, composite, key, &bytes, false),
        })
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// The hashed credentials, to try them on another vault file.
    pub fn composite(&self) -> SecretKey {
        self.file.composite.clone()
    }

    /// Applies a change and saves it. If the change or the save fails, the vault in memory and
    /// the file both stay exactly as they were.
    pub fn modify<T>(&mut self, change: impl FnOnce(&mut Vault) -> Result<T>) -> Result<(T, SaveOutcome)> {
        let mut draft = Zeroizing::new(self.vault.clone());
        let value = change(&mut draft)?;
        let outcome = self.file.save(&mut draft, None)?;
        std::mem::swap(&mut self.vault, &mut *draft);
        Ok((value, outcome))
    }

    /// Re-encrypts the vault with new credentials. `key` must come from `VaultKey::generate`
    /// with the same `composite`; it is derived beforehand because that step is slow.
    pub fn change_key(&mut self, composite: SecretKey, key: VaultKey) -> Result<SaveOutcome> {
        let mut draft = Zeroizing::new(self.vault.clone());
        let outcome = self.file.save(&mut draft, Some((composite, key)))?;
        std::mem::swap(&mut self.vault, &mut *draft);
        Ok(outcome)
    }

    /// Merges another vault into this one and saves the result.
    pub fn merge_from(&mut self, other: &Vault) -> Result<(MergeReport, SaveOutcome)> {
        self.modify(|vault| {
            let merged = merge::merge(vault, other);
            replace(vault, merged.vault);
            Ok(merged.report)
        })
    }

    /// Picks up changes saved by other windows. Called periodically.
    pub fn sync(&mut self) -> Result<SyncOutcome> {
        match self.file.check_disk(true)? {
            DiskChange::None => Ok(SyncOutcome::Unchanged),
            DiskChange::Touched(stamp) => {
                self.file.disk_stamp = stamp;
                Ok(SyncOutcome::Unchanged)
            }
            DiskChange::Readable {
                remote,
                hash,
                stamp,
                new_key,
            } => {
                if let Some(key) = new_key {
                    self.file.key = key;
                }
                let merged = merge::merge(&self.vault, &remote);
                let mut combined = Zeroizing::new(merged.vault);
                if merged.remote_outdated {
                    // The file lacks something this window has: write the combined version back.
                    let saved = self.file.save(&mut combined, None)?;
                    if let Some(path) = saved.conflict_copy {
                        std::mem::swap(&mut self.vault, &mut *combined);
                        return Ok(SyncOutcome::Diverged(path));
                    }
                } else {
                    self.file.disk_hash = hash;
                    self.file.disk_stamp = stamp;
                }
                std::mem::swap(&mut self.vault, &mut *combined);
                Ok(SyncOutcome::Merged {
                    report: merged.report,
                    local_changed: merged.local_changed,
                })
            }
            DiskChange::Unreadable => {
                // Never overwrite a file that cannot be read: keep working on a copy next to it.
                let mut copy = Zeroizing::new(self.vault.clone());
                let saved = self.file.save(&mut copy, None)?;
                Ok(match saved.conflict_copy {
                    Some(path) => SyncOutcome::Diverged(path),
                    None => SyncOutcome::Unchanged,
                })
            }
        }
    }
}

/// Decrypts another vault file, to merge it into the open one. Slow: derives the key.
pub fn read_other(path: &Path, composite: &SecretKey) -> Result<Zeroizing<Vault>> {
    let bytes = storage::read_file(path)?;
    let header = Header::parse(&bytes)?;
    let key = VaultKey::for_header(composite, &header)?;
    Ok(Zeroizing::new(storage::decrypt_vault(&bytes, &key)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::composite_key;
    use crate::models::EntryContent;

    fn content(title: &str) -> EntryContent {
        EntryContent {
            title: title.into(),
            password: "pw".into(),
            ..Default::default()
        }
    }

    fn create(dir: &Path, password: &str) -> OpenVault {
        OpenVault::create(dir.join("vault.lados"), "Test", composite_key(password, None), KdfParams::FOR_TESTS)
            .unwrap()
    }

    #[test]
    fn changes_are_saved_and_a_backup_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "pw").path().to_path_buf();

        let mut open = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();
        open.modify(|vault| vault.add_entry(None, content("Mail"))).unwrap();
        assert!(dir.path().join("vault.lados.bak").exists());

        let reopened = OpenVault::open(path, composite_key("pw", None)).unwrap();
        assert_eq!(reopened.vault().entries.len(), 1);
    }

    #[test]
    fn a_failed_change_leaves_memory_and_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut open = create(dir.path(), "pw");
        let before = std::fs::read(open.path()).unwrap();

        assert!(open.modify(|vault| vault.add_entry(None, content("  "))).is_err());
        assert!(open.vault().entries.is_empty());
        assert_eq!(std::fs::read(open.path()).unwrap(), before);
    }

    #[test]
    fn two_windows_on_the_same_file_see_each_others_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "pw").path().to_path_buf();
        let mut a = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();
        let mut b = OpenVault::open(path, composite_key("pw", None)).unwrap();

        a.modify(|vault| vault.add_entry(None, content("From A"))).unwrap();
        let (_, outcome) = b.modify(|vault| vault.add_entry(None, content("From B"))).unwrap();
        assert_eq!(outcome.merged.unwrap().entries_added, 1);
        assert_eq!(b.vault().entries.len(), 2);

        match a.sync().unwrap() {
            SyncOutcome::Merged { local_changed, .. } => assert!(local_changed),
            _ => panic!("window A should have merged B's entry"),
        }
        assert_eq!(a.vault().entries.len(), 2);
        assert!(matches!(a.sync().unwrap(), SyncOutcome::Unchanged));
    }

    #[test]
    fn a_file_re_encrypted_elsewhere_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "old").path().to_path_buf();
        let mut a = OpenVault::open(path.clone(), composite_key("old", None)).unwrap();
        let mut b = OpenVault::open(path.clone(), composite_key("old", None)).unwrap();

        let new_composite = composite_key("new", None);
        let new_key = VaultKey::generate(&new_composite, KdfParams::FOR_TESTS).unwrap();
        a.change_key(new_composite, new_key).unwrap();

        let copy = match b.sync().unwrap() {
            SyncOutcome::Diverged(copy) => copy,
            _ => panic!("window B should have moved to a copy"),
        };
        assert_ne!(copy, path);
        assert_eq!(b.path(), copy);
        assert!(OpenVault::open(path.clone(), composite_key("new", None)).is_ok());
        assert!(OpenVault::open(copy, composite_key("old", None)).is_ok());
    }

    #[test]
    fn a_new_key_with_the_same_credentials_is_adopted_by_other_windows() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "pw").path().to_path_buf();
        let mut a = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();
        let mut b = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();
        let mut c = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();

        // Window A re-encrypts the file with the same password and a higher cost.
        let stronger = KdfParams {
            iterations: 2,
            ..KdfParams::FOR_TESTS
        };
        let key = VaultKey::generate(&composite_key("pw", None), stronger).unwrap();
        a.change_key(composite_key("pw", None), key).unwrap();

        // Window B notices it while polling, window C only when it saves. Neither undoes it.
        assert!(matches!(b.sync().unwrap(), SyncOutcome::Merged { .. }));
        b.modify(|vault| vault.add_entry(None, content("From B"))).unwrap();
        c.modify(|vault| vault.add_entry(None, content("From C"))).unwrap();
        let header = Header::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(header.kdf, stronger);
        assert_eq!(c.vault().entries.len(), 2);
    }

    #[test]
    fn other_vault_files_can_be_merged_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut main = create(dir.path(), "pw");
        let other_path = dir.path().join("other.lados");
        let mut other =
            OpenVault::create(other_path.clone(), "Other", composite_key("pw2", None), KdfParams::FOR_TESTS).unwrap();
        other.modify(|vault| vault.add_entry(None, content("Shared"))).unwrap();

        assert!(matches!(
            read_other(&other_path, &main.composite()),
            Err(Error::InvalidCredentials)
        ));
        let remote = read_other(&other_path, &composite_key("pw2", None)).unwrap();
        let (report, _) = main.merge_from(&remote).unwrap();
        assert_eq!(report.entries_added, 1);
        assert_eq!(main.vault().entries.len(), 1);
    }
}
