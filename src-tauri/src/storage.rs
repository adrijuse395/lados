//! The `.lados` file format and everything that touches the disk.
//!
//! Layout (integers are little-endian):
//!
//! | Offset | Size | Field                                           |
//! |--------|------|-------------------------------------------------|
//! | 0      | 8    | Magic bytes `LADOSVLT`                          |
//! | 8      | 2    | Format version                                  |
//! | 10     | 1    | Key derivation function (1 = Argon2id)          |
//! | 11     | 4    | Argon2 memory, in KiB                           |
//! | 15     | 4    | Argon2 iterations                               |
//! | 19     | 4    | Argon2 parallelism                              |
//! | 23     | 32   | Salt                                            |
//! | 55     | 1    | Cipher (1 = XChaCha20-Poly1305)                 |
//! | 56     | 24   | Nonce                                           |
//! | 80     | ...  | Encrypted padded vault, followed by a 16-byte tag |
//!
//! The header is authenticated together with the ciphertext, and the vault is padded to
//! 64 KiB blocks so the file size does not reveal how many entries it holds.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::Local;
use zeroize::Zeroizing;

use crate::crypto::{self, KdfParams, NONCE_LEN, SALT_LEN, SecretKey};
use crate::error::{Error, Result};
use crate::models::{SCHEMA_VERSION, Vault};

pub const FILE_EXTENSION: &str = "lados";
const MAGIC: &[u8; 8] = b"LADOSVLT";
const FORMAT_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_XCHACHA20_POLY1305: u8 = 1;
const HEADER_LEN: usize = 80;
const PADDING_BLOCK: usize = 64 * 1024;
const MAX_KEY_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// The unencrypted part of a vault file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl Header {
    fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[0..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[10] = KDF_ARGON2ID;
        bytes[11..15].copy_from_slice(&self.kdf.memory_kib.to_le_bytes());
        bytes[15..19].copy_from_slice(&self.kdf.iterations.to_le_bytes());
        bytes[19..23].copy_from_slice(&self.kdf.parallelism.to_le_bytes());
        bytes[23..55].copy_from_slice(&self.salt);
        bytes[55] = CIPHER_XCHACHA20_POLY1305;
        bytes[56..80].copy_from_slice(&self.nonce);
        bytes
    }

    pub fn parse(bytes: &[u8]) -> Result<Header> {
        if bytes.len() < 10 || &bytes[0..8] != MAGIC {
            return Err(Error::NotAVault);
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != FORMAT_VERSION {
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
        Ok(Header {
            kdf,
            salt: bytes[23..55].try_into().map_err(|_| Error::Corrupted)?,
            nonce: bytes[56..80].try_into().map_err(|_| Error::Corrupted)?,
        })
    }
}

/// The derived encryption key plus the parameters needed to write a matching header.
pub struct VaultKey {
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    key: SecretKey,
}

impl VaultKey {
    /// A key with a fresh random salt, for a new vault or a new master password.
    pub fn generate(composite: &[u8; 32], kdf: KdfParams) -> Result<VaultKey> {
        let salt = crypto::random_bytes()?;
        let key = crypto::derive_key(composite, &salt, kdf)?;
        Ok(VaultKey { kdf, salt, key })
    }

    /// The key that matches an existing file. This is the slow step of unlocking.
    pub fn for_header(composite: &[u8; 32], header: &Header) -> Result<VaultKey> {
        let key = crypto::derive_key(composite, &header.salt, header.kdf)?;
        Ok(VaultKey {
            kdf: header.kdf,
            salt: header.salt,
            key,
        })
    }

    pub fn matches(&self, header: &Header) -> bool {
        self.kdf == header.kdf && self.salt == header.salt
    }
}

/// Serializes, pads and encrypts the vault into the bytes of a `.lados` file.
/// Every call uses a new random nonce, as XChaCha20-Poly1305 requires.
pub fn encrypt_vault(vault: &Vault, key: &VaultKey) -> Result<Vec<u8>> {
    let header = Header {
        kdf: key.kdf,
        salt: key.salt,
        nonce: crypto::random_bytes()?,
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
    let mut file = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    file.extend_from_slice(&header_bytes);
    file.extend_from_slice(&ciphertext);
    Ok(file)
}

pub fn decrypt_vault(bytes: &[u8], key: &VaultKey) -> Result<Vault> {
    let header = Header::parse(bytes)?;
    let plaintext = crypto::decrypt(&key.key, &header.nonce, &bytes[HEADER_LEN..], &bytes[..HEADER_LEN])?;

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
        let wrong = VaultKey::for_header(&crypto::composite_key("nope", None), &header).unwrap();
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
}
