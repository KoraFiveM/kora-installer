#!/usr/bin/env bash
# Kora's own GitHub CLI: with gh hidden from PATH, `kora tools install` downloads it over HTTPS, verifies the pinned
# checksum and unpacks it under $KORA_HOME/tools; then gh runs from there, the system check lists it, and
# `kora uninstall --yes` removes it. Git on Linux stays the system package (never downloaded).
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"
HOME_T="$SMOKE_ROOT/tools-home"; BIN="$SMOKE_ROOT/tools-bin"; mkdir -p "$BIN"
for b in git id systemctl which; do p="$(command -v "$b" || true)"; [ -n "$p" ] && ln -sf "$p" "$BIN/$b"; done

step "1. kora tools status with gh hidden from PATH"
set +e; OUT="$(env PATH="$BIN" KORA_HOME="$HOME_T" "$KORA_BIN" tools status 2>&1)"; CODE=$?; set -e; echo "$OUT"
assert_eq "exit 1 (gh missing)" 1 "$CODE"
assert_contains "gh is reported missing" "$OUT" "GitHub CLI: not installed"

step "2. kora tools install"
set +e; OUT="$(env PATH="$BIN" KORA_HOME="$HOME_T" "$KORA_BIN" tools install 2>&1)"; CODE=$?; set -e; echo "$OUT"
assert_eq "exit 0" 0 "$CODE"
assert "gh is unpacked into the Kora folder" test -x "$HOME_T/tools/gh/bin/gh"
assert "no partial downloads are left" test -z "$(ls -A "$HOME_T/tools" | grep -E '^\.(dl|stage)-' || true)"

step "3. gh runs from Kora's own copy without any PATH change"
set +e; OUT="$(env PATH="$BIN" KORA_HOME="$HOME_T" "$KORA_BIN" tools status 2>&1)"; CODE=$?; set -e; echo "$OUT"
assert_eq "exit 0" 0 "$CODE"
assert_matches "gh shows Kora's own copy" "$OUT" "GitHub CLI: [0-9]+\.[0-9]+.*Kora's own copy"
set +e; OUT="$(env PATH="$BIN" KORA_HOME="$HOME_T" "$KORA_BIN" check 2>&1)"; set -e; echo "$OUT"
assert_contains "the system check lists the GitHub CLI" "$OUT" "GitHub CLI: Installed"

step "4. kora uninstall --yes removes it"
set +e; OUT="$(env PATH="$BIN" KORA_HOME="$HOME_T" "$KORA_BIN" uninstall --yes 2>&1)"; set -e; echo "$OUT"
assert "the tools folder is gone" test ! -e "$HOME_T/tools"

echo; echo "tools: $PASS assertions passed"
