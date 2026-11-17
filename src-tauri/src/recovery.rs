//! The optional recovery key: a high-entropy code, kept by the user in a file, that can open a
//! vault when the master password is lost.
//!
//! Lados never stores the code — it exists only in the file the user saves. Whoever holds that
//! file can open the vault, which is exactly why it must be kept away from the vault itself.

use chrono::Local;
use zeroize::Zeroizing;

use crate::crypto::{self, KEY_LEN, SecretKey};
use crate::error::Result;

/// Hex characters in a code (32 bytes).
const CODE_CHARS: usize = KEY_LEN * 2;

/// A new recovery code: the text the user keeps, and the 32 bytes it encodes.
pub fn generate() -> Result<(Zeroizing<String>, SecretKey)> {
    let key = Zeroizing::new(crypto::random_bytes::<KEY_LEN>()?);
    let text = Zeroizing::new(group(&hex(&key[..])));
    Ok((text, key))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// Groups hexadecimal characters in fours: `A1B2 C3D4 …`.
fn group(hex: &str) -> String {
    hex.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads a code back out of whatever the user pasted: the file, or just the string.
///
/// Only a line made *entirely* of hexadecimal characters, spaces and dashes can be the code, so
/// the rest of the file — the vault name, the date — can never be mistaken for it.
pub fn parse_code(text: &str) -> Option<SecretKey> {
    for line in text.lines() {
        let only_code = line
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ' ' || c == '-' || c == '\t');
        if !only_code {
            continue;
        }
        if let Some(key) = decode(line) {
            return Some(key);
        }
    }
    // A bare code pasted on its own, possibly wrapped over several lines.
    let only_code = text
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c.is_whitespace() || c == '-');
    if only_code {
        return decode(text);
    }
    None
}

/// Decodes 64 hexadecimal characters (spaces and dashes ignored) into the 32-byte code.
fn decode(text: &str) -> Option<SecretKey> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if digits.len() != CODE_CHARS {
        return None;
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    for (index, pair) in digits.chunks(2).enumerate() {
        let high = pair[0].to_digit(16)?;
        let low = pair[1].to_digit(16)?;
        key[index] = (high * 16 + low) as u8;
    }
    Some(key)
}

/// The contents of the recovery file the user saves.
pub fn file_contents(vault: &str, code: &str) -> Zeroizing<String> {
    let stamp = Local::now().format("%Y-%m-%d %H:%M");
    Zeroizing::new(format!(
        "Lados recovery key\n\
         ==================\n\n\
         Vault:    {vault}\n\
         Created:  {stamp}\n\n\
         Keep this file somewhere safe, and away from the vault and its backups. Anyone who has\n\
         it can open the vault without the master password. Lados keeps no copy: if you lose\n\
         both this file and the password, the vault can no longer be opened.\n\n\
         The key has no expiry and can be used as often as you need. Creating a new recovery\n\
         key for this vault replaces it, and then this file stops working.\n\n\
         Recovery key\n\
         ------------\n\n\
         {code}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_round_trips_through_the_file() {
        let (code, _key) = generate().unwrap();
        let file = file_contents("personal.lados", &code);
        let parsed = parse_code(&file).expect("the code must be readable back");
        let expected = decode(&code).expect("the generated code is well formed");
        assert_eq!(&parsed[..], &expected[..]);
    }

    #[test]
    fn codes_are_unique_and_well_formed() {
        let (first, _) = generate().unwrap();
        let (second, _) = generate().unwrap();
        assert_ne!(*first, *second);
        let digits: String = first.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        assert_eq!(digits.len(), CODE_CHARS);
    }

    #[test]
    fn stray_digits_in_the_file_are_never_mistaken_for_the_code() {
        let (code, _key) = generate().unwrap();
        let file = file_contents("20260101-20270101.lados", &code);
        assert!(parse_code(&file).is_some());
        assert!(parse_code("Vault: 1234\nCreated: 2026-11-17 14:03\n").is_none());
        assert!(parse_code("").is_none());
    }

    #[test]
    fn a_short_or_damaged_code_is_rejected() {
        assert!(parse_code("A1B2 C3D4").is_none());
        assert!(parse_code(&"A".repeat(CODE_CHARS + 1)).is_none());
    }

    #[test]
    fn a_code_pasted_on_its_own_is_accepted() {
        let (code, _key) = generate().unwrap();
        assert!(parse_code(&code).is_some());
        let bare: String = code.chars().filter(|c| *c != ' ').collect();
        assert!(parse_code(&bare).is_some());
    }
}
