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

use crate::domain::Strength;

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

/// Size of the alphabet drawn from when sampling with `classes`. Mirrors
/// `generate`'s union-of-classes rule, including the lowercase-fallback when
/// every class is disabled. Used by the entropy estimate.
pub fn alphabet_size(classes: CharClasses) -> usize {
    let mut size = 0;
    if classes.upper {
        size += UPPER.len();
    }
    if classes.lower {
        size += LOWER.len();
    }
    if classes.digits {
        size += DIGITS.len();
    }
    if classes.symbols {
        size += SYMBOLS.len();
    }
    if size == 0 {
        size = LOWER.len();
    }
    size
}

/// Shannon-style entropy estimate for a uniformly-sampled password of
/// `length` chars over the union of the enabled classes:
/// `bits = length * log2(|alphabet|)`. This is an *upper bound* on real
/// entropy (it ignores user-visible patterns like dictionary words), so
/// `generate`'s output should hit it almost exactly while user-typed
/// passwords often score lower under zxcvbn.
pub fn estimate_bits(length: usize, classes: CharClasses) -> u32 {
    let size = alphabet_size(classes) as f32;
    (length as f32 * size.log2()) as u32
}

/// Bucket entropy bits into the same three-band Strength enum used elsewhere
/// in the UI (so the generator card and the entry-detail health bar use one
/// vocabulary). Thresholds chosen to match common guidance: <40 bits is
/// brute-forceable, 40–60 bits is online-attack-resistant, ≥60 bits is
/// offline-attack-resistant.
pub fn strength_from_bits(bits: u32) -> Strength {
    if bits < 40 {
        Strength::Weak
    } else if bits < 60 {
        Strength::Fair
    } else {
        Strength::Strong
    }
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
    fn alphabet_sizes_match_class_lengths() {
        // Sanity-check that the byte-string constants above are still 26/26/10/21
        // — the entropy estimate hard-depends on these magnitudes.
        assert_eq!(UPPER.len(), 26);
        assert_eq!(LOWER.len(), 26);
        assert_eq!(DIGITS.len(), 10);
        assert_eq!(SYMBOLS.len(), 21);

        assert_eq!(alphabet_size(CharClasses::default()), 26 + 26 + 10 + 21);
        assert_eq!(
            alphabet_size(CharClasses { upper: false, lower: true, digits: false, symbols: false }),
            26
        );
    }

    #[test]
    fn estimate_bits_scales_with_length_and_classes() {
        let lower_only = CharClasses { upper: false, lower: true, digits: false, symbols: false };
        // 18 chars * log2(26) ≈ 18 * 4.7 = 84
        let b = estimate_bits(18, lower_only);
        assert!((83..=85).contains(&b), "bits={b}");

        // Adding upper doubles the alphabet (52) → log2 grows by 1 → +18 bits.
        let with_upper = CharClasses { upper: true, ..lower_only };
        let b2 = estimate_bits(18, with_upper);
        assert!(b2 >= b + 17 && b2 <= b + 19, "bits={b2} expected ~{}", b + 18);

        // Length scales linearly: doubling length doubles bits.
        let half = estimate_bits(9, CharClasses::default());
        let full = estimate_bits(18, CharClasses::default());
        assert!(full >= 2 * half - 1 && full <= 2 * half + 1, "half={half} full={full}");
    }

    #[test]
    fn strength_buckets_match_thresholds() {
        assert_eq!(strength_from_bits(0), crate::domain::Strength::Weak);
        assert_eq!(strength_from_bits(39), crate::domain::Strength::Weak);
        assert_eq!(strength_from_bits(40), crate::domain::Strength::Fair);
        assert_eq!(strength_from_bits(59), crate::domain::Strength::Fair);
        assert_eq!(strength_from_bits(60), crate::domain::Strength::Strong);
        assert_eq!(strength_from_bits(200), crate::domain::Strength::Strong);
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
