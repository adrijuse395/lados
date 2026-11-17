//! One synchronization pass: fold the remote copy into the local vault, then upload
//! the result. Every merge happens here, on the device; the server never sees plaintext
//! and never resolves conflicts (see `docs/SYNC.md` §7).
//!
//! The encryption key is derived from the **header stored in the encrypted blob**, so a
//! brand-new device (empty vault under the same master password) can decrypt and adopt
//! the vault's salt simply by syncing.

use crate::crypto::{KdfParams, SecretKey};
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
    /// The cloud version this pass ended at, for rollback protection.
    pub version: u64,
}

/// Derives the vault key for a given encrypted blob, from its header and the master
/// credentials. This is what lets any device open the synced vault.
fn key_for(composite: &SecretKey, bytes: &[u8]) -> Result<VaultKey> {
    let header = Header::parse(bytes)?;
    VaultKey::for_header(composite, &header, bytes)
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
    last_seen: Option<u64>,
) -> Result<SyncOutcome> {
    let remote_now = remote.get(vault_id)?;

    // Rollback protection: never apply a cloud copy older than the last version this device
    // saw. A server that silently serves an old blob cannot resurrect or lose data.
    if let (Some(seen), Some(remote_vault)) = (last_seen, remote_now.as_ref())
        && remote_vault.version < seen
    {
        return Err(Error::invalid(
            "The cloud copy is older than the last one this device saw; refusing it in case of a rollback",
        ));
    }

    let mut merged = local.clone();
    let mut report = MergeReport::default();
    let mut changed_locally = false;
    let mut base: Option<u64> = None;
    let mut needs_upload = remote_now.is_none();
    let mut key = local_key.clone();
    let mut version = remote_now.as_ref().map(|vault| vault.version).unwrap_or(0);

    if let Some(remote_vault) = &remote_now {
        let remote_key = key_for(composite, &remote_vault.bytes)?;
        let remote_plain = decrypt(&remote_key, remote_vault)?;
        let outcome = merge::merge(local, &remote_plain);
        merged = outcome.vault;
        report = outcome.report;
        changed_locally = outcome.local_changed;
        needs_upload = outcome.remote_outdated;
        base = Some(remote_vault.version);
        version = remote_vault.version;

        // **A recovery key must survive syncing.** Taking the cloud copy's header wholesale would
        // drop this device's recovery wrap whenever the cloud copy came from an older build or from
        // a device that has no recovery key — the wrap would disappear silently and the file the
        // user kept would stop opening the vault. So this device keeps its wrapped form and uploads
        // it, which also carries the wrap to the other copies.
        if local_key.has_recovery() && !remote_key.has_recovery() {
            needs_upload = true;
        } else {
            key = remote_key;
        }
    }

    if !needs_upload {
        return Ok(SyncOutcome {
            vault: merged,
            changed_locally,
            uploaded: false,
            report,
            key,
            version,
        });
    }

    let bytes = storage::encrypt_vault(&merged, &key)?;
    match remote.put(vault_id, base, &bytes)? {
        PutOutcome::Stored { version: stored } => Ok(SyncOutcome {
            vault: merged,
            changed_locally,
            uploaded: true,
            report,
            key,
            version: stored,
        }),
        PutOutcome::Conflict { current } => {
            // Somebody wrote first: their copy may carry a different header, so derive
            // its key, fold it in, and retry once.
            let current_key = key_for(composite, &current.bytes)?;
            let remote_plain = decrypt(&current_key, &current)?;
            let outcome = merge::merge(&merged, &remote_plain);
            // Same rule as above: never trade a recovery wrap for a copy that has none.
            let write_key = if key.has_recovery() && !current_key.has_recovery() {
                &key
            } else {
                &current_key
            };
            let bytes = storage::encrypt_vault(&outcome.vault, write_key)?;
            match remote.put(vault_id, Some(current.version), &bytes)? {
                PutOutcome::Stored { version: stored } => Ok(SyncOutcome {
                    vault: outcome.vault,
                    changed_locally: changed_locally || outcome.local_changed,
                    uploaded: true,
                    report: outcome.report,
                    key: write_key.clone(),
                    version: stored,
                }),
                PutOutcome::Conflict { .. } => Err(Error::invalid(
                    "The vault changed again while syncing; please retry",
                )),
            }
        }
    }
}

