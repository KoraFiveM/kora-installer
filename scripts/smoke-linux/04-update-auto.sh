#!/usr/bin/env bash
# update-auto: `kora update --auto` on a development-mode server whose forks and upstreams are local bare
# git repos (no GitHub). Scenarios: up to date, fast-forward, diverged + dirty skipped, health failure
# rolled back, and the daily timer in the install plan.
. "$(dirname "$0")/common.sh"
rm -rf "$SMOKE_ROOT"; mkdir -p "$SMOKE_ROOT"
export GIT_AUTHOR_NAME=smoke GIT_AUTHOR_EMAIL=smoke@example.invalid GIT_COMMITTER_NAME=smoke GIT_COMMITTER_EMAIL=smoke@example.invalid
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_TERMINAL_PROMPT=0
R="$SMOKE_ROOT/git"; D="$SMOKE_ROOT/dev"; NAME=devsmoke
g() { git -c init.defaultBranch=dev -c protocol.file.allow=always "$@"; }

# id | path under resources/
RES=("kora-hud|[kora]/kora_hud" "kora-chat|[kora]/kora_chat" "kora-map|[kora]/kora_map" "kora-ui|[kora]/kora_ui" "kora-core|[pre]/kora_core")
rid() { echo "${1%%|*}"; }
rpath() { echo "$D/resources/${1#*|}"; }
fork() { local e; for e in "${RES[@]}"; do if [ "$(rid "$e")" = "$1" ]; then rpath "$e"; fi; done; }
head_of() { git -C "$(fork "$1")" rev-parse HEAD; }
up_head() { git --git-dir="$R/upstream/$1.git" rev-parse dev; }
upstream_commit() { # upstream_commit id  (a new commit on the upstream's dev)
  local w="$R/work/$1"; echo "$RANDOM$RANDOM" >> "$w/file.txt"; g -C "$w" add -A; g -C "$w" commit -qm "upstream change"; g -C "$w" push -q origin dev
}

step "0. build local upstream + fork repos and a development-mode server by hand"
mkdir -p "$R/upstream" "$R/origin" "$R/work" "$D/.kora" "$KORA_HOME"
for e in "${RES[@]}"; do
  id="$(rid "$e")"
  g init -q --bare -b dev "$R/upstream/$id.git"
  g clone -q "$R/upstream/$id.git" "$R/work/$id" 2>/dev/null
  echo "$id" > "$R/work/$id/file.txt"; g -C "$R/work/$id" add -A; g -C "$R/work/$id" commit -qm initial; g -C "$R/work/$id" push -q origin dev
  g clone -q --bare "$R/upstream/$id.git" "$R/origin/$id.git"             # the "fork on GitHub"
  mkdir -p "$(dirname "$(rpath "$e")")"
  g clone -q "$R/origin/$id.git" "$(rpath "$e")"
  g -C "$(rpath "$e")" remote add upstream "$R/upstream/$id.git"
  g -C "$(rpath "$e")" fetch -q upstream
done
python3 -I - "$D" "$NAME" "$KORA_HOME" "${RES[@]}" <<'PY'
import json, sys, time
d, name, home, *res = sys.argv[1:]
rs = []
for e in res:
    i, p = e.split("|")
    rs.append({"id": i, "path": p, "repository": "https://example.invalid/" + i + ".git", "branch": "dev"})
sf = {"name": name, "mode": "development", "dir": d, "ports": {"game": 31120, "txadmin": 41120}, "game_build": 0,
      "artifact_build": 0, "database": None, "github_owner": "smoke-owner", "created_unix": int(time.time()),
      "kora_version": "0.0.0", "resources": rs}
json.dump(sf, open(d + "/kora-server.json", "w"), indent=2)
json.dump({"servers": [{"name": name, "dir": d, "mode": "development", "ports": {"game": 31120, "txadmin": 41120},
                        "created_unix": int(time.time())}]}, open(home + "/servers.json", "w"), indent=2)
PY
kora status
assert_matches "the hand-made server is registered as dev" "$OUT" "$NAME[[:space:]]+dev[[:space:]]+31120"

step "1. everything up to date: nothing to do, exit 0"
kora update --auto $NAME
assert_eq "exit 0" 0 "$CODE"
assert_contains "summary: nothing updated" "$OUT" "0 updated"
assert_contains "summary: nothing rolled back" "$OUT" "0 rolled back"
assert "update.log written" test -s "$D/update.log"

