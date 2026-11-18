//! The `.lados` file format and everything that touches the disk.
//!
//! Every file starts with the same 80-byte **core header** (integers are little-endian):
//!
//! | Offset | Size | Field                                              |
//! |--------|------|----------------------------------------------------|
//! | 0      | 8    | Magic bytes `LADOSVLT`                             |
//! | 8      | 2    | Format version (1 or 2)                            |
//! | 10     | 1    | Key derivation function (1 = Argon2id)             |
//! | 11     | 4    | Argon2 memory, in KiB                              |
//! | 15     | 4    | Argon2 iterations                                  |
//! | 19     | 4    | Argon2 parallelism                                 |
//! | 23     | 32   | Salt                                               |
//! | 55     | 1    | Cipher (1 = XChaCha20-Poly1305)                    |
//! | 56     | 24   | Nonce                                              |
//!
//! **Version 1** puts the encrypted, padded vault straight after the header, and the payload is
//! encrypted with `Argon2(composite, salt)` — a key that changes with the master password.
//!
//! **Version 2** (written when a recovery key is in use) inserts a small key-wrapping section
//! between the header and the payload:
//!
//! | Offset | Size | Field                                                           |
//! |--------|------|-----------------------------------------------------------------|
//! | 80     | 1    | Flags (bit 0: a recovery wrap follows)                           |
//! | 81     | 24   | Nonce of the password wrap                                       |
//! | 105    | 48   | The vault secret, wrapped under the password key                 |
//! | 153    | 32   | *(recovery)* Salt of the recovery wrap                           |
//! | 185    | 24   | *(recovery)* Nonce of the recovery wrap                          |
//! | 209    | 48   | *(recovery)* The same vault secret, wrapped under the recovery key |
//! | …      | …    | Encrypted padded vault, followed by a 16-byte tag                |
//!
//! A version-2 vault is encrypted with a **random** 32-byte secret instead of one derived from the
//! password. That secret is wrapped twice — once under the password key, once (optionally) under
//! the recovery key — so **changing the master password re-wraps the secret and the recovery key
//! keeps working**. Version-1 files stay readable and writable; a file only becomes version 2 when
//! a recovery key is created for it.
//!
//! In both versions the 80-byte core header is authenticated together with the ciphertext, and the
//! vault is padded to 64 KiB blocks so the file size does not reveal how many entries it holds.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::Local;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::{self, KEY_LEN, KdfParams, NONCE_LEN, SALT_LEN, SecretKey};
use crate::error::{Error, Result};
use crate::models::{SCHEMA_VERSION, Vault};

pub const FILE_EXTENSION: &str = "lados";
const MAGIC: &[u8; 8] = b"LADOSVLT";
const FORMAT_VERSION: u16 = 1;
const FORMAT_VERSION_V2: u16 = 2;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_XCHACHA20_POLY1305: u8 = 1;
const HEADER_LEN: usize = 80;
const PADDING_BLOCK: usize = 64 * 1024;
const MAX_KEY_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Room for the authentication tag of XChaCha20-Poly1305.
const TAG_LEN: usize = 16;
/// A wrapped 32-byte secret: nonce ‖ ciphertext ‖ tag.
const WRAP_LEN: usize = NONCE_LEN + KEY_LEN + TAG_LEN;
/// A recovery wrap: its own salt, then a wrapped secret.
const RECOVERY_WRAP_LEN: usize = SALT_LEN + WRAP_LEN;
/// Flags byte, at offset 80 of a version-2 file.
const FLAG_RECOVERY: u8 = 0b0000_0001;

/// Argon2id parameters for the recovery key. **Fixed and never stored**, so every device derives
/// the same key from the same code; the code is 256 random bits, so this only adds defence in
/// depth. Changing it would strand every existing recovery file.
const RECOVERY_KDF: KdfParams = KdfParams {
    memory_kib: 64 * 1024,
    iterations: 3,
    parallelism: 4,
};

/// The payload key of a version-2 vault, derived from its random secret. A fast hash is enough
/// because the secret is 256 random bits, unlike a password.
fn payload_key(secret: &SecretKey) -> SecretKey {
    let mut hasher = Sha256::new();
    hasher.update(b"lados-vault-key-v2");
    hasher.update(&secret[..]);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&hasher.finalize());
    key
}

