#!/usr/bin/env bash
# Kora's administrator service on a real systemd machine: build the .deb, install it, use the service
# from NON-root accounts (one allowed, one not), check the ownership rules, then remove and purge and
# check nothing is left. Needs sudo, systemd and python3. Diagnostic-rich on purpose: every step prints
# what it saw, and a failure prints the service's journal.
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A=kora-smoke-a   # allowed
B=kora-smoke-b   # on the machine, never allowed
C=kora-smoke-c   # allowed, but not the owner of A's server
D=kora-smoke-d   # used the OLD helper (v0.7.0), so it stays allowed after the upgrade
WORK="$(mktemp -d)"
DEB="$WORK/kora.deb"
# The other accounts cannot read the runner's home (the repository is in it): give them a copy of the client to run.
PUB="$(mktemp -d)"; chmod 755 "$PUB"; install -m 0644 "$HERE/svc_client.py" "$PUB/svc_client.py"
CLIENT="$PUB/svc_client.py"

diag() {
  echo "----- diagnostics -----"
  systemctl status kora.service --no-pager 2>&1 | tail -n 20 || true
  sudo journalctl -u kora --no-pager -n 60 2>&1 || true
  ls -la /usr/lib/kora /etc/kora /var/lib/kora /var/log/kora /run/kora 2>&1 || true
  sudo cat /etc/kora/service.json /var/lib/kora/owners.json 2>&1 || true
  sudo tail -n 30 /var/log/kora/audit.log 2>&1 || true
}
trap 'rc=$?; [ $rc -ne 0 ] && diag; sudo userdel -r "$A" 2>/dev/null; sudo userdel -r "$B" 2>/dev/null; sudo userdel -r "$C" 2>/dev/null; sudo userdel -r "$D" 2>/dev/null; sudo userdel kora-old 2>/dev/null; sudo systemctl disable --now kora-helper.service 2>/dev/null; sudo rm -f /etc/systemd/system/kora-helper.service /etc/systemd/system/kora-old.service; rm -rf "$WORK"; exit $rc' EXIT

as_user() { local u="$1"; shift; sudo -u "$u" env -i HOME="/home/$u" PATH=/usr/bin:/bin "$@"; }
# svc USER VERB ARGS...: ask the service as USER; sets OUT and CODE (never aborts)
svc() { local u="$1"; shift; set +e; OUT="$(as_user "$u" python3 -I "$CLIENT" "$@" 2>&1)"; CODE=$?; set -e; printf -- '--- as %s: %s (exit %s) ---\n%s\n' "$u" "$*" "$CODE" "$OUT"; }

step "accounts and the package"
for u in $A $B $C $D; do sudo useradd -m -s /bin/bash "$u"; done
if [ -n "${KORA_DEB:-}" ]; then
  # the package a release shipped (release.yml runs this against its own artifact)
  cp "$KORA_DEB" "$DEB"
else
  [ -x "$KORA_BIN" ] || fail "no kora binary at $KORA_BIN (cargo build --release -p kora-cli first)"
  "$REPO_ROOT/packaging/linux/build-deb.sh" "$KORA_BIN" "0.0.0-smoke" "$DEB"
