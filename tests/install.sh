#!/usr/bin/env bash
# Run after build-linux-release.sh; exercise the real installer without a network service.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
asset=wt-linux-x86_64.tar.gz
temp=$(mktemp -d)
trap 'rm -rf -- "$temp"' EXIT
mkdir "$temp/release"
cp "$root/dist/$asset" "$root/dist/$asset.sha256" "$temp/release/"

printf 'tampered' >> "$temp/release/$asset"
if WT_RELEASE_BASE_URL="file://$temp/release" WT_INSTALL_DIR="$temp/bin" \
    sh "$root/install.sh" > "$temp/output" 2>&1; then
    printf 'Installer accepted a corrupt archive\n' >&2
    exit 1
fi
test ! -e "$temp/bin/wt"

cp "$root/dist/$asset" "$temp/release/$asset"
WT_RELEASE_BASE_URL="file://$temp/release" WT_INSTALL_DIR="$temp/bin" sh "$root/install.sh"
WT_RELEASE_BASE_URL="file://$temp/release" WT_INSTALL_DIR="$temp/bin" sh "$root/install.sh"
test "$("$temp/bin/wt" --version)" = "$("$temp/bin/watchtower" --version)"
"$temp/bin/wt" schema result >/dev/null
printf 'Installer smoke test passed\n'
