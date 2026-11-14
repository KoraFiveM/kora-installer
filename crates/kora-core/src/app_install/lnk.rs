//! Windows `.lnk` shell links: the one byte we patch ("Run as administrator").
//!
//! A shell link starts with a 76-byte header: `HeaderSize` (u32 = 0x4C), the link CLSID, then
//! `LinkFlags` (u32 at offset 0x14). `RunAsUser` is flag bit 13 = 0x2000, i.e. bit 0x20 of the byte at
//! offset 0x15. With it set, Explorer elevates the target on double-click (UAC prompt).

const HEADER_SIZE: usize = 0x4C;
const FLAGS_BYTE: usize = 0x15;
const RUN_AS_USER: u8 = 0x20;
const CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

fn is_link(b: &[u8]) -> bool {
    b.len() >= HEADER_SIZE && b[..4] == [0x4C, 0, 0, 0] && b[4..20] == CLSID
}

/// Set or clear the "Run as administrator" flag. `Err` if the bytes are not a shell link.
pub fn set_run_as_admin(bytes: &[u8], on: bool) -> Result<Vec<u8>, String> {
    if !is_link(bytes) {
        return Err("not a Windows shortcut (.lnk) file".into());
    }
    let mut out = bytes.to_vec();
    if on {
        out[FLAGS_BYTE] |= RUN_AS_USER;
    } else {
        out[FLAGS_BYTE] &= !RUN_AS_USER;
    }
    Ok(out)
}

/// Is the "Run as administrator" flag set?
pub fn run_as_admin(bytes: &[u8]) -> Option<bool> {
    is_link(bytes).then(|| bytes[FLAGS_BYTE] & RUN_AS_USER != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 76-byte shell-link header (magic, CLSID, LinkFlags with only low-byte flags set, zeroed rest)
    /// followed by a few body bytes.
    fn golden() -> Vec<u8> {
        let mut v = vec![0x4C, 0, 0, 0];
        v.extend_from_slice(&CLSID);
        v.extend_from_slice(&[0xFF, 0x00, 0x00, 0x00]); // LinkFlags = 0x000000FF: byte 0x15 is 0
        v.extend_from_slice(&[0u8; HEADER_SIZE - 24]);
        v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        v
    }

    #[test]
    fn golden_flag_patch_changes_exactly_one_byte() {
        let before = golden();
        assert_eq!(run_as_admin(&before), Some(false));
        let after = set_run_as_admin(&before, true).unwrap();
        assert_eq!(run_as_admin(&after), Some(true));
        let diff: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
        assert_eq!(diff, vec![0x15], "only byte 0x15 changes");
        assert_eq!(after[0x15], 0x20, "bit 0x20 of byte 0x15 (LinkFlags bit 13)");
        assert_eq!(&after[..0x15], &before[..0x15]);
        assert_eq!(&after[0x16..], &before[0x16..]);
    }

    #[test]
    fn patch_is_idempotent_reversible_and_keeps_other_bits() {
        let mut b = golden();
        b[0x15] = 0x01; // some other flag bit in the same byte
        let on = set_run_as_admin(&b, true).unwrap();
        assert_eq!(on[0x15], 0x21);
        assert_eq!(set_run_as_admin(&on, true).unwrap(), on);
        assert_eq!(set_run_as_admin(&on, false).unwrap(), b);
    }

    #[test]
    fn non_links_are_rejected() {
        assert!(set_run_as_admin(b"MZ not a link", true).is_err());
        assert!(set_run_as_admin(&[], true).is_err());
        let mut bad = golden();
        bad[4] = 0;
        assert_eq!(run_as_admin(&bad), None);
    }
}
