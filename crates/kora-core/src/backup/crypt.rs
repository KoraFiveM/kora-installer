//! Password protection for archives: the `age` file format (passphrase mode: scrypt + ChaCha20-Poly1305,
//! streamed in 64 KiB chunks), through the `age` crate. Nothing here is home-made cryptography: this file
//! only connects `age` to readers and writers and counts checksums.
//!
//! Threat model: the archive holds secrets (licence key, database login, txAdmin admin). Without a
//! passphrase it is as private as its folder (owner-only permissions). With one, someone who gets the file
//! (a cloud drive, a stolen disk, an email) still cannot read it, and `age` authenticates every chunk, so a
//! damaged or tampered file is refused rather than half-restored. A lost passphrase cannot be recovered by
//! anyone: Kora does not keep it. Anyone with the file can try passphrases offline; scrypt makes each try
//! slow, but a short passphrase is still weak. The passphrase never goes in a command line, a log or a job event.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use age::secrecy::SecretString;
use sha2::{Digest, Sha256};

/// scrypt cost used when encrypting (log2 of N); 0 = the `age` default (about a second of work). Tests lower it.
static WORK_FACTOR: AtomicU8 = AtomicU8::new(0);

/// For tests only: make encryption fast. Never called by the program.
#[doc(hidden)]
pub fn set_work_factor_for_tests(log_n: u8) {
    WORK_FACTOR.store(log_n, Ordering::Relaxed);
}

/// The first bytes of every age file. Used to tell an encrypted archive from a plain one.
pub const AGE_MAGIC: &[u8] = b"age-encryption.org/";

/// A passphrase that does not print itself.
#[derive(Clone)]
pub struct Passphrase(String);

impl Passphrase {
    pub fn new(s: impl Into<String>) -> Self {
        Passphrase(s.into())
    }
    pub fn reveal(&self) -> &str {
        &self.0
    }
    fn secret(&self) -> SecretString {
        SecretString::from(self.0.clone())
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

/// Why opening an encrypted archive failed.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    NeedsPassphrase,
    WrongPassphrase,
    NotAnArchive,
    Io(String),
}

/// Checks a passphrase is worth accepting. Plain words, shown to the person.
pub fn check_passphrase(p: &str) -> Result<(), String> {
    if p.chars().count() < 8 {
        return Err("Use at least 8 characters for the password. A longer sentence is easier to remember and much harder to guess.".into());
    }
    if p.trim() != p {
        return Err("The password starts or ends with a space. Remove it, so you do not have to remember it.".into());
    }
    Ok(())
}

/// A writer that counts and hashes what passes through.
pub struct Hashing<W> {
    inner: W,
    hash: Sha256,
    pub bytes: u64,
}

impl<W: Write> Hashing<W> {
    pub fn new(inner: W) -> Self {
        Hashing { inner, hash: Sha256::new(), bytes: 0 }
    }
    pub fn finish(self) -> (W, String, u64) {
        (self.inner, hex::encode(self.hash.finalize()), self.bytes)
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A reader that hashes what it reads.
pub struct HashingReader<R> {
    inner: R,
    hash: Sha256,
    pub bytes: u64,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R) -> Self {
        HashingReader { inner, hash: Sha256::new(), bytes: 0 }
    }
    pub fn finish(self) -> (String, u64) {
        (hex::encode(self.hash.finalize()), self.bytes)
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hash.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}

/// The write side: either straight to `out`, or through age.
pub enum Sealer<W: Write> {
    Plain(W),
    Age(age::stream::StreamWriter<W>),
}

impl<W: Write> Sealer<W> {
    pub fn new(out: W, passphrase: Option<&Passphrase>) -> io::Result<Self> {
        match passphrase {
            None => Ok(Sealer::Plain(out)),
            Some(p) => {
                let mut rec = age::scrypt::Recipient::new(p.secret());
                let wf = WORK_FACTOR.load(Ordering::Relaxed);
                if wf > 0 {
                    rec.set_work_factor(wf);
                }
                let enc = age::Encryptor::with_recipients(std::iter::once(&rec as &dyn age::Recipient)).map_err(|e| io::Error::other(e.to_string()))?;
                Ok(Sealer::Age(enc.wrap_output(out)?))
            }
        }
    }

    /// Write the closing chunk (age needs it) and give the inner writer back.
    pub fn finish(self) -> io::Result<W> {
        match self {
            Sealer::Plain(w) => Ok(w),
            Sealer::Age(w) => w.finish(),
        }
    }
}

impl<W: Write> Write for Sealer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Sealer::Plain(w) => w.write(buf),
            Sealer::Age(w) => w.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sealer::Plain(w) => w.flush(),
            Sealer::Age(w) => w.flush(),
        }
    }
}

