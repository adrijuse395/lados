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
    /// The file had been deleted or moved away, and was written again at the same path.
    pub missing: bool,
}

pub enum SyncOutcome {
    Unchanged,
    /// Changes saved by another window were merged into this one.
    Merged { report: MergeReport, local_changed: bool },
    /// The file can no longer be read: this window now works on the copy at this path.
    Diverged(PathBuf),
    /// The file is gone: deleted, renamed or moved while the vault was open.
    Missing,
}

/// How the file differs from what this window last read or wrote.
enum DiskChange {
    None,
    /// Same content with a new timestamp.
    Touched(Option<FileStamp>),
    /// The file is gone: deleted, renamed or moved while this vault was open.
    Missing,
    Readable {
        remote: Box<Zeroizing<Vault>>,
        hash: [u8; 32],
        stamp: Option<FileStamp>,
        /// Another window re-encrypted the file with the same credentials but a new salt or
        /// cost. This window must adopt that key, or its next save would undo the change.
        /// Boxed: `VaultKey` is far larger than the other variants, and this value is local and
        /// short-lived.
        new_key: Option<Box<VaultKey>>,
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
        // The file is gone: someone deleted or moved it. That is not "nothing happened" — the user
        // is told (see `vault-missing`), because a deliberate deletion was being quietly undone.
        if stamp.is_none() {
            return Ok(DiskChange::Missing);
        }
        if trust_stamp && stamp == self.disk_stamp {
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
            match VaultKey::for_header(&self.composite, &header, &bytes) {
                Ok(key) => Some(Box::new(key)),
                Err(Error::Corrupted) => return Ok(DiskChange::Unreadable),
                Err(other) => return Err(other),
            }
        };
        match storage::decrypt_vault(&bytes, new_key.as_deref().unwrap_or(&self.key)) {
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
            DiskChange::Missing => outcome.missing = true,
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

        let key = new_key.as_ref().map(|(_, key)| key).or(adopted.as_deref()).unwrap_or(&self.key);
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
            self.key = *key;
        }
        self.remember(&bytes);
        Ok(outcome)
    }
}

