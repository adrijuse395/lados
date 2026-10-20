use rand::RngExt;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?/~";
/// Characters that are easy to confuse when a password has to be read or typed by hand.
const AMBIGUOUS: &str = "Il1O0o";

pub const MIN_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneratorOptions {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        GeneratorOptions {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: false,
        }
    }
}

/// Generates a random password with `rand`'s thread generator, a cryptographically secure
/// generator seeded by the operating system.
pub fn generate(options: &GeneratorOptions) -> Result<String> {
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&options.length) {
        return Err(Error::invalid(format!(
            "The length must be between {MIN_LENGTH} and {MAX_LENGTH} characters"
        )));
    }

    let groups: Vec<Vec<char>> = [
        (options.lowercase, LOWERCASE),
        (options.uppercase, UPPERCASE),
        (options.digits, DIGITS),
        (options.symbols, SYMBOLS),
    ]
    .into_iter()
    .filter(|(enabled, _)| *enabled)
    .map(|(_, characters)| {
        characters
            .chars()
            .filter(|c| !(options.exclude_ambiguous && AMBIGUOUS.contains(*c)))
            .collect()
    })
    .collect();
    if groups.is_empty() {
        return Err(Error::invalid("Choose at least one type of character"));
    }

    let mut rng = rand::rng();
    let pool: Vec<char> = groups.concat();
    let mut password = Zeroizing::new(Vec::with_capacity(options.length));
    // One character of each selected type first, so the result always follows the chosen rules.
    for group in &groups {
        password.push(group[rng.random_range(0..group.len())]);
    }
    while password.len() < options.length {
        password.push(pool[rng.random_range(0..pool.len())]);
    }
    password.shuffle(&mut rng);
    Ok(password.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_follow_the_options() {
        let options = GeneratorOptions {
            length: 32,
            ..Default::default()
        };
        for _ in 0..50 {
            let password = generate(&options).unwrap();
            assert_eq!(password.chars().count(), 32);
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(password.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn ambiguous_characters_can_be_excluded() {
        let options = GeneratorOptions {
            length: MAX_LENGTH,
            symbols: false,
            exclude_ambiguous: true,
            ..Default::default()
        };
        for _ in 0..20 {
            let password = generate(&options).unwrap();
            assert!(!password.chars().any(|c| AMBIGUOUS.contains(c)));
        }
    }

    #[test]
    fn invalid_options_are_rejected() {
        let nothing_selected = GeneratorOptions {
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            ..Default::default()
        };
        assert!(generate(&nothing_selected).is_err());
        let too_short = GeneratorOptions {
            length: 2,
            ..Default::default()
        };
        assert!(generate(&too_short).is_err());
    }

    #[test]
    fn passwords_are_not_repeated() {
        let options = GeneratorOptions::default();
        assert_ne!(generate(&options).unwrap(), generate(&options).unwrap());
    }
}
