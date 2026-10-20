//! All the cryptography of Lados. This module only sees bytes: it does not know what a
//! vault, a file or an entry is.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::TryRng;
use rand::rngs::SysRng;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

/// 32 bytes of secret key material, wiped from memory when dropped.
pub type SecretKey = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id cost parameters. They live in the file header, so future versions can raise
/// them without breaking existing vaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// 64 MiB of memory, 4 lanes and 3 passes. New vaults use at least this much.
    pub const DEFAULT: KdfParams = KdfParams {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 4,
    };

    /// How long deriving the key of a new vault should take on the computer that creates it.
    const TARGET_DURATION: Duration = Duration::from_millis(500);

    /// The default parameters with as many passes as fit in the target duration on this
    /// computer, like KeePass does: the faster the machine, the costlier each guess becomes.
    pub fn calibrated() -> KdfParams {
        let probe = Self::DEFAULT;
        let started = Instant::now();
        if derive_key(&[0; KEY_LEN], &[0; SALT_LEN], probe).is_err() {
            return probe;
        }
        let per_pass = started.elapsed().as_secs_f64() / f64::from(probe.iterations);
        let passes = (Self::TARGET_DURATION.as_secs_f64() / per_pass) as u32;
        KdfParams {
            iterations: passes.clamp(probe.iterations, Self::MAX_ITERATIONS),
            ..probe
        }
    }

    /// Cheap parameters so the test suite stays fast. Never used by the application.
    #[cfg(test)]
    pub const FOR_TESTS: KdfParams = KdfParams {
        memory_kib: 256,
        iterations: 1,
        parallelism: 1,
    };

    const MAX_MEMORY_KIB: u32 = 4 * 1024 * 1024;
    const MAX_ITERATIONS: u32 = 64;

    /// Rejects values that could freeze the app or exhaust memory when opening a crafted file.
    pub fn validate(&self) -> Result<()> {
        let parallelism_ok = (1..=64).contains(&self.parallelism);
        let iterations_ok = (1..=Self::MAX_ITERATIONS).contains(&self.iterations);
        let memory_ok = parallelism_ok
            && self.memory_kib >= 8 * self.parallelism
            && self.memory_kib <= Self::MAX_MEMORY_KIB;
        if parallelism_ok && iterations_ok && memory_ok {
            Ok(())
        } else {
            Err(Error::Corrupted)
        }
    }
}

/// Fills an array with bytes from the operating system's secure random generator.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(bytes)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Condenses the user's credentials (password and optional key file) into one fixed-size secret.
///
/// The open session keeps this value instead of the password, so it can reopen other copies
/// of the vault when merging without asking again.
pub fn composite_key(password: &str, key_file: Option<&[u8]>) -> SecretKey {
    let mut hasher = Sha256::new();
    hasher.update(b"lados-composite-key-v1");
    hasher.update(Sha256::digest(password.as_bytes()));
    if let Some(key_file) = key_file {
        hasher.update(Sha256::digest(key_file));
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&hasher.finalize());
    key
}

/// Stretches the composite key with Argon2id into the key that actually encrypts the vault.
/// Argon2id is deliberately slow and memory hungry, which makes guessing passwords expensive.
pub fn derive_key(composite: &[u8; KEY_LEN], salt: &[u8; SALT_LEN], params: KdfParams) -> Result<SecretKey> {
    params.validate()?;
    let argon_params = Params::new(params.memory_kib, params.iterations, params.parallelism, Some(KEY_LEN))
        .map_err(|e| Error::Crypto(e.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(composite, salt, &mut key[..])
        .map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(key)
}

/// Encrypts and authenticates `plaintext` with XChaCha20-Poly1305.
///
/// `associated_data` is not encrypted but is authenticated: changing a single bit of it
/// (for example, the file header) makes decryption fail.
pub fn encrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    associated_data: &[u8],
) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(<&Key>::from(key));
    cipher
        .encrypt(
            <&XNonce>::from(nonce),
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| Error::Crypto("encryption failed".into()))
}

/// Reverses [`encrypt`]. A wrong key and a tampered file are indistinguishable by design,
/// so both are reported as invalid credentials.
pub fn decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    associated_data: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = XChaCha20Poly1305::new(<&Key>::from(key));
    cipher
        .decrypt(
            <&XNonce>::from(nonce),
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::InvalidCredentials)
}

