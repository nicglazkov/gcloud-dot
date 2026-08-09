#!/usr/bin/env bash
# Builds an AppImage for distributions the .deb does not cover.
set -euo pipefail

VERSION="${1:?usage: build-appimage.sh <version> <arch> <target-dir>}"
ARCH="${2:?}"
TARGET_DIR="${3:?}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

APPDIR="$STAGE/GCloudDot.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps"

install -m 0755 "$TARGET_DIR/gcloud-dot" "$APPDIR/usr/bin/gcloud-dot"
install -m 0755 "$TARGET_DIR/gcloud-dot-tray" "$APPDIR/usr/bin/gcloud-dot-tray"
install -m 0644 "$ROOT/packaging/linux/gcloud-dot.desktop" \
                "$APPDIR/gcloud-dot.desktop"
cp "$APPDIR/gcloud-dot.desktop" "$APPDIR/usr/share/applications/"
cp "$ROOT/site/img/appicon.png" "$APPDIR/gcloud-dot.png"
cp "$ROOT/site/img/appicon.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/gcloud-dot.png"

# AppRun forwards arguments so one AppImage serves both the tray and the CLI:
#   ./GCloud_Dot.AppImage            starts the tray
#   ./GCloud_Dot.AppImage status     runs the command
# Without this the CLI inside would be unreachable, which matters most on the
# servers where an AppImage is the only practical install.
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
if [ $# -eq 0 ]; then
  exec "$HERE/usr/bin/gcloud-dot-tray"
fi
case "$1" in
  tray) shift; exec "$HERE/usr/bin/gcloud-dot-tray" "$@" ;;
  *)    exec "$HERE/usr/bin/gcloud-dot" "$@" ;;
esac
EOF
chmod +x "$APPDIR/AppRun"

TOOL="$STAGE/appimagetool"
curl -fsSL -o "$TOOL" \
  "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x "$TOOL"

# --appimage-extract-and-run avoids needing FUSE, which GitHub runners lack.
ARCH="$ARCH" "$TOOL" --appimage-extract-and-run \
  "$APPDIR" "$ROOT/GCloud_Dot-${VERSION}-${ARCH}.AppImage" >/dev/null
echo "wrote GCloud_Dot-${VERSION}-${ARCH}.AppImage"
