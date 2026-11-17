#!/usr/bin/env bash
# install-service: production install with --service under systemd (runs kora as root via sudo -E),
# start/stop/restart, txAdmin answering, then kora uninstall and kora delete remove everything.
export SMOKE_ROOT="${SMOKE_ROOT:-/opt/kora-smoke}"
export KORA_SUDO=1
. "$(dirname "$0")/common.sh"
sudo rm -rf "$SMOKE_ROOT"; sudo mkdir -p "$SMOKE_ROOT"
D="$SMOKE_ROOT/srv"
NAME=smoke
SVC=kora-$NAME
UNIT=/etc/systemd/system/$SVC.service
ACCOUNT=kora-$NAME
export CFX_LICENSE_KEY="$FAKE_KEY"
state() { systemctl is-active "$SVC" 2>/dev/null || true; }
install_it() { kora install --mode prod --name $NAME --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --service -y; }
txadmin_up() { curl -s -o /dev/null --max-time 5 "http://127.0.0.1:$TX_PORT/"; }
assert_gone() {
  assert "unit file $UNIT is gone" test ! -e "$UNIT"
  assert "service user $ACCOUNT is gone" bash -c "! id -u $ACCOUNT"
  assert "server folder is gone" test ! -e "$D"
  assert "no registry entry" registry_lacks "$NAME"
  assert "systemd does not know the unit" bash -c "! systemctl list-unit-files $SVC.service --no-legend | grep -q ."
}

step "1. install --service"
install_it
assert_eq "install exits 0" 0 "$CODE"
assert_not_contains "licence key never printed" "$OUT" "$FAKE_KEY"
assert_eq "service is registered but not started" inactive "$(state)"
assert_eq "enabled at boot" enabled "$(systemctl is-enabled $SVC)"

step "2. unit file and service account"
U="$(cat "$UNIT")"; echo "$U"
assert_contains "runs as the dedicated user" "$U" "User=$ACCOUNT"
assert_contains "working directory is the server folder" "$U" "WorkingDirectory=$D"
assert_matches "ExecStart is 'kora service-host <folder>'" "$U" "^ExecStart=.*kora.* service-host $D\$"
assert_contains "restarts on failure" "$U" "Restart=on-failure"
assert_contains "hardened (NoNewPrivileges)" "$U" "NoNewPrivileges=true"
assert "service user exists" id -u $ACCOUNT
assert_eq "service user has no login shell" /usr/sbin/nologin "$(getent passwd $ACCOUNT | cut -d: -f7)"
assert "service user is a system account (uid < 1000)" bash -c "[ \$(id -u $ACCOUNT) -lt 1000 ]"
assert_eq "server folder is owned by the service user" $ACCOUNT "$(stat -c %U "$D")"
assert_eq "secrets.cfg is 0600" 600 "$(perms_of "$D/secrets.cfg")"
assert "service user can read secrets.cfg" sudo -u $ACCOUNT cat "$D/secrets.cfg"
assert "other users cannot read secrets.cfg" bash -c "! sudo -u nobody cat '$D/secrets.cfg'"
KORA_COPY="$(sed -n 's/^ExecStart=\([^ ]*\) service-host.*/\1/p' "$UNIT" | tr -d '"')"
note "service runs: $KORA_COPY"
assert_eq "the service program is a root-owned copy" root "$(stat -c %U "$KORA_COPY")"
kora status
assert_matches "status shows stopped" "$OUT" "$NAME[[:space:]]+clone, released[[:space:]]+$GAME_PORT[[:space:]]+$TX_PORT[[:space:]]+stopped"

step "3. uninstall --dry-run leaves everything"
kora uninstall --dry-run
assert_eq "dry run exits 0" 0 "$CODE"
assert_contains "dry run says nothing was changed" "$OUT" "Dry run: nothing was changed"
assert "unit still there" test -e "$UNIT"
assert "user still there" id -u $ACCOUNT

step "4. start / restart / stop"
kora start $NAME
assert_eq "kora start exits 0" 0 "$CODE"
assert_eq "systemctl is-active: active after start" active "$(state)"
if ! wait_for "txAdmin" 180 txadmin_up; then sudo journalctl -u $SVC --no-pager -n 80 || true; fail "txAdmin does not answer on 127.0.0.1:$TX_PORT"; fi
assert "txAdmin answers on 127.0.0.1:$TX_PORT" txadmin_up
# The fake key is rejected by FXServer once txAdmin has started it: expected, and proves FXServer really ran.
jr() { sudo journalctl -u $SVC --no-pager 2>/dev/null; }
logfound() { local t; t="$({ jr; sudo cat "$D"/txData/*/logs/* 2>/dev/null; } 2>/dev/null)"; grep -qiE 'Could not authenticate server license key' <<<"$t"; }
wait_for "licence rejection logged" 120 logfound || { jr | tail -n 80; fail "no 'licence key rejected' line in the logs"; }
assert "log shows FXServer rejected the fake licence key (expected)" logfound
assert "an FXServer process of the service user runs" pgrep -u $ACCOUNT -f 'FXServer|run.sh'
kora status
assert_matches "status shows running" "$OUT" "$NAME[[:space:]]+clone, released[[:space:]]+$GAME_PORT[[:space:]]+$TX_PORT[[:space:]]+running"
sleep 15
assert_eq "still active 15 s after start" active "$(state)"
PID1="$(systemctl show -p MainPID --value $SVC)"
kora restart $NAME
assert_eq "kora restart exits 0" 0 "$CODE"
assert_eq "active after restart" active "$(state)"
PID2="$(systemctl show -p MainPID --value $SVC)"
assert "restart really replaced the process ($PID1 -> $PID2)" test "$PID1" != "$PID2"
assert "txAdmin answers again after restart" wait_for txadmin 180 txadmin_up

