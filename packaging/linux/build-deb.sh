#!/usr/bin/env bash
# Builds a .deb from binaries already produced by cargo.
#
# Assembled by hand rather than with cargo-deb, because the package needs two
# binaries, a desktop entry, and five icon sizes, and describing that to a
# generator is longer than writing it out.
set -euo pipefail

VERSION="${1:?usage: build-deb.sh <version> <arch> <target-dir>}"
ARCH="${2:?}"
TARGET_DIR="${3:?}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

PKGDIR="$STAGE/gcloud-dot_${VERSION}_${ARCH}"
mkdir -p "$PKGDIR/DEBIAN" \
         "$PKGDIR/usr/bin" \
         "$PKGDIR/usr/share/applications" \
         "$PKGDIR/usr/share/doc/gcloud-dot"

install -m 0755 "$TARGET_DIR/gcloud-dot" "$PKGDIR/usr/bin/gcloud-dot"
install -m 0755 "$TARGET_DIR/gcloud-dot-tray" "$PKGDIR/usr/bin/gcloud-dot-tray"
install -m 0644 "$ROOT/packaging/linux/gcloud-dot.desktop" \
                "$PKGDIR/usr/share/applications/gcloud-dot.desktop"
install -m 0644 "$ROOT/LICENSE" "$PKGDIR/usr/share/doc/gcloud-dot/copyright"

for size in 16 32 48 128 256; do
  dir="$PKGDIR/usr/share/icons/hicolor/${size}x${size}/apps"
  mkdir -p "$dir"
  if command -v convert >/dev/null 2>&1; then
    convert "$ROOT/site/img/appicon.png" -resize "${size}x${size}" "$dir/gcloud-dot.png"
  else
    cp "$ROOT/site/img/appicon.png" "$dir/gcloud-dot.png"
  fi
done

# libayatana-appindicator is a Recommends rather than a Depends on purpose:
# without it the tray cannot appear, but `gcloud-dot` the command still works
# perfectly, and that is the whole interface on a headless machine.
cat > "$PKGDIR/DEBIAN/control" <<EOF
Package: gcloud-dot
Version: $VERSION
Section: devel
Priority: optional
Architecture: $ARCH
Depends: libc6, libgtk-3-0, libwebkit2gtk-4.1-0, libxdo3, libdbus-1-3
Recommends: libayatana-appindicator3-1
Maintainer: Nicholas Glazkov <nic@glazkov.com>
Homepage: https://nicglazkov.github.io/gcloud-dot/
Description: Shows how long your gcloud auth session has left
 GCloud Dot puts a dot in the system tray that reports whether your
 gcloud credentials are still valid and, when they are, roughly how long
 they have left.
 .
 Whether the session is alive is measured by running gcloud. How long it
 has left is predicted from sessions already observed ending, and the
 interface always says which of the two it is showing.
EOF

dpkg-deb --build --root-owner-group "$PKGDIR" >/dev/null
mv "$PKGDIR.deb" "$ROOT/gcloud-dot_${VERSION}_${ARCH}.deb"
echo "wrote gcloud-dot_${VERSION}_${ARCH}.deb"
