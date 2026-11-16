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
mod opaque;
mod remote;

pub use account::{
    AccountInfo, CloudVault, DEFAULT_SERVER, auth_secret, billing_cancel, billing_checkout, delete_account,
    delete_vault, ensure_vault, legacy_auth_secret, list_vaults, login, me, opaque_login_finish,
    opaque_login_start, opaque_register_finish, opaque_register_start, opaque_upgrade_finish,
    opaque_upgrade_start, previous_auth_secret, register, set_vault_label, sign_out_everywhere,
};
pub use engine::{SyncOutcome, rekey, sync};
pub use http::HttpRemote;
pub use opaque::{login_finish, login_start, registration_finish, registration_start};
pub use remote::{MemoryRemote, PutOutcome, Remote, RemoteVault};
