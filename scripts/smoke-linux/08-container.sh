#!/usr/bin/env bash
# container: a server in container mode (Linux), with ONE engine given as the first argument: `podman` (rootless: no
# administrator rights at all) or `docker`. Installs it with the MariaDB container, starts it, checks txAdmin through
# the panel's gateway, file ownership, start/stop/restart, the console copy, an update, a backup and a restore with the
# database container, a move (export and import as a new container server), and a delete; then checks that nothing is left behind
# (containers, networks, volumes, user units). Prints engine info, `inspect` and logs on failure.
#
# This is the only place container mode is exercised against a real engine (the unit tests use a fake runner). A third
# phase tries a read-only root file system and REPORTS what FXServer does with it (a warning, not a failure: the default
# stays read-write until this shows it is tolerated).
ENGINE="${1:?usage: 08-container.sh podman|docker}"
case "$ENGINE" in podman|docker) ;; *) echo "engine must be podman or docker" >&2; exit 2 ;; esac
export SMOKE_ROOT="${SMOKE_ROOT:-$HOME/kora-smoke}"
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"
D="$SMOKE_ROOT/srv"
NAME=boxsmoke
SLUG=$NAME
GAME="kora-$SLUG"
DBC="kora-$SLUG-db"
NET="kora-$SLUG-net"
VOL="kora-$SLUG-dbdata"
export CFX_LICENSE_KEY="$FAKE_KEY"
ME="$(id -un)"

# ---- diagnostics on failure ---------------------------------------------------------------------
container_diag() {
  echo "::group::container diagnostics ($ENGINE)"
  "$ENGINE" version 2>&1 | head -n 30 || true
  "$ENGINE" info 2>&1 | head -n 60 || true
  echo "== all containers"; "$ENGINE" ps -a --format '{{.Names}}  {{.Status}}  {{.Image}}' 2>&1 || true
  for c in "$GAME" "$DBC"; do
    echo "== inspect $c"; "$ENGINE" inspect "$c" 2>&1 | head -n 120 || true
    echo "== logs $c"; "$ENGINE" logs --tail 80 "$c" 2>&1 || true
  done
  echo "== networks"; "$ENGINE" network ls 2>&1 || true
  echo "== volumes"; "$ENGINE" volume ls 2>&1 || true
  echo "== console.log"; tail -n 60 "$D/.kora/console.log" 2>&1 || true
  echo "== registered"; registry_json || true
  echo "== user units"; systemctl --user list-units --all --no-pager 2>&1 | head -n 30 || true
  echo "::endgroup::"
}
trap 'rc=$?; if [ $rc -ne 0 ]; then container_diag; dump_logs; fi' EXIT

jget() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
txadmin_up() { curl -s -o /dev/null --max-time 5 "http://127.0.0.1:$TX_PORT/"; }
state() { "$ENGINE" inspect --format '{{.State.Status}}' "$GAME" 2>/dev/null || echo missing; }
dbsql() { # dbsql "SQL": run as the server's own login INSIDE the database container
  local pw; pw="$(jget "$D/database.json" password)"
  MYSQL_PWD="$pw" "$ENGINE" exec -e MYSQL_PWD "$DBC" mariadb --protocol=tcp -h127.0.0.1 -u"$(jget "$D/database.json" user)" -N -B -e "$1" "$(jget "$D/database.json" database)"
}
objects_left() { # everything on the engine that carries or is named for this server
  {
    "$ENGINE" ps -a --filter "label=kora.server=$1" --format '{{.Names}}'
    "$ENGINE" network ls --filter "label=kora.server=$1" --format '{{.Name}}'
    "$ENGINE" volume ls --filter "label=kora.server=$1" --format '{{.Name}}'
    "$ENGINE" ps -a --format '{{.Names}}' | grep -E "^kora-$1(-db)?\$" || true
    "$ENGINE" network ls --format '{{.Name}}' | grep -E "^kora-$1-net\$" || true
    "$ENGINE" volume ls --format '{{.Name}}' | grep -E "^kora-$1-dbdata\$" || true
  } 2>/dev/null
}

