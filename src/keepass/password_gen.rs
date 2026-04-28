//! Random password generator used by the Add Entry modal's "Generate" button.
//!
//! Uses [`rand`]'s `OsRng`-backed thread RNG so generated passwords are
//! cryptographically suitable for credentials. Each character is picked
//! uniformly from the union of the enabled character classes, then we
//! post-check that at least one character from every enabled class is present
//! and resample until that holds — this guarantees the user's class choices
//! are reflected even at short lengths.

use rand::Rng;
use rand::seq::IndexedRandom;

const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{}<>?";

#[derive(Clone, Copy, Debug)]
pub struct CharClasses {
    pub upper: bool,
    pub lower: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for CharClasses {
    fn default() -> Self {
        Self {
            upper: true,
            lower: true,
            digits: true,
            symbols: true,
        }
    }
}

/// Generate a password of `length` characters drawn from the enabled classes.
/// Falls back to lowercase letters if every class was disabled (which the UI
/// shouldn't allow, but we don't want to panic on a contradictory input).
pub fn generate(length: usize, classes: CharClasses) -> String {
    let length = length.clamp(4, 128);

    let mut alphabet: Vec<u8> = Vec::with_capacity(80);
    let mut required: Vec<&[u8]> = Vec::with_capacity(4);
    if classes.upper {
        alphabet.extend_from_slice(UPPER);
        required.push(UPPER);
    }
    if classes.lower {
        alphabet.extend_from_slice(LOWER);
        required.push(LOWER);
    }
    if classes.digits {
        alphabet.extend_from_slice(DIGITS);
        required.push(DIGITS);
    }
    if classes.symbols {
        alphabet.extend_from_slice(SYMBOLS);
        required.push(SYMBOLS);
    }
    if alphabet.is_empty() {
        alphabet.extend_from_slice(LOWER);
        required.push(LOWER);
    }

    let mut rng = rand::rng();

    // Sample until every required class shows up at least once. With 4
    // classes and length ≥ 8 this almost always passes on the first try.
    loop {
        let bytes: Vec<u8> = (0..length)
            .map(|_| {
                let i = rng.random_range(0..alphabet.len());
                alphabet[i]
            })
            .collect();
        if required
            .iter()
            .all(|class| bytes.iter().any(|b| class.contains(b)))
        {
            // Safety: alphabet is ASCII so the bytes are always valid UTF-8.
            return String::from_utf8(bytes).expect("ascii alphabet");
        }
    }
}

/// Pick one random char from a class. Convenience for unit tests.
#[allow(dead_code)]
fn pick_one(class: &[u8]) -> u8 {
    *class.choose(&mut rand::rng()).expect("non-empty class")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_password_length_matches() {
        let pw = generate(20, CharClasses::default());
        assert_eq!(pw.chars().count(), 20);
    }

    #[test]
    fn respects_only_lowercase() {
        let classes = CharClasses {
            upper: false,
            lower: true,
            digits: false,
            symbols: false,
        };
        let pw = generate(40, classes);
        assert!(pw.chars().all(|c| c.is_ascii_lowercase()));
    }

    #[test]
    fn always_includes_each_required_class() {
        // 100 trials × 4 classes — a single failure means the resample loop
        // is broken (or astronomically unlucky).
        for _ in 0..100 {
            let pw = generate(8, CharClasses::default());
            let bytes = pw.as_bytes();
            assert!(bytes.iter().any(|b| UPPER.contains(b)), "no upper: {pw}");
            assert!(bytes.iter().any(|b| LOWER.contains(b)), "no lower: {pw}");
            assert!(bytes.iter().any(|b| DIGITS.contains(b)), "no digit: {pw}");
            assert!(bytes.iter().any(|b| SYMBOLS.contains(b)), "no symbol: {pw}");
        }
    }

    #[test]
    fn no_classes_falls_back_to_lowercase() {
        let pw = generate(10, CharClasses {
            upper: false,
            lower: false,
            digits: false,
            symbols: false,
        });
        assert!(pw.chars().all(|c| c.is_ascii_lowercase()));
    }
}