pub struct OpenVault {
    vault: Vault,
    file: VaultFile,
    /// Bumped every time the in-memory vault changes. A cloud sync clones the vault, does its
    /// network round trip without holding the session lock, and adapts this before adopting the
    /// result, so a change the user made in the meantime is never overwritten (code review).
    generation: u64,
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
            generation: 0,
        })
    }

    /// Opens an existing vault. Slow: derives the key.
    pub fn open(path: PathBuf, composite: SecretKey) -> Result<Self> {
        let bytes = storage::read_file(&path)?;
        let header = Header::parse(&bytes)?;
        let key = VaultKey::for_header(&composite, &header, &bytes)?;
        let vault = storage::decrypt_vault(&bytes, &key)?;
        Ok(OpenVault {
            vault,
            file: VaultFile::new(path, composite, key, &bytes, false),
            generation: 0,
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

    /// The key the vault is currently encrypted with.
    pub fn key(&self) -> &VaultKey {
        &self.file.key
    }

    /// A counter that changes with every modification of the in-memory vault, so a slow task can
    /// tell whether the vault changed underneath it (code review).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Re-encrypts the vault for new credentials, keeping any recovery key working.
    pub fn rekey(&mut self, composite: SecretKey, kdf: KdfParams) -> Result<SaveOutcome> {
        let key = self.file.key.rewrap(&composite, kdf)?;
        self.change_key(composite, key)
    }

    /// Applies a change and saves it. If the change or the save fails, the vault in memory and
    /// the file both stay exactly as they were.
    pub fn modify<T>(&mut self, change: impl FnOnce(&mut Vault) -> Result<T>) -> Result<(T, SaveOutcome)> {
        let mut draft = Zeroizing::new(self.vault.clone());
        let value = change(&mut draft)?;
        let outcome = self.file.save(&mut draft, None)?;
        std::mem::swap(&mut self.vault, &mut *draft);
        self.generation += 1;
        Ok((value, outcome))
    }

    /// Re-encrypts the vault with new credentials. `key` must come from `VaultKey::generate`
    /// with the same `composite`; it is derived beforehand because that step is slow.
    pub fn change_key(&mut self, composite: SecretKey, key: VaultKey) -> Result<SaveOutcome> {
        let mut draft = Zeroizing::new(self.vault.clone());
        let outcome = self.file.save(&mut draft, Some((composite, key)))?;
        std::mem::swap(&mut self.vault, &mut *draft);
        self.generation += 1;
        Ok(outcome)
    }

    /// Replaces the vault with `vault` and saves it under new credentials. Used when a vault is
    /// re-encrypted with a new master password during a coordinated password change.
    pub fn adopt(&mut self, vault: Vault, composite: SecretKey, key: VaultKey) -> Result<SaveOutcome> {
        let mut draft = Zeroizing::new(vault);
        let outcome = self.file.save(&mut draft, Some((composite, key)))?;
        std::mem::swap(&mut self.vault, &mut *draft);
        self.generation += 1;
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

    /// Folds the remote copy into this vault (via the sync engine) and, if the local
    /// vault changed or the vault's salt must be adopted, saves it to disk. The
    /// encryption key here never leaves the device.
    #[cfg(feature = "sync")]
    pub fn sync_remote(
        &mut self,
        remote: &dyn crate::sync::Remote,
        vault_id: &str,
        last_seen: Option<u64>,
    ) -> Result<(MergeReport, u64)> {
        let composite = self.file.composite.clone();
        let key = self.file.key.clone();
        let outcome = crate::sync::sync(remote, vault_id, &self.vault, &composite, &key, last_seen)?;
        self.adopt_sync(outcome)
    }

    /// Adopts the result of a sync pass and saves the merged vault if it changed.
    ///
    /// Separate from `sync_remote` so a caller can run the network round trip and the key
    /// derivation with the session lock released, then take it again only to save (code review).
    #[cfg(feature = "sync")]
    pub fn adopt_sync(&mut self, outcome: crate::sync::SyncOutcome) -> Result<(MergeReport, u64)> {
        let crate::sync::SyncOutcome {
            vault,
            changed_locally,
            key,
            report,
            version,
            ..
        } = outcome;
        let key_changed = key.salt != self.file.key.salt || key.kdf != self.file.key.kdf;
        if changed_locally || key_changed {
            // Adopt the remote header so every device converges on one salt.
            let mut draft = Zeroizing::new(vault);
            let composite = self.file.composite.clone();
            self.file.save(&mut draft, Some((composite, key)))?;
            std::mem::swap(&mut self.vault, &mut *draft);
            self.generation += 1;
        }
        Ok((report, version))
    }

    /// Picks up changes saved by other windows. Called periodically.
    pub fn sync(&mut self) -> Result<SyncOutcome> {
        match self.file.check_disk(true)? {
            DiskChange::None => Ok(SyncOutcome::Unchanged),
            DiskChange::Touched(stamp) => {
                self.file.disk_stamp = stamp;
                Ok(SyncOutcome::Unchanged)
            }
            DiskChange::Missing => Ok(SyncOutcome::Missing),
            DiskChange::Readable {
                remote,
                hash,
                stamp,
                new_key,
            } => {
                if let Some(key) = new_key {
                    self.file.key = *key;
                }
                let merged = merge::merge(&self.vault, &remote);
                let mut combined = Zeroizing::new(merged.vault);
                if merged.remote_outdated {
                    // The file lacks something this window has: write the combined version back.
                    let saved = self.file.save(&mut combined, None)?;
                    if let Some(path) = saved.conflict_copy {
                        std::mem::swap(&mut self.vault, &mut *combined);
                        self.generation += 1;
                        return Ok(SyncOutcome::Diverged(path));
                    }
                } else {
                    self.file.disk_hash = hash;
                    self.file.disk_stamp = stamp;
                }
                std::mem::swap(&mut self.vault, &mut *combined);
                self.generation += 1;
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
    let key = VaultKey::for_header(composite, &header, &bytes)?;
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
    fn the_generation_advances_only_when_the_vault_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut open = create(dir.path(), "pw");
        let start = open.generation();

        open.modify(|vault| vault.add_entry(None, content("Mail"))).unwrap();
        assert!(open.generation() > start);

        // A change that fails leaves the vault, and the generation, exactly as they were.
        let after = open.generation();
        assert!(open.modify(|vault| vault.add_entry(None, content("  "))).is_err());
        assert_eq!(open.generation(), after);
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
    fn a_recovery_key_created_for_an_open_vault_reopens_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.lados");
        let composite = composite_key("pw", None);
        // Created in one session, then reopened, as the app does before a recovery key is made.
        {
            let mut open = OpenVault::create(path.clone(), "Test", composite.clone(), KdfParams::FOR_TESTS).unwrap();
            open.modify(|vault| vault.add_entry(None, content("Mail"))).unwrap();
        }
        let mut open = OpenVault::open(path.clone(), composite.clone()).unwrap();

        // Exactly what `create_recovery_key` does: give the vault a wrap, then save it again.
        let code = [3u8; 32];
        let key = open.key().with_recovery(&composite, &code).unwrap();
        open.change_key(composite.clone(), key).unwrap();

        // The file is version 2 with a recovery wrap, and the previous one was kept.
        let bytes = std::fs::read(&path).unwrap();
        let header = Header::parse(&bytes).unwrap();
        assert_eq!(header.version, 2);
        assert!(header.has_recovery());
        assert!(dir.path().join("vault.lados.bak").exists());

        // The password still opens it, and so does the code on its own.
        let with_password = VaultKey::for_header(&composite, &header, &bytes).unwrap();
        assert_eq!(storage::decrypt_vault(&bytes, &with_password).unwrap().entries.len(), 1);
        let recovered = storage::key_from_recovery(&header, &bytes, &code).unwrap();
        assert_eq!(storage::decrypt_vault(&bytes, &recovered).unwrap().entries.len(), 1);
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

    /// A file deleted while the vault is open must not be recreated in silence: the poll reports it
    /// and a later save says so too (the interface shows `vault-missing`).
    #[test]
    fn a_deleted_file_is_reported_instead_of_being_recreated_in_silence() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "pw").path().to_path_buf();
        let mut open = OpenVault::open(path.clone(), composite_key("pw", None)).unwrap();

        std::fs::remove_file(&path).unwrap();
        assert!(matches!(open.sync().unwrap(), SyncOutcome::Missing));

        let (_, outcome) = open.modify(|vault| vault.add_entry(None, content("After delete"))).unwrap();
        assert!(outcome.missing, "the caller must be told the file had gone");
        assert!(path.is_file(), "the change is still saved, at the same path");
    }
}
