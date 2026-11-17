//! One-time codes (TOTP, RFC 6238) for where a passkey cannot be used: a bare-IP address on the home network
//! (WebAuthn needs a domain name), or an old browser.
//!
//! * the secret is 20 random bytes, shown once as base32 and as a QR code for an authenticator app;
//! * a code is 6 digits from HMAC-SHA1 over the 30-second step (RFC 4226 truncation), the algorithm every
//!   authenticator app supports; HMAC and SHA-1 are RustCrypto's (SHA-1 only as HMAC input, where it is sound);
//! * a code is accepted for the current step and one step either side ([`WINDOW`]), compared in constant time;
//! * a step that was accepted once is never accepted again ([`accept`] returns it; the caller stores it in the
//!   same transaction, refusing anything not newer than the last one), so a code seen over a shoulder or
//!   in a log cannot be replayed.
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

use crate::security::ct_eq;

pub const STEP_SECS: u64 = 30;
pub const DIGITS: u32 = 6;
/// Steps either side of the current one that are still accepted (clock drift).
pub const WINDOW: u64 = 1;

pub fn new_secret() -> Result<[u8; 20], String> {
    let mut s = [0u8; 20];
    getrandom::fill(&mut s).map_err(|_| "the system random number generator is unavailable".to_string())?;
    Ok(s)
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32 without padding.
pub fn base32(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The 6-digit code for `secret` at `step` (a 30-second counter).
pub fn code_at(secret: &[u8], step: u64) -> String {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let d = mac.finalize().into_bytes();
    let off = (d[d.len() - 1] & 0x0f) as usize;
    let n = u32::from_be_bytes([d[off] & 0x7f, d[off + 1], d[off + 2], d[off + 3]]);
    format!("{:0width$}", n % 10u32.pow(DIGITS), width = DIGITS as usize)
}

pub fn step_of(unix: u64) -> u64 {
    unix / STEP_SECS
}

/// Which step, if any, `given` is the code for. It must be newer than `last_step`. Every candidate is compared
/// (no early exit), so timing does not say which step matched.
pub fn accept(secret: &[u8], last_step: u64, given: &str, now_unix: u64) -> Option<u64> {
    let given: String = given.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if given.len() != DIGITS as usize || !given.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = step_of(now_unix);
    let mut found = None;
    for step in now.saturating_sub(WINDOW)..=now + WINDOW {
        if ct_eq(&code_at(secret, step), &given) && step > last_step {
            found = Some(step);
        }
    }
    found
}

/// The `otpauth://` address an authenticator app reads from the QR code.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}", enc(issuer), enc(account), base32(secret), enc(issuer))
}

/// The QR code of `data` as an SVG image (dark modules on white, with the quiet zone).
pub fn qr_svg(data: &str) -> Result<String, String> {
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::M).map_err(|e| format!("could not make the QR code ({e})"))?;
    Ok(code.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).dark_color(qrcode::render::svg::Color("#000000")).light_color(qrcode::render::svg::Color("#ffffff")).build())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc_6238_vectors() {
        // The RFC's SHA-1 vectors are 8 digits; the last six are ours.
        for (t, eight) in [(59u64, "94287082"), (1_111_111_109, "07081804"), (1_111_111_111, "14050471"), (1_234_567_890, "89005924"), (2_000_000_000, "69279037"), (20_000_000_000, "65353130")] {
            assert_eq!(code_at(RFC_SECRET, t / 30), eight[2..], "T={t}");
        }
    }

    #[test]
    fn base32_matches_the_rfc() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(RFC_SECRET), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn a_code_works_for_its_step_and_one_either_side_but_not_further() {
        let now = 1_700_000_000u64;
        let s = step_of(now);
        for (step, ok) in [(s - 2, false), (s - 1, true), (s, true), (s + 1, true), (s + 2, false)] {
            assert_eq!(accept(RFC_SECRET, 0, &code_at(RFC_SECRET, step), now).is_some(), ok, "step offset {}", step as i64 - s as i64);
        }
    }

    #[test]
    fn a_code_cannot_be_used_twice_or_after_a_later_one() {
        let now = 1_700_000_000u64;
        let s = step_of(now);
        let c = code_at(RFC_SECRET, s);
        let used = accept(RFC_SECRET, 0, &c, now).unwrap();
        assert_eq!(used, s);
        assert!(accept(RFC_SECRET, used, &c, now).is_none(), "replay of the same code");
        assert!(accept(RFC_SECRET, used, &code_at(RFC_SECRET, s - 1), now).is_none(), "an older code after a newer one");
        assert!(accept(RFC_SECRET, used, &code_at(RFC_SECRET, s + 1), now).is_some(), "the next one is fine");
    }

    #[test]
    fn malformed_codes_are_refused() {
        let now = 1_700_000_000u64;
        let c = code_at(RFC_SECRET, step_of(now));
        assert!(accept(RFC_SECRET, 0, &format!("{} {}", &c[..3], &c[3..]), now).is_some(), "spaces are fine");
        for bad in ["", "12345", "1234567", "abcdef", "12 34 5x", "０１２３４５"] {
            assert!(accept(RFC_SECRET, 0, bad, now).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_app_address_and_qr_are_made() {
        let u = otpauth_uri("Kora Panel", "sam@box", RFC_SECRET);
        assert_eq!(u, "otpauth://totp/Kora%20Panel:sam%40box?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Kora%20Panel&algorithm=SHA1&digits=6&period=30");
        let svg = qr_svg(&u).unwrap();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"), "{}", &svg[..60]);
        assert!(new_secret().unwrap() != new_secret().unwrap());
    }
}
