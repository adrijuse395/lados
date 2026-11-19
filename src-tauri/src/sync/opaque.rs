//! OPAQUE (RFC 9807) for the sync account, so the server never learns a password-equivalent.
//!
//! The ciphersuite **must** match the server's `@serenity-kit/opaque` (the same `opaque-ke`
//! compiled to WebAssembly): Ristretto255 OPRF and group, TripleDH over SHA-512, and an
//! Argon2id key-stretching function with a **zero salt** (t = 3, m = 65536 KiB, p = 4) — the
//! binding's "memory-constrained" default. Any mismatch breaks interoperability.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use opaque_ke::ciphersuite::CipherSuite;
use opaque_ke::errors::{InternalError, ProtocolError};
use opaque_ke::generic_array::{ArrayLength, GenericArray};
use opaque_ke::ksf::Ksf;
use rand_08::rngs::OsRng;
use opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, ClientRegistration, ClientRegistrationFinishParameters,
    CredentialResponse, RegistrationResponse,
};

use crate::error::{Error, Result};

/// Argon2id key-stretching function matching the server binding's default.
#[derive(Default)]
struct StretchArgon2;

impl Ksf for StretchArgon2 {
    fn hash<L: ArrayLength<u8>>(
        &self,
        input: GenericArray<u8, L>,
    ) -> std::result::Result<GenericArray<u8, L>, InternalError> {
        let params = Params::new(65_536, 3, 4, None).map_err(|_| InternalError::KsfError)?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut output = GenericArray::<u8, L>::default();
        argon
            .hash_password_into(&input, &[0u8; 16], &mut output)
            .map_err(|_| InternalError::KsfError)?;
        Ok(output)
    }
}

/// The ciphersuite, fixed to match the server binding.
struct Suite;

impl CipherSuite for Suite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, sha2_010::Sha512>;
    type Ksf = StretchArgon2;
}

fn decode(value: &str) -> Result<Vec<u8>> {
    BASE64.decode(value).map_err(|_| Error::invalid("Invalid OPAQUE message"))
}

fn protocol<T>(result: std::result::Result<T, ProtocolError>) -> Result<T> {
    result.map_err(|error| match error {
        // A wrong password — or a missing/wrong key file — surfaces here, on the client. It is a
        // credential problem and must read like one, not like a crash (see `sync-center.ts`).
        ProtocolError::InvalidLoginError => Error::InvalidCredentials,
        other => Error::System(format!("OPAQUE protocol error: {other:?}")),
    })
}

/// Starts registration. Returns `(client state, registration request)`, both base64.
pub fn registration_start(password: &[u8]) -> Result<(String, String)> {
    let result = protocol(ClientRegistration::<Suite>::start(&mut OsRng, password))?;
    Ok((BASE64.encode(result.state.serialize()), BASE64.encode(result.message.serialize())))
}

/// Finishes registration, returning the registration record (base64) for the server.
pub fn registration_finish(password: &[u8], state: &str, response: &str) -> Result<String> {
    let state = protocol(ClientRegistration::<Suite>::deserialize(&decode(state)?))?;
    let response = protocol(RegistrationResponse::deserialize(&decode(response)?))?;
    let result = protocol(state.finish(
        &mut OsRng,
        password,
        response,
        ClientRegistrationFinishParameters::default(),
    ))?;
    Ok(BASE64.encode(result.message.serialize()))
}

/// Starts login. Returns `(client state, credential request)`, both base64.
pub fn login_start(password: &[u8]) -> Result<(String, String)> {
    let result = protocol(ClientLogin::<Suite>::start(&mut OsRng, password))?;
    Ok((BASE64.encode(result.state.serialize()), BASE64.encode(result.message.serialize())))
}

/// Finishes login, returning the credential finalization (base64) to send to the server.
pub fn login_finish(password: &[u8], state: &str, response: &str) -> Result<String> {
    let state = protocol(ClientLogin::<Suite>::deserialize(&decode(state)?))?;
    let response = protocol(CredentialResponse::deserialize(&decode(response)?))?;
    let result = protocol(state.finish(
        &mut OsRng,
        password,
        response,
        ClientLoginFinishParameters::default(),
    ))?;
    Ok(BASE64.encode(result.message.serialize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opaque_ke::{
        CredentialFinalization, CredentialRequest, RegistrationRequest, RegistrationUpload,
        ServerLogin, ServerLoginParameters, ServerRegistration, ServerSetup,
    };

    /// Runs the whole protocol in-process (the server role uses the same `opaque-ke`), to prove
    /// the client functions and ciphersuite are wired correctly.
    #[test]
    fn registration_and_login_round_trip() {
        let password = b"correct horse battery staple";
        let identifier = b"user@example.com";
        let mut rng = OsRng;
        let server_setup = ServerSetup::<Suite>::new(&mut rng);

        let (client_state, request) = registration_start(password).unwrap();
        let server_start = ServerRegistration::<Suite>::start(
            &server_setup,
            RegistrationRequest::deserialize(&decode(&request).unwrap()).unwrap(),
            identifier,
        )
        .unwrap();
        let record = registration_finish(password, &client_state, &BASE64.encode(server_start.message.serialize())).unwrap();
        let password_file =
            ServerRegistration::<Suite>::finish(RegistrationUpload::deserialize(&decode(&record).unwrap()).unwrap());

        let (login_state, credential_request) = login_start(password).unwrap();
        let server_login = ServerLogin::<Suite>::start(
            &mut rng,
            &server_setup,
            Some(password_file),
            CredentialRequest::deserialize(&decode(&credential_request).unwrap()).unwrap(),
            identifier,
            ServerLoginParameters::default(),
        )
        .unwrap();
        let finalization = login_finish(password, &login_state, &BASE64.encode(server_login.message.serialize())).unwrap();
        let server_finish = server_login
            .state
            .finish(
                CredentialFinalization::deserialize(&decode(&finalization).unwrap()).unwrap(),
                ServerLoginParameters::default(),
            )
            .unwrap();
        assert!(!server_finish.session_key.is_empty(), "the server derives a session key");
    }

    #[test]
    fn a_wrong_password_fails_to_log_in() {
        let identifier = b"user@example.com";
        let mut rng = OsRng;
        let server_setup = ServerSetup::<Suite>::new(&mut rng);

        let (client_state, request) = registration_start(b"right password").unwrap();
        let server_start = ServerRegistration::<Suite>::start(
            &server_setup,
            RegistrationRequest::deserialize(&decode(&request).unwrap()).unwrap(),
            identifier,
        )
        .unwrap();
        let record = registration_finish(b"right password", &client_state, &BASE64.encode(server_start.message.serialize())).unwrap();
        let password_file =
            ServerRegistration::<Suite>::finish(RegistrationUpload::deserialize(&decode(&record).unwrap()).unwrap());

        let (login_state, credential_request) = login_start(b"wrong password").unwrap();
        let server_login = ServerLogin::<Suite>::start(
            &mut rng,
            &server_setup,
            Some(password_file),
            CredentialRequest::deserialize(&decode(&credential_request).unwrap()).unwrap(),
            identifier,
            ServerLoginParameters::default(),
        )
        .unwrap();
        // The client refuses to complete a login with the wrong password.
        let result = login_finish(b"wrong password", &login_state, &BASE64.encode(server_login.message.serialize()));
        assert!(
            matches!(result, Err(Error::InvalidCredentials)),
            "a wrong password must read as a credential problem, not a system fault"
        );
    }
}
