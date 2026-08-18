#!/usr/bin/env bash
# Write the release notes for a tag, from the commits it contains.
#
# GitHub's generated notes are built from merged pull requests. This repository
# is pushed to directly, so they came out as a single "Full Changelog" link and
# nothing else. That matters more here than it usually would: the update banner
# in the app offers "See what changed" and sends people to this page, so an
# empty page is a promise the app makes and the release breaks.
#
# The commit messages already explain what changed and why, so they are the
# notes. Nothing gets written by hand twice.
#
#   ./release-notes.sh v1.1.8 > NOTES.md
set -euo pipefail

TAG="${1:?usage: release-notes.sh <tag>}"
PREV="$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)"

printf '## Install\n\n'
printf 'Pick your platform on the [website](https://nicglazkov.github.io/gcloud-dot/),\n'
printf 'or take a file below. Every download is listed in `SHA256SUMS.txt`.\n\n'
printf 'Already running it? It updates itself: open the window and choose Update now,\n'
printf 'or run `gcloud-dot upgrade`.\n\n'
printf '## What changed\n\n'

if [ -z "$PREV" ]; then
  printf 'First release.\n'
  exit 0
fi

# Oldest first, which reads as a story rather than as a stack.
git log --reverse --no-merges --format=%H "$PREV..$TAG" | while read -r h; do
  subject="$(git log -1 --format=%s "$h")"
  body="$(git log -1 --format=%b "$h" | sed -E '/^Co-Authored-By:/d; /^Claude-Session:/d')"
  printf '### %s\n\n' "$subject"
  # Skip a body that is only whitespace once the trailers are gone.
  if [ -n "$(printf '%s' "$body" | tr -d '[:space:]')" ]; then
    printf '%s\n\n' "$body"
  fi
done

printf '**Full changelog**: https://github.com/nicglazkov/gcloud-dot/compare/%s...%s\n' "$PREV" "$TAG"