/// The key-encryption key of a recovery code.
fn recovery_kek(code: &[u8; KEY_LEN], salt: &[u8; SALT_LEN]) -> Result<SecretKey> {
    crypto::derive_key(code, salt, RECOVERY_KDF)
}

/// The unencrypted beginning of a vault file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    /// Version 2 only; always 0 in version 1.
    pub flags: u8,
}

impl Header {
    fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[0..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&self.version.to_le_bytes());
        bytes[10] = KDF_ARGON2ID;
        bytes[11..15].copy_from_slice(&self.kdf.memory_kib.to_le_bytes());
        bytes[15..19].copy_from_slice(&self.kdf.iterations.to_le_bytes());
        bytes[19..23].copy_from_slice(&self.kdf.parallelism.to_le_bytes());
        bytes[23..55].copy_from_slice(&self.salt);
        bytes[55] = CIPHER_XCHACHA20_POLY1305;
        bytes[56..80].copy_from_slice(&self.nonce);
        bytes
    }

    /// Whether a recovery wrap follows the header.
    pub fn has_recovery(&self) -> bool {
        self.flags & FLAG_RECOVERY != 0
    }

    /// How many bytes the header and any key wraps occupy before the encrypted payload.
    pub fn prefix_len(&self) -> usize {
        if self.version == FORMAT_VERSION {
            return HEADER_LEN;
        }
        HEADER_LEN + 1 + WRAP_LEN + if self.has_recovery() { RECOVERY_WRAP_LEN } else { 0 }
    }

    pub fn parse(bytes: &[u8]) -> Result<Header> {
        if bytes.len() < 10 || &bytes[0..8] != MAGIC {
            return Err(Error::NotAVault);
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != FORMAT_VERSION && version != FORMAT_VERSION_V2 {
            return Err(Error::UnsupportedVersion(version));
        }
        if bytes.len() < HEADER_LEN || bytes[10] != KDF_ARGON2ID || bytes[55] != CIPHER_XCHACHA20_POLY1305 {
            return Err(Error::Corrupted);
        }
        let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default());
        let kdf = KdfParams {
            memory_kib: u32_at(11),
            iterations: u32_at(15),
            parallelism: u32_at(19),
        };
        kdf.validate()?;

        // Version 2 carries a flags byte — and, usually, key wraps — before the payload.
        let mut flags = 0u8;
        if version == FORMAT_VERSION_V2 {
            flags = *bytes.get(HEADER_LEN).ok_or(Error::Corrupted)?;
            if flags & !FLAG_RECOVERY != 0 {
                return Err(Error::Corrupted);
            }
        }
        let header = Header {
            version,
            kdf,
            salt: bytes[23..55].try_into().map_err(|_| Error::Corrupted)?,
            nonce: bytes[56..80].try_into().map_err(|_| Error::Corrupted)?,
            flags,
        };
        if bytes.len() <= header.prefix_len() {
            return Err(Error::Corrupted);
        }
        Ok(header)
    }
}

/// A recovery wrap: the salt and nonce, and the vault secret sealed under the recovery key.
type RecoveryWrap = ([u8; SALT_LEN], [u8; NONCE_LEN], Vec<u8>);

/// The key material of a version-2 file: a random secret plus the wraps that protect it.
#[derive(Clone)]
struct Wrapping {
    secret: SecretKey,
    /// Nonce of the password wrap.
    nonce: [u8; NONCE_LEN],
    /// The secret, encrypted under the password key.
    wrap: Vec<u8>,
    /// The same secret, encrypted under the recovery key.
    recovery: Option<RecoveryWrap>,
}

/// The key that encrypts a vault, plus everything needed to write a matching header.
#[derive(Clone)]
pub struct VaultKey {
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    key: SecretKey,
    wrapping: Option<Wrapping>,
}

impl VaultKey {
    /// A version-1 key: the payload is encrypted directly with `Argon2(composite, salt)`.
    pub fn generate(composite: &[u8; 32], kdf: KdfParams) -> Result<VaultKey> {
        let salt = crypto::random_bytes()?;
        let key = crypto::derive_key(composite, &salt, kdf)?;
        Ok(VaultKey {
            kdf,
            salt,
            key,
            wrapping: None,
        })
    }

