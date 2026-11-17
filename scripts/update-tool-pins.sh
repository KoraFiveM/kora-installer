#!/usr/bin/env bash
# Print the pinned entries for crates/kora-core/src/tools/pins.rs for a newer GitHub CLI and MinGit.
#
#   scripts/update-tool-pins.sh 2.104.0 v2.56.1.windows.1
#
# The checksums come from the releases themselves (gh: its checksums.txt; MinGit: the digest GitHub publishes for
# the asset), and each file is downloaded and hashed here too: a mismatch stops the script. Review the output
# and paste it into pins.rs, then run `cargo test -p kora-core tools`. Nothing is written for you on purpose.
set -euo pipefail
GH_VERSION="${1:?usage: $0 <gh-version e.g. 2.104.0> <git-for-windows-tag e.g. v2.56.1.windows.1>}"
GIT_TAG="${2:?usage: $0 <gh-version> <git-for-windows-tag>}"
GIT_VERSION="${GIT_TAG#v}"; GIT_VERSION="${GIT_VERSION%%.windows.*}"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT

gh_base="https://github.com/cli/cli/releases/download/v${GH_VERSION}"
curl -fsSL "$gh_base/gh_${GH_VERSION}_checksums.txt" -o "$work/gh.sums"

entry() { # tool version os file url sha format strip exe
  printf '    Pin {\n        tool: Tool::%s,\n        version: "%s",\n        os: "%s",\n        arch: "x86_64",\n        file: "%s",\n        url: "%s",\n        sha256: "%s",\n        format: Format::%s,\n        strip_top: %s,\n        exe: "%s",\n        size: %s,\n    },\n' "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}
fetch_check() { # url want file -> prints size
  curl -fsSL "$1" -o "$work/$3"
  got="$(sha256sum "$work/$3" | cut -d' ' -f1)"
  [ "$got" = "$2" ] || { echo "CHECKSUM MISMATCH for $1: wanted $2, got $got" >&2; exit 1; }
  stat -c %s "$work/$3"
}

for spec in "linux_amd64.tar.gz linux TarGz true bin/gh" "windows_amd64.zip windows Zip false bin/gh.exe"; do
  set -- $spec
  file="gh_${GH_VERSION}_$1"; sha="$(awk -v f="$file" '$2==f {print $1}' "$work/gh.sums")"
  [ -n "$sha" ] || { echo "no checksum for $file" >&2; exit 1; }
  size="$(fetch_check "$gh_base/$file" "$sha" "$file")"
  entry Gh "$GH_VERSION" "$2" "$file" "$gh_base/$file" "$sha" "$3" "$4" "$5" "$size"
done

git_base="https://github.com/git-for-windows/git/releases/download/${GIT_TAG}"
file="MinGit-${GIT_VERSION}-64-bit.zip"
sha="$(curl -fsSL "https://api.github.com/repos/git-for-windows/git/releases/tags/${GIT_TAG}" | python3 -I -c "import json,sys; d=json.load(sys.stdin); print(next(a['digest'] for a in d['assets'] if a['name']=='$file').split(':',1)[1])")"
size="$(fetch_check "$git_base/$file" "$sha" "$file")"
entry Git "$GIT_VERSION" windows "$file" "$git_base/$file" "$sha" Zip false cmd/git.exe "$size"