step "0. the engine"
if [ "$ENGINE" = "podman" ]; then
  # Rootless Podman needs subuid/subgid ranges and a user runtime folder; a fresh runner may lack them (this is the setup
  # a person would do once; Kora's System check says the same).
  grep -q "^$ME:" /etc/subuid 2>/dev/null || sudo usermod --add-subuids 100000-165535 --add-subgids 100000-165535 "$ME"
  sudo loginctl enable-linger "$ME" || true
  export XDG_RUNTIME_DIR="/run/user/$(id -u)"
  podman system migrate >/dev/null 2>&1 || true
fi
"$ENGINE" version | head -n 5
if [ "$ENGINE" = "podman" ]; then
  assert_eq "podman is rootless" true "$(podman info --format '{{.Host.Security.Rootless}}')"
fi
# Kora's System check must see the same engine (Containers line)
kora check
note "$(printf '%s\n' "$OUT" | grep -i -A1 'containers' | head -n 3)"

step "1. install in container mode (database container, no administrator rights)"
kora install --name $NAME --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --run-mode container --engine "$ENGINE" --db container --start -y
assert_eq "install exits 0" 0 "$CODE"
assert_not_contains "licence key never printed" "$OUT" "$FAKE_KEY"
assert "the record exists" test -f "$D/kora-container.json"
REC="$(cat "$D/kora-container.json")"
assert_not_contains "the record holds no password" "$REC" "password"
assert_contains "the record names the engine" "$REC" "\"engine\": \"$ENGINE\""
assert_contains "the image is pinned by digest" "$REC" "@sha256:"
assert_eq "kora never used sudo: no kora account, no unit" "" "$(ls /etc/systemd/system/kora-$NAME.service 2>/dev/null || true)"

step "2. the objects, labelled, and the container's flags"
assert_eq "game container runs" running "$(state)"
assert_eq "database container runs" running "$("$ENGINE" inspect --format '{{.State.Status}}' "$DBC")"
assert_eq "game container is labelled" "$SLUG" "$("$ENGINE" inspect --format '{{index .Config.Labels "kora.server"}}' "$GAME")"
assert "the private network exists" "$ENGINE" network inspect "$NET"
assert "the data volume exists" "$ENGINE" volume inspect "$VOL"
INSPECT="$("$ENGINE" inspect "$GAME")"
assert_not_contains "not privileged" "$INSPECT" '"Privileged": true'
assert_not_contains "not the host network" "$("$ENGINE" inspect --format '{{.HostConfig.NetworkMode}}' "$GAME")" "host"
assert_contains "all capabilities dropped" "$INSPECT" "ALL"
assert_eq "the txAdmin port is published on loopback only" "127.0.0.1" "$("$ENGINE" port "$GAME" "$TX_PORT/tcp" | head -n1 | cut -d: -f1)"
assert_matches "the game port is published for TCP and UDP" "$("$ENGINE" port "$GAME")" "$GAME_PORT/udp"
assert_not_contains "the database is not published" "$("$ENGINE" port "$DBC" 2>&1 || true)" "3306/tcp ->"

step "3. txAdmin answers, directly and through the panel's gateway"
if ! wait_for "txAdmin" 240 txadmin_up; then "$ENGINE" logs --tail 80 "$GAME" || true; fail "txAdmin does not answer on 127.0.0.1:$TX_PORT"; fi
assert "txAdmin answers on 127.0.0.1:$TX_PORT" txadmin_up
logfound() { grep -qiE 'Could not authenticate server license key' "$D/.kora/console.log" 2>/dev/null || "$ENGINE" logs "$GAME" 2>&1 | grep -qiE 'Could not authenticate server license key'; }
if ! wait_for "licence rejection logged" 180 logfound; then
  echo "----- why FXServer did not run: the container's processes, its libraries, and its log without the relocation noise -----"
  "$ENGINE" exec "$GAME" sh -c 'ps -o pid,args | head -n 20; echo; ls -la '"$D"'/artifacts/*/alpine/usr/lib 2>&1 | grep -iE "stdc|gcc|v8" | head; echo; for p in $(pidof FXServer); do echo "pid $p"; tr "\0" "\n" </proc/$p/environ | grep -E "^(LD_|PATH)" ; done' 2>&1 || true
  "$ENGINE" logs "$GAME" 2>&1 | grep -v "Error relocating" | head -n 40 || true
  "$ENGINE" logs "$GAME" 2>&1 | grep "Error relocating" | head -n 3 || true
  fail "no 'licence key rejected' line: FXServer did not really run in the container"
