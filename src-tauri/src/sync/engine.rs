//! One synchronization pass: fold the remote copy into the local vault, then upload
//! the result. Every merge happens here, on the device; the server never sees plaintext
//! and never resolves conflicts (see `docs/SYNC.md` §7).
//!
//! The encryption key is derived from the **header stored in the encrypted blob**, so a
//! brand-new device (empty vault under the same master password) can decrypt and adopt
//! the vault's salt simply by syncing.

use crate::crypto::SecretKey;
use crate::error::{Error, Result};
use crate::merge::{self, MergeReport};
use crate::models::Vault;
use crate::storage::{self, Header, VaultKey};

use super::remote::{PutOutcome, Remote, RemoteVault};

/// What one synchronization pass did.
pub struct SyncOutcome {
    /// The vault after the merge; the caller should adopt this.
    pub vault: Vault,
    /// The local vault changed, so the interface has to refresh.
    pub changed_locally: bool,
    /// Something was written to the remote.
    pub uploaded: bool,
    /// What the merge brought in.
    pub report: MergeReport,
    /// The key matching the remote header. The caller adopts it so the local file and
    /// the server converge on one salt.
    pub key: VaultKey,
}

/// Derives the vault key for a given encrypted blob, from its header and the master
/// credentials. This is what lets any device open the synced vault.
fn key_for(composite: &SecretKey, bytes: &[u8]) -> Result<VaultKey> {
    let header = Header::parse(bytes)?;
    VaultKey::for_header(composite, &header)
}

fn decrypt(key: &VaultKey, remote: &RemoteVault) -> Result<Vault> {
    storage::decrypt_vault(&remote.bytes, key)
}