step "2. upstream moved on a clean fork: fast-forwarded"
before="$(head_of kora-chat)"; hud_before="$(head_of kora-hud)"
upstream_commit kora-chat
kora update --auto $NAME
assert_eq "exit 0" 0 "$CODE"
assert_contains "summary: 1 updated" "$OUT" "1 updated"
assert_contains "the chat resource was updated" "$OUT" "updated kora-chat"
assert_eq "chat fork now equals upstream" "$(up_head kora-chat)" "$(head_of kora-chat)"
assert "chat fork moved" test "$before" != "$(head_of kora-chat)"
assert_eq "the unchanged hud fork was left alone" "$hud_before" "$(head_of kora-hud)"
assert "an update snapshot was recorded for rollback" test -s "$D/snapshots/last-update.json"

step "3. diverged and dirty forks are skipped, exit 3 (needs attention)"
# map: local commit on the fork AND a new upstream commit = diverged
fm="$(fork kora-map)"; echo local >> "$fm/file.txt"; g -C "$fm" commit -qam "local work"
upstream_commit kora-map
map_before="$(head_of kora-map)"
# ui: uncommitted edit AND a new upstream commit = dirty
fu="$(fork kora-ui)"; echo "my uncommitted notes" > "$fu/notes.txt"; echo edit >> "$fu/file.txt"
upstream_commit kora-ui
ui_before="$(head_of kora-ui)"; ui_file="$(cat "$fu/file.txt")"
kora update --auto $NAME
assert_eq "exit code 3 (needs attention, same as the manual flow)" 3 "$CODE"
assert "update-status.json written for the manager" test -s "$D/.kora/update-status.json"
assert_eq "status: two forks need attention" 2 "$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["attention"])' "$D/.kora/update-status.json")"
assert_matches "status: checked_at is RFC3339 UTC" "$(cat "$D/.kora/update-status.json")" '"checked_at": "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z"'
assert_matches "diverged fork is reported as skipped" "$OUT" "skipped kora-map: diverged"
assert_matches "dirty fork is reported as skipped" "$OUT" "skipped kora-ui: uncommitted"
assert_eq "diverged fork's HEAD untouched" "$map_before" "$(head_of kora-map)"
assert_eq "dirty fork's HEAD untouched" "$ui_before" "$(head_of kora-ui)"
assert_eq "dirty fork's file content untouched" "$ui_file" "$(cat "$fu/file.txt")"
assert "untracked file kept" test -s "$fu/notes.txt"
assert "the forks stayed on branch dev" test "$(git -C "$fm" rev-parse --abbrev-ref HEAD)" = dev
# clean the skipped forks up again so the next scenario starts with one candidate
g -C "$fm" reset -q --hard "$(git -C "$fm" rev-parse upstream/dev)"; g -C "$fu" checkout -q -- . ; rm -f "$fu/notes.txt"
g -C "$fu" merge -q --ff-only upstream/dev

step "4. unhealthy after update: automatic rollback (KORA_TEST_FAIL_HEALTH)"
core_before="$(head_of kora-core)"
upstream_commit kora-core
KORA_TEST_FAIL_HEALTH=1 kora update --auto $NAME
assert_eq "exit code 1 (rolled back)" 1 "$CODE"
assert_matches "summary says rolled back" "$OUT" "1 rolled back"
assert_contains "names the resource" "$OUT" "rolled back kora-core"
assert_eq "kora-core is back on the commit it had before" "$core_before" "$(head_of kora-core)"
assert "the update log recorded it" grep -q "rolled back" "$D/update.log"
# and without the fault the same update goes through
kora update --auto $NAME
assert_eq "healthy retry: exit 0" 0 "$CODE"
assert_eq "status: nothing left" 0 "$(python3 -I -c 'import json,sys; s=json.load(open(sys.argv[1])); print(s["attention"]+s["updates"])' "$D/.kora/update-status.json")"
assert_contains "retry updated kora-core" "$OUT" "updated kora-core"
assert_eq "kora-core now equals upstream" "$(up_head kora-core)" "$(head_of kora-core)"

step "5. production servers are refused"
python3 -I - "$D" <<'PY'
import json, sys
p = sys.argv[1] + "/kora-server.json"; s = json.load(open(p)); s["mode"] = "production"; json.dump(s, open(p, "w"))
PY
kora update --auto $NAME
assert_eq "exit 1 (failure)" 1 "$CODE"
assert_contains "explains development only" "$OUT" "development"
python3 -I - "$D" <<'PY'
import json, sys
p = sys.argv[1] + "/kora-server.json"; s = json.load(open(p)); s["mode"] = "development"; json.dump(s, open(p, "w"))
PY

step "6. the install plan schedules the daily update timer (dev + --service)"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --mode dev --github-owner smoke-owner --name timer --dir "$SMOKE_ROOT/timer" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --service --dry-run -y
assert_eq "dry run exits 0" 0 "$CODE"
assert_matches "plan has the daily 04:00 update step" "$OUT" "Schedule daily automatic updates \(04:00\)"
assert "dry run created nothing" test ! -e "$SMOKE_ROOT/timer"

echo; echo "update-auto: $PASS assertions passed"
