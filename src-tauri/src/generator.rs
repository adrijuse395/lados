use rand::RngExt;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?/~";
const SPACES: &str = " ";
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
    /// Include the space character in the pool (KeePass-style optional set).
    pub spaces: bool,
    /// Extra characters that must never appear in the result.
    pub exclude_characters: String,
    /// Prefer a letter (a–z / A–Z) as the first character.
    pub begin_with_letter: bool,
    /// Do not reuse any character within the same password.
    pub no_repeating: bool,
    /// Guarantee at least one character from every selected class (or the configured minimum).
    pub require_each_type: bool,
    pub min_lowercase: u8,
    pub min_uppercase: u8,
    pub min_digits: u8,
    pub min_symbols: u8,
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
            spaces: false,
            exclude_characters: String::new(),
            begin_with_letter: false,
            no_repeating: false,
            require_each_type: true,
            min_lowercase: 0,
            min_uppercase: 0,
            min_digits: 0,
            min_symbols: 0,
        }
    }
}

#[derive(Clone, Copy)]
enum CharClass {
    Lowercase,
    Uppercase,
    Digits,
    Symbols,
    Spaces,
}

impl CharClass {
    fn alphabet(self) -> &'static str {
        match self {
            CharClass::Lowercase => LOWERCASE,
            CharClass::Uppercase => UPPERCASE,
            CharClass::Digits => DIGITS,
            CharClass::Symbols => SYMBOLS,
            CharClass::Spaces => SPACES,
        }
    }

    fn enabled(self, options: &GeneratorOptions) -> bool {
        match self {
            CharClass::Lowercase => options.lowercase,
            CharClass::Uppercase => options.uppercase,
            CharClass::Digits => options.digits,
            CharClass::Symbols => options.symbols,
            CharClass::Spaces => options.spaces,
        }
    }

    fn minimum(self, options: &GeneratorOptions) -> usize {
        let configured = match self {
            CharClass::Lowercase => options.min_lowercase,
            CharClass::Uppercase => options.min_uppercase,
            CharClass::Digits => options.min_digits,
            CharClass::Symbols => options.min_symbols,
            CharClass::Spaces => 0,
        } as usize;
        if !self.enabled(options) {
            return 0;
        }
        if configured > 0 {
            configured
        } else if options.require_each_type && !matches!(self, CharClass::Spaces) {
            1
        } else {
            0
        }
    }
}

const CLASSES: [CharClass; 5] = [
    CharClass::Lowercase,
    CharClass::Uppercase,
    CharClass::Digits,
    CharClass::Symbols,
    CharClass::Spaces,
];

fn is_excluded(c: char, options: &GeneratorOptions) -> bool {
    (options.exclude_ambiguous && AMBIGUOUS.contains(c)) || options.exclude_characters.contains(c)
}

fn chars_for(class: CharClass, options: &GeneratorOptions) -> Vec<char> {
    class
        .alphabet()
        .chars()
        .filter(|c| !is_excluded(*c, options))
        .collect()
}

