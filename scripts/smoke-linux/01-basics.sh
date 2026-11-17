#!/usr/bin/env bash
# basics: --version, the removed --code flag, dry-run plan, port handling, github owners without gh login.
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"

step "1. kora --version"
kora --version
assert_eq "exit 0" 0 "$CODE"
assert_matches "prints 'kora X.Y.Z'" "$OUT" '^kora [0-9]+\.[0-9]+\.[0-9]+'

step "2. kora install --code: removed, friendly error, exit 2"
kora install --code KORA-0000-0000
assert_eq "exit 2" 2 "$CODE"
assert_contains "explains that setup codes are gone" "$OUT" "setup codes were removed"

step "3. kora status with nothing registered"
kora status
assert_eq "exit 0" 0 "$CODE"
assert_contains "says no servers" "$OUT" "No Kora servers registered"

step "4. install --dry-run prints the plan and changes nothing"
D="$SMOKE_ROOT/dry"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --source clone --versions released --name dry --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --dry-run -y
assert_eq "exit 0" 0 "$CODE"
assert_contains "plan header" "$OUT" "Plan (nothing was changed)"
assert_matches "choices show the ports" "$OUT" "ports:[[:space:]]+game $GAME_PORT, txAdmin $TX_PORT"
assert_matches "plan lists the clone step" "$OUT" "build\.resources"
assert_not_contains "licence key never printed" "$OUT" "$FAKE_KEY"
assert "dry run created no server folder" test ! -e "$D"
assert_eq "dry run registered no server" "" "$(registry_json)"

step "5. busy port: clear failure, then --auto-ports"
python3 - "$GAME_PORT" "$TX_PORT" <<'PY' &
import socket, sys, time
socks = []
for p in map(int, sys.argv[1:]):
    t = socket.socket(); t.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); t.bind(("0.0.0.0", p)); t.listen(5); socks.append(t)
    u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); u.bind(("0.0.0.0", p)); socks.append(u)
time.sleep(600)
PY
HOLD=$!
trap 'rc=$?; kill $HOLD 2>/dev/null || true; [ $rc -ne 0 ] && dump_logs; exit $rc' EXIT
wait_for "listener up" 20 bash -c "exec 3<>/dev/tcp/127.0.0.1/$GAME_PORT"
D="$SMOKE_ROOT/busy"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --mode prod --name busy --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip -y
assert_eq "busy port: exit 1" 1 "$CODE"
assert_contains "message names the port" "$OUT" "$GAME_PORT"
assert_contains "message suggests --auto-ports" "$OUT" "--auto-ports"
assert "nothing was created" test ! -e "$D"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --name busy --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --dry-run --auto-ports -y
assert_eq "--auto-ports (dry run): exit 0" 0 "$CODE"
assert_matches "reports the moved port" "$OUT" "Port $GAME_PORT is in use, using [0-9]+"
assert_matches "plan uses a different game port" "$OUT" "ports:[[:space:]]+game (3112[1-9]|311[3-9][0-9]|31[2-9][0-9]{2}|3[2-9][0-9]{3})"
kill $HOLD 2>/dev/null || true

step "6. kora github owners without a gh login: friendly message"
EMPTY_HOME="$(mktemp -d)"
set +e
OUT="$(env -u GH_TOKEN -u GITHUB_TOKEN HOME="$EMPTY_HOME" GH_CONFIG_DIR="$EMPTY_HOME/gh" "$KORA_BIN" github owners </dev/null 2>&1)"; CODE=$?
set -e
echo "$OUT"
assert_eq "exit 1" 1 "$CODE"
assert_contains "tells the user to run gh auth login" "$OUT" "gh auth login"

step "7. kora uninstall without --yes and without a terminal"
kora uninstall
assert_eq "exit 2" 2 "$CODE"

step "8. saved licence keys: private file, masked list, named install"
SAVED_KEY='cfxk_SAVEDSAVEDSAVEDSAVED_1234'
OUT="$(printf '%s\n' "$SAVED_KEY" | "$KORA_BIN" keys add Main 2>&1)"; CODE=$?
echo "$OUT"
assert_eq "keys add exits 0" 0 "$CODE"
assert_not_contains "add never prints the key" "$OUT" "$SAVED_KEY"
assert_matches "the data file is 0600 (saved keys live in it)" "$(stat -c %a "$KORA_HOME/store/kora.db")" '^600$'
assert_matches "the data folder is 0700" "$(stat -c %a "$KORA_HOME/store")" '^700$'
assert "no old secrets.json is written any more" test ! -e "$KORA_HOME/secrets.json"
for f in "$KORA_HOME"/store/kora.db-wal "$KORA_HOME"/store/kora.db-shm; do
  [ -e "$f" ] && assert_matches "$(basename "$f") is 0600 too" "$(stat -c %a "$f")" '^600$' || true
done
kora keys list
assert_eq "keys list exits 0" 0 "$CODE"
assert_contains "list shows the last four characters" "$OUT" "1234"
assert_not_contains "list never shows the key" "$OUT" "SAVEDSAVED"
printf '%s\n' "$SAVED_KEY" | "$KORA_BIN" keys add Again >/dev/null 2>&1 && CODE=0 || CODE=$?
assert_eq "the same key twice is refused" 1 "$CODE"
D="$SMOKE_ROOT/named"
kora install --name named --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --license-key-name main --dry-run -y
assert_eq "install --license-key-name (dry run): exit 0" 0 "$CODE"
assert_not_contains "the plan never prints the key" "$OUT" "$SAVED_KEY"
kora install --name named --dir "$D" --license-key-name nope --dry-run -y
assert_eq "an unknown key name: exit 2" 2 "$CODE"
assert_contains "says there is no such key" "$OUT" "no saved licence key called"
kora keys rename main Live
assert_eq "keys rename exits 0" 0 "$CODE"
kora keys remove live -y
assert_eq "keys remove exits 0" 0 "$CODE"
assert_not_contains "the data file no longer holds the key" "$(cat "$KORA_HOME"/store/kora.db* | strings)" "SAVEDSAVED"
kora audit
assert_eq "audit exits 0" 0 "$CODE"
assert_contains "the audit log recorded the key being added" "$OUT" "key_add"
assert_not_contains "the audit log never holds a key" "$OUT" "SAVEDSAVED"
kora store status
assert_eq "store status exits 0" 0 "$CODE"
assert_contains "store status names the data file" "$OUT" "store/kora.db"

echo; echo "basics: $PASS assertions passed"