    /// A version-2 key: a random secret, wrapped under the password key.
    fn generate_wrapped(composite: &[u8; 32], kdf: KdfParams) -> Result<VaultKey> {
        let secret = Zeroizing::new(crypto::random_bytes::<KEY_LEN>()?);
        let salt = crypto::random_bytes()?;
        let nonce = crypto::random_bytes()?;
        let kek = crypto::derive_key(composite, &salt, kdf)?;
        let wrap = crypto::encrypt(&kek, &nonce, &secret[..], &[])?;
        Ok(VaultKey {
            kdf,
            salt,
            key: payload_key(&secret),
            wrapping: Some(Wrapping {
                secret,
                nonce,
                wrap,
                recovery: None,
            }),
        })
    }

    /// The key that matches an existing file. This is the slow step of unlocking.
    pub fn for_header(composite: &[u8; 32], header: &Header, bytes: &[u8]) -> Result<VaultKey> {
        if header.version == FORMAT_VERSION {
            let key = crypto::derive_key(composite, &header.salt, header.kdf)?;
            return Ok(VaultKey {
                kdf: header.kdf,
                salt: header.salt,
                key,
                wrapping: None,
            });
        }
        let kek = crypto::derive_key(composite, &header.salt, header.kdf)?;
        let nonce: [u8; NONCE_LEN] = bytes[HEADER_LEN + 1..HEADER_LEN + 1 + NONCE_LEN]
            .try_into()
            .map_err(|_| Error::Corrupted)?;
        let wrap = bytes[HEADER_LEN + 1 + NONCE_LEN..HEADER_LEN + 1 + WRAP_LEN].to_vec();
        let secret = wrap_secret(&kek, &nonce, &wrap)?;
        let recovery = read_recovery(header, bytes)?;
        Ok(VaultKey {
            kdf: header.kdf,
            salt: header.salt,
            key: payload_key(&secret),
            wrapping: Some(Wrapping {
                secret,
                nonce,
                wrap,
                recovery,
            }),
        })
    }

    /// Whether this key writes version-2 files.
    pub fn is_wrapped(&self) -> bool {
        self.wrapping.is_some()
    }

    /// Whether this key carries a recovery wrap.
    pub fn has_recovery(&self) -> bool {
        self.wrapping.as_ref().is_some_and(|wrapping| wrapping.recovery.is_some())
    }

    pub fn matches(&self, header: &Header) -> bool {
        let version = if self.wrapping.is_some() { FORMAT_VERSION_V2 } else { FORMAT_VERSION };
        self.kdf == header.kdf && self.salt == header.salt && version == header.version
    }

    /// Re-wraps the same secret under new credentials, a new salt and new cost. Every
    /// master-password change goes through here, which is what keeps a recovery key working.
    pub fn rewrap(&self, composite: &[u8; 32], kdf: KdfParams) -> Result<VaultKey> {
        let Some(wrapping) = &self.wrapping else {
            return VaultKey::generate(composite, kdf);
        };
        let salt = crypto::random_bytes()?;
        let nonce = crypto::random_bytes()?;
        let kek = crypto::derive_key(composite, &salt, kdf)?;
        let wrap = crypto::encrypt(&kek, &nonce, &wrapping.secret[..], &[])?;
        Ok(VaultKey {
            kdf,
            salt,
            key: payload_key(&wrapping.secret),
            wrapping: Some(Wrapping {
                secret: wrapping.secret.clone(),
                nonce,
                wrap,
                recovery: wrapping.recovery.clone(),
            }),
        })
    }