fi
assert "FXServer ran in the container (the fake licence key was rejected, as expected)" logfound
PANEL_PORT=47132
GW_PORT=$((TX_PORT + 10000))
PANEL_TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
mkdir -p "$KORA_HOME"; printf '%s' "$PANEL_TOKEN" >"$KORA_HOME/panel-token"; chmod 600 "$KORA_HOME/panel-token"
run_kora panel --foreground --no-open --port "$PANEL_PORT" >"$SMOKE_ROOT/panel.log" 2>&1 &
PANEL_PID=$!
panel_up() { curl -fsS --max-time 3 "http://127.0.0.1:$PANEL_PORT/api/v1/meta" >/dev/null; }
wait_for "panel" 40 panel_up || { cat "$SMOKE_ROOT/panel.log" || true; kill "$PANEL_PID" 2>/dev/null || true; fail "the panel did not start"; }
JAR="$SMOKE_ROOT/panel.jar"; GJAR="$SMOKE_ROOT/gate.jar"; rm -f "$JAR" "$GJAR"
SESSION="$(curl -fsS -c "$JAR" -H 'X-Kora: 1' -H 'Content-Type: application/json' -d "{\"token\":\"$PANEL_TOKEN\"}" "http://127.0.0.1:$PANEL_PORT/api/v1/session")"
SECRET="$(sed -n 's/.*"secret":"\([^"]*\)".*/\1/p' <<<"$SESSION")"
assert "panel sign-in gave a session secret" test -n "$SECRET"
# the panel's own server list says how it runs
SERVERS="$(curl -fsS -b "$JAR" -H "X-Kora-Session: $SECRET" -H 'X-Kora: 1' "http://127.0.0.1:$PANEL_PORT/api/v1/servers")"
assert_contains "the panel's server list says container" "$SERVERS" '"run_mode":"container"'
assert_contains "and in words" "$SERVERS" "in a container ($ENGINE"
TICKET="$(curl -fsS -b "$JAR" -H "X-Kora-Session: $SECRET" -H 'X-Kora: 1' -X POST "http://127.0.0.1:$PANEL_PORT/api/v1/servers/$NAME/txadmin/ticket")"
URL="$(sed -n 's/.*"url":"\([^"]*\)".*/\1/p' <<<"$TICKET")"
assert_contains "the ticket points at the panel-side port" "$URL" ":$GW_PORT/__kora/enter?t="
assert_eq "entering with the one-time ticket redirects" 303 "$(curl -sS -o /dev/null -w '%{http_code}' -c "$GJAR" "$URL")"
DIRECT="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 "http://127.0.0.1:$TX_PORT/")"
VIA="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 -b "$GJAR" "http://127.0.0.1:$GW_PORT/")"
assert_eq "txAdmin answers through the panel like it does directly (HTTP $DIRECT)" "$DIRECT" "$VIA"
READS="$(curl -fsS -b "$JAR" -H "X-Kora-Session: $SECRET" -H 'X-Kora: 1' "http://127.0.0.1:$PANEL_PORT/api/v1/servers/$NAME/reach" || true)"
note "reachability: ${READS:0:300}"
kill "$PANEL_PID" 2>/dev/null || true; wait "$PANEL_PID" 2>/dev/null || true
rm -f "$KORA_HOME/panel-token" "$JAR" "$GJAR"

step "4. files stay the person's, and Kora manages what the container wrote"
assert_eq "no file in the server folder belongs to anyone else" "" "$(find "$D" \( -path "$D/resources" -prune -o -path "$D/artifacts" -prune \) -o ! -user "$ME" -print | head -n 5)"
assert "txData was written by the container as the person" test -d "$D/txData"
"$ENGINE" exec "$GAME" /bin/sh -c "echo from-container > '$D/from-container.txt'" 2>/dev/null || "$ENGINE" exec "$GAME" sh -c "echo from-container > '$D/from-container.txt'"
assert_eq "a file made inside the container is the person's" "$ME" "$(stat -c %U "$D/from-container.txt")"
assert "and the person can delete it" rm "$D/from-container.txt"
assert "the console copy exists and has the server's output" bash -c "[ -s '$D/.kora/console.log' ]"
kora status
assert_matches "status shows the server running" "$OUT" "$NAME[[:space:]]+clone, released[[:space:]]+$GAME_PORT[[:space:]]+$TX_PORT[[:space:]]+running"
assert_contains "status says how it runs" "$OUT" "runs in a container ($ENGINE"

step "5. stop / start / restart through Kora"
ID1="$("$ENGINE" inspect --format '{{.Id}}' "$GAME")"
kora stop $NAME
assert_eq "kora stop exits 0" 0 "$CODE"
assert_eq "the container is stopped (not removed)" exited "$(state)"
assert "the database container keeps running" bash -c "[ \"\$('$ENGINE' inspect --format '{{.State.Status}}' '$DBC')\" = running ]"
kora start $NAME
assert_eq "kora start exits 0" 0 "$CODE"
assert_eq "running after start" running "$(state)"
assert "txAdmin answers again" wait_for txadmin 240 txadmin_up
kora restart $NAME
assert_eq "kora restart exits 0" 0 "$CODE"
assert_eq "running after restart" running "$(state)"
assert "txAdmin answers after restart" wait_for txadmin 240 txadmin_up
assert_eq "the same container was reused" "$ID1" "$("$ENGINE" inspect --format '{{.Id}}' "$GAME")"
sleep 10
assert "the console copy is being written after a restart" bash -c "[ -s '$D/.kora/console.log' ]"

step "6. an update (through the engine, with a full restart)"
kora update $NAME --all --yes --restart full
assert_eq "kora update exits 0" 0 "$CODE"
assert_eq "running after the update" running "$(state)"
assert_eq "still every file the person's" "" "$(find "$D" \( -path "$D/resources" -prune -o -path "$D/artifacts" \) -o ! -user "$ME" -print | head -n 3)"

step "7. backup and restore with the database container"
# `--folder` is a one-off copy kept elsewhere: `latest`, verify and restore read Kora's own backups folder.
BK="$KORA_HOME/server-backups"
dbsql "CREATE TABLE players (id INT PRIMARY KEY, name VARCHAR(40))"
dbsql "INSERT INTO players VALUES (1,'alice'),(2,'bob')"
kora backup create $NAME
assert_eq "backup create exits 0 (the dump ran inside the database container)" 0 "$CODE"
PW="$(jget "$D/database.json" password)"
assert_not_contains "the database password is never printed" "$OUT" "$PW"
ARCHIVE="$(ls "$BK"/*/*.tar.gz | head -n1)"
assert "the archive exists" test -f "$ARCHIVE"
assert "the archive holds the dump and the container record" bash -c "tar -tzf '$ARCHIVE' | grep -q '^database/dump.sql' && tar -tzf '$ARCHIVE' | grep -q '^server/kora-container.json'"
kora backup verify $NAME latest
assert_eq "backup verify exits 0" 0 "$CODE"
dbsql "DELETE FROM players"
assert_eq "the table is wiped" 0 "$(dbsql 'SELECT COUNT(*) FROM players')"
kora backup restore $NAME latest --yes
assert_eq "backup restore exits 0" 0 "$CODE"
assert_eq "both rows are back after the restore" 2 "$(dbsql 'SELECT COUNT(*) FROM players')"
assert "the restore did not change how the server runs" test -f "$D/kora-container.json"
assert "the password never travelled on any command line in this job" bash -c "! grep -r -- '$PW' '$SMOKE_ROOT'/*.log 2>/dev/null"

step "8. move it: an encrypted export imported as a new container server"
MOVE="$KORA_HOME/server-backups"
export KORA_BACKUP_PASSWORD='a smoke test passphrase'
kora backup create $NAME --for-move
assert_eq "backup create --for-move exits 0" 0 "$CODE"
MOVEFILE="$(ls "$MOVE"/*/*.tar.gz.age | head -n1)"
MOVED="$SMOKE_ROOT/moved"
kora backup import "$MOVEFILE" --name ${NAME}2 --dir "$MOVED" --game-port 32120 --txadmin-port 42120 --run-mode container --yes
assert_eq "import exits 0" 0 "$CODE"
assert "the imported server has its own record" test -f "$MOVED/kora-container.json"
echo "----- on the engine after the import -----"; "$ENGINE" ps -a --format '{{.Names}} {{.Status}}' || true; "$ENGINE" inspect --format '{{.State.Status}}' kora-${NAME}2 2>&1 | head -n 3 || true
assert_eq "its game container exists (not started)" "created" "$("$ENGINE" inspect --format '{{.State.Status}}' kora-${NAME}2 2>/dev/null || echo missing)"
assert_eq "its database holds the rows" 2 "$(MYSQL_PWD="$(jget "$MOVED/database.json" password)" "$ENGINE" exec -e MYSQL_PWD kora-${NAME}2-db mariadb --protocol=tcp -h127.0.0.1 -u"$(jget "$MOVED/database.json" user)" -N -B -e 'SELECT COUNT(*) FROM players' "$(jget "$MOVED/database.json" database)")"
kora delete ${NAME}2 -y
assert_eq "delete of the imported server exits 0" 0 "$CODE"
assert_eq "nothing of the imported server is left on the engine" "" "$(objects_left ${NAME}2)"
unset KORA_BACKUP_PASSWORD

step "9. delete removes everything"
kora delete $NAME -y
assert_eq "delete exits 0" 0 "$CODE"
assert_eq "no container, network or volume is left" "" "$(objects_left $SLUG)"
assert "the server folder is gone" test ! -e "$D"
assert "no registry entry" registry_lacks $NAME
assert "no user unit for the server" bash -c "! systemctl --user list-units --all --no-legend 2>/dev/null | grep -q 'kora-$NAME'"
assert "no generated quadlet or unit file" bash -c "! ls \"\$HOME/.config/containers/systemd/kora-$NAME\"* \"\$HOME/.config/systemd/user/kora-$NAME\"* 2>/dev/null | grep -q ."

step "10. a read-only root file system (reported, not enforced)"
RO=boxro
RO_DIR="$SMOKE_ROOT/ro"
export KORA_CONTAINER_READ_ONLY=1
kora install --name $RO --dir "$RO_DIR" --game-port $GAME_PORT --txadmin-port $TX_PORT --run-mode container --engine "$ENGINE" --db skip --start -y
if [ "$CODE" = 0 ] && wait_for "txAdmin (read-only root)" 180 txadmin_up; then
  echo "READ-ONLY ROOT: FXServer and txAdmin ran with --read-only (txAdmin answers)."
else
  echo "::warning::READ-ONLY ROOT: not tolerated by FXServer here; the default stays read-write. $("$ENGINE" logs --tail 20 "kora-$RO" 2>&1 | tr '\n' ' ' | cut -c1-400)"
fi
unset KORA_CONTAINER_READ_ONLY
kora delete $RO -y
assert_eq "the read-only server is removed again" "" "$(objects_left $RO)"

echo; echo "container ($ENGINE): $PASS assertions passed"
