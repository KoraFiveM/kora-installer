//! Recovery codes: ten single-use codes shown once when the one-time-code sign-in is set up, for a lost phone.
//! 50 random bits each (`abcde-fghij`), so a fast salted SHA-256 is enough to store them (they cannot be
//! guessed, and an argon2 cost per code would only slow the sign-in); a used code is removed.
use sha2::{Digest, Sha256};

use crate::security::ct_eq;

pub const COUNT: usize = 10;
const ALPHABET: &[u8; 32] = b"abcdefghjkmnopqrstuvwxyz23456789"; // no i, l, 0, 1: easy to read out

fn pick(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|_| "the system random number generator is unavailable".to_string())?;
    Ok(b.iter().map(|x| ALPHABET[(*x & 31) as usize] as char).collect())
}

/// `COUNT` new codes.
pub fn generate() -> Result<Vec<String>, String> {
    (0..COUNT).map(|_| pick(10).map(|c| format!("{}-{}", &c[..5], &c[5..]))).collect()
}

/// Lower-case, without spaces or dashes: how a code is compared.
pub fn normalise(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn digest(salt: &str, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update([0u8]);
    h.update(normalise(code).as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `salt:hash` for storing.
pub fn store_form(code: &str) -> Result<String, String> {
    let salt = pick(16)?;
    Ok(format!("{salt}:{}", digest(&salt, code)))
}

/// Which stored entry `given` matches, comparing against all of them.
pub fn find(stored: &[String], given: &str) -> Option<usize> {
    let mut found = None;
    for (i, s) in stored.iter().enumerate() {
        if let Some((salt, hash)) = s.split_once(':') {
            if ct_eq(&digest(salt, given), hash) {
                found = Some(i);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stored_hashed_and_found_in_any_spelling() {
        let codes = generate().unwrap();
        assert_eq!(codes.len(), COUNT);
        assert!(codes.iter().all(|c| c.len() == 11 && c.as_bytes()[5] == b'-'));
        let stored: Vec<String> = codes.iter().map(|c| store_form(c).unwrap()).collect();
        assert!(stored.iter().all(|s| !codes.iter().any(|c| s.contains(&normalise(c)))), "no code in the store");
        assert_eq!(find(&stored, &codes[3]), Some(3));
        assert_eq!(find(&stored, &codes[3].to_uppercase().replace('-', " ")), Some(3));
        assert_eq!(find(&stored, "aaaaa-aaaaa"), None);
        assert_eq!(find(&[], &codes[0]), None);
        let mut c = codes.clone();
        c.sort();
        c.dedup();
        assert_eq!(c.len(), COUNT, "no repeats");
    }
}
