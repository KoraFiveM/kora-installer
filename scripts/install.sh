#!/bin/sh
# Kora installer bootstrap for Linux: downloads the `kora` program from the newest stable release,
# checks its SHA-256 and puts it on your PATH. It does not need root and never replaces a file
# it cannot identify as an earlier kora.
#
# Then it starts Kora Panel as a background service (a systemd user service that also starts after
# a restart) with `kora panel --setup`, opens it in your browser when there is a screen and prints
# the sign-in link (plus the SSH tunnel command on a server with no screen). If that step fails the
# install still counts as done and the manual next step is printed.
#
#   curl -fsSL <release-base>/install.sh | sh
#
# Settings (environment variables, all optional):
#   KORA_VERSION       release to install, e.g. v0.2.0 (any release, pre-releases too).
#                      Default: the newest stable release (not a pre-release, not a draft).
#   KORA_PRERELEASE=1  without KORA_VERSION: take the newest release even if it is a pre-release.
#   KORA_RELEASE_BASE  where the release files live. Must be https:// (http:// only for localhost).
#   KORA_INSTALL_DIR   target folder. Default: /usr/local/bin as root, else ~/.local/bin.
#   KORA_FORCE=1       replace an existing file at the target even if it is not a kora program.
#   KORA_NO_LAUNCH=1   only install the program: do not start Kora Panel or open a browser.
# For tests only (same https:// rule): KORA_API_BASE (default https://api.github.com) and
# KORA_DOWNLOAD_BASE (default https://github.com) point the release lookup at a fake server.
#
# Downloads use https only, redirects included (a redirect to http:// is refused), and the whole
# script is one function called on the last line, so a download cut short runs nothing.

set -eu

# Everything runs from main, called on the very last line: if `curl | sh` gets a truncated
# script, sh reaches the end of the input before main exists and nothing runs.
main() {
  REPO="KoraFiveM/kora-installer"
  DEFAULT_BASE="https://github.com/$REPO/releases/latest/download"
  BASE="${KORA_RELEASE_BASE:-$DEFAULT_BASE}"
  BASE="${BASE%/}"

  say() { printf '%s\n' "$*"; }
  die() { printf 'kora install: %s\n' "$*" >&2; exit 1; }

  # wget, https only: follow at most 5 redirects by hand, each of which must stay https.
  wget_https() {
    _url="$1"; _hops=0
    while [ "$_hops" -le 5 ]; do
      _hdr="$(wget -q -S --max-redirect=0 -O "$2" "$_url" 2>&1)" && return 0
      _loc="$(printf '%s\n' "$_hdr" | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r')"
      case "$_loc" in
        https://*) _url="$_loc" ;;
        /*) _url="$(printf '%s\n' "$_url" | sed 's#^\(https://[^/]*\).*#\1#')$_loc" ;;
        "") return 1 ;;
        *) printf 'kora install: refusing a redirect to %s (not https).\n' "$_loc" >&2; return 1 ;;
      esac
      _hops=$((_hops + 1))
    done
    return 1
  }

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
  # https stays https: curl may only use and redirect to https (plain http only for the localhost test
  # address, which check_url allows). GNU wget's --https-only does not cover redirects, so wget follows
  # them one hop at a time and refuses any that is not https.
  if command -v curl >/dev/null 2>&1; then
    fetch() {
      case "$1" in
        https://*) curl -fsSL --proto '=https' --proto-redir '=https' --retry 2 -o "$2" "$1" ;;
        *) curl -fsSL --proto '=http' --proto-redir '=http' --retry 2 -o "$2" "$1" ;;
      esac
    }
  elif command -v wget >/dev/null 2>&1; then
    fetch() {
      case "$1" in
        https://*) wget_https "$1" "$2" ;;
        *) wget -q --max-redirect=0 -O "$2" "$1" ;;
      esac
    }
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

  # Which release: KORA_VERSION=vX.Y.Z picks one explicitly (pre-releases too). Otherwise the newest
  # stable release (not a draft, not a pre-release) from /releases?per_page=30, or with
  # KORA_PRERELEASE=1 the newest release of any kind. If the lookup fails we fall back to GitHub's
  # "latest", which is also the newest stable release. Skipped entirely when KORA_RELEASE_BASE is set.
  if [ -z "${KORA_RELEASE_BASE:-}" ]; then
    TAG=""
    if [ -n "${KORA_VERSION:-}" ]; then
      TAG="$KORA_VERSION"
      case "$TAG" in v*) ;; *) TAG="v$TAG" ;; esac
      case "$TAG" in *[!A-Za-z0-9._-]*) die "KORA_VERSION '$KORA_VERSION' is not a valid release name." ;; esac
    else
      PRE=0; [ "${KORA_PRERELEASE:-}" = "1" ] && PRE=1
      LIST="$(mktemp)"
      if fetch "$API_BASE/repos/$REPO/releases?per_page=30" "$LIST" 2>/dev/null; then
        # Only top-level fields of each release count (4 spaces in GitHub's JSON), in whatever order.
        # Drafts are never listed by the public API; the check is belt and braces.
        TAG="$(awk -v pre="$PRE" '
          /^    "tag_name"/ { t=$0; sub(/.*"tag_name"[^"]*"/, "", t); sub(/".*/, "", t); d=""; p="" }
          /^    "draft":/ { d = ($0 ~ /false/) ? "no" : "yes" }
          /^    "prerelease":/ { p = ($0 ~ /false/) ? "no" : "yes" }
          t != "" && d != "" && p != "" { if (d == "no" && (p == "no" || pre == 1)) { print t; exit } t="" }' "$LIST")"
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

  # --- start Kora Panel ----------------------------------------------------------------------
  case ":$PATH:" in
    *":$DEST_DIR:"*) ;;
    *) say "Note: $DEST_DIR is not on your PATH yet. Add it:  export PATH=\"$DEST_DIR:\$PATH\"" ;;
  esac
  say ""
  if [ "${KORA_NO_LAUNCH:-}" = "1" ]; then
    say "Next step: run   kora panel   (opens Kora Panel in your browser)"
    say "Prefer the terminal? Run   kora install   instead. (On a server with no screen: kora panel --no-open)"
    exit 0
  fi
  say "Starting Kora Panel ..."
  # Absolute path: the folder may not be on PATH yet. stdin is the piped script under `curl | sh`, so
  # the program must not read it. A failure here does not undo the install.
  if "$DEST" panel --setup </dev/null; then
    say ""
    say "Prefer the terminal? Run   kora install   instead."
  else
    say ""
    say "Kora Panel did not start by itself. kora is installed; start the panel with:"
    say "    $DEST panel"
    say "Prefer the terminal? Run   kora install   instead."
  fi
  exit 0
}

main "$@"