    /// Adds a recovery wrap, turning a version-1 key into a version-2 one. The caller must then
    /// save the vault, which re-encrypts it under the new payload key.
    pub fn with_recovery(&self, composite: &[u8; 32], code: &[u8; KEY_LEN]) -> Result<VaultKey> {
        let base = if self.wrapping.is_some() {
            self.clone()
        } else {
            VaultKey::generate_wrapped(composite, self.kdf)?
        };
        let wrapping = base.wrapping.as_ref().ok_or(Error::Corrupted)?;
        let salt: [u8; SALT_LEN] = crypto::random_bytes()?;
        let nonce: [u8; NONCE_LEN] = crypto::random_bytes()?;
        let kek = recovery_kek(code, &salt)?;
        let sealed = crypto::encrypt(&kek, &nonce, &wrapping.secret[..], &[])?;
        Ok(VaultKey {
            kdf: base.kdf,
            salt: base.salt,
            key: base.key.clone(),
            wrapping: Some(Wrapping {
                secret: wrapping.secret.clone(),
                nonce: wrapping.nonce,
                wrap: wrapping.wrap.clone(),
                recovery: Some((salt, nonce, sealed)),
            }),
        })
    }

    /// Forgets the recovery wrap, keeping the version-2 secret.
    pub fn without_recovery(&self) -> VaultKey {
        let mut key = self.clone();
        if let Some(wrapping) = &mut key.wrapping {
            wrapping.recovery = None;
        }
        key
    }
}

/// Unwraps a 32-byte secret that was sealed with `kek`.
fn wrap_secret(kek: &SecretKey, nonce: &[u8; NONCE_LEN], sealed: &[u8]) -> Result<SecretKey> {
    let plain = crypto::decrypt(kek, nonce, sealed, &[])?;
    if plain.len() != KEY_LEN {
        return Err(Error::Corrupted);
    }
    let mut secret = Zeroizing::new([0u8; KEY_LEN]);
    secret.copy_from_slice(&plain);
    Ok(secret)
}

/// Reads the recovery wrap of a version-2 file, if it has one.
fn read_recovery(header: &Header, bytes: &[u8]) -> Result<Option<RecoveryWrap>> {
    if !header.has_recovery() {
        return Ok(None);
    }
    let start = HEADER_LEN + 1 + WRAP_LEN;
    let salt: [u8; SALT_LEN] = bytes[start..start + SALT_LEN].try_into().map_err(|_| Error::Corrupted)?;
    let nonce: [u8; NONCE_LEN] = bytes[start + SALT_LEN..start + SALT_LEN + NONCE_LEN]
        .try_into()
        .map_err(|_| Error::Corrupted)?;
    let sealed = bytes[start + SALT_LEN + NONCE_LEN..start + RECOVERY_WRAP_LEN].to_vec();
    Ok(Some((salt, nonce, sealed)))
}

/// Unwraps the vault key with a recovery code, without the master password. Used by the
/// "unlock with recovery key" flow, immediately before the user sets a new master password.
pub fn key_from_recovery(header: &Header, bytes: &[u8], code: &[u8; KEY_LEN]) -> Result<VaultKey> {
    if header.version != FORMAT_VERSION_V2 {
        return Err(Error::Corrupted);
    }
    let (salt, nonce, sealed) = read_recovery(header, bytes)?.ok_or(Error::Corrupted)?;
    let kek = recovery_kek(code, &salt)?;
    let secret = wrap_secret(&kek, &nonce, &sealed)?;
    let password_nonce: [u8; NONCE_LEN] = bytes[HEADER_LEN + 1..HEADER_LEN + 1 + NONCE_LEN]
        .try_into()
        .map_err(|_| Error::Corrupted)?;
    let password_wrap = bytes[HEADER_LEN + 1 + NONCE_LEN..HEADER_LEN + 1 + WRAP_LEN].to_vec();
    Ok(VaultKey {
        kdf: header.kdf,
        salt: header.salt,
        key: payload_key(&secret),
        wrapping: Some(Wrapping {
            secret,
            nonce: password_nonce,
            wrap: password_wrap,
            recovery: Some((salt, nonce, sealed)),
        }),
    })
}

