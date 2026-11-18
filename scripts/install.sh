#!/bin/sh
# Kora installer bootstrap for Linux: downloads the `kora` program from the newest stable release,
# checks its SHA-256 and installs it.
#
# On a computer where you can use administrator rights it first offers the SYSTEM install (one
# administrator approval, asked for once): Kora goes into /usr/lib/kora (the .deb on Debian and Ubuntu,
# the same layout elsewhere) together with Kora's small administrator service, so installing servers,
# firewall rules and databases later needs no password and no terminal. Answer no (or have no sudo)
# and it installs just for you instead (~/.local/bin, no administrator rights; the parts of a server
# that need them are then "continue without" or a command to run). It never replaces a file it cannot
# identify as an earlier kora.
#
# It also makes a launcher in your applications menu (yours only, desktop sessions only) that opens
# Kora Panel in your browser.
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
#   KORA_SYSTEM        1: do the system install (fail if it cannot); 0: never, install just for you; unset: ask
#                      when someone can answer, and only when KORA_INSTALL_DIR is not set.
#   KORA_INSTALL_DIR   target folder (a per-user install: no system install is offered). Default:
#                      /usr/local/bin as root, else ~/.local/bin.
#   KORA_FORCE=1       replace an existing file at the target even if it is not a kora program.
#   KORA_NO_LAUNCH=1   only install the program: do not start Kora Panel or open a browser.
#   KORA_NO_SHORTCUTS=1   do not make the applications-menu launcher.
#   KORA_TAILSCALE=1   on a server with Tailscale, open Kora Panel to your own Tailscale devices without asking
#                      (KORA_TAILSCALE=0: never ask). See `kora panel --tailscale`.
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
      # (a wget that still prints "Location: <url> [following]" gets that note taken off)
      _loc="$(printf '%s\n' "$_hdr" | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r' | sed 's/ \[following\]$//')"
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
  elif command -v wget >/dev/null 2>&1 && wget --help 2>&1 | grep -q -- 'max-redirect'; then
    fetch() {
      case "$1" in
        https://*) wget_https "$1" "$2" ;;
        *) wget -q --max-redirect=0 -O "$2" "$1" ;;
      esac
    }
  elif command -v wget >/dev/null 2>&1; then
    # A wget without --max-redirect (BusyBox) follows redirects by itself and cannot be told to
    # refuse plain http, so it is only given https addresses (the SHA-256 check still applies).
    fetch() {
      case "$1" in
        https://*) wget -q -O "$2" "$1" ;;
        *) printf 'kora install: this wget cannot refuse redirects, so it is only used for https addresses (not %s). Install curl to use a local address.\n' "$1" >&2; return 1 ;;
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

  # --- the system install first: one administrator approval, then no more prompts --------------
  # Debian / Ubuntu: the .deb. Other systemd distributions: `kora service install` (the same layout).
  # Anything that goes wrong falls back to the per-user install below (unless KORA_SYSTEM=1).
  SYSTEM_MODE="${KORA_SYSTEM:-}"
  want_system() {
    [ "$SYSTEM_MODE" = "0" ] && return 1
    [ "$SYSTEM_MODE" = "1" ] && return 0
    [ -z "${KORA_INSTALL_DIR:-}" ] || return 1
    [ -d /run/systemd/system ] || return 1
    [ "$(id -u)" = "0" ] || command -v sudo >/dev/null 2>&1 || return 1
    # Ask only when somebody can answer (stdin is the script itself under curl | sh).
    ( : </dev/tty ) 2>/dev/null || return 1
    printf 'Install Kora for this whole computer (recommended)?\n  Asks for your password once; after that Kora can set up servers, firewall rules and\n  databases by itself. Answer n to install just for you, without administrator rights. [Y/n] ' >/dev/tty
    read -r _ans </dev/tty || return 1
    case "$_ans" in "" | y | Y | yes | YES | Yes) return 0 ;; *) return 1 ;; esac
  }
  system_install() {
    _sudo=""; [ "$(id -u)" = "0" ] || _sudo="sudo"
    _who="$(id -un)"; [ "$(id -u)" = "0" ] && _who="${SUDO_USER:-}"
    if command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
      fetch "$BASE/$ASSET.deb" "$TMP/kora.deb" || { say "Could not download the package $BASE/$ASSET.deb"; return 1; }
      fetch "$BASE/$ASSET.deb.sha256" "$TMP/kora.deb.sha256" || { say "Could not download the package checksum."; return 1; }
      _want="$(cut -d ' ' -f 1 "$TMP/kora.deb.sha256" | head -n 1 | tr 'A-F' 'a-f')"
      [ "$(hash_of "$TMP/kora.deb")" = "$_want" ] || { say "The package does not match its checksum (SHA-256). Nothing was installed."; return 1; }
      say "Installing the Kora package (it asks for your password once) ..."
      # dpkg's maintainer script allows the account that ran sudo (SUDO_UID) and KORA_SERVICE_USERS.
      $_sudo env KORA_SERVICE_USERS="$_who" dpkg -i "$TMP/kora.deb" || return 1
    else
      chmod 0755 "$TMP/kora"
      say "Installing Kora's administrator service (it asks for your password once) ..."
      if [ -n "$_who" ]; then $_sudo "$TMP/kora" service install --allow "$_who" || return 1; else $_sudo "$TMP/kora" service install || return 1; fi
    fi
    [ -x /usr/lib/kora/kora ]
  }
  if want_system; then
    if system_install; then
      DEST_DIR=/usr/bin; DEST=/usr/bin/kora
      # A copy from an earlier per-user install would come first on PATH and run an older Kora.
      if [ "$(id -u)" != "0" ] && [ -f "$HOME/.local/bin/kora" ] && grep -aq 'Kora installer and server manager' "$HOME/.local/bin/kora"; then
        mv -f "$HOME/.local/bin/kora" "$HOME/.local/bin/kora.previous" && say "Your earlier per-user copy was kept as ~/.local/bin/kora.previous (the system one is used now)."
      fi
      say "Kora is installed for this computer, with its administrator service."
    elif [ "$SYSTEM_MODE" = "1" ]; then
      die "the system install did not work (KORA_SYSTEM=1, so nothing else was tried). Run it again, or KORA_SYSTEM=0 to install just for you."
    else
      say "The system install was not done; installing just for you instead (no administrator rights needed)."
    fi
  fi

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
    # mktemp creates the file exclusively (mode 0600), so a planted symlink or a guessable name in a
    # shared folder cannot redirect the write. It is only made executable after the copy.
    NEW="$(mktemp "$DEST_DIR/.kora.new.XXXXXX")" || die "cannot write to $DEST_DIR"
    trap 'rm -rf "$TMP"; rm -f "$NEW"' EXIT INT TERM
    cp "$TMP/kora" "$NEW" || die "cannot write to $DEST_DIR"
    chmod 0755 "$NEW"
    if [ -n "$BACKUP" ]; then
      mv -f "$DEST" "$BACKUP"
      say "The earlier version was kept as $BACKUP."
    fi
    mv -f "$NEW" "$DEST"
    say "Installed kora to $DEST"
  fi

  # --- start Kora Panel ----------------------------------------------------------------------
  case ":$PATH:" in
    *":$DEST_DIR:"*) ;;
    *) say "Note: $DEST_DIR is not on your PATH yet. Add it:  export PATH=\"$DEST_DIR:\$PATH\"" ;;
  esac
  # The applications-menu launcher (per user; not from a root shell, which would put it in root's home).
  # A failure here does not undo the install.
  if [ "${KORA_NO_SHORTCUTS:-}" != "1" ] && { [ "$(id -u)" != "0" ] || [ "${KORA_ALLOW_ROOT_PANEL:-}" = "1" ]; }; then
    "$DEST" app install --yes </dev/null || say "The applications-menu launcher could not be made. You can still open Kora by running   kora"
  fi
  say ""
  if [ "${KORA_NO_LAUNCH:-}" = "1" ]; then
    say "Next step: run   kora   (opens Kora Panel in your browser; on a server with no screen it prints the link and the tunnel command)"
    exit 0
  fi
  # Kora Panel is for a normal account, never root (it would then sit in root's home and run with full
  # rights). Under `sudo`/as root the program is installed and the person is told how to start it.
  if [ "$(id -u)" = "0" ] && [ "${KORA_ALLOW_ROOT_PANEL:-}" != "1" ]; then
    say "Kora is installed. Kora Panel is not started from a root shell."
    say "As your normal user (not root / not sudo), run:   kora panel --setup"
    say ""
    say "If this server only has the root account, create a user for Kora first (it asks you to choose a password):"
    say "    adduser kora"
    say "    usermod -aG sudo kora"
    say "    loginctl enable-linger kora"
    say "    su - kora"
    say "Then, as that user, run:   kora panel --setup"
    exit 0
  fi
  say "Starting Kora Panel ..."
  # Absolute path: the folder may not be on PATH yet. stdin is the piped script under `curl | sh`, so
  # the program must not read it. A failure here does not undo the install.
  if "$DEST" panel --setup </dev/null; then
    say ""
    say "Open Kora again any time by running   kora"
  else
    say ""
    say "Kora Panel did not start by itself. kora is installed; start the panel with:"
    say "    $DEST panel"
  fi
  exit 0
}

main "$@"