/// Generates a random password with `rand`'s thread generator, a cryptographically secure
/// generator seeded by the operating system.
pub fn generate(options: &GeneratorOptions) -> Result<String> {
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&options.length) {
        return Err(Error::invalid(format!(
            "The length must be between {MIN_LENGTH} and {MAX_LENGTH} characters"
        )));
    }

    let groups: Vec<(CharClass, Vec<char>)> = CLASSES
        .into_iter()
        .filter(|class| class.enabled(options))
        .map(|class| (class, chars_for(class, options)))
        .filter(|(_, chars)| !chars.is_empty())
        .collect();

    if groups.is_empty() {
        return Err(Error::invalid(
            "Choose at least one type of character that is not fully excluded",
        ));
    }

    let required: Vec<(CharClass, usize, &Vec<char>)> = groups
        .iter()
        .filter_map(|(class, chars)| {
            let min = class.minimum(options);
            (min > 0).then_some((*class, min, chars))
        })
        .collect();

    let required_total: usize = required.iter().map(|(_, min, _)| *min).sum();
    if required_total > options.length {
        return Err(Error::invalid(format!(
            "The minimum character counts ({required_total}) exceed the password length ({})",
            options.length
        )));
    }

    let pool: Vec<char> = groups.iter().flat_map(|(_, chars)| chars.iter().copied()).collect();
    if pool.is_empty() {
        return Err(Error::invalid("No characters left after applying exclusions"));
    }

    if options.no_repeating && options.length > pool.len() {
        return Err(Error::invalid(format!(
            "With “no repeating characters”, the length cannot exceed {} for the current alphabet",
            pool.len()
        )));
    }

    let letter_pool: Vec<char> = groups
        .iter()
        .filter(|(class, _)| matches!(class, CharClass::Lowercase | CharClass::Uppercase))
        .flat_map(|(_, chars)| chars.iter().copied())
        .collect();
    if options.begin_with_letter && letter_pool.is_empty() {
        return Err(Error::invalid(
            "“Begin with a letter” needs lowercase or uppercase characters enabled",
        ));
    }

    use std::collections::HashSet;

    let mut rng = rand::rng();
    let mut password = Zeroizing::new(Vec::with_capacity(options.length));
    let mut used = HashSet::<char>::new();

    let pick = |rng: &mut rand::rngs::ThreadRng,
                alphabet: &[char],
                used: &HashSet<char>,
                no_repeat: bool|
     -> Result<char> {
        let available: Vec<char> = if no_repeat {
            alphabet.iter().copied().filter(|c| !used.contains(c)).collect()
        } else {
            alphabet.to_vec()
        };
        if available.is_empty() {
            return Err(Error::invalid(
                "Not enough distinct characters for the current options",
            ));
        }
        Ok(available[rng.random_range(0..available.len())])
    };

    if options.begin_with_letter {
        let c = pick(&mut rng, &letter_pool, &used, options.no_repeating)?;
        password.push(c);
        used.insert(c);
    }

    for (_, min, alphabet) in &required {
        let already = password
            .iter()
            .filter(|c| alphabet.contains(c))
            .count();
        for _ in already..*min {
            let c = pick(&mut rng, alphabet, &used, options.no_repeating)?;
            password.push(c);
            used.insert(c);
        }
    }

    while password.len() < options.length {
        let c = pick(&mut rng, &pool, &used, options.no_repeating)?;
        password.push(c);
        used.insert(c);
    }

    if options.begin_with_letter {
        // Keep the first character fixed; shuffle only the rest so the letter rule holds.
        password[1..].shuffle(&mut rng);
    } else {
        password.shuffle(&mut rng);
    }

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

    #[test]
    fn begin_with_letter_and_custom_excludes() {
        let options = GeneratorOptions {
            length: 24,
            begin_with_letter: true,
            exclude_characters: "aeiou".into(),
            digits: false,
            symbols: false,
            ..Default::default()
        };
        for _ in 0..30 {
            let password = generate(&options).unwrap();
            let first = password.chars().next().unwrap();
            assert!(first.is_ascii_alphabetic());
            assert!(!password.chars().any(|c| "aeiou".contains(c)));
        }
    }

    #[test]
    fn no_repeating_and_minimum_digits() {
        let options = GeneratorOptions {
            length: 16,
            symbols: false,
            no_repeating: true,
            min_digits: 4,
            ..Default::default()
        };
        for _ in 0..30 {
            let password = generate(&options).unwrap();
            let mut chars: Vec<char> = password.chars().collect();
            let original = chars.clone();
            chars.sort_unstable();
            chars.dedup();
            assert_eq!(chars.len(), original.len());
            assert!(password.chars().filter(|c| c.is_ascii_digit()).count() >= 4);
        }
    }

    #[test]
    fn spaces_can_be_included() {
        let options = GeneratorOptions {
            length: 40,
            spaces: true,
            require_each_type: false,
            lowercase: true,
            uppercase: false,
            digits: false,
            symbols: false,
            ..Default::default()
        };
        let mut saw_space = false;
        for _ in 0..40 {
            if generate(&options).unwrap().contains(' ') {
                saw_space = true;
                break;
            }
        }
        assert!(saw_space);
    }
}