/// Serializes, pads and encrypts the vault into the bytes of a `.lados` file.
/// Every call uses a new random nonce, as XChaCha20-Poly1305 requires.
pub fn encrypt_vault(vault: &Vault, key: &VaultKey) -> Result<Vec<u8>> {
    let header = Header {
        version: if key.wrapping.is_some() { FORMAT_VERSION_V2 } else { FORMAT_VERSION },
        kdf: key.kdf,
        salt: key.salt,
        nonce: crypto::random_bytes()?,
        flags: if key.has_recovery() { FLAG_RECOVERY } else { 0 },
    };
    let header_bytes = header.to_bytes();

    let json = Zeroizing::new(serde_json::to_vec(vault)?);
    let length = u32::try_from(json.len()).map_err(|_| Error::invalid("The vault is too large"))?;
    let padded_len = (4 + json.len()).div_ceil(PADDING_BLOCK) * PADDING_BLOCK;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(padded_len));
    plaintext.extend_from_slice(&length.to_le_bytes());
    plaintext.extend_from_slice(&json);
    plaintext.resize(padded_len, 0);

    let ciphertext = crypto::encrypt(&key.key, &header.nonce, &plaintext, &header_bytes)?;
    let mut file = Vec::with_capacity(header.prefix_len() + ciphertext.len());
    file.extend_from_slice(&header_bytes);
    if let Some(wrapping) = &key.wrapping {
        file.push(header.flags);
        file.extend_from_slice(&wrapping.nonce);
        file.extend_from_slice(&wrapping.wrap);
        if let Some((salt, nonce, sealed)) = &wrapping.recovery {
            file.extend_from_slice(salt);
            file.extend_from_slice(nonce);
            file.extend_from_slice(sealed);
        }
    }
    file.extend_from_slice(&ciphertext);
    Ok(file)
}

pub fn decrypt_vault(bytes: &[u8], key: &VaultKey) -> Result<Vault> {
    let header = Header::parse(bytes)?;
    let payload = &bytes[header.prefix_len()..];
    let plaintext = crypto::decrypt(&key.key, &header.nonce, payload, &bytes[..HEADER_LEN])?;

    let length = plaintext
        .get(..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or(Error::Corrupted)?;
    let json = plaintext.get(4..4 + length).ok_or(Error::Corrupted)?;
    let vault: Vault = serde_json::from_slice(json).map_err(|_| Error::Corrupted)?;
    if vault.version > SCHEMA_VERSION {
        return Err(Error::UnsupportedVersion(vault.version.into()));
    }
    Ok(vault)
}

// ----- Disk access -----

pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::invalid(format!("File not found: {}", path.display())),
        _ => Error::Io(e),
    })
}

/// Reads a key file. Any file can act as a key file, so only its size is checked.
pub fn read_key_file(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let metadata = fs::metadata(path).map_err(|_| Error::invalid("The key file could not be found"))?;
    if metadata.len() > MAX_KEY_FILE_SIZE {
        return Err(Error::invalid("The key file is too large (maximum 10 MB)"));
    }
    Ok(Zeroizing::new(read_file(path)?))
}

/// Writes the file so that it is either completely replaced or left untouched:
/// a crash or a power cut in the middle can never leave a half-written vault.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let suffix: [u8; 4] = crypto::random_bytes()?;
    let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = sibling(path, &format!(".{suffix}.tmp"));

    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    // On Windows the destination can be briefly locked by an antivirus or another
    // Lados window reading it, so the final rename is retried for a moment.
    let mut attempt = 0;
    loop {
        match fs::rename(&temporary, path) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 10 => {
                attempt += 1;
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
        }
    }
}

/// Keeps a copy of the file as it was before this session changed it.
pub fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        fs::copy(path, sibling(path, ".bak"))?;
    }
    Ok(())
}

/// Where to save local changes when the original file can no longer be overwritten safely.
pub fn conflict_copy_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stamp = Local::now().format("%Y-%m-%d %H%M%S");
    path.with_file_name(format!("{stem} (conflict {stamp}).{FILE_EXTENSION}"))
}

/// Cheap fingerprint used to notice that another program changed the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