step "4b. txAdmin through the panel's own address (the txAdmin gateway)"
PANEL_PORT=47131
GW_PORT=$((TX_PORT + 10000))
PANEL_TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
# The panel runs as whoever runs kora in this job (root when KORA_SUDO=1: the home was made by root).
SUDO=""; if [ "${KORA_SUDO:-}" = "1" ]; then SUDO="sudo"; fi
$SUDO mkdir -p "$KORA_HOME"
printf '%s' "$PANEL_TOKEN" | $SUDO tee "$KORA_HOME/panel-token" >/dev/null
$SUDO chmod 600 "$KORA_HOME/panel-token"
run_kora panel --foreground --no-open --port "$PANEL_PORT" >"$SMOKE_ROOT/panel.log" 2>&1 &
PANEL_PID=$!
panel_up() { curl -fsS --max-time 3 "http://127.0.0.1:$PANEL_PORT/api/v1/meta" >/dev/null; }
wait_for "panel" 40 panel_up || { cat "$SMOKE_ROOT/panel.log" || true; $SUDO kill "$PANEL_PID" 2>/dev/null || true; fail "the panel did not start"; }
JAR="$SMOKE_ROOT/panel.jar"; GJAR="$SMOKE_ROOT/gate.jar"; rm -f "$JAR" "$GJAR"
SESSION="$(curl -fsS -c "$JAR" -H 'X-Kora: 1' -H 'Content-Type: application/json' -d "{\"token\":\"$PANEL_TOKEN\"}" "http://127.0.0.1:$PANEL_PORT/api/v1/session")"
SECRET="$(sed -n 's/.*"secret":"\([^"]*\)".*/\1/p' <<<"$SESSION")"
assert "panel sign-in gave a session secret" test -n "$SECRET"
TICKET="$(curl -fsS -b "$JAR" -H "X-Kora-Session: $SECRET" -H 'X-Kora: 1' -X POST "http://127.0.0.1:$PANEL_PORT/api/v1/servers/$NAME/txadmin/ticket")"
URL="$(sed -n 's/.*"url":"\([^"]*\)".*/\1/p' <<<"$TICKET")"
assert_contains "the ticket points at the panel-side port ($GW_PORT = txAdmin $TX_PORT + 10000)" "$URL" ":$GW_PORT/__kora/enter?t="
assert_eq "entering with the one-time ticket redirects" 303 "$(curl -sS -o /dev/null -w '%{http_code}' -c "$GJAR" "$URL")"
assert_eq "a ticket works once" 401 "$(curl -sS -o /dev/null -w '%{http_code}' "$URL")"
DIRECT="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 "http://127.0.0.1:$TX_PORT/")"
VIA="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 -b "$GJAR" "http://127.0.0.1:$GW_PORT/")"
assert_eq "txAdmin answers through the panel like it does directly (HTTP $DIRECT)" "$DIRECT" "$VIA"
assert_eq "the gateway answers nothing without the panel sign-in" 401 "$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 "http://127.0.0.1:$GW_PORT/")"
$SUDO kill "$PANEL_PID" 2>/dev/null || true; wait "$PANEL_PID" 2>/dev/null || true
$SUDO rm -f "$KORA_HOME/panel-token"; rm -f "$JAR" "$GJAR"
sudo systemctl stop $SVC
assert_eq "plain systemctl stop works too" inactive "$(state)"
kora start $NAME
assert_eq "active again" active "$(state)"
kora stop $NAME
assert_eq "kora stop exits 0" 0 "$CODE"
assert_eq "inactive after kora stop" inactive "$(state)"
assert_eq "unit is not in a failed state after a clean stop" inactive "$(systemctl is-active $SVC || true)"
sleep 3
assert "no FXServer left behind" bash -c "! pgrep -u $ACCOUNT -f 'FXServer'"

step "5. kora uninstall --yes --delete-files removes unit, user, files"
kora uninstall --yes --delete-files
assert_eq "uninstall exits 0" 0 "$CODE"
assert_gone
assert "the root-owned program copy is gone" test ! -e "$KORA_COPY"

step "6. second install, then kora delete does the same"
install_it
assert_eq "install exits 0" 0 "$CODE"
assert "unit exists again" test -e "$UNIT"
assert "service user exists again" id -u $ACCOUNT
kora delete $NAME -y
assert_eq "delete exits 0" 0 "$CODE"
assert_gone

echo; echo "install-service: $PASS assertions passed"
