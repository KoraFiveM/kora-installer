#!/usr/bin/env bash
# Shared helpers for the Linux smoke jobs. Source it: . scripts/smoke-linux/common.sh
# Uses only ports 31120 (game) and 41120 (txAdmin).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KORA_BIN="${KORA_BIN:-$REPO_ROOT/target/release/kora}"
SMOKE_ROOT="${SMOKE_ROOT:-$HOME/kora-smoke}"
export KORA_HOME="${KORA_HOME:-$SMOKE_ROOT/home/.kora}"
export KORA_CATALOG="$REPO_ROOT/tests/fixtures/smoke-catalog.json"
FAKE_KEY='cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'
GAME_PORT=31120
TX_PORT=41120
# Set KORA_SUDO=1 to run kora as root (service jobs). The environment is passed explicitly with `env`
# because sudo-rs (Ubuntu 26.04) ignores -E.
run_kora() {
  if [ "${KORA_SUDO:-}" = "1" ]; then
    sudo env "KORA_HOME=$KORA_HOME" "KORA_CATALOG=$KORA_CATALOG" "CFX_LICENSE_KEY=${CFX_LICENSE_KEY:-}" \
      ${KORA_TEST_FAIL_HEALTH:+"KORA_TEST_FAIL_HEALTH=$KORA_TEST_FAIL_HEALTH"} ${KORA_ALLOW_ROOT_PANEL:+"KORA_ALLOW_ROOT_PANEL=$KORA_ALLOW_ROOT_PANEL"} "$KORA_BIN" "$@"
  else
    "$KORA_BIN" "$@"
  fi
}

PASS=0
step() { printf '\n=== %s ===\n' "$*"; }
note() { printf '  %s\n' "$*"; }
fail() { printf 'ASSERTION FAILED: %s\n' "$*" >&2; exit 1; }
# assert "message" cmd args...   (the command's success is the assertion)
assert() {
  local msg="$1"; shift
  if "$@" >/dev/null 2>&1; then PASS=$((PASS + 1)); printf 'ok: %s\n' "$msg"; else fail "$msg  [$*]"; fi
}
assert_eq() { # assert_eq "message" expected actual
  if [ "$2" = "$3" ]; then PASS=$((PASS + 1)); printf 'ok: %s\n' "$1"; else fail "$1 (expected '$2', got '$3')"; fi
}
assert_contains() { # assert_contains "message" haystack needle   (fixed string)
  if grep -qF -- "$3" <<<"$2"; then PASS=$((PASS + 1)); printf 'ok: %s\n' "$1"; else fail "$1 (missing '$3' in: ${2:0:600})"; fi
}
assert_matches() { # assert_matches "message" haystack ERE
  if grep -qE -- "$3" <<<"$2"; then PASS=$((PASS + 1)); printf 'ok: %s\n' "$1"; else fail "$1 (no match for /$3/ in: ${2:0:600})"; fi
}
assert_not_contains() {
  if grep -qF -- "$3" <<<"$2"; then fail "$1 (found '$3')"; else PASS=$((PASS + 1)); printf 'ok: %s\n' "$1"; fi
}
perms_of() { sudo stat -c '%a' "$1"; }
as_root() { sudo "$@"; }

# kora ARGS...: run the binary, never abort on a non-zero exit; sets OUT (stdout+stderr) and CODE.
# stdin is /dev/null and stdout is captured, so kora sees no terminal.
kora() {
  set +e
  OUT="$(run_kora "$@" </dev/null 2>&1)"
  CODE=$?
  set -e
  printf -- '--- kora %s (exit %s) ---\n%s\n' "$*" "$CODE" "$OUT"
}

# wait_for "description" seconds cmd...
wait_for() {
  local what="$1" secs="$2"; shift 2
  local end=$((SECONDS + secs))
  while [ "$SECONDS" -lt "$end" ]; do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 2
  done
  return 1
}

dump_logs() { # called on failure: tail interesting logs
  echo "::group::logs for debugging"
  for f in "$SMOKE_ROOT"/*/.kora/install.log "$SMOKE_ROOT"/*/update.log "$SMOKE_ROOT"/*/txData/*/logs/* "$SMOKE_ROOT"/*/.kora/service-host.log; do
    [ -f "$f" ] && { echo "== $f"; sudo tail -n 60 "$f" 2>/dev/null || true; }
  done
  [ -d "$KORA_HOME" ] && { echo "== $KORA_HOME"; sudo ls -la "$KORA_HOME" "$KORA_HOME/store" || true; registry_json || true; }
  for u in ${DUMP_UNITS:-kora-smoke}; do
    echo "== journal $u"; sudo journalctl -u "$u" --no-pager -n 60 2>/dev/null || true
  done
  echo "::endgroup::"
}
trap 'rc=$?; if [ $rc -ne 0 ]; then dump_logs; fi' EXIT

# The server list lives in Kora's data file ($KORA_HOME/store/kora.db), not in servers.json any more. `kora store export-json`
# writes it out in the old servers.json format, which is what these checks read (nothing is printed when no server is registered).
registry_json() {
  local d; d="$(mktemp -d)"
  sudo env "KORA_HOME=$KORA_HOME" "$KORA_BIN" store export-json --to "$d/out" >/dev/null 2>&1 || true
  sudo cat "$d/out/servers.json" 2>/dev/null || true
  sudo rm -rf "$d"
}
registry_has() { registry_json | grep -q "\"name\": \"$1\""; }
registry_lacks() { ! registry_has "$1"; }
