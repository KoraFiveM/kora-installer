#!/usr/bin/env bash
# Print the pin entries for crates/kora-core/src/container/pins.rs from Docker Hub.
#
#   scripts/update-container-pins.sh <alpine-tag> <mariadb-tag>
#   scripts/update-container-pins.sh 3.23 11.8
#
# The digest printed is the one of the multi-architecture index (what `docker pull repo:tag` resolves),
# read from the registry's own answer. Paste the lines into pins.rs, then run
#   cargo test -p kora-core container::pins
# and let the Linux smoke test (container job) confirm a server still starts on the new images.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <alpine-tag> <mariadb-tag>" >&2
  exit 2
fi

digest_of() {
  local repo=$1 tag=$2 token digest
  token=$(curl -fsS "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/${repo}:pull" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
  [ -n "$token" ] || { echo "no registry token for ${repo}" >&2; exit 1; }
  digest=$(curl -fsSI -H "Authorization: Bearer ${token}" \
    -H "Accept: application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json" \
    "https://registry-1.docker.io/v2/library/${repo}/manifests/${tag}" | tr -d '\r' | sed -n 's/^[Dd]ocker-[Cc]ontent-[Dd]igest: //p')
  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????) echo "$digest" ;;
    *) echo "unexpected digest for ${repo}:${tag}: '${digest}'" >&2; exit 1 ;;
  esac
}

alpine=$(digest_of alpine "$1")
mariadb=$(digest_of mariadb "$2")

cat <<EOF
pub const BASE: Image = Image { repository: "docker.io/library/alpine", tag: "$1", digest: "$alpine" };

pub const MARIADB: Image = Image { repository: "docker.io/library/mariadb", tag: "$2", digest: "$mariadb" };
EOF
