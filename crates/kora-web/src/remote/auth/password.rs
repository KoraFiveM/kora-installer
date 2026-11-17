//! Passwords for the one-time-code sign-in (where a passkey cannot be used): argon2id, a strength check,
//! and a dummy verification so an unknown name costs the same time as a wrong password.
//!
//! The hash is the PHC string (`$argon2id$v=19$m=19456,t=2,p=1$salt$hash`), which names its own parameters, so
//! they can be raised later and old hashes still verify. 19 MiB, 2 passes, 1 lane is the OWASP minimum for
//! argon2id; sign-in is rare, so the cost is not felt. No primitive is implemented here: hashing and
//! the constant-time comparison are the `argon2` crate's (RustCrypto).
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

pub const MIN_LEN: usize = 12;
pub const MAX_LEN: usize = 128;

fn hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(19_456, 2, 1, None).expect("fixed, valid parameters"))
}

pub fn hash(password: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| "the system random number generator is unavailable".to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
    hasher().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| format!("could not hash the password ({e})"))
}

/// Does `password` match the saved PHC `hash`? An unreadable hash never matches.
pub fn verify(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(h) => Argon2::default().verify_password(password.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

/// Spend the time of a verification when there is nothing to compare with (unknown user, no password set), so
/// the answer's timing does not say which names exist.
pub fn verify_dummy(password: &str) {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let h = DUMMY.get_or_init(|| hash("kora dummy password for timing").unwrap_or_default());
    let _ = verify(h, password);
}

const COMMON: &[&str] = &[
    "password", "passw0rd", "123456789", "1234567890", "qwertyuiop", "iloveyou", "letmein", "welcome", "administrator", "changeme", "admin", "football", "monkey", "dragon", "master", "sunshine", "princess", "trustno1",
    "baseball", "superman", "password1", "qwerty", "abc123", "fivem", "kora", "txadmin", "gameserver",
];

/// Is `password` good enough? The messages say what to do. Length matters most; this is not a composition
/// puzzle. `user` is the account name (a password containing it is refused).
pub fn strength(password: &str, user: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < MIN_LEN {
        return Err(format!("Use at least {MIN_LEN} characters. A few random words in a row make a strong, easy-to-remember password."));
    }
    if n > MAX_LEN {
        return Err(format!("Use at most {MAX_LEN} characters."));
    }
    let lower = password.to_lowercase();
    let squashed: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    let distinct = lower.chars().collect::<std::collections::HashSet<_>>().len();
    if distinct < 5 {
        return Err("That password repeats the same few characters. Use a longer, less predictable one.".into());
    }
    if !user.is_empty() && squashed.contains(&user.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>()) && user.len() >= 3 {
        return Err("The password must not contain your user name.".into());
    }
    if COMMON.iter().any(|c| squashed.contains(c)) {
        return Err("That password is one of the first ones anybody tries. Pick something else, such as a few unrelated words.".into());
    }
    let bytes: Vec<u32> = lower.chars().map(|c| c as u32).collect();
    let run = bytes.windows(2).filter(|w| w[1] == w[0] + 1 || w[0] == w[1] + 1).count();
    if run + 1 >= n.saturating_sub(2) {
        return Err("That password is a simple sequence (like abcdef or 123456). Pick something less predictable.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_verifies_only_its_password_and_names_its_parameters() {
        let h = hash("correct horse battery staple").unwrap();
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert!(verify(&h, "correct horse battery staple"));
        assert!(!verify(&h, "correct horse battery stapl") && !verify(&h, ""));
        assert_ne!(h, hash("correct horse battery staple").unwrap(), "a new salt each time");
        assert!(!verify("not a hash", "x") && !verify("", ""));
        verify_dummy("anything");
    }

    #[test]
    fn weak_passwords_are_refused_with_advice() {
        assert!(strength("short", "sam").unwrap_err().contains("at least 12"));
        assert!(strength(&"a".repeat(129), "sam").unwrap_err().contains("at most"));
        assert!(strength("aaaaaaaaaaaaaaaa", "sam").unwrap_err().contains("repeats"));
        assert!(strength("Password123456", "sam").unwrap_err().contains("first ones"));
        assert!(strength("my-sam-secret-1234", "sam").unwrap_err().contains("user name"));
        assert!(strength("abcdefghijklmnop", "sam").unwrap_err().contains("sequence"));
        assert!(strength("12345678901234", "x").is_err());
        assert!(strength("plum otter lantern quietly", "sam").is_ok());
        assert!(strength("Tr1cky-but-L0ng-enough!", "sam").is_ok());
    }
}
