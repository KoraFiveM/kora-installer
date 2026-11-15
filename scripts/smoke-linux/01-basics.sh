#!/usr/bin/env bash
# basics: --version, friendly no-TTY error, dry-run plan, port handling, github owners without gh login.
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"

step "1. kora --version"
kora --version
assert_eq "exit 0" 0 "$CODE"
assert_matches "prints 'kora X.Y.Z'" "$OUT" '^kora [0-9]+\.[0-9]+\.[0-9]+'

step "2. kora install with no terminal: friendly error, exit 2"
kora install
assert_eq "exit 2" 2 "$CODE"
assert_contains "explains that a terminal is needed" "$OUT" "needs an interactive terminal"

step "3. kora status with nothing registered"
kora status
assert_eq "exit 0" 0 "$CODE"
assert_contains "says no servers" "$OUT" "No Kora servers registered"

step "4. install --dry-run prints the plan and changes nothing"
D="$SMOKE_ROOT/dry"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --name dry --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --dry-run -y
assert_eq "exit 0" 0 "$CODE"
assert_contains "plan header" "$OUT" "Plan (nothing was changed)"
assert_matches "choices show the ports" "$OUT" "ports:[[:space:]]+game $GAME_PORT, txAdmin $TX_PORT"
assert_matches "plan lists the clone step" "$OUT" "build\.resources"
assert_not_contains "licence key never printed" "$OUT" "$FAKE_KEY"
assert "dry run created no server folder" test ! -e "$D"
assert "dry run wrote no registry" test ! -e "$KORA_HOME/servers.json"

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
CFX_LICENSE_KEY="$FAKE_KEY" kora install --name busy --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip -y
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

echo; echo "basics: $PASS assertions passed"
