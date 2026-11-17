#!/usr/bin/env bash
# install-prod: real production install (tiny catalog, pinned clones, real FXServer download) against
# an existing MariaDB (--db existing), then kora delete removes everything.
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"
D="$SMOKE_ROOT/srv"
export CFX_LICENSE_KEY="$FAKE_KEY"
export KORA_DB_ADMIN_PASSWORD=root
NAME=smoke
sql() { docker exec -e MYSQL_PWD=root kora-smoke-db mariadb -h127.0.0.1 -uroot -N -B -e "$1"; }
jget() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }

step "1. kora install (plain clones of released versions, NEW flags, --db existing)"
kora install --source clone --versions released --name $NAME --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db existing --db-admin-user root -y
assert_eq "install exits 0" 0 "$CODE"
assert_not_contains "licence key never printed" "$OUT" "$FAKE_KEY"
assert_contains "finished" "$OUT" "Install finished."

step "2. folder tree"
for lib in kora_lib kora_sql kora_core; do
  assert "resource $lib cloned under resources/[pre]" test -d "$D/resources/[pre]/$lib/.git"
done
assert "kora_theme (dependency of kora_lib) present" test -d "$D/resources/[standalone]/kora_theme"
assert "cfx-default dependency present" test -d "$D/resources/[cfx-default]"
assert "FXServer run.sh extracted" bash -c "find '$D/artifacts' -name run.sh | grep -q ."
for f in server.cfg permissions.cfg secrets.cfg database.json kora-server.json .kora/created.json txData/default/config.json; do
  assert "$f exists" test -e "$D/$f"
done
# pinned clones: HEAD is the commit from the catalog
want="$(python3 -I -c 'import json,sys; c=json.load(open(sys.argv[1])); print([r for r in c["resources"] if r["id"]=="kora-core"][0]["versions"][0]["commit"])' "$KORA_CATALOG")"
have="$(git -C "$D/resources/[pre]/kora_core" rev-parse HEAD)"
assert_eq "kora_core is pinned to the catalog commit" "$want" "$have"

step "3. server.cfg and secrets.cfg"
CFG="$(cat "$D/server.cfg")"; SEC="$(cat "$D/secrets.cfg")"
assert_contains "server.cfg execs secrets.cfg" "$CFG" "exec secrets.cfg"
assert_contains "server.cfg uses the game port" "$CFG" "endpoint_add_tcp \"0.0.0.0:$GAME_PORT\""
assert_contains "server.cfg ensures the [pre] folder (folders, not single resources)" "$CFG" "ensure [pre]"
assert_not_contains "server.cfg holds no licence key" "$CFG" "$FAKE_KEY"
assert_contains "secrets.cfg holds the licence key" "$SEC" "$FAKE_KEY"
assert_contains "secrets.cfg holds the oxmysql connection string" "$SEC" "mysql_connection_string"
assert_eq "secrets.cfg is 0600" 600 "$(perms_of "$D/secrets.cfg")"
assert_eq "database.json is 0600" 600 "$(perms_of "$D/database.json")"
leak="$(grep -rlF --exclude-dir=resources --exclude-dir=artifacts --exclude=secrets.cfg "$FAKE_KEY" "$D" || true)"
assert_eq "the licence key is written nowhere else" "" "$leak"

step "4. database: new db + user, grants only on that db"
DB="$(jget "$D/database.json" database)"; DBUSER="$(jget "$D/database.json" user)"; DBPW="$(jget "$D/database.json" password)"
assert_matches "db name is kora_*" "$DB" '^kora_'
assert_matches "user name is kora_*" "$DBUSER" '^kora_'
assert_eq "database exists" 1 "$(sql "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='$DB'")"
assert_eq "user exists" 1 "$(sql "SELECT COUNT(*) FROM mysql.user WHERE user='$DBUSER'")"
GRANTS="$(sql "SHOW GRANTS FOR '$DBUSER'@'127.0.0.1'")"; echo "$GRANTS"
assert_contains "grant on the new database" "$GRANTS" "\`$DB\`.*"
global="$(grep -E ' ON \*\.\* ' <<<"$GRANTS" | grep -vE '^GRANT USAGE ON' || true)"
assert_eq "no global privileges beyond USAGE" "" "$global"
other="$(grep -vE '^GRANT USAGE ON |^GRANT .* ON `'"$DB"'`\.\* TO ' <<<"$GRANTS" || true)"
assert_eq "no grant on any other schema" "" "$other"
assert_eq "the generated login works" 1 "$(docker exec -e MYSQL_PWD="$DBPW" kora-smoke-db mariadb -h127.0.0.1 -u"$DBUSER" -N -B -e 'SELECT 1' "$DB")"

step "5. registry and status"
assert "registry lists the server" registry_has $NAME
kora status
assert_eq "status exits 0" 0 "$CODE"
assert_matches "status shows the server as stopped" "$OUT" "$NAME[[:space:]]+clone, released[[:space:]]+$GAME_PORT[[:space:]]+$TX_PORT[[:space:]]+stopped"

step "6. running the same install again resumes cleanly"
kora install --mode prod --name $NAME --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db existing --db-admin-user root -y
assert_eq "second install exits 0" 0 "$CODE"
assert_eq "still the same database" "$DB" "$(jget "$D/database.json" database)"

step "7. kora delete removes db, user, folder, registry entry"
kora delete $NAME -y
assert_eq "delete exits 0" 0 "$CODE"
assert_eq "database and user are gone" 0 "$(sql "SELECT (SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='$DB') + (SELECT COUNT(*) FROM mysql.user WHERE user='$DBUSER')")"
assert "server folder is gone" test ! -e "$D"
assert "registry entry is gone" registry_lacks "$NAME"
assert_eq "the database server itself is untouched" 1 "$(sql 'SELECT 1')"

echo; echo "install-prod: $PASS assertions passed"
