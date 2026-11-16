#!/bin/bash
# Tests scripts/install.sh against a local HTTP server that serves a fake release layout.
# Usage: scripts/test-install.sh   (needs python3; runs entirely in a temp dir, no root, no network)
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/install.sh"
WORK="$(mktemp -d)"
PORT="${KORA_TEST_PORT:-$((20000 + RANDOM % 20000))}"
fails=0
SERVER_PID=""

cleanup() { [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

ok() { printf 'ok   - %s\n' "$1"; }
bad() { printf 'FAIL - %s\n' "$1"; fails=$((fails + 1)); }
run_fail() { ! env HOME="$WORK/home" "$@" sh "$SCRIPT" >"$WORK/out" 2>&1; }
check() { # check "description" command...
  local what="$1"; shift
  if "$@"; then ok "$what"; else bad "$what"; fi
}

# --- fake release layout: <WORK>/rel/{kora-linux-x86_64, kora-linux-x86_64.sha256} -----------
mkdir -p "$WORK/rel" "$WORK/home"
printf '#!/bin/sh\necho "kora 9.9.9"\n# Kora installer and server manager\n' > "$WORK/rel/kora-linux-x86_64"
chmod 755 "$WORK/rel/kora-linux-x86_64"
GOOD="$(sha256sum "$WORK/rel/kora-linux-x86_64" | cut -d ' ' -f 1)"
echo "$GOOD  kora-linux-x86_64" > "$WORK/rel/kora-linux-x86_64.sha256"

(cd "$WORK/rel" && exec python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1) &
SERVER_PID=$!
for _ in $(seq 1 50); do
  python3 -c "import socket,sys; s=socket.socket(); sys.exit(s.connect_ex(('127.0.0.1',$PORT)))" 2>/dev/null && break
  sleep 0.1
done

BASE="http://127.0.0.1:$PORT"
run() { # run <extra env...> -- ; always with a private HOME, output into $WORK/out
  env HOME="$WORK/home" "$@" sh "$SCRIPT" >"$WORK/out" 2>&1
}
out_has() { grep -q -- "$1" "$WORK/out"; }

# 1. fresh install to a chosen folder
D="$WORK/t1/bin"
check "fresh install succeeds" run KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$D"
check "binary is installed and executable" test -x "$D/kora"
check "installed file is the downloaded one" test "$(sha256sum "$D/kora" | cut -d ' ' -f 1)" = "$GOOD"
check "next step is printed" out_has "Next step: run   kora panel"
check "PATH hint is printed when the folder is not on PATH" out_has "not on your PATH"
check "no temp files left behind" test "$(find "$D" -mindepth 1 ! -name kora | wc -l)" -eq 0

# 2. idempotent: same version again
check "re-run with identical file succeeds" run KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$D"
check "says it is already up to date" out_has "already up to date"
check "no backup is made for an identical file" test ! -e "$D/kora.previous"

# 3. upgrade of an older kora keeps the previous one
D="$WORK/t3/bin"; mkdir -p "$D"
printf '#!/bin/sh\necho old\n# Kora installer and server manager\n' > "$D/kora"; chmod 755 "$D/kora"
OLD="$(sha256sum "$D/kora" | cut -d ' ' -f 1)"
check "upgrade over an older kora succeeds" run KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$D"
check "new version is in place" test "$(sha256sum "$D/kora" | cut -d ' ' -f 1)" = "$GOOD"
check "old version kept as kora.previous" test "$(sha256sum "$D/kora.previous" | cut -d ' ' -f 1)" = "$OLD"

# 4. a file that is not kora is never replaced...
D="$WORK/t4/bin"; mkdir -p "$D"
echo "precious" > "$D/kora"
check "refuses to overwrite a foreign file" run_fail KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$D"
check "foreign file untouched" test "$(cat "$D/kora")" = "precious"
check "explains how to proceed" out_has "KORA_FORCE=1"
# ...unless forced
check "KORA_FORCE=1 replaces it" run KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$D" KORA_FORCE=1
check "forced replacement keeps the old file" test "$(cat "$D/kora.previous")" = "precious"

# 5. checksum mismatch installs nothing
mkdir -p "$WORK/rel2"
cp "$WORK/rel/kora-linux-x86_64" "$WORK/rel2/"
printf '%064d  kora-linux-x86_64\n' 0 > "$WORK/rel2/kora-linux-x86_64.sha256"
(cd "$WORK/rel2" && exec python3 -m http.server "$((PORT + 1))" --bind 127.0.0.1 >/dev/null 2>&1) &
BAD_PID=$!
sleep 0.5
D="$WORK/t5/bin"
check "checksum mismatch fails" run_fail KORA_RELEASE_BASE="http://127.0.0.1:$((PORT + 1))" KORA_INSTALL_DIR="$D"
check "mismatch message" out_has "SHA-256 mismatch"
check "nothing installed on mismatch" test ! -e "$D/kora"
kill "$BAD_PID" 2>/dev/null

# 6. missing release files
check "404 fails cleanly" run_fail KORA_RELEASE_BASE="$BASE/nope" KORA_INSTALL_DIR="$WORK/t6"
check "404 message" out_has "could not download"

# 7. insecure bases are refused before any network use
for bad in "http://example.com/rel" "ftp://x/y" "http://localhost.evil.com/rel" "http://127.0.0.1.evil.com/rel" \
  "http://localhost:80@evil.com/rel" "http://127.0.0.1@evil.com/rel" "http://localhost:80x/rel" "file:///tmp/x" "example.com"; do
  check "refuses base $bad" run_fail KORA_RELEASE_BASE="$bad" KORA_INSTALL_DIR="$WORK/t7"
  check "  ... with the https message" out_has "must start with https://"
done
check "nothing created for refused bases" test ! -e "$WORK/t7"

# 8. unsupported processor (fake uname first on PATH)
mkdir -p "$WORK/fakebin"
cat > "$WORK/fakebin/uname" <<'FAKE'
#!/bin/sh
if [ "$1" = "-m" ]; then echo aarch64; else /usr/bin/uname "$@"; fi
FAKE
chmod +x "$WORK/fakebin/uname"
check "aarch64 is refused" run_fail PATH="$WORK/fakebin:$PATH" KORA_RELEASE_BASE="$BASE" KORA_INSTALL_DIR="$WORK/t8"
check "aarch64 message" out_has "no kora build for the 'aarch64'"

# 9. default install folder for a non-root user without /usr/local/bin access is ~/.local/bin
if [ "$(id -u)" != "0" ] && [ ! -w /usr/local/bin ]; then
  check "default folder is ~/.local/bin" run KORA_RELEASE_BASE="$BASE"
  check "installed under HOME/.local/bin" test -x "$WORK/home/.local/bin/kora"
else
  printf 'skip - default-folder test (running as root or /usr/local/bin is writable)\n'
fi

# 10. default lookup: newest published release (pre-releases included), explicit KORA_VERSION
mkdir -p "$WORK/api/repos/KoraFiveM/kora-installer" "$WORK/api/KoraFiveM/kora-installer/releases/download/v0.2.0-rc1"
cp "$WORK/rel/"kora-linux-x86_64* "$WORK/api/KoraFiveM/kora-installer/releases/download/v0.2.0-rc1/"
cat > "$WORK/api/repos/KoraFiveM/kora-installer/releases" <<'JSON'
[
  {
    "id": 3,
    "tag_name": "v0.3.0",
    "draft": true,
    "prerelease": false,
    "assets": [
      {
        "name": "x",
        "uploader": { "tag_name": "nested", "draft": false }
      }
    ]
  },
  {
    "id": 2,
    "tag_name": "v0.2.0-rc1",
    "draft": false,
    "prerelease": true
  },
  {
    "id": 1,
    "tag_name": "v0.1.0",
    "draft": false,
    "prerelease": false
  }
]
JSON
# the fake API serves the list for the path with a query string too (python ignores the query)
(cd "$WORK/api" && exec python3 -m http.server "$((PORT + 2))" --bind 127.0.0.1 >/dev/null 2>&1) &
API_PID=$!
sleep 0.5
D="$WORK/t10/bin"
check "default lookup installs the newest non-draft pre-release" run KORA_API_BASE="http://127.0.0.1:$((PORT + 2))" KORA_DOWNLOAD_BASE="http://127.0.0.1:$((PORT + 2))" KORA_INSTALL_DIR="$D"
check "  ... from v0.2.0-rc1 (draft v0.3.0 skipped)" out_has "releases/download/v0.2.0-rc1"
D="$WORK/t10b/bin"
check "KORA_VERSION without v is accepted" run KORA_VERSION=0.2.0-rc1 KORA_DOWNLOAD_BASE="http://127.0.0.1:$((PORT + 2))" KORA_INSTALL_DIR="$D"
check "  ... uses that tag" out_has "releases/download/v0.2.0-rc1"
check "bad KORA_VERSION refused" run_fail KORA_VERSION='v1/../x' KORA_INSTALL_DIR="$WORK/t10c"
check "insecure KORA_API_BASE refused" run_fail KORA_API_BASE="http://evil.example" KORA_INSTALL_DIR="$WORK/t10d"
check "insecure KORA_DOWNLOAD_BASE refused" run_fail KORA_DOWNLOAD_BASE="http://evil.example" KORA_INSTALL_DIR="$WORK/t10d"
kill "$API_PID" 2>/dev/null

echo
if [ "$fails" -eq 0 ]; then echo "all install.sh tests passed"; else echo "$fails install.sh test(s) FAILED"; exit 1; fi
