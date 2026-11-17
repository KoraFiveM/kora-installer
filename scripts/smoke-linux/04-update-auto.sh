#!/usr/bin/env bash
# update-auto: `kora update --auto` on a server of own copies that follows the latest changes (an old record: mode development) whose forks and upstreams are local bare
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
# (an old-format servers.json: Kora imports it into its data file the first time it starts, so this seeds the registry)
PY
kora status
assert_matches "the hand-made server (an old record with only mode: development) reads as own copies + latest changes" "$OUT" "$NAME[[:space:]]+fork, latest[[:space:]]+31120"

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
# Automatic updates save their snapshot with the manual ones, so `kora update --rollback` can use it.
assert "an update snapshot was recorded for rollback" sh -c "ls '$D/.kora/snapshots/'*.json"
assert "no old-style snapshot file" test ! -e "$D/snapshots/last-update.json"

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
# the version that was rolled back is held: the next automatic run leaves it alone
kora update --auto $NAME
assert_eq "held retry: exit 0" 0 "$CODE"
assert_contains "the rolled-back version is held" "$OUT" "held kora-core"
assert_eq "kora-core still on the commit it had before" "$core_before" "$(head_of kora-core)"
# a newer upstream version (the fix) goes through
upstream_commit kora-core
kora update --auto $NAME
assert_eq "healthy retry: exit 0" 0 "$CODE"
assert_eq "status: nothing left" 0 "$(python3 -I -c 'import json,sys; s=json.load(open(sys.argv[1])); print(s["attention"]+s["updates"])' "$D/.kora/update-status.json")"
assert_contains "retry updated kora-core" "$OUT" "updated kora-core"
assert_eq "kora-core now equals upstream" "$(up_head kora-core)" "$(head_of kora-core)"

step "4b. a resource put on another branch is left alone by updates, and one command brings it back"
# a `feature` branch on hud's upstream, then hud is switched to it with `kora branch`
w="$R/work/kora-hud"; g -C "$w" checkout -q -b feature; echo feature > "$w/feat.txt"; g -C "$w" add -A; g -C "$w" commit -qm "feature work"
g -C "$w" push -q origin feature; g -C "$w" checkout -q dev
kora branch show $NAME kora-hud --refresh
assert_eq "show works" 0 "$CODE"
assert_contains "the branch is offered from the original repository" "$OUT" "feature"
kora branch switch $NAME kora-hud feature --from upstream
assert_eq "switch exit 0" 0 "$CODE"
assert_eq "hud is on feature" feature "$(git -C "$(fork kora-hud)" rev-parse --abbrev-ref HEAD)"
assert "the feature file is there" test -s "$(fork kora-hud)/feat.txt"
kora branch list $NAME
assert_matches "the list says which branch" "$OUT" "kora-hud +branch feature"
hud_on_feature="$(head_of kora-hud)"
upstream_commit kora-hud
kora update --auto $NAME
assert_eq "update exit 0 (a switched resource is not a problem)" 0 "$CODE"
assert_contains "the update says it left hud alone, and why" "$OUT" "on branch feature"
assert_eq "hud was not touched by the update" "$hud_on_feature" "$(head_of kora-hud)"
assert_eq "hud is still on feature" feature "$(git -C "$(fork kora-hud)" rev-parse --abbrev-ref HEAD)"
# changes of your own are never lost silently
echo "my notes" > "$(fork kora-hud)/notes.txt"
kora branch default $NAME kora-hud
assert_eq "needs a choice: exit 3" 3 "$CODE"
assert_contains "names the way out" "$OUT" "--stash"
assert_eq "nothing moved" feature "$(git -C "$(fork kora-hud)" rev-parse --abbrev-ref HEAD)"
assert "the notes are still there" test -s "$(fork kora-hud)/notes.txt"
kora branch default $NAME kora-hud --stash
assert_eq "default with --stash exit 0" 0 "$CODE"
assert_contains "says where the changes are" "$OUT" "kept aside"
assert_eq "hud is back on dev" dev "$(git -C "$(fork kora-hud)" rev-parse --abbrev-ref HEAD)"
assert "the notes were put aside, not deleted" sh -c "git -C '$(fork kora-hud)' stash list | grep -q Kora"
kora update --auto $NAME
assert_eq "update after going back: exit 0" 0 "$CODE"
assert_contains "hud updates again" "$OUT" "updated kora-hud"
assert_eq "hud now equals upstream" "$(up_head kora-hud)" "$(head_of kora-hud)"

step "5. which versions a server follows is its own setting (released or latest), and nothing moves when it changes"
kora resource versions $NAME released
assert_eq "switch to released exit 0" 0 "$CODE"
assert_contains "says nothing changed yet" "$OUT" "nothing has changed yet"
kora status
assert_matches "status says released" "$OUT" "released"
# the old `mode` word is still written next to the new fields, so an older Kora reads the file
assert_eq "the file keeps the old word" production "$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]+"/kora-server.json"))["mode"])' "$D")"
kora resource versions $NAME latest
assert_eq "switch back exit 0" 0 "$CODE"
assert_eq "the file says development again" development "$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]+"/kora-server.json"))["mode"])' "$D")"
assert_eq "hud was not touched" dev "$(git -C "$(fork kora-hud)" rev-parse --abbrev-ref HEAD)"

step "6. the install plan schedules the daily update timer (OLD spelling on purpose: --mode dev --github-owner, + --service)"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --mode dev --github-owner smoke-owner --name timer --dir "$SMOKE_ROOT/timer" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --service --dry-run -y
assert_eq "dry run exits 0" 0 "$CODE"
assert_matches "plan has the daily 04:00 update step" "$OUT" "Schedule daily automatic updates \(04:00\)"
assert "dry run created nothing" test ! -e "$SMOKE_ROOT/timer"

step "7. the same plan with the NEW spelling (--source fork --versions latest --fork-owner)"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --source fork --versions latest --fork-owner smoke-owner --name timer2 --dir "$SMOKE_ROOT/timer2" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --service --dry-run -y
assert_eq "dry run exits 0" 0 "$CODE"
assert_matches "plan has the daily 04:00 update step" "$OUT" "Schedule daily automatic updates \(04:00\)"
assert_contains "the plan says own copies" "$OUT" "your own copies"
assert "dry run created nothing" test ! -e "$SMOKE_ROOT/timer2"
CFX_LICENSE_KEY="$FAKE_KEY" kora install --source clone --versions released --name timer3 --dir "$SMOKE_ROOT/timer3" --game-port $GAME_PORT --txadmin-port $TX_PORT --db skip --service --dry-run -y
assert_eq "clone + released dry run exits 0" 0 "$CODE"
assert_contains "the plan says plain clones" "$OUT" "plain clones"

echo; echo "update-auto: $PASS assertions passed"