/// Re-encrypts one synced vault under a new master password, folding in the current cloud copy
/// first. The cloud copy (and a local copy left over from an interrupted run) is decrypted with
/// whichever of the two credentials opens it, so a retry still works. The server still only ever
/// sees ciphertext. Returns the merged vault (when there is one), the cloud version it ended at,
/// and the key the vault was written with.
///
/// The new key is a **re-wrap** of the vault's own secret, never a fresh one, so a recovery key
/// created for this vault keeps working after the password change.
#[cfg(feature = "sync")]
pub fn rekey(
    remote: &dyn Remote,
    vault_id: &str,
    local: Option<&Vault>,
    local_key: Option<&VaultKey>,
    old_composite: &SecretKey,
    new_composite: &SecretKey,
    kdf: KdfParams,
) -> Result<(Option<Vault>, u64, VaultKey)> {
    let remote_now = remote.get(vault_id)?;
    let new_key = match (&remote_now, local_key) {
        // Prefer whichever key carries a recovery wrap, for the same reason `sync` does: taking the
        // cloud copy's key wholesale would silently drop this device's recovery key.
        (Some(remote_vault), Some(key)) if key.has_recovery() => {
            let from_cloud = key_either(&remote_vault.bytes, old_composite, new_composite)?;
            if from_cloud.has_recovery() {
                from_cloud.rewrap(new_composite, kdf)?
            } else {
                key.rewrap(new_composite, kdf)?
            }
        }
        (Some(remote_vault), _) => key_either(&remote_vault.bytes, old_composite, new_composite)?.rewrap(new_composite, kdf)?,
        (None, Some(key)) => key.rewrap(new_composite, kdf)?,
        (None, None) => VaultKey::generate(new_composite, kdf)?,
    };

    let remote_plain = match &remote_now {
        Some(remote_vault) => Some(decrypt_either(&remote_vault.bytes, old_composite, new_composite)?),
        None => None,
    };

    let merged = match (local, remote_plain) {
        (Some(local), Some(remote)) => Some(merge::merge(local, &remote).vault),
        (Some(local), None) => Some(local.clone()),
        (None, Some(remote)) => Some(remote),
        (None, None) => None,
    };

    let Some(merged) = merged else {
        return Ok((None, remote_now.map(|vault| vault.version).unwrap_or(0), new_key));
    };

    let base = remote_now.as_ref().map(|vault| vault.version);
    let bytes = storage::encrypt_vault(&merged, &new_key)?;
    match remote.put(vault_id, base, &bytes)? {
        PutOutcome::Stored { version } => Ok((Some(merged), version, new_key)),
        PutOutcome::Conflict { current } => {
            // Someone wrote first; their copy carries either the old or the new credentials.
            let their_plain = decrypt_either(&current.bytes, old_composite, new_composite)?;
            let outcome = merge::merge(&merged, &their_plain);
            let bytes = storage::encrypt_vault(&outcome.vault, &new_key)?;
            match remote.put(vault_id, Some(current.version), &bytes)? {
                PutOutcome::Stored { version } => Ok((Some(outcome.vault), version, new_key)),
                PutOutcome::Conflict { .. } => Err(Error::invalid(
                    "The vault changed again while the master password was being changed; please retry",
                )),
            }
        }
    }
}

/// The key of a blob under whichever of the two credentials it carries.
#[cfg(feature = "sync")]
fn key_either(bytes: &[u8], old_composite: &SecretKey, new_composite: &SecretKey) -> Result<VaultKey> {
    match key_for(old_composite, bytes) {
        Ok(key) => Ok(key),
        Err(_) => key_for(new_composite, bytes),
    }
}

