//! Optional cloud synchronization, compiled only with the `sync` Cargo feature.
//!
//! See `docs/SYNC.md`. The server stores an **opaque, encrypted** copy of the vault;
//! every merge happens here, on the device, and the server never sees plaintext.
//!
//! This module is deliberately self-contained: it does not change the local app in any
//! way and stays dormant until the interface is wired to it (a later phase).

mod engine;
mod remote;

pub use engine::{SyncOutcome, sync};
pub use remote::{MemoryRemote, PutOutcome, Remote, RemoteVault};
