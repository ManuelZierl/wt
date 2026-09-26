#!/usr/bin/env bash
# Print the CHANGELOG.md section for a release tag, without its heading.
# Fails when the section is missing or empty, so a release cannot ship
# without notes. Used by .github/workflows/release.yml.
#
# Usage: scripts/release-notes.sh <tag>
#   <tag>    the release tag, e.g. v0.0.2; its section heading is "## v0.0.2"
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
tag=${1:?usage: release-notes.sh <tag>}

notes=$(awk -v heading="## $tag" '
    $0 == heading { found = 1; next }
    found && /^## / { exit }
    found { print }
' "$root/CHANGELOG.md")

if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
    printf 'error: CHANGELOG.md has no notes under "## %s"\n' "$tag" >&2
    exit 1
fi

printf '%s\n' "$notes"