/// Decrypts a blob with whichever of the two credentials opens it. During a password change a
/// blob may still carry the old password's header, or already the new one after an interrupted
/// run; both are readable.
fn decrypt_either(bytes: &[u8], old_composite: &SecretKey, new_composite: &SecretKey) -> Result<Vault> {
    for composite in [old_composite, new_composite] {
        if let Ok(key) = key_for(composite, bytes)
            && let Ok(vault) = storage::decrypt_vault(bytes, &key)
        {
            return Ok(vault);
        }
    }
    Err(Error::InvalidCredentials)
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

        let uploaded = sync(&remote, "v1", &first_device, &composite, &key, None).unwrap();
        assert!(uploaded.uploaded);

        // A device that only had the (older, empty) copy learns about the entry.
        let received = sync(&remote, "v1", &base, &composite, &key, None).unwrap();
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
        sync(&remote, "v1", &device_a, &composite, &key_a, None).unwrap();

        // Device B has its own (different) salt but the same master password.
        let key_b = VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap();
        assert_ne!(key_a.salt, key_b.salt);

        let received = sync(&remote, "v1", &base, &composite, &key_b, None).unwrap();
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
        let a = sync(&remote, "v1", &device_a, &composite, &key, None).unwrap().vault;

        let mut device_b = base.clone();
        device_b.add_entry(None, content("B", "2")).unwrap();
        let b = sync(&remote, "v1", &device_b, &composite, &key, None).unwrap();
        assert_eq!(b.vault.entries.len(), 2);
        assert!(b.uploaded);

        let a2 = sync(&remote, "v1", &a, &composite, &key, None).unwrap();
        assert_eq!(a2.vault.entries.len(), 2);
        assert!(a2.changed_locally);
    }

    #[test]
    fn nothing_changed_means_nothing_is_uploaded() {
        let remote = MemoryRemote::new();
        let (composite, key) = credentials();
        let vault = Vault::new("Main").unwrap();

        sync(&remote, "v1", &vault, &composite, &key, None).unwrap();
        let again = sync(&remote, "v1", &vault, &composite, &key, None).unwrap();
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

        let outcome = sync(&remote, "v1", &mine, &composite, &key, None).unwrap();
        let titles: Vec<&str> = outcome.vault.entries.iter().map(|e| e.content.title.as_str()).collect();
        assert!(titles.contains(&"Mine"), "local change kept");
        assert!(titles.contains(&"Theirs"), "concurrent change merged");
    }

    #[test]
    fn a_rollback_is_refused() {
        let remote = MemoryRemote::new();
        let (composite, key) = credentials();
        let vault = Vault::new("Main").unwrap();
        sync(&remote, "v1", &vault, &composite, &key, None).unwrap();

        // Having already seen version 1, a cloud copy older than that must be refused.
        let result = sync(&remote, "v1", &vault, &composite, &key, Some(2));
        assert!(result.is_err(), "an older cloud version is rejected as a rollback");

        // Seeing the same or a newer version is fine.
        assert!(sync(&remote, "v1", &vault, &composite, &key, Some(1)).is_ok());
    }

    /// A recovery wrap must never be traded for a cloud copy that has none. Doing so destroys the
    /// recovery key the user saved, silently, and their file stops opening the vault.
    #[test]
    fn a_recovery_wrap_survives_syncing_with_an_unwrapped_copy() {
        let remote = MemoryRemote::new();
        let (composite, plain) = credentials();
        let mut vault = Vault::new("Main").unwrap();
        vault.add_entry(None, content("Mail", "1")).unwrap();

        // Another device — or an older build — put an unwrapped, version-1 copy in the cloud.
        sync(&remote, "v1", &vault, &composite, &plain, None).unwrap();
        let cloud = remote.get("v1").unwrap().unwrap();
        assert_eq!(Header::parse(&cloud.bytes).unwrap().version, 1);

        // This device has a recovery key for the same vault.
        let code = [7u8; 32];
        let wrapped = plain.with_recovery(&composite, &code).unwrap();

        let outcome = sync(&remote, "v1", &vault, &composite, &wrapped, Some(1)).unwrap();
        assert!(outcome.key.has_recovery(), "the wrap must not be dropped by a sync");

        // And the cloud copy carries it now, so the other copies receive the wrap too.
        let cloud = remote.get("v1").unwrap().unwrap();
        let header = Header::parse(&cloud.bytes).unwrap();
        assert!(header.has_recovery(), "the wrap must reach the cloud copy");
        assert!(storage::key_from_recovery(&header, &cloud.bytes, &code).is_ok());
    }

    /// The same rule applies to a master-password change: it re-encrypts every vault, and it must
    /// not use that as an excuse to drop a recovery wrap.
    #[test]
    fn a_password_change_keeps_the_wrap_when_the_cloud_copy_has_none() {
        let remote = MemoryRemote::new();
        let (old_composite, plain) = credentials();
        let mut vault = Vault::new("Main").unwrap();
        vault.add_entry(None, content("Mail", "1")).unwrap();

        // The cloud copy is unwrapped — another device, or an older build, wrote it.
        sync(&remote, "v1", &vault, &old_composite, &plain, None).unwrap();
        let cloud = remote.get("v1").unwrap().unwrap();
        assert_eq!(Header::parse(&cloud.bytes).unwrap().version, 1);

        // This device has a recovery key for the vault.
        let code = [9u8; 32];
        let wrapped = plain.with_recovery(&old_composite, &code).unwrap();

        let new_composite = composite_key("new-pw", None);
        let (_, _, new_key) = rekey(
            &remote,
            "v1",
            Some(&vault),
            Some(&wrapped),
            &old_composite,
            &new_composite,
            KdfParams::FOR_TESTS,
        )
        .unwrap();
        assert!(new_key.has_recovery(), "a password change must not drop the recovery wrap");
    }

    #[test]
    fn rekey_reencrypts_so_the_new_password_opens_the_cloud_copy() {
        let remote = MemoryRemote::new();
        let (old_composite, old_key) = credentials();

        let mut vault = Vault::new("Main").unwrap();
        vault.add_entry(None, content("Mail", "1")).unwrap();
        sync(&remote, "v1", &vault, &old_composite, &old_key, None).unwrap();

        let new_composite = composite_key("new-pw", None);
        let (merged, version, _new_key) = rekey(
            &remote,
            "v1",
            Some(&vault),
            Some(&old_key),
            &old_composite,
            &new_composite,
            KdfParams::FOR_TESTS,
        )
        .unwrap();
        assert_eq!(merged.unwrap().entries.len(), 1);
        assert!(version >= 1);

        // A brand-new device with only the new password derives the key and reads the copy.
        let stored = remote.get("v1").unwrap().unwrap();
        let derived = key_for(&new_composite, &stored.bytes).unwrap();
        let plain = decrypt(&derived, &stored).unwrap();
        assert_eq!(plain.entries.len(), 1);
        assert_eq!(plain.entries[0].content.title, "Mail");
    }

    #[test]
    fn rekey_can_reencrypt_a_vault_that_is_not_on_this_device() {
        let remote = MemoryRemote::new();
        let (old_composite, old_key) = credentials();
        let mut vault = Vault::new("Main").unwrap();
        vault.add_entry(None, content("Remote", "1")).unwrap();
        sync(&remote, "v1", &vault, &old_composite, &old_key, None).unwrap();

        let new_composite = composite_key("new-pw", None);
        // No local copy at all: it is still re-encrypted from its ciphertext alone.
        let (merged, _version, _new_key) = rekey(
            &remote,
            "v1",
            None,
            None,
            &old_composite,
            &new_composite,
            KdfParams::FOR_TESTS,
        )
        .unwrap();
        assert_eq!(merged.unwrap().entries.len(), 1);

        let stored = remote.get("v1").unwrap().unwrap();
        let derived = key_for(&new_composite, &stored.bytes).unwrap();
        let plain = decrypt(&derived, &stored).unwrap();
        assert_eq!(plain.entries[0].content.title, "Remote");
    }
}
