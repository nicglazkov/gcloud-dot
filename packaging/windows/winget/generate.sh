#!/usr/bin/env bash
# Writes the winget manifests for a released version.
#
# Generated rather than edited, because three files repeat the version and one
# carries a checksum. Hand maintaining that is how a manifest ends up pointing
# at one release with the hash of another, which winget rejects only after the
# pull request has been opened.
#
# The hash is read from the release's own SHA256SUMS.txt rather than recomputed
# locally, so the manifest describes the artifact people will actually download.
#
#   ./generate.sh 1.1.1
set -euo pipefail

VERSION="${1:?usage: generate.sh <version>}"
REPO="nicglazkov/gcloud-dot"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/manifests/$VERSION"
SETUP="GCloud-Dot-$VERSION-setup.exe"

sums="$(curl -fsSL "https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS.txt")"
hash="$(printf '%s\n' "$sums" \
  | awk -v f="$SETUP" '{ n = $2; sub("^\\./", "", n); if (n == f) print toupper($1) }')"
[ -n "$hash" ] || { echo "no published checksum for $SETUP in v$VERSION" >&2; exit 1; }

# The date the release was published, not the date this script ran. Rerunning
# it a week later must not change what the manifest says.
date="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/v$VERSION" \
  | sed -n 's/.*"published_at": "\([0-9-]*\)T.*/\1/p' | head -1)"

mkdir -p "$OUT"

cat > "$OUT/nicglazkov.GCloudDot.installer.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: nicglazkov.GCloudDot
PackageVersion: $VERSION
InstallerType: nullsoft
# The installer writes only inside the user's profile and never elevates.
Scope: user
InstallModes:
  - interactive
  - silent
UpgradeBehavior: install
ReleaseDate: $date
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/$REPO/releases/download/v$VERSION/$SETUP
    InstallerSha256: $hash
ManifestType: installer
ManifestVersion: 1.6.0
YAML

cat > "$OUT/nicglazkov.GCloudDot.locale.en-US.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: nicglazkov.GCloudDot
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: Nicholas Glazkov
PublisherUrl: https://github.com/nicglazkov
PublisherSupportUrl: https://github.com/$REPO/issues
PackageName: GCloud Dot
PackageUrl: https://nicglazkov.github.io/gcloud-dot/
License: MIT
LicenseUrl: https://github.com/$REPO/blob/main/LICENSE
ShortDescription: Shows whether your gcloud session is alive and how long it has left
Description: |-
  GCloud Dot puts a dot in the system tray that reports whether your gcloud
  credentials are still valid and, when they are, roughly how long they have
  left.

  Whether the session is alive is measured by running gcloud. How long it has
  left is predicted from sessions already observed ending, and the interface
  always says which of the two it is showing. A dropped network is reported as
  unknown rather than as an expiry.

  Includes the gcloud-dot command for scripts, shell prompts, and servers.
Moniker: gcloud-dot
Tags:
  - gcloud
  - gcp
  - google-cloud
  - authentication
  - developer-tools
  - tray
ReleaseNotesUrl: https://github.com/$REPO/releases/tag/v$VERSION
ManifestType: defaultLocale
ManifestVersion: 1.6.0
YAML

cat > "$OUT/nicglazkov.GCloudDot.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: nicglazkov.GCloudDot
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
YAML

echo "wrote $OUT"
echo "  $SETUP  $hash"
