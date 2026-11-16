#!/bin/sh
# Kora installer bootstrap for Linux: downloads the `kora` program from the latest release,
# checks its SHA-256 and puts it on your PATH. It does not need root and never replaces a file
# it cannot identify as an earlier kora.
#
#   curl -fsSL <release-base>/install.sh | sh
#
# Settings (environment variables, all optional):
#   KORA_VERSION       release to install, e.g. v0.2.0. Default: the newest published release (pre-releases included).
#   KORA_RELEASE_BASE  where the release files live. Must be https:// (http:// only for localhost).
#   KORA_INSTALL_DIR   target folder. Default: /usr/local/bin as root, else ~/.local/bin.
#   KORA_FORCE=1       replace an existing file at the target even if it is not a kora program.

set -eu

REPO="KoraFiveM/kora-installer"
DEFAULT_BASE="https://github.com/$REPO/releases/latest/download"
BASE="${KORA_RELEASE_BASE:-$DEFAULT_BASE}"
BASE="${BASE%/}"

say() { printf '%s\n' "$*"; }
die() { printf 'kora install: %s\n' "$*" >&2; exit 1; }

# --- the download address must be safe ---------------------------------------------------
check_url() {
  case "$1" in
    https://*) ;;
    *)
      # Plain http only for the machine itself: exactly localhost or 127.0.0.1, optional numeric port, then a path.
      if ! printf '%s\n' "$1" | grep -Eq '^http://(localhost|127\.0\.0\.1)(:[0-9]+)?(/|$)'; then
        die "refusing to download from '$1': the release address must start with https:// (plain http:// is only allowed for localhost)."
      fi
      ;;
  esac
}
check_url "$BASE"
API_BASE="${KORA_API_BASE:-https://api.github.com}"; check_url "$API_BASE"
DL_BASE="${KORA_DOWNLOAD_BASE:-https://github.com}"; check_url "$DL_BASE"

# --- platform ------------------------------------------------------------------------------
[ "$(uname -s)" = "Linux" ] || die "this script is for Linux. On Windows use install.ps1."
case "$(uname -m)" in
  x86_64 | amd64) ASSET="kora-linux-x86_64" ;;
  *) die "there is no kora build for the '$(uname -m)' processor yet (only x86_64)." ;;
esac

# --- tools ---------------------------------------------------------------------------------
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --proto-redir '=https,http' --retry 2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  die "needs curl or wget to download the program."
fi
if command -v sha256sum >/dev/null 2>&1; then
  hash_of() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
  hash_of() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
  die "needs sha256sum (or shasum) to verify the download."
fi

# GitHub's "latest" never points at a pre-release, so by default ask for the newest published release
# (pre-releases included) from /releases?per_page=10; KORA_VERSION=vX.Y.Z picks one explicitly.
# If the lookup fails we fall back to "latest". Skipped entirely when KORA_RELEASE_BASE is set.
if [ -z "${KORA_RELEASE_BASE:-}" ]; then
  TAG=""
  if [ -n "${KORA_VERSION:-}" ]; then
    TAG="$KORA_VERSION"
    case "$TAG" in v*) ;; *) TAG="v$TAG" ;; esac
    case "$TAG" in *[!A-Za-z0-9._-]*) die "KORA_VERSION '$KORA_VERSION' is not a valid release name." ;; esac
  else
    LIST="$(mktemp)"
    if fetch "$API_BASE/repos/$REPO/releases?per_page=10" "$LIST" 2>/dev/null; then
      # First entry that is not a draft (the public API never lists drafts; the check is belt and braces).
      TAG="$(awk '/^    "tag_name"/ { t=$0; sub(/.*"tag_name"[^"]*"/, "", t); sub(/".*/, "", t) }
                  /^    "draft": *false/ { if (t != "") { print t; exit } }' "$LIST")"
      case "$TAG" in *[!A-Za-z0-9._-]*) TAG="" ;; esac
    fi
    rm -f "$LIST"
  fi
  [ -n "$TAG" ] && BASE="$DL_BASE/$REPO/releases/download/$TAG"
fi

# --- where to install ----------------------------------------------------------------------
if [ -n "${KORA_INSTALL_DIR:-}" ]; then
  DEST_DIR="$KORA_INSTALL_DIR"
elif [ "$(id -u)" = "0" ] || [ -w /usr/local/bin ]; then
  DEST_DIR="/usr/local/bin"
else
  DEST_DIR="${HOME:?HOME is not set}/.local/bin"
fi
DEST="$DEST_DIR/kora"

# --- download and verify -------------------------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "Downloading $ASSET from $BASE ..."
fetch "$BASE/$ASSET" "$TMP/kora" || die "could not download $BASE/$ASSET"
fetch "$BASE/$ASSET.sha256" "$TMP/kora.sha256" || die "could not download the checksum $BASE/$ASSET.sha256"

EXPECTED="$(cut -d ' ' -f 1 "$TMP/kora.sha256" | head -n 1 | tr 'A-F' 'a-f')"
case "$EXPECTED" in
  *[!0-9a-f]* | "") die "the checksum file is not a SHA-256 value." ;;
esac
[ "${#EXPECTED}" -eq 64 ] || die "the checksum file is not a SHA-256 value."
ACTUAL="$(hash_of "$TMP/kora")"
[ "$ACTUAL" = "$EXPECTED" ] || die "the download is corrupt or has been tampered with (SHA-256 mismatch). Nothing was installed."
say "Checksum OK ($ACTUAL)."

# --- never overwrite blindly ---------------------------------------------------------------
BACKUP=""
if [ -e "$DEST" ] || [ -L "$DEST" ]; then
  if [ -f "$DEST" ] && [ "$(hash_of "$DEST")" = "$ACTUAL" ]; then
    say "kora is already up to date at $DEST."
    NEXT_ONLY=1
  elif [ -f "$DEST" ] && grep -aq 'Kora installer and server manager' "$DEST"; then
    BACKUP="$DEST.previous"
  elif [ "${KORA_FORCE:-}" = "1" ]; then
    BACKUP="$DEST.previous"
  else
    die "$DEST already exists and is not a kora program, so it was left alone. Choose another folder (KORA_INSTALL_DIR=...) or set KORA_FORCE=1 to replace it."
  fi
fi

if [ "${NEXT_ONLY:-}" != "1" ]; then
  mkdir -p "$DEST_DIR" || die "cannot create $DEST_DIR"
  [ -w "$DEST_DIR" ] || die "cannot write to $DEST_DIR. Run with sudo, or set KORA_INSTALL_DIR to a folder you own."
  cp "$TMP/kora" "$DEST_DIR/.kora.new.$$" || die "cannot write to $DEST_DIR"
  chmod 0755 "$DEST_DIR/.kora.new.$$"
  if [ -n "$BACKUP" ]; then
    mv -f "$DEST" "$BACKUP"
    say "The earlier version was kept as $BACKUP."
  fi
  mv -f "$DEST_DIR/.kora.new.$$" "$DEST"
  say "Installed kora to $DEST"
fi

# --- next step -----------------------------------------------------------------------------
case ":$PATH:" in
  *":$DEST_DIR:"*) ;;
  *) say "Note: $DEST_DIR is not on your PATH yet. Add it:  export PATH=\"$DEST_DIR:\$PATH\"" ;;
esac
say ""
say "Next step: run   kora panel   (opens Kora Panel in your browser)"
say "Prefer the terminal? Run   kora install   instead. (On a server with no screen: kora panel --no-open)"