pub fn file_stamp(path: &Path) -> Option<FileStamp> {
    fs::metadata(path).ok().map(|metadata| FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryContent;

    fn test_key() -> VaultKey {
        let composite = crypto::composite_key("password", None);
        VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap()
    }

    fn vault_with_entries(count: usize) -> Vault {
        let mut vault = Vault::new("Test").unwrap();
        for i in 0..count {
            let content = EntryContent {
                title: format!("Entry {i}"),
                password: "secret".into(),
                ..Default::default()
            };
            vault.add_entry(None, content).unwrap();
        }
        vault
    }

    #[test]
    fn vaults_round_trip_and_the_password_never_appears_in_clear() {
        let key = test_key();
        let vault = vault_with_entries(3);
        let bytes = encrypt_vault(&vault, &key).unwrap();

        assert!(!bytes.windows(6).any(|w| w == b"secret"));
        assert_eq!(decrypt_vault(&bytes, &key).unwrap(), vault);
    }

    #[test]
    fn a_wrong_password_is_reported_as_such() {
        let bytes = encrypt_vault(&vault_with_entries(1), &test_key()).unwrap();
        let header = Header::parse(&bytes).unwrap();
        let wrong = VaultKey::for_header(&crypto::composite_key("nope", None), &header, &bytes).unwrap();
        assert!(matches!(decrypt_vault(&bytes, &wrong), Err(Error::InvalidCredentials)));
    }

    #[test]
    fn tampering_with_the_header_is_detected() {
        let key = test_key();
        let mut bytes = encrypt_vault(&vault_with_entries(1), &key).unwrap();
        bytes[60] ^= 0xFF; // inside the nonce
        assert!(decrypt_vault(&bytes, &key).is_err());
    }

    #[test]
    fn foreign_and_future_files_are_recognized() {
        assert!(matches!(Header::parse(b"PK\x03\x04 not a vault"), Err(Error::NotAVault)));

        let mut bytes = encrypt_vault(&vault_with_entries(0), &test_key()).unwrap();
        bytes[8] = 99;
        assert!(matches!(Header::parse(&bytes), Err(Error::UnsupportedVersion(99))));
    }

    #[test]
    fn the_file_size_does_not_reveal_the_number_of_entries() {
        let key = test_key();
        let empty = encrypt_vault(&vault_with_entries(0), &key).unwrap();
        let small = encrypt_vault(&vault_with_entries(40), &key).unwrap();
        assert_eq!(empty.len(), small.len());
    }

    #[test]
    fn atomic_writes_replace_the_file_without_leftovers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.lados");

        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        backup(&path).unwrap();

        assert_eq!(read_file(&path).unwrap(), b"second");
        let names: Vec<String> = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "unexpected files: {names:?}");
        assert!(names.contains(&"vault.lados.bak".to_string()));
    }

    #[test]
    fn conflict_copies_live_next_to_the_original() {
        let path = Path::new("C:/vaults/personal.lados");
        let copy = conflict_copy_path(path);
        assert_eq!(copy.parent(), path.parent());
        let name = copy.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("personal (conflict "));
        assert!(name.ends_with(".lados"));
    }

    // ----- Recovery keys (version 2) -----

    fn code(byte: u8) -> [u8; KEY_LEN] {
        [byte; KEY_LEN]
    }

    fn wrapped(composite: &[u8; 32], code: &[u8; KEY_LEN]) -> VaultKey {
        VaultKey::generate(composite, KdfParams::FOR_TESTS)
            .unwrap()
            .with_recovery(composite, code)
            .unwrap()
    }

    /// The point of an account-wide key: one code, many vaults. Nothing in the format ties a code
    /// to a single vault — each vault stores its own salt, nonce and wrap — so the same code opens
    /// all of them.
    #[test]
    fn one_code_wraps_and_opens_several_vaults() {
        let composite = crypto::composite_key("password", None);
        let key_code = code(9);

        for name in ["Personal", "Work"] {
            let mut vault = Vault::new(name).unwrap();
            vault.add_entry(None, EntryContent { title: name.into(), ..Default::default() }).unwrap();
            let bytes = encrypt_vault(&vault, &wrapped(&composite, &key_code)).unwrap();
            let header = Header::parse(&bytes).unwrap();
            assert!(header.has_recovery());

            // The one code opens this vault...
            let opened = key_from_recovery(&header, &bytes, &key_code).unwrap();
            assert_eq!(decrypt_vault(&bytes, &opened).unwrap().name, name);
            // ...and a different code does not.
            assert!(key_from_recovery(&header, &bytes, &code(4)).is_err());
        }
    }

    #[test]
    fn a_version_two_vault_round_trips_and_version_one_is_unchanged() {
        let composite = crypto::composite_key("password", None);
        let vault = vault_with_entries(2);

        let plain = VaultKey::generate(&composite, KdfParams::FOR_TESTS).unwrap();
        let bytes = encrypt_vault(&vault, &plain).unwrap();
        let header = Header::parse(&bytes).unwrap();
        assert_eq!(header.version, 1);
        assert!(!header.has_recovery());
        // A version-1 file has no recovery wrap, so a code cannot open it.
        assert!(matches!(key_from_recovery(&header, &bytes, &code(1)), Err(Error::Corrupted)));

        let key = wrapped(&composite, &code(7));
        let bytes = encrypt_vault(&vault, &key).unwrap();
        let header = Header::parse(&bytes).unwrap();
        assert_eq!(header.version, 2);
        assert!(header.has_recovery());

        let reopened = VaultKey::for_header(&composite, &header, &bytes).unwrap();
        assert_eq!(decrypt_vault(&bytes, &reopened).unwrap(), vault);
    }

    #[test]
    fn the_recovery_code_opens_a_vault_without_the_password() {
        let composite = crypto::composite_key("password", None);
        let vault = vault_with_entries(3);
        let bytes = encrypt_vault(&vault, &wrapped(&composite, &code(9))).unwrap();
        let header = Header::parse(&bytes).unwrap();

        let recovered = key_from_recovery(&header, &bytes, &code(9)).unwrap();
        assert_eq!(decrypt_vault(&bytes, &recovered).unwrap(), vault);

        assert!(matches!(
            key_from_recovery(&header, &bytes, &code(10)),
            Err(Error::InvalidCredentials)
        ));
    }

    #[test]
    fn changing_the_master_password_keeps_the_recovery_key_working() {
        let first = crypto::composite_key("first", None);
        let second = crypto::composite_key("second", None);
        let vault = vault_with_entries(2);

        let bytes = encrypt_vault(&vault, &wrapped(&first, &code(5))).unwrap();
        let header = Header::parse(&bytes).unwrap();
        let opened = VaultKey::for_header(&first, &header, &bytes).unwrap();
        assert!(opened.has_recovery());

        // Exactly what a master-password change does: re-wrap the same secret.
        let rewrapped = opened.rewrap(&second, KdfParams::FOR_TESTS).unwrap();
        let bytes = encrypt_vault(&vault, &rewrapped).unwrap();
        let header = Header::parse(&bytes).unwrap();

        // The new password opens it, the old one no longer does...
        let with_new = VaultKey::for_header(&second, &header, &bytes).unwrap();
        assert_eq!(decrypt_vault(&bytes, &with_new).unwrap(), vault);
        assert!(matches!(
            VaultKey::for_header(&first, &header, &bytes),
            Err(Error::InvalidCredentials)
        ));
        // ...and the recovery code still does, which is the whole point.
        let recovered = key_from_recovery(&header, &bytes, &code(5)).unwrap();
        assert_eq!(decrypt_vault(&bytes, &recovered).unwrap(), vault);
    }

    #[test]
    fn a_damaged_recovery_wrap_does_not_lock_out_the_password() {
        let composite = crypto::composite_key("password", None);
        let vault = vault_with_entries(1);
        let mut bytes = encrypt_vault(&vault, &wrapped(&composite, &code(3))).unwrap();
        let header = Header::parse(&bytes).unwrap();

        // Corrupt the last byte of the prefix, which is inside the recovery wrap.
        bytes[header.prefix_len() - 1] ^= 0xFF;

        let opened = VaultKey::for_header(&composite, &header, &bytes).unwrap();
        assert_eq!(decrypt_vault(&bytes, &opened).unwrap(), vault);
        assert!(key_from_recovery(&header, &bytes, &code(3)).is_err());
    }

    #[test]
    fn forgetting_the_recovery_wrap_leaves_a_working_version_two_vault() {
        let composite = crypto::composite_key("password", None);
        let plain = wrapped(&composite, &code(1)).without_recovery();
        assert!(!plain.has_recovery());

        let bytes = encrypt_vault(&vault_with_entries(0), &plain).unwrap();
        let header = Header::parse(&bytes).unwrap();
        assert_eq!(header.version, 2);
        assert!(!header.has_recovery());
        assert!(key_from_recovery(&header, &bytes, &code(1)).is_err());
        assert!(VaultKey::for_header(&composite, &header, &bytes).is_ok());
    }
}
