#!/usr/bin/env bash
# Point the Homebrew cask at a published release.
#
# This exists because the checksum has been published empty twice, by hand,
# and an empty one is the worst possible failure: `brew install` refuses
# before it downloads anything, so the cask is broken for everybody while
# looking entirely plausible in the diff.
#
# Two rules, enforced rather than remembered:
#
#   The hash is read through `gh`, not from the release download URL. That URL
#   is behind a CDN which happily serves the copy from before the disk image
#   was added, and an empty result read from a stale file is exactly how this
#   went wrong the second time.
#
#   Nothing is committed unless the hash is 64 hex characters.
#
#   ./update-cask.sh 1.1.6
set -euo pipefail

VERSION="${1:?usage: update-cask.sh <version>}"
REPO="nicglazkov/gcloud-dot"
TAP="${TAP_DIR:-$HOME/Developer/homebrew-tap}"
CASK="$TAP/Casks/gcloud-dot.rb"
DMG="GCloud-Dot-$VERSION.dmg"

[ -f "$CASK" ] || { echo "no cask at $CASK" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh release download "v$VERSION" --repo "$REPO" --pattern SHA256SUMS.txt --dir "$tmp" --clobber

hash="$(awk -v f="$DMG" '{ n = $2; sub("^\\./", "", n); if (n == f) print $1 }' "$tmp/SHA256SUMS.txt")"

if [ "${#hash}" -ne 64 ]; then
  echo "refusing to touch the cask: no 64 character checksum for $DMG in v$VERSION" >&2
  echo "found: '${hash}' (${#hash} characters)" >&2
  echo "if the disk image was only just uploaded, run make publish-macos first" >&2
  exit 1
fi

cd "$TAP"
git pull --rebase --quiet
/usr/bin/sed -i '' -E "s/version \"[^\"]*\"/version \"$VERSION\"/" "$CASK"
/usr/bin/sed -i '' -E "s/sha256 \"[^\"]*\"/sha256 \"$hash\"/" "$CASK"

# Read it back rather than trusting the edit.
got="$(sed -nE 's/.*sha256 "([0-9a-f]*)".*/\1/p' "$CASK" | head -1)"
[ "$got" = "$hash" ] || { echo "the cask did not take the checksum" >&2; git checkout -- "$CASK"; exit 1; }

brew style --fix "$CASK" >/dev/null
git add "$CASK"
git commit --quiet -m "gcloud-dot $VERSION"
git push --quiet
echo "cask now at $VERSION with $hash"
