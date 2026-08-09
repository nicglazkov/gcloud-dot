#!/usr/bin/env bash
# Generates winget manifests for a published release.
#
#   packaging/windows/winget/generate.sh 1.0.0
#
# Written from the published installer rather than a local build, because the
# hash winget records has to be the hash of the file people will actually
# download. Generating it from a local artifact is how a manifest ends up
# describing a file that was never released.
set -euo pipefail

VERSION="${1:?usage: generate.sh <version>}"
REPO="nicglazkov/gcloud-dot"
PKGID="nicglazkov.GCloudDot"
OUT="$(cd "$(dirname "$0")" && pwd)/manifests/$VERSION"
URL="https://github.com/$REPO/releases/download/v$VERSION/GCloud-Dot-$VERSION-setup.exe"

mkdir -p "$OUT"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading the published installer"
curl -fsSL -o "$tmp/setup.exe" "$URL"
SHA="$(shasum -a 256 "$tmp/setup.exe" | cut -d' ' -f1 | tr '[:lower:]' '[:upper:]')"
echo "sha256 $SHA"

RELEASE_DATE="$(date -u +%Y-%m-%d)"

cat > "$OUT/$PKGID.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: $PKGID
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
EOF

cat > "$OUT/$PKGID.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: $PKGID
PackageVersion: $VERSION
InstallerType: nullsoft
# The installer writes only inside the user's profile and never elevates.
Scope: user
InstallModes:
  - interactive
  - silent
UpgradeBehavior: install
ReleaseDate: $RELEASE_DATE
Installers:
  - Architecture: x64
    InstallerUrl: $URL
    InstallerSha256: $SHA
ManifestType: installer
ManifestVersion: 1.6.0
EOF

cat > "$OUT/$PKGID.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: $PKGID
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
EOF

echo
echo "wrote $OUT"
echo
echo "Validate and submit:"
echo "  winget validate --manifest $OUT"
echo "  wingetcreate submit --token <gh-token> $OUT"
echo
echo "Submission opens a pull request against microsoft/winget-pkgs."
