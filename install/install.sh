#!/usr/bin/env sh
# GCloud Dot installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/nicglazkov/gcloud-dot/main/install/install.sh | sh
#
# Flags:
#   --no-gui     install only the `gcloud-dot` command (for servers)
#   --prefix DIR install somewhere other than ~/.local/bin
#   --version V  install a specific release instead of the latest
#
# POSIX sh, not bash: this is piped into whatever /bin/sh is on a machine
# nobody has looked at, which on Debian is dash.
set -eu

REPO="nicglazkov/gcloud-dot"
PREFIX="${HOME}/.local/bin"
WANT_GUI=1
VERSION=""

while [ $# -gt 0 ]; do
  case "$1" in
    --no-gui) WANT_GUI=0 ;;
    --prefix) PREFIX="${2:?--prefix needs a directory}"; shift ;;
    --version) VERSION="${2:?--version needs a tag}"; shift ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '%s\n' "$*"; }
die() { printf '\n  %s\n\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required."
command -v tar  >/dev/null 2>&1 || die "tar is required."

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) platform="macos-universal" ;;
  Linux)
    case "$arch" in
      x86_64|amd64)  platform="linux-x86_64" ;;
      aarch64|arm64) platform="linux-aarch64" ;;
      *) die "Unsupported architecture: $arch" ;;
    esac
    ;;
  *) die "Unsupported system: $os. Windows users want install.ps1." ;;
esac

if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$VERSION" ] || die "Could not work out the latest version. Pass --version."
fi

asset="gcloud-dot-${platform}.tar.gz"
url="https://github.com/$REPO/releases/download/${VERSION}/${asset}"

say ""
say "GCloud Dot ${VERSION}  (${platform})"
say ""

tmp="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$tmp'" EXIT INT TERM

say "  downloading"
curl -fsSL -o "$tmp/pkg.tar.gz" "$url" || die "Download failed: $url"
tar -xzf "$tmp/pkg.tar.gz" -C "$tmp"

mkdir -p "$PREFIX"

# Stop a running tray before replacing anything. Upgrading in place otherwise
# leaves the old process running the old code, and it would keep the countdown
# it had rather than pick up whatever this install fixes.
if [ -x "$PREFIX/gcloud-dot" ]; then
  "$PREFIX/gcloud-dot" quit >/dev/null 2>&1 || true
fi
pkill -f "$PREFIX/gcloud-dot-tray" 2>/dev/null || true

# Stage beside the target, then rename over it.
#
# `install` and `cp` write into the existing file. Doing that to a running
# executable fails outright on Linux with ETXTBSY, and on macOS corrupts the
# process that is running it, because the pages it is still executing are
# rewritten underneath it. A rename leaves the old inode alone, so anything
# still running keeps working until it exits.
place() {
  cp "$1" "$2.new"
  chmod 0755 "$2.new"
  mv -f "$2.new" "$2"
}

place "$tmp/gcloud-dot" "$PREFIX/gcloud-dot"
say "  installed $PREFIX/gcloud-dot"

if [ "$WANT_GUI" -eq 1 ] && [ -f "$tmp/gcloud-dot-tray" ]; then
  place "$tmp/gcloud-dot-tray" "$PREFIX/gcloud-dot-tray"
  say "  installed $PREFIX/gcloud-dot-tray"
fi

case ":${PATH}:" in
  *":${PREFIX}:"*) ;;
  *)
    say ""
    say "  ! $PREFIX is not on your PATH. Add this to your shell profile:"
    say "        export PATH=\"$PREFIX:\$PATH\""
    ;;
esac

say ""
say "  Run 'gcloud-dot' for a one-off check."
if [ "$WANT_GUI" -eq 1 ]; then
  say "  Run 'gcloud-dot-tray' to put the dot in your menu bar or tray."
  if [ "$os" = "Darwin" ]; then
    say ""
    # A bare binary has no bundle identifier, and macOS attributes a
    # notification to a bundle. The dot and the menu work either way.
    say "  On macOS the DMG is the better route for the app itself: notifications"
    say "  need a bundle identifier, which only the packaged app has."
    say "  https://github.com/$REPO/releases/latest"
  fi
  if [ "$os" = "Linux" ]; then
    say ""
    say "  On GNOME the tray needs the AppIndicator extension; without it the"
    say "  dot has nowhere to appear. Everything else works regardless:"
    say "  https://extensions.gnome.org/extension/615/appindicator-support/"
  fi
fi
say ""