fi
assert "dpkg accepts the package" dpkg-deb -I "$DEB"
step "a v0.7.0 layout on the machine first: the old helper, and a server installed the old way (root-run, folder owned by its own account)"
sudo mkdir -p /etc/kora /usr/local/libexec/kora
sudo tee /etc/kora/helper.json >/dev/null <<JSON
{"uid":$(id -u $D),"gid":$(id -g $D),"user":"$D","home":"/home/$D","version":"0.7.0"}
JSON
echo '#!/bin/sh' | sudo tee /usr/local/libexec/kora/kora >/dev/null; sudo chmod 755 /usr/local/libexec/kora/kora
sudo tee /etc/systemd/system/kora-helper.service >/dev/null <<'UNIT'
[Unit]
Description=Kora privileged helper (the old one)
[Service]
ExecStart=/bin/sleep 100000
[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload && sudo systemctl enable --now kora-helper.service
assert "the old helper runs" systemctl is-active --quiet kora-helper.service
sudo useradd -r -M -s /usr/sbin/nologin kora-old
OLD="/home/$A/kora/old"
sudo -u $A mkdir -p "$OLD/.kora" "/home/$A/.kora"
sudo -u $A tee "$OLD/.kora/created.json" >/dev/null <<'JSON'
{"created_dir":true,"resources":[],"artifacts":false,"files":[],"service_unit":"/etc/systemd/system/kora-old.service","update_timer":false,"firewall":false,"registered":true,"service_account":"kora-old","service_account_created":true}
JSON
sudo -u $A tee "/home/$A/.kora/servers.json" >/dev/null <<JSON
{"servers":[{"name":"old","dir":"$OLD","mode":"production","ports":{"game":30121,"txadmin":40121},"created_unix":1}]}
JSON
sudo tee /etc/systemd/system/kora-old.service >/dev/null <<'UNIT'
[Unit]
Description=an old Kora server service
[Service]
ExecStart=/bin/true
UNIT
sudo chown -R kora-old:kora-old "$OLD"
sudo env KORA_SERVICE_USERS="$A,$C" dpkg -i "$DEB"

step "layout: a protected program, a root-owned socket, a settings file"
assert "the service is active" systemctl is-active --quiet kora.service
assert_eq "the program is root:root 755" "root:root 755" "$(stat -c '%U:%G %a' /usr/lib/kora/kora)"
assert_eq "the program folder is root:root 755" "root:root 755" "$(stat -c '%U:%G %a' /usr/lib/kora)"
assert "/usr/bin/kora points at the protected program" test "$(readlink /usr/bin/kora)" = /usr/lib/kora/kora
assert_eq "the socket is root-owned" "root" "$(stat -c '%U' /run/kora/service.sock)"
assert_eq "the settings are root:root 644" "root:root 644" "$(stat -c '%U:%G %a' /etc/kora/service.json)"
POLICY="$(cat /etc/kora/service.json)"
assert_contains "$A is on the list" "$POLICY" "uid:$(id -u $A)"
assert_contains "$C is on the list" "$POLICY" "uid:$(id -u $C)"
assert_not_contains "$B is NOT on the list" "$POLICY" "uid:$(id -u $B)"

step "the upgrade brought the v0.7.0 layout over without touching the server"
assert "the old helper service is gone" bash -c '! systemctl is-active --quiet kora-helper.service && [ ! -e /etc/systemd/system/kora-helper.service ]'
assert "the old helper's copy and settings are gone" bash -c '[ ! -e /usr/local/libexec/kora/kora ] && [ ! -e /etc/kora/helper.json ]'
POLICY="$(cat /etc/kora/service.json)"
assert_contains "the old helper's account ($D) is allowed in the new service" "$POLICY" "uid:$(id -u $D)"
assert_contains "the old server's service is recorded as $A's" "$(sudo cat /var/lib/kora/owners.json)" "\"service:kora-old\": \"uid:$(id -u $A)\""
assert "the old server's folder and files are untouched" test -s "$OLD/.kora/created.json"
assert_eq "...still owned by its own account" "kora-old" "$(stat -c '%U' "$OLD")"
svc $C admin-step undo --step system.service --dir "$OLD" --name old --game-port 30121 --txadmin-port 40121
assert_eq "C cannot touch the old server" "3" "$CODE"
sudo /usr/bin/kora service migrate --quiet && assert "running the migration again is harmless" test -s /var/lib/kora/owners.json

step "status as each account (no elevation anywhere)"
OUT="$(as_user $A /usr/bin/kora service status)"; echo "$OUT"
assert_contains "$A: the service serves this account" "$OUT" "serves this account: yes"
OUT="$(as_user $B /usr/bin/kora service status)"; echo "$OUT"
assert_contains "$B: running but not for this account" "$OUT" "serves this account: no"

step "$A has a server folder (made as $A) and asks the service for its firewall step"
SRV="/home/$A/kora/smoke-a"
sudo -u $A mkdir -p "$SRV/.kora"
sudo -u $A tee "$SRV/.kora/request.json" >/dev/null <<JSON
{"mode":"production","source":"clone","versions":"released","name":"smoke-a","dir":"$SRV","hostname":"","ports":{"game":31120,"txadmin":41120},"resources":"All","db":"Skip","service":false,"firewall":true,"dry_run":false}
JSON
svc $A admin-step run --step system.firewall --dir "$SRV"
assert_eq "the step ran and succeeded" "0" "$CODE"
assert_contains "the service reports success" "$OUT" "RESULT ok=true"
LEDGER="$(sudo cat /var/lib/kora/owners.json)"
assert_contains "the ledger says $A owns the firewall rules of smoke-a" "$LEDGER" "\"firewall:smoke-a\": \"uid:$(id -u $A)\""
AUDIT="$(sudo cat /var/log/kora/audit.log)"
assert_matches "the audit log has who, what and the result" "$AUDIT" "uid:$(id -u $A) \($A\) \| run system\.firewall \| done"
assert_not_contains "the audit log holds no paths" "$AUDIT" "/home/"

step "refusals: another account, the wrong folder owner, forbidden requests, forged ports"
svc $B admin-step run --step system.firewall --dir "$SRV"
assert_eq "$B is refused" "3" "$CODE"
assert_contains "the reason is plain" "$OUT" "not turned on for this account"
svc $C admin-step undo --step system.firewall --dir "$SRV" --name smoke-a --game-port 31120 --txadmin-port 41120
assert_eq "$C (allowed) cannot touch $A's server" "3" "$CODE"
assert_contains "because the folder is not theirs" "$OUT" "does not belong to you"
svc $A install --dir "$SRV"
assert_eq "install is not a service operation" "3" "$CODE"
svc $A uninstall --yes
assert_eq "uninstall is not a service operation" "3" "$CODE"
svc $A schedule set smoke-a --as-system
assert_eq "an update task as the system account is refused" "3" "$CODE"
svc $A admin-step run --step system.firewall --dir "/home/$A/kora/../../$B"
assert_eq "a path with .. is refused" "3" "$CODE"
sudo -u $A ln -s "$SRV" "/home/$A/kora/alias"
svc $A admin-step run --step system.firewall --dir "/home/$A/kora/alias"
assert_eq "a link in the folder path is refused" "3" "$CODE"
sudo -u $A sed -i 's/"game":31120/"game":3389/' "$SRV/.kora/request.json"
svc $A admin-step run --step system.firewall --dir "$SRV"
assert_eq "remote desktop port from a forged record is refused" "3" "$CODE"
assert_contains "and says why" "$OUT" "3389"

step "allow and deny take effect at once"
sudo /usr/bin/kora service deny $C
svc $C admin-step run --step system.firewall --dir "/home/$C/none"
assert_eq "$C is refused after deny" "3" "$CODE"
assert_contains "the list no longer has $C" "$(sudo /usr/bin/kora service list)" "$A"
sudo /usr/bin/kora service allow $C
assert_contains "allow puts them back" "$(sudo /usr/bin/kora service list)" "$C"

step "restart keeps the records"
sudo systemctl restart kora.service
wait_for "the service answers again" 20 test -S /run/kora/service.sock
assert_contains "the ledger survived" "$(sudo cat /var/lib/kora/owners.json)" "firewall:smoke-a"

step "remove keeps the settings; purge leaves nothing behind"
sudo dpkg -r kora
assert "the service is gone" bash -c '! systemctl is-active --quiet kora.service'
assert "the program is gone" test ! -e /usr/lib/kora/kora
assert "the unit is gone" test ! -e /lib/systemd/system/kora.service
assert "the settings stay after remove" test -s /etc/kora/service.json
sudo dpkg -P kora
assert "no settings after purge" test ! -e /etc/kora
assert "no records after purge" test ! -e /var/lib/kora
assert "no audit log after purge" test ! -e /var/log/kora
assert "no socket" test ! -e /run/kora/service.sock
assert "the server folder was never touched" test -s "$SRV/.kora/request.json"
printf '\n%s checks passed\n' "$PASS"
