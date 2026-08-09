#!/usr/bin/env sh
# Removes GCloud Dot from macOS or Linux.
#
#   sh uninstall.sh            remove the app, keep measured session lengths
#   sh uninstall.sh --purge    remove everything, including the measurements
set -eu

PURGE=0
[ "${1:-}" = "--purge" ] && PURGE=1

say() { printf '  %s\n' "$*"; }

# Stop it first, or the login item removal races a running process.
pkill -f 'gcloud-dot-tray' 2>/dev/null || true
pkill -f 'GCloud Dot.app/Contents/MacOS/GCloudDot' 2>/dev/null || true

case "$(uname -s)" in
  Darwin)
    launchctl bootout "gui/$(id -u)/com.nic.gclouddot" 2>/dev/null || true
    rm -f "$HOME/Library/LaunchAgents/com.nic.gclouddot.plist"
    rm -rf "/Applications/GCloud Dot.app" "$HOME/Applications/GCloud Dot.app"
    STATE="$HOME/Library/Application Support/GCloudDot"
    ;;
  Linux)
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/gcloud-dot.desktop"
    STATE="${XDG_DATA_HOME:-$HOME/.local/share}/gcloud-dot"
    ;;
  *) say "Unsupported system."; exit 1 ;;
esac

for dir in "$HOME/.local/bin" /usr/local/bin; do
  rm -f "$dir/gcloud-dot" "$dir/gcloud-dot-tray" 2>/dev/null || true
done

say "Removed GCloud Dot."

if [ "$PURGE" -eq 1 ]; then
  rm -rf "$STATE"
  say "Removed measured session lengths in $STATE."
else
  # Each of these took a real session's worth of wall-clock time to observe.
  say "Kept measured session lengths in $STATE (--purge removes them)."
fi
