#!/usr/bin/env bash
# uninstall: `kora uninstall --dry-run` and a real run keep the user's folders and databases by default;
# only --delete-files / --delete-databases remove them.
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"
D="$SMOKE_ROOT/srv"; NAME=smoke
export CFX_LICENSE_KEY="$FAKE_KEY" KORA_DB_ADMIN_PASSWORD=root
sql() { docker exec -e MYSQL_PWD=root kora-smoke-db mariadb -h127.0.0.1 -uroot -N -B -e "$1"; }
jget() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
install_it() { kora install --name $NAME --dir "$D" --game-port $GAME_PORT --txadmin-port $TX_PORT --db existing --db-admin-user root -y; }
db_rows() { sql "SELECT (SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='$DB') + (SELECT COUNT(*) FROM mysql.user WHERE user='$DBUSER')"; }

step "1. install a server with a database"
install_it
assert_eq "install exits 0" 0 "$CODE"
DB="$(jget "$D/database.json" database)"; DBUSER="$(jget "$D/database.json" user)"
assert_eq "database and user exist" 2 "$(db_rows)"

step "2. uninstall with nothing registered"
cp "$KORA_HOME/servers.json" "$SMOKE_ROOT/servers.json.bak"
echo '{"servers":[]}' > "$KORA_HOME/servers.json"
kora uninstall --dry-run
assert_eq "exit 0" 0 "$CODE"
assert_contains "says there is nothing to do" "$OUT" "Nothing to do"
cp "$SMOKE_ROOT/servers.json.bak" "$KORA_HOME/servers.json"

step "3. uninstall --dry-run changes nothing"
kora uninstall --dry-run
assert_eq "exit 0" 0 "$CODE"
assert_contains "names the server" "$OUT" "== $NAME"
assert_contains "says folder and database are kept" "$OUT" "KEEP the server folder and database"
assert_contains "says nothing was changed" "$OUT" "Dry run: nothing was changed"
assert "folder still there" test -d "$D/resources"
assert_eq "database and user still there" 2 "$(db_rows)"
assert "still registered" registry_has $NAME
kora uninstall --dry-run --delete-files --delete-databases
assert_eq "dry run with --delete-* exits 0" 0 "$CODE"
assert "dry run with --delete-*: folder still there" test -d "$D/resources"
assert_eq "dry run with --delete-*: database still there" 2 "$(db_rows)"

step "4. real uninstall --yes keeps the folder and the database"
kora uninstall --yes
assert_eq "exit 0" 0 "$CODE"
assert_contains "reports what was removed/kept" "$OUT" "Removed:"
assert "server folder kept" test -d "$D/resources"
assert "secrets.cfg kept" test -s "$D/secrets.cfg"
assert "database.json kept" test -s "$D/database.json"
assert_eq "database and user kept" 2 "$(db_rows)"
assert "server forgotten in the Kora list" bash -c "! grep -q '\"name\": \"$NAME\"' '$KORA_HOME/servers.json' 2>/dev/null"

step "5. register it again, uninstall --yes --delete-databases drops only the database"
install_it
assert_eq "re-install (resume) exits 0" 0 "$CODE"
assert "registered again" registry_has $NAME
kora uninstall --yes --delete-databases
assert_eq "exit 0" 0 "$CODE"
assert_eq "database and user are gone" 0 "$(db_rows)"
assert "the server folder is still there" test -d "$D/resources"
assert_eq "the database server itself is untouched" 1 "$(sql 'SELECT 1')"

step "6. kora delete on the leftover folder cleans it, --delete-files path"
kora delete "$D" -y
assert_eq "delete exits 0" 0 "$CODE"
assert "folder is gone" test ! -e "$D"

step "7. uninstall --yes --delete-files removes the folder"
install_it
assert_eq "install exits 0" 0 "$CODE"
DB="$(jget "$D/database.json" database)"; DBUSER="$(jget "$D/database.json" user)"
kora uninstall --yes --delete-files
assert_eq "exit 0" 0 "$CODE"
assert "folder removed" test ! -e "$D"
assert_eq "database kept (only files were asked for)" 2 "$(db_rows)"
sql "DROP DATABASE IF EXISTS \`$DB\`; DROP USER IF EXISTS '$DBUSER'@'127.0.0.1';" || true

echo; echo "uninstall: $PASS assertions passed"