/// Creates a random key file in the KeePass 2.0 XML format, so the same file also works
/// with KeePass and KeePassXC.
pub fn new_key_file() -> Result<Zeroizing<String>> {
    let key = Zeroizing::new(random_bytes::<32>()?);
    let checksum: String = sha256(&key[..])[..4].iter().map(|b| format!("{b:02X}")).collect();
    let hex = Zeroizing::new(key.iter().map(|b| format!("{b:02X}")).collect::<String>());
    let groups: Vec<&str> = (0..hex.len()).step_by(8).map(|i| &hex[i..i + 8]).collect();

    Ok(Zeroizing::new(format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <KeyFile>\n\
         \t<Meta>\n\
         \t\t<Version>2.0</Version>\n\
         \t</Meta>\n\
         \t<Key>\n\
         \t\t<Data Hash=\"{checksum}\">\n\
         \t\t\t{}\n\
         \t\t\t{}\n\
         \t\t</Data>\n\
         \t</Key>\n\
         </KeyFile>\n",
        groups[..4].join(" "),
        groups[4..].join(" "),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];
    const NONCE: [u8; NONCE_LEN] = [9; NONCE_LEN];

    #[test]
    fn encryption_round_trips() {
        let ciphertext = encrypt(&KEY, &NONCE, b"top secret", b"header").unwrap();
        assert_ne!(&ciphertext[..10], b"top secret");
        let plaintext = decrypt(&KEY, &NONCE, &ciphertext, b"header").unwrap();
        assert_eq!(plaintext.as_slice(), b"top secret");
    }

    #[test]
    fn wrong_key_tampering_and_altered_header_are_rejected() {
        let ciphertext = encrypt(&KEY, &NONCE, b"top secret", b"header").unwrap();

        let wrong_key = [8; KEY_LEN];
        assert!(matches!(
            decrypt(&wrong_key, &NONCE, &ciphertext, b"header"),
            Err(Error::InvalidCredentials)
        ));

        let mut tampered = ciphertext.clone();
        tampered[0] ^= 1;
        assert!(decrypt(&KEY, &NONCE, &tampered, b"header").is_err());

        assert!(decrypt(&KEY, &NONCE, &ciphertext, b"HEADER").is_err());
    }

    #[test]
    fn key_derivation_depends_on_password_key_file_and_salt() {
        let salt = [1; SALT_LEN];
        let password_only = composite_key("correct horse", None);
        let with_key_file = composite_key("correct horse", Some(b"key file bytes"));
        assert_ne!(*password_only, *with_key_file);
        assert_eq!(*password_only, *composite_key("correct horse", None));

        let a = derive_key(&password_only, &salt, KdfParams::FOR_TESTS).unwrap();
        let b = derive_key(&password_only, &salt, KdfParams::FOR_TESTS).unwrap();
        let c = derive_key(&password_only, &[2; SALT_LEN], KdfParams::FOR_TESTS).unwrap();
        assert_eq!(*a, *b);
        assert_ne!(*a, *c);
    }

    #[test]
    fn absurd_kdf_parameters_are_rejected() {
        let huge_memory = KdfParams {
            memory_kib: u32::MAX,
            ..KdfParams::DEFAULT
        };
        let no_iterations = KdfParams {
            iterations: 0,
            ..KdfParams::DEFAULT
        };
        assert!(huge_memory.validate().is_err());
        assert!(no_iterations.validate().is_err());
        assert!(KdfParams::DEFAULT.validate().is_ok());
    }

    #[test]
    fn calibration_never_goes_below_the_defaults_nor_above_the_limits() {
        let calibrated = KdfParams::calibrated();
        assert!(calibrated.validate().is_ok());
        assert!(calibrated.iterations >= KdfParams::DEFAULT.iterations);
        assert_eq!(calibrated.memory_kib, KdfParams::DEFAULT.memory_kib);
        assert_eq!(calibrated.parallelism, KdfParams::DEFAULT.parallelism);
    }

    #[test]
    fn key_files_use_the_keepass_xml_format() {
        let key_file = new_key_file().unwrap();
        assert!(key_file.contains("<Version>2.0</Version>"));
        let data_line_count = key_file.lines().filter(|l| l.trim().len() == 35).count();
        assert_eq!(data_line_count, 2);
        assert_ne!(*new_key_file().unwrap(), *key_file);
    }
}
