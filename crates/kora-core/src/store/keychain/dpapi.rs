//! Windows DPAPI (`CryptProtectData` / `CryptUnprotectData`): a value sealed with the signed-in user's own key.
//!
//! Chosen over Credential Manager (see the module docs of [`super`]). `CRYPTPROTECT_UI_FORBIDDEN` so a background panel never
//! shows a prompt; the optional entropy is a fixed application string (it keeps unrelated programs that merely call
//! `CryptUnprotectData` with no entropy from opening the blob by accident; it is not a secret). The sealed bytes are stored as
//! hex text in the private table. A blob made by another Windows user or on another computer fails to open: the key then
//! shows the "enter it again" problem.
//!
//! UNVERIFIED on Windows from the Linux build machine (type-checked only): the FFI is ~40 lines; the decisions around it are
//! tested with [`super::fake::FakeSealer`].
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

use super::{Place, Sealer};

pub struct Dpapi;

const ENTROPY: &[u8] = b"Kora licence key v1";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

/// Copy a DPAPI-allocated output blob into a Vec and free it.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    LocalFree(out.pbData as *mut core::ffi::c_void);
    v
}

impl Sealer for Dpapi {
    fn place(&self) -> Place {
        Place::Dpapi
    }
    fn seal(&self, secret: &str) -> Result<String, String> {
        let input = blob(secret.as_bytes());
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        // SAFETY: the blobs point at live slices for the duration of the call; the output is allocated by the system and freed in `take`.
        let ok = unsafe { CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 {
            return Err(format!("Windows could not protect the value (error {})", std::io::Error::last_os_error()));
        }
        Ok(hex::encode(unsafe { take(out) }))
    }
    fn open(&self, sealed: &str) -> Result<String, String> {
        let bytes = hex::decode(sealed).map_err(|_| "the saved value is damaged".to_string())?;
        let input = blob(&bytes);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        // SAFETY: as in `seal`.
        let ok = unsafe { CryptUnprotectData(&input, null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 {
            return Err("Windows could not open the value for this account".into());
        }
        String::from_utf8(unsafe { take(out) }).map_err(|_| "the opened value is not text".to_string())
    }
}
