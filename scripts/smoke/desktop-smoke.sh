#!/usr/bin/env bash
# Smoke test of the Kora desktop app on Linux, under xvfb (a virtual screen). Usage: desktop-smoke.sh <folder with kora and kora-desktop>
#
# Starts `kora-desktop --smoke` against a throw-away Kora home. The app loads the panel in its window, reads the page
# title, drives one tray action (open the settings page), starts nothing in the background, and writes a report. A second
# launch must reach the first (single instance). Then the report is checked.
# A tray host is usually missing under xvfb: only the tray's MENU MODEL is asserted, not the icon.
set -euo pipefail
dist="$(cd "${1:?folder with kora and kora-desktop}" && pwd)"
work="$(mktemp -d)"
export KORA_HOME="$work/kora-home" KORA_DESKTOP_HOME="$work/desktop" HOME="$work/home" XDG_CONFIG_HOME="$work/home/.config"
mkdir -p "$KORA_HOME" "$KORA_DESKTOP_HOME" "$HOME"
export KORA_DESKTOP_SMOKE_WAIT_SECOND=1
report="$KORA_DESKTOP_HOME/report.json"

xvfb-run -a dbus-run-session -- "$dist/kora-desktop" --kora "$dist/kora" --smoke "$report" &
pid=$!

for _ in $(seq 1 120); do
  [ -f "$report.ready" ] && break
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done
if [ ! -f "$report.ready" ]; then
  echo "::error::the desktop app never showed the panel"; cat "$report" 2>/dev/null || true; kill "$pid" 2>/dev/null || true; exit 1
fi

# A second launch asks the first to show its window and exits at once, successfully.
second="$(timeout 30 "$dist/kora-desktop" --open /system; echo "exit=$?")"
echo "$second"
case "$second" in *exit=0) ;; *) echo "::error::the second launch did not exit cleanly"; exit 1 ;; esac

wait "$pid" || { echo "::error::the desktop app exited with an error"; cat "$report" 2>/dev/null || true; exit 1; }
cat "$report"

check() { jq -e "$1" "$report" >/dev/null || { echo "::error::report check failed: $1"; exit 1; }; }
check '.ok == true'
check '.window_title == "Kora"'
check '.page_title | test("Kora")'
check '.url_before_action | startswith("http://127.0.0.1:")'
check '.action_url | contains("/settings")'
check '.connected == true'
check '.menu_ids | index("quit") != null'
check '.menu_ids | index("quit-stop") != null'
check '.menu_ids | index("open:/") != null'
check '.second_launch == "/system"'
# It left no background service and no start-up entry behind.
[ ! -e "$HOME/.config/autostart/kora-desktop.desktop" ] || { echo "::error::smoke mode must not change autostart"; exit 1; }
pkill -f "kora panel --foreground" || true
echo "desktop smoke test passed"
