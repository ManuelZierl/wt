#!/usr/bin/env bash
# Verify that a release tag is safe to ship: it must be reachable from the
# release branch (normally `main`) and it must equal the version declared in
# the workspace Cargo.toml. Used by .github/workflows/release.yml, and can be
# run locally against a scratch tag/branch to demonstrate the checks without
# touching a real branch.
#
# Usage: scripts/verify-release-tag.sh <tag> [branch]
#   <tag>    the tag to verify, e.g. v0.0.2 (the "v" prefix is stripped when
#            comparing against Cargo.toml's version)
#   [branch] the ref the tag must be reachable from; defaults to "main"
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

tag=${1:?usage: verify-release-tag.sh <tag> [branch]}
branch=${2:-main}

if ! git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
    printf 'error: tag %s does not exist locally\n' "$tag" >&2
    exit 1
fi

if ! git merge-base --is-ancestor "$tag" "$branch"; then
    printf 'error: tag %s is not reachable from %s\n' "$tag" "$branch" >&2
    exit 1
fi

# Read Cargo.toml as it exists AT THE TAGGED COMMIT, not from the working
# tree, so this check is correct regardless of what happens to be checked out
# when the script runs.
cargo_version=$(git show "$tag:Cargo.toml" | python3 -c '
import re, sys
text = sys.stdin.read()
match = re.search(r"(?m)^\[workspace\.package\]\n(?:.*\n)*?^version\s*=\s*\"([^\"]+)\"", text)
if not match:
    sys.exit("error: could not find [workspace.package] version in Cargo.toml")
print(match.group(1))
')

tag_version=${tag#v}
if [ "$tag_version" != "$cargo_version" ]; then
    printf 'error: tag %s does not match Cargo.toml version %s\n' "$tag" "$cargo_version" >&2
    exit 1
fi

printf 'ok: tag %s is reachable from %s and matches Cargo.toml version %s\n' "$tag" "$branch" "$cargo_version"