/// Merges the remote copy into `local` and uploads the result.
///
/// `composite` is the master credential, used to derive keys for whatever header the
/// remote carries. `local_key` is this device's current key, used for the very first
/// upload (its salt then becomes the vault's canonical salt).
pub fn sync(
    remote: &dyn Remote,
    vault_id: &str,
    local: &Vault,
    composite: &SecretKey,
    local_key: &VaultKey,
) -> Result<SyncOutcome> {
    let remote_now = remote.get(vault_id)?;

    let mut merged = local.clone();
    let mut report = MergeReport::default();
    let mut changed_locally = false;
    let mut base: Option<u64> = None;
    let mut needs_upload = remote_now.is_none();
    let mut key = local_key.clone();

    if let Some(remote_vault) = &remote_now {
        key = key_for(composite, &remote_vault.bytes)?;
        let remote_plain = decrypt(&key, remote_vault)?;
        let outcome = merge::merge(local, &remote_plain);
        merged = outcome.vault;
        report = outcome.report;
        changed_locally = outcome.local_changed;
        needs_upload = outcome.remote_outdated;
        base = Some(remote_vault.version);
    }

    if !needs_upload {
        return Ok(SyncOutcome {
            vault: merged,
            changed_locally,
            uploaded: false,
            report,
            key,
        });
    }

    let bytes = storage::encrypt_vault(&merged, &key)?;
    match remote.put(vault_id, base, &bytes)? {
        PutOutcome::Stored { .. } => Ok(SyncOutcome {
            vault: merged,
            changed_locally,
            uploaded: true,
            report,
            key,
        }),
        PutOutcome::Conflict { current } => {
            // Somebody wrote first: their copy may carry a different header, so derive
            // its key, fold it in, and retry once.
            let current_key = key_for(composite, &current.bytes)?;
            let remote_plain = decrypt(&current_key, &current)?;
            let outcome = merge::merge(&merged, &remote_plain);
            let bytes = storage::encrypt_vault(&outcome.vault, &current_key)?;
            match remote.put(vault_id, Some(current.version), &bytes)? {
                PutOutcome::Stored { .. } => Ok(SyncOutcome {
                    vault: outcome.vault,
                    changed_locally: changed_locally || outcome.local_changed,
                    uploaded: true,
                    report: outcome.report,
                    key: current_key,
                }),
                PutOutcome::Conflict { .. } => Err(Error::invalid(
                    "The vault changed again while syncing; please retry",
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{composite_key, KdfParams};
    use crate::models::EntryContent;

    use super::super::remote::MemoryRemote;

    fn credentials() -> (SecretKey, VaultKey) {
        let composite = composite_key("pw", None);
        let key = VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap();
        (composite, key)
    }

    fn content(title: &str, password: &str) -> EntryContent {
        EntryContent {
            title: title.into(),
            password: password.into(),
            ..Default::default()
        }
    }

    #[test]
    fn uploads_the_first_copy_and_a_second_device_receives_it() {
        let remote = MemoryRemote::new();
        let (composite, key) = credentials();

        let base = Vault::new("Main").unwrap();
        let mut first_device = base.clone();
        first_device.add_entry(None, content("Mail", "1")).unwrap();

        let uploaded = sync(&remote, "v1", &first_device, &composite, &key).unwrap();
        assert!(uploaded.uploaded);

        // A device that only had the (older, empty) copy learns about the entry.
        let received = sync(&remote, "v1", &base, &composite, &key).unwrap();
        assert!(!received.uploaded, "nothing to write back");
        assert!(received.changed_locally);
        assert_eq!(received.vault.entries.len(), 1);
        assert_eq!(received.vault.entries[0].content.title, "Mail");
    }

    #[test]
    fn a_brand_new_device_adopts_the_remote_salt() {
        let remote = MemoryRemote::new();
        let (composite, key_a) = credentials();

        let base = Vault::new("Main").unwrap();
        let mut device_a = base.clone();
        device_a.add_entry(None, content("Mail", "1")).unwrap();
        sync(&remote, "v1", &device_a, &composite, &key_a).unwrap();

        // Device B has its own (different) salt but the same master password.
        let key_b = VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap();
        assert_ne!(key_a.salt, key_b.salt);

        let received = sync(&remote, "v1", &base, &composite, &key_b).unwrap();
        assert_eq!(received.vault.entries.len(), 1, "it could decrypt the remote copy");
        assert_eq!(received.key.salt, key_a.salt, "and adopted the vault's salt");
    }

    #[test]
    fn edits_made_on_two_devices_are_both_kept() {
        let remote = MemoryRemote::new();
        let (composite, key) = credentials();
        let base = Vault::new("Main").unwrap();

        let mut device_a = base.clone();
        device_a.add_entry(None, content("A", "1")).unwrap();
        let a = sync(&remote, "v1", &device_a, &composite, &key).unwrap().vault;

        let mut device_b = base.clone();
        device_b.add_entry(None, content("B", "2")).unwrap();
        let b = sync(&remote, "v1", &device_b, &composite, &key).unwrap();
        assert_eq!(b.vault.entries.len(), 2);
        assert!(b.uploaded);

        let a2 = sync(&remote, "v1", &a, &composite, &key).unwrap();
        assert_eq!(a2.vault.entries.len(), 2);
        assert!(a2.changed_locally);
    }

    #[test]
    fn nothing_changed_means_nothing_is_uploaded() {
        let remote = MemoryRemote::new();
        let (composite, key) = credentials();
        let vault = Vault::new("Main").unwrap();

        sync(&remote, "v1", &vault, &composite, &key).unwrap();
        let again = sync(&remote, "v1", &vault, &composite, &key).unwrap();
        assert!(!again.uploaded);
        assert!(!again.changed_locally);
    }

    #[test]
    fn a_concurrent_write_is_merged_on_conflict() {
        struct RacingRemote {
            inner: MemoryRemote,
            other_bytes: Vec<u8>,
            injected: std::cell::Cell<bool>,
        }
        impl Remote for RacingRemote {
            fn get(&self, id: &str) -> Result<Option<RemoteVault>> {
                self.inner.get(id)
            }
            fn put(&self, id: &str, base: Option<u64>, bytes: &[u8]) -> Result<PutOutcome> {
                if !self.injected.get()
                    && let Some(version) = base
                {
                    self.injected.set(true);
                    self.inner.put(id, Some(version), &self.other_bytes)?;
                }
                self.inner.put(id, base, bytes)
            }
        }

        let (composite, key) = credentials();
        let base = Vault::new("Main").unwrap();

        let mut mine = base.clone();
        mine.add_entry(None, content("Mine", "1")).unwrap();
        let mut theirs = base.clone();
        theirs.add_entry(None, content("Theirs", "2")).unwrap();

        let remote = RacingRemote {
            inner: MemoryRemote::new(),
            other_bytes: storage::encrypt_vault(&theirs, &key).unwrap(),
            injected: std::cell::Cell::new(false),
        };
        remote
            .inner
            .put("v1", None, &storage::encrypt_vault(&base, &key).unwrap())
            .unwrap();

        let outcome = sync(&remote, "v1", &mine, &composite, &key).unwrap();
        let titles: Vec<&str> = outcome.vault.entries.iter().map(|e| e.content.title.as_str()).collect();
        assert!(titles.contains(&"Mine"), "local change kept");
        assert!(titles.contains(&"Theirs"), "concurrent change merged");
    }
}
