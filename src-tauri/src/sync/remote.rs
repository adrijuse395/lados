//! The remote side of sync: an opaque, versioned store of encrypted vault bytes.
//!
//! The real server lives in a separate repository (see `docs/SYNC.md`); this module
//! only defines the contract the client depends on, plus an in-memory implementation
//! used by the tests. The server never sees plaintext — it stores bytes it cannot read.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::error::Result;

/// One stored copy of a vault, exactly as the server keeps it: opaque bytes plus a
/// monotonically increasing version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteVault {
    pub version: u64,
    pub bytes: Vec<u8>,
}

/// The result of an attempted upload.
#[derive(Debug)]
pub enum PutOutcome {
    /// The blob was written and now has this version.
    Stored { version: u64 },
    /// Somebody else wrote first: the caller must merge and retry.
    Conflict { current: RemoteVault },
}

/// A versioned, opaque store of vault blobs. Implemented by the HTTP client against
/// Lados Sync, and by [`MemoryRemote`] in tests.
pub trait Remote {
    fn get(&self, vault_id: &str) -> Result<Option<RemoteVault>>;

    /// Stores `bytes` only if the current version equals `base`.
    /// `base == None` means "nothing must exist yet".
    fn put(&self, vault_id: &str, base: Option<u64>, bytes: &[u8]) -> Result<PutOutcome>;
}

/// In-memory remote used by tests (a reference implementation of [`Remote`]).
#[derive(Default)]
pub struct MemoryRemote {
    vaults: RefCell<HashMap<String, RemoteVault>>,
}

impl MemoryRemote {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Remote for MemoryRemote {
    fn get(&self, vault_id: &str) -> Result<Option<RemoteVault>> {
        Ok(self.vaults.borrow().get(vault_id).cloned())
    }

    fn put(&self, vault_id: &str, base: Option<u64>, bytes: &[u8]) -> Result<PutOutcome> {
        let mut vaults = self.vaults.borrow_mut();
        let current = vaults.get(vault_id).cloned();

        let base_matches = match (&current, base) {
            (None, None) => true,
            (Some(stored), Some(base)) => stored.version == base,
            _ => false,
        };
        if !base_matches {
            return Ok(PutOutcome::Conflict {
                current: current.unwrap_or(RemoteVault {
                    version: 0,
                    bytes: Vec::new(),
                }),
            });
        }

        let version = current.map(|v| v.version).unwrap_or(0) + 1;
        vaults.insert(
            vault_id.to_string(),
            RemoteVault {
                version,
                bytes: bytes.to_vec(),
            },
        );
        Ok(PutOutcome::Stored { version })
    }
}
