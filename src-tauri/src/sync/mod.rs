//! Optional cloud synchronization, compiled only with the `sync` Cargo feature.
//!
//! See `docs/SYNC.md`. The server stores an **opaque, encrypted** copy of the vault;
//! every merge happens here, on the device, and the server never sees plaintext.
//!
//! This module is deliberately self-contained: with the feature off, the shipping app
//! is completely unchanged.

mod account;
mod engine;
mod http;
mod remote;

pub use account::{DEFAULT_SERVER, auth_secret, ensure_vault, legacy_auth_secret, login, register};
pub use engine::{SyncOutcome, sync};
pub use http::HttpRemote;
pub use remote::{MemoryRemote, PutOutcome, Remote, RemoteVault};
