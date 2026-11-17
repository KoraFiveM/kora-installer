#!/bin/sh
# Build the `kora-desktop` .deb (the tray icon and window; docs/desktop-app.md) from an already built kora-desktop.
#
#   packaging/linux/build-desktop-deb.sh <kora-desktop-binary> <version> <output.deb>
#
# A SEPARATE package on purpose: servers without a screen need none of the GTK / WebKit libraries. It depends on the `kora`
# package of the same version (the app finds `kora` next to itself: /usr/bin/kora is a link to the protected program).
# No system-wide autostart: the app writes a per-user one at its first run and removes it with `--uninstall`.
set -eu
[ "$#" -eq 3 ] || { echo "usage: $0 <kora-desktop-binary> <version> <output.deb>" >&2; exit 2; }
BIN="$1"; VERSION="${2#v}"; OUT="$3"
DEBVER="$(printf '%s' "$VERSION" | sed 's/-/~/')"
W="$(mktemp -d)"; chmod 0755 "$W"; trap 'rm -rf "$W"' EXIT
install -d -m 0755 "$W/usr/bin" "$W/usr/share/applications" "$W/DEBIAN"
install -m 0755 "$BIN" "$W/usr/bin/kora-desktop"
cat > "$W/usr/share/applications/kora-desktop.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Kora
Comment=Your FiveM servers in a window, with a tray icon
Exec=/usr/bin/kora-desktop
Icon=kora
Terminal=false
Categories=Network;Utility;
DESKTOP
chmod 0644 "$W/usr/share/applications/kora-desktop.desktop"
SIZE="$(du -sk "$W/usr" | cut -f1)"
cat > "$W/DEBIAN/control" <<CTRL
Package: kora-desktop
Version: $DEBVER
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Kora <noreply@users.noreply.github.com>
Installed-Size: $SIZE
Depends: kora (>= $DEBVER), libwebkit2gtk-4.1-0, libgtk-3-0
Recommends: libayatana-appindicator3-1
Suggests: xdg-desktop-portal
Homepage: https://github.com/KoraFiveM/kora-installer
Description: Kora desktop app (tray icon and window)
 A tray icon that shows whether your FiveM servers run, a small native window that shows Kora Panel, and desktop
 notifications. It is a client of Kora's panel; the panel and the \`kora\` program never depend on it.
CTRL
cat > "$W/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
case "$1" in
  purge)
    # Per-user start-up entries the app wrote (marked X-Kora-Desktop=1) point at a program that is gone.
    for f in /home/*/.config/autostart/kora-desktop.desktop /root/.config/autostart/kora-desktop.desktop; do
      [ -f "$f" ] && grep -q '^X-Kora-Desktop=1' "$f" && rm -f "$f"
    done
    ;;
esac
exit 0
POSTRM
chmod 0755 "$W/DEBIAN/postrm"
dpkg-deb --root-owner-group --build "$W" "$OUT" >/dev/null
echo "built $OUT ($(du -h "$OUT" | cut -f1))"
