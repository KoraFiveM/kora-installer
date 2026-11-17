//! The exact copies of `gh` and Git (MinGit) Kora downloads, with their SHA-256 checksums.
//!
//! Nothing here is looked up at run time: the version, address and checksum are written down in the
//! repository, so a changed or tampered release can never be installed. The checksums were taken from
//! the release's own published digests (for `gh` also its `checksums.txt`) and checked against a
//! download. Every address must be a `https://github.com/` release asset (a unit test enforces it).
//!
//! To move to a newer release: run `scripts/update-tool-pins.sh <gh-version> <mingit-tag>`, which prints
//! the new entries from the official checksum lists; paste them here, and run `cargo test -p kora-core tools`.

use crate::problem::Tool;

/// How the download is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    TarGz,
}

/// One pinned download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub tool: Tool,
    pub version: &'static str,
    /// `linux` or `windows` (`std::env::consts::OS`).
    pub os: &'static str,
    /// `x86_64` (`std::env::consts::ARCH`).
    pub arch: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the downloaded file.
    pub sha256: &'static str,
    pub format: Format,
    /// The archive has one top folder around everything (`gh_2.104.0_linux_amd64/`): drop it.
    pub strip_top: bool,
    /// The program, relative to the tool's folder once unpacked.
    pub exe: &'static str,
    /// Approximate download size, for the progress text only.
    pub size: u64,
}

pub const PINS: &[Pin] = &[
    Pin {
        tool: Tool::Gh,
        version: "2.104.0",
        os: "linux",
        arch: "x86_64",
        file: "gh_2.104.0_linux_amd64.tar.gz",
        url: "https://github.com/cli/cli/releases/download/v2.104.0/gh_2.104.0_linux_amd64.tar.gz",
        sha256: "98cb101ad87dbbd3f6b2f2bfcbd43197ccd07643140121d8a9cfaa91eadbc24e",
        format: Format::TarGz,
        strip_top: true,
        exe: "bin/gh",
        size: 15_007_172,
    },
    Pin {
        tool: Tool::Gh,
        version: "2.104.0",
        os: "windows",
        arch: "x86_64",
        file: "gh_2.104.0_windows_amd64.zip",
        url: "https://github.com/cli/cli/releases/download/v2.104.0/gh_2.104.0_windows_amd64.zip",
        sha256: "08c06d10482f12c478018cd259f64acbd5f625712cadf331aad19ba973c879e5",
        format: Format::Zip,
        strip_top: false,
        exe: "bin/gh.exe",
        size: 15_169_772,
    },
    Pin {
        tool: Tool::Git,
        version: "2.56.1",
        os: "windows",
        arch: "x86_64",
        file: "MinGit-2.56.1-64-bit.zip",
        url: "https://github.com/git-for-windows/git/releases/download/v2.56.1.windows.1/MinGit-2.56.1-64-bit.zip",
        sha256: "4a92850b6e917da4fd010669b614b55dccf5b0ec333a534cc4be4700dfefdafd",
        format: Format::Zip,
        strip_top: false,
        exe: "cmd/git.exe",
        size: 39_849_768,
    },
];

/// The pin for `tool` on this kind of computer (`None`: Kora has no private copy for it, for example
/// Git on Linux, which comes from the system package manager).
pub fn pin_for(tool: Tool, os: &str, arch: &str) -> Option<&'static Pin> {
    PINS.iter().find(|p| p.tool == tool && p.os == os && p.arch == arch)
}

/// The pin for this computer.
pub fn pin_here(tool: Tool) -> Option<&'static Pin> {
    pin_for(tool, std::env::consts::OS, std::env::consts::ARCH)
}
