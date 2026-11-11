//! One synchronization pass: fold the remote copy into the local vault, then upload
//! the result. Every merge happens here, on the device; the server never sees plaintext
//! and never resolves conflicts (see `docs/SYNC.md` §7).

use crate::error::{Error, Result};
use crate::merge::{self, MergeReport};
use crate::models::Vault;
use crate::storage::{self, VaultKey};

use super::remote::{PutOutcome, Remote, RemoteVault};

/// What one synchronization pass did.
#[derive(Debug)]
pub struct SyncOutcome {
    /// The vault after the merge; the caller should adopt this.
    pub vault: Vault,
    /// The local vault changed, so the interface has to refresh.
    pub changed_locally: bool,
    /// Something was written to the remote.
    pub uploaded: bool,
    /// What the merge brought in.
    pub report: MergeReport,
}

fn decrypt(key: &VaultKey, remote: &RemoteVault) -> Result<Vault> {
    storage::decrypt_vault(&remote.bytes, key)
}

/// Merges the remote copy into `local` and uploads the result.
///
/// `key` is the vault's encryption key. It must match the remote's header (same
/// credentials and salt), which holds for copies of the same vault. A conflict is
/// resolved by merging the other copy and retrying once; a second conflict is
/// reported so the caller can simply sync again.
pub fn sync(remote: &dyn Remote, vault_id: &str, local: &Vault, key: &VaultKey) -> Result<SyncOutcome> {
    let remote_now = remote.get(vault_id)?;

    let mut merged = local.clone();
    let mut report = MergeReport::default();
    let mut changed_locally = false;
    let mut base: Option<u64> = None;
    let mut needs_upload = remote_now.is_none();

    if let Some(remote_vault) = &remote_now {
        let remote_plain = decrypt(key, remote_vault)?;
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
        });
    }

    let bytes = storage::encrypt_vault(&merged, key)?;
    match remote.put(vault_id, base, &bytes)? {
        PutOutcome::Stored { .. } => Ok(SyncOutcome {
            vault: merged,
            changed_locally,
            uploaded: true,
            report,
        }),
        PutOutcome::Conflict { current } => {
            // Somebody wrote first: fold their copy in and retry once.
            let remote_plain = decrypt(key, &current)?;
            let outcome = merge::merge(&merged, &remote_plain);
            let bytes = storage::encrypt_vault(&outcome.vault, key)?;
            match remote.put(vault_id, Some(current.version), &bytes)? {
                PutOutcome::Stored { .. } => Ok(SyncOutcome {
                    vault: outcome.vault,
                    changed_locally: changed_locally || outcome.local_changed,
                    uploaded: true,
                    report: outcome.report,
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

    fn key() -> VaultKey {
        VaultKey::generate(&composite_key("pw", None), KdfParams::FOR_TESTS).unwrap()
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
        let key = key();

        let base = Vault::new("Main").unwrap();
        let mut first_device = base.clone();
        first_device.add_entry(None, content("Mail", "1")).unwrap();

        let uploaded = sync(&remote, "v1", &first_device, &key).unwrap();
        assert!(uploaded.uploaded);

        // A device that only had the (older, empty) copy learns about the entry.
        let second_device = base;
        let received = sync(&remote, "v1", &second_device, &key).unwrap();
        assert!(!received.uploaded, "nothing to write back");
        assert!(received.changed_locally);
        assert_eq!(received.vault.entries.len(), 1);
        assert_eq!(received.vault.entries[0].content.title, "Mail");
    }

    #[test]
    fn edits_made_on_two_devices_are_both_kept() {
        let remote = MemoryRemote::new();
        let key = key();
        let base = Vault::new("Main").unwrap();

        let mut device_a = base.clone();
        device_a.add_entry(None, content("A", "1")).unwrap();
        let a = sync(&remote, "v1", &device_a, &key).unwrap().vault;

        let mut device_b = base.clone();
        device_b.add_entry(None, content("B", "2")).unwrap();
        let b = sync(&remote, "v1", &device_b, &key).unwrap();
        assert_eq!(b.vault.entries.len(), 2);
        assert!(b.uploaded);

        // Device A syncs again and now sees both entries.
        let a2 = sync(&remote, "v1", &a, &key).unwrap();
        assert_eq!(a2.vault.entries.len(), 2);
        assert!(a2.changed_locally);
    }

    #[test]
    fn nothing_changed_means_nothing_is_uploaded() {
        let remote = MemoryRemote::new();
        let key = key();
        let vault = Vault::new("Main").unwrap();

        sync(&remote, "v1", &vault, &key).unwrap();
        let again = sync(&remote, "v1", &vault, &key).unwrap();
        assert!(!again.uploaded);
        assert!(!again.changed_locally);
    }

    #[test]
    fn a_concurrent_write_is_merged_on_conflict() {
        // A remote that injects another device's change right before our first upload,
        // once, forcing the conflict path. The injected blob is encrypted with the same
        // key so the engine can decrypt and merge it.
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

        let key = key();
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
        // Seed a valid first version so our upload carries `base = Some(1)`.
        remote
            .inner
            .put("v1", None, &storage::encrypt_vault(&base, &key).unwrap())
            .unwrap();

        let outcome = sync(&remote, "v1", &mine, &key).unwrap();
        let titles: Vec<&str> = outcome.vault.entries.iter().map(|e| e.content.title.as_str()).collect();
        assert!(titles.contains(&"Mine"), "local change kept");
        assert!(titles.contains(&"Theirs"), "concurrent change merged");
    }
}
