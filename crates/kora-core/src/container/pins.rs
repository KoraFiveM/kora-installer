//! The exact container images container mode runs, pinned by digest.
//!
//! Kora publishes no image of its own. A server runs on a small, well-known base image and the FXServer
//! build is downloaded by Kora into the server folder as always (same source, same verification) and
//! bind-mounted. Nothing is looked up at run time: the image reference written here is
//! `<name>@sha256:<digest>`, so a moved or tampered tag can never be run. The tag is kept next to the digest
//! only so a person (and the update script) can see which release it was.
//!
//! To move to newer images: run `scripts/update-container-pins.sh <alpine-tag> <mariadb-tag>`, which prints the
//! new entries from the registry; paste them here and run `cargo test -p kora-core container::pins`. A pin
//! change is a code change: it ships with a Kora release like the tool pins in `tools/pins.rs`.
//!
//! What was verified, and what is assumed (see also `container/mod.rs`):
//! * Verified from FXServer's own layout (artifact 38435, `run.sh`): the build is `run.sh` plus an `alpine/`
//!   root with its own musl loader (`alpine/opt/cfx-server/ld-musl-x86_64.so.1`) and libraries. `run.sh` is a
//!   `#!/bin/bash` script that only locates itself and `exec`s that loader with `--library-path` and
//!   `FXServer`. The container therefore starts the loader directly (no shell, no bash): the base image only
//!   has to supply an empty root, `/etc/ssl` certificates and `/tmp`. Alpine does, in about 8 MB.
//! * Assumed, and checked by the CI smoke test (it cannot run here): txAdmin's child process starts the same
//!   way inside a container, and FXServer writes only to the server folder and `/tmp`.

/// One pinned image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Image {
    /// Fully qualified repository (Podman refuses short names without a registry list).
    pub repository: &'static str,
    /// The release this digest is, for people only.
    pub tag: &'static str,
    /// `sha256:` + 64 lowercase hex digits of the multi-architecture index.
    pub digest: &'static str,
}

impl Image {
    /// What is passed to `pull` and `run`: `repository@digest`.
    pub fn reference(&self) -> String {
        format!("{}@{}", self.repository, self.digest)
    }
}

/// The base of a game server container (Alpine, nothing installed on top).
pub const BASE: Image = Image { repository: "docker.io/library/alpine", tag: "3.23", digest: "sha256:469c9ea410b9215eaee7976ba6c0639039b444be8e5d638f7abbada189971eb2" };

/// The database container (the official MariaDB image, current long-term release).
pub const MARIADB: Image = Image { repository: "docker.io/library/mariadb", tag: "11.8", digest: "sha256:d36f18d65a0459510b8b0f643b226a6d299e3206486e2b64ff20754a6d291707" };

pub const ALL: &[Image] = &[BASE, MARIADB];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_full_digest_of_a_docker_hub_library_image() {
        for i in ALL {
            let hex = i.digest.strip_prefix("sha256:").expect("sha256 digest");
            assert_eq!(hex.len(), 64, "{}", i.repository);
            assert!(hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", i.repository);
            assert!(i.repository.starts_with("docker.io/library/"), "fully qualified: {}", i.repository);
            assert!(!i.tag.is_empty());
            assert!(i.reference().contains("@sha256:"), "never a bare tag");
            assert!(!i.reference().contains(':') || i.reference().matches(':').count() == 1, "no tag in the reference: {}", i.reference());
        }
    }
}
