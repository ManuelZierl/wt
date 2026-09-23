#!/usr/bin/env bash
# Build Ubuntu 22.04-compatible release assets locally, without Actions.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$root"
for tool in cargo cargo-zigbuild readelf python3 sha256sum tar; do
    command -v "$tool" >/dev/null || { printf 'Missing required command: %s\n' "$tool" >&2; exit 1; }
done
command -v "${CARGO_ZIGBUILD_ZIG_PATH:-zig}" >/dev/null || {
    printf 'Zig is required to build the Ubuntu 22.04 release.\n' >&2
    exit 1
}

target_dir=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
cargo zigbuild -p wt-cli --release --locked --target x86_64-unknown-linux-gnu.2.35
binary_dir="$target_dir/x86_64-unknown-linux-gnu/release"
python3 - "$binary_dir/wt" "$binary_dir/watchtower" <<'PY'
import re
import subprocess
import sys

for binary in sys.argv[1:]:
    versions = re.findall(r'GLIBC_(\d+)\.(\d+)', subprocess.check_output(
        ['readelf', '--version-info', binary], text=True))
    if not versions or max((int(major), int(minor)) for major, minor in versions) > (2, 35):
        sys.exit(f'{binary}: requires glibc newer than 2.35, or has no glibc symbols')
PY

mkdir -p dist
asset=wt-linux-x86_64.tar.gz
tar -czf "dist/$asset" -C "$binary_dir" wt watchtower
(cd dist && sha256sum "$asset" > "$asset.sha256")
printf 'Release assets: dist/%s and dist/%s.sha256\n' "$asset" "$asset"
