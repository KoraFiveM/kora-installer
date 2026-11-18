#!/bin/sh
# Build the Kora .deb (Debian / Ubuntu) from an already built `kora` program.
#
#   packaging/linux/build-deb.sh <kora-binary> <version> <output.deb>
#
# Layout (the same one `kora service install` makes on other distributions):
#   /usr/lib/kora/kora                 the program, root-owned (the service runs ONLY this file)
#   /usr/bin/kora                      a link to it
#   /lib/systemd/system/kora.service   written by `kora service unit` (one source of truth)
# The maintainer scripts enrol the installing account, start the service and, when removing with
# purge, take away everything the service kept (/etc/kora, /var/lib/kora, /var/log/kora).
# Built in CI with dpkg-deb (already on the runner): no extra tool is downloaded.
set -eu
[ "$#" -eq 3 ] || { echo "usage: $0 <kora-binary> <version> <output.deb>" >&2; exit 2; }
BIN="$1"; VERSION="${2#v}"; OUT="$3"
here="$(cd "$(dirname "$0")" && pwd)"
# Debian orders 0.8.0-rc1 AFTER 0.8.0; a tilde sorts before: 0.8.0~rc1.
DEBVER="$(printf '%s' "$VERSION" | sed 's/-/~/')"
W="$(mktemp -d)"; chmod 0755 "$W"; trap 'rm -rf "$W"' EXIT
install -d -m 0755 "$W/usr/lib/kora" "$W/usr/bin" "$W/lib/systemd/system" "$W/DEBIAN"
install -m 0755 "$BIN" "$W/usr/lib/kora/kora"
ln -s /usr/lib/kora/kora "$W/usr/bin/kora"
"$BIN" service unit > "$W/lib/systemd/system/kora.service"
chmod 0644 "$W/lib/systemd/system/kora.service"
SIZE="$(du -sk "$W/usr" | cut -f1)"
cat > "$W/DEBIAN/control" <<CTRL
Package: kora
Version: $DEBVER
Section: admin
Priority: optional
Architecture: amd64
Maintainer: Kora <noreply@users.noreply.github.com>
Installed-Size: $SIZE
Depends: libc6
Recommends: git
Homepage: https://github.com/KoraFiveM/kora-installer
Description: Installer and manager for FiveM servers (with Kora's administrator service)
 Kora installs and manages FiveM servers through a local web panel. This package puts the
 program in a folder only administrators can change and runs a small administrator service,
 so installing a server (service, firewall rule, database) needs no password after this install.
 Only the accounts an administrator allows (kora service allow) can use it.
CTRL
for f in postinst prerm postrm; do
  install -m 0755 "$here/deb/$f" "$W/DEBIAN/$f"
done
dpkg-deb --root-owner-group --build "$W" "$OUT" >/dev/null
echo "built $OUT ($(du -h "$OUT" | cut -f1))"
