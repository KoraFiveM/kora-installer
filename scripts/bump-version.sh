#!/bin/sh
# Set the workspace version for a release.   Usage: scripts/bump-version.sh 0.2.0
# Refuses a dirty tree, edits [workspace.package] version in Cargo.toml, refreshes Cargo.lock
# (offline) and prints the commands to finish the release. It never commits, tags or pushes.
set -eu
die() { printf 'bump-version: %s\n' "$*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: $0 <x.y.z>   (e.g. 0.2.0 or 0.2.0-rc1)"
VER="$1"
printf '%s\n' "$VER" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || die "'$VER' is not a version like 0.2.0"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[ -f Cargo.toml ] || die "no Cargo.toml in $ROOT"
git rev-parse --git-dir >/dev/null 2>&1 || die "not a git checkout"
[ -z "$(git status --porcelain)" ] || die "the working tree is not clean; commit or stash first."

OLD="$(awk '/^\[workspace\.package\]/ {s=1; next} /^\[/ {s=0} s && /^version *=/ {gsub(/.*= *"|".*/, ""); print; exit}' Cargo.toml)"
[ -n "$OLD" ] || die "no version in [workspace.package]"
[ "$OLD" != "$VER" ] || die "version is already $VER"

awk -v v="$VER" '/^\[workspace\.package\]/ {s=1; print; next} /^\[/ {s=0}
  s && !done && /^version *=/ {print "version = \"" v "\""; done=1; next} {print}' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml

# Refresh only the workspace members in Cargo.lock; no network.
cargo update -w --offline >/dev/null 2>&1 || cargo metadata --offline --format-version 1 >/dev/null 2>&1 \
  || die "could not refresh Cargo.lock; run 'cargo update -w --offline' yourself"

printf 'Version %s -> %s\n' "$OLD" "$VER"
git --no-pager diff --stat
cat <<NEXT

Next:
  git commit -am "release: v$VER"
  git push origin main
  # rehearse (optional): gh workflow run release.yml -f publish=false
  git tag v$VER && git push origin v$VER      # CI builds a DRAFT release with RELEASE_NOTES.md
  # review the draft on GitHub, then publish it
NEXT
