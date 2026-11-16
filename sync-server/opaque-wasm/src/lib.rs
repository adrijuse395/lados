//! The OPAQUE **server** role, compiled to WebAssembly for the Lados Sync worker.
//!
//! It uses the same `opaque-ke` crate (and the same ciphersuite) as the app's Rust client, so
//! the two interoperate by construction. Randomness is supplied by the caller (a 32-byte seed),
//! so the module never needs an OS RNG.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use opaque_ke::ciphersuite::CipherSuite;
use opaque_ke::{
    CredentialFinalization, CredentialRequest, RegistrationRequest, ServerLogin, ServerLoginParameters,
    ServerRegistration, ServerSetup,
};
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use wasm_bindgen::prelude::*;

/// The ciphersuite; the KSF is never used on the server, so the identity is fine here.
struct Suite;

impl CipherSuite for Suite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, sha2::Sha512>;
    type Ksf = opaque_ke::ksf::Identity;
}

fn decode(value: &str) -> Result<Vec<u8>, JsError> {
    BASE64.decode(value).map_err(|error| JsError::new(&format!("base64: {error}")))
}

fn setup(value: &str) -> Result<ServerSetup<Suite>, JsError> {
    ServerSetup::<Suite>::deserialize(&decode(value)?).map_err(|error| JsError::new(&format!("setup: {error:?}")))
}

/// The server's OPRF response to a client registration request (base64).
#[wasm_bindgen]
pub fn create_registration_response(
    server_setup: &str,
    user_identifier: &str,
    registration_request: &str,
) -> Result<String, JsError> {
    let setup = setup(server_setup)?;
    let request = RegistrationRequest::deserialize(&decode(registration_request)?)
        .map_err(|error| JsError::new(&format!("request: {error:?}")))?;
    let start = ServerRegistration::<Suite>::start(&setup, request, user_identifier.as_bytes())
        .map_err(|error| JsError::new(&format!("start: {error:?}")))?;
    Ok(BASE64.encode(start.message.serialize()))
}

/// Starts a login. `seed` is 32 bytes of caller-supplied randomness. Returns
/// `{ "serverLoginState", "loginResponse" }` as JSON.
#[wasm_bindgen]
pub fn start_login(
    server_setup: &str,
    registration_record: Option<String>,
    start_login_request: &str,
    user_identifier: &str,
    seed: &[u8],
) -> Result<String, JsError> {
    let setup = setup(server_setup)?;
    let record = match registration_record {
        Some(value) => Some(
            ServerRegistration::<Suite>::deserialize(&decode(&value)?)
                .map_err(|error| JsError::new(&format!("record: {error:?}")))?,
        ),
        None => None,
    };
    let request = CredentialRequest::deserialize(&decode(start_login_request)?)
        .map_err(|error| JsError::new(&format!("request: {error:?}")))?;
    let seed: [u8; 32] = seed.try_into().map_err(|_| JsError::new("seed must be 32 bytes"))?;
    let mut rng = ChaCha20Rng::from_seed(seed);
    let start = ServerLogin::start(
        &mut rng,
        &setup,
        record,
        request,
        user_identifier.as_bytes(),
        ServerLoginParameters::default(),
    )
    .map_err(|error| JsError::new(&format!("start: {error:?}")))?;
    Ok(serde_json::json!({
        "serverLoginState": BASE64.encode(start.state.serialize()),
        "loginResponse": BASE64.encode(start.message.serialize()),
    })
    .to_string())
}

/// Finishes a login, returning `{ "sessionKey": ... }` as JSON.
#[wasm_bindgen]
pub fn finish_login(server_login_state: &str, finish_login_request: &str) -> Result<String, JsError> {
    let state = ServerLogin::<Suite>::deserialize(&decode(server_login_state)?)
        .map_err(|error| JsError::new(&format!("state: {error:?}")))?;
    let finalization = CredentialFinalization::deserialize(&decode(finish_login_request)?)
        .map_err(|error| JsError::new(&format!("finalization: {error:?}")))?;
    let finish = state
        .finish(finalization, ServerLoginParameters::default())
        .map_err(|error| JsError::new(&format!("finish: {error:?}")))?;
    Ok(serde_json::json!({ "sessionKey": BASE64.encode(finish.session_key) }).to_string())
}