/// Does this file start like an age file? (`first` = its first bytes.)
pub fn looks_encrypted(first: &[u8]) -> bool {
    first.starts_with(AGE_MAGIC)
}

/// Open `input` for reading: through age when it is an age file (a passphrase is then needed), as is otherwise.
pub fn open_reader<R: Read + 'static>(mut input: R, passphrase: Option<&Passphrase>) -> Result<Box<dyn Read>, OpenError> {
    // Peek at the start without losing it.
    let mut head = vec![0u8; AGE_MAGIC.len()];
    let mut got = 0;
    while got < head.len() {
        let n = input.read(&mut head[got..]).map_err(|e| OpenError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        got += n;
    }
    head.truncate(got);
    let whole = std::io::Cursor::new(head.clone()).chain(input);
    if !looks_encrypted(&head) {
        return Ok(Box::new(whole));
    }
    let Some(p) = passphrase else { return Err(OpenError::NeedsPassphrase) };
    let dec = age::Decryptor::new(whole).map_err(|e| match e {
        age::DecryptError::Io(e) => OpenError::Io(e.to_string()),
        _ => OpenError::NotAnArchive,
    })?;
    let mut id = age::scrypt::Identity::new(p.secret());
    // A file made on a faster computer asks for more work than this one would pick; allow what a human can wait for.
    id.set_max_work_factor(24);
    match dec.decrypt(std::iter::once(&id as &dyn age::Identity)) {
        Ok(r) => Ok(Box::new(r)),
        Err(age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys | age::DecryptError::KeyDecryptionFailed | age::DecryptError::InvalidMac) => Err(OpenError::WrongPassphrase),
        Err(age::DecryptError::Io(e)) => Err(OpenError::Io(e.to_string())),
        Err(_) => Err(OpenError::NotAnArchive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() {
        set_work_factor_for_tests(8);
    }

    fn seal(data: &[u8], pass: Option<&Passphrase>) -> Vec<u8> {
        fast();
        let mut s = Sealer::new(Vec::new(), pass).unwrap();
        s.write_all(data).unwrap();
        s.finish().unwrap()
    }

    #[test]
    fn plain_passes_through() {
        let out = seal(b"hello", None);
        assert_eq!(out, b"hello");
        let mut r = open_reader(std::io::Cursor::new(out), None).unwrap();
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn encrypted_round_trip_and_errors() {
        let p = Passphrase::new("correct horse battery");
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let out = seal(&data, Some(&p));
        assert!(looks_encrypted(&out));
        assert!(!out.windows(8).any(|w| w == &data[1000..1008]), "plaintext must not appear");
        let mut back = Vec::new();
        open_reader(std::io::Cursor::new(out.clone()), Some(&p)).unwrap().read_to_end(&mut back).unwrap();
        assert_eq!(back, data);
        assert_eq!(open_reader(std::io::Cursor::new(out.clone()), None).err(), Some(OpenError::NeedsPassphrase));
        assert_eq!(open_reader(std::io::Cursor::new(out.clone()), Some(&Passphrase::new("wrong wrong wrong"))).err(), Some(OpenError::WrongPassphrase));
        // a flipped byte in the body is caught while reading
        let mut bad = out;
        let at = bad.len() - 40;
        bad[at] ^= 1;
        let mut r = open_reader(std::io::Cursor::new(bad), Some(&p)).unwrap();
        assert!(r.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    fn passphrases_are_checked_and_never_printed() {
        assert!(check_passphrase("short").is_err());
        assert!(check_passphrase(" spaces around ").is_err());
        assert!(check_passphrase("a good long sentence").is_ok());
        assert!(!format!("{:?}", Passphrase::new("hunter2hunter2")).contains("hunter"));
    }

    #[test]
    fn hashing_counts() {
        let mut h = Hashing::new(Vec::new());
        h.write_all(b"abc").unwrap();
        let (v, hash, n) = h.finish();
        assert_eq!((v.as_slice(), n), (&b"abc"[..], 3));
        assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
