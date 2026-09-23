#!/usr/bin/env bash
# Build a Debian package on the oldest Ubuntu version you intend to support.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$root"

for tool in cargo dpkg dpkg-deb getconf python3; do
    command -v "$tool" >/dev/null || { printf 'Missing required tool: %s\n' "$tool" >&2; exit 1; }
done

libc=$(getconf GNU_LIBC_VERSION)
case "$libc" in
    'glibc '*) libc_version=${libc#glibc } ;;
    *) printf 'This package builder requires Ubuntu/glibc (found %s)\n' "$libc" >&2; exit 1 ;;
esac

metadata=$(cargo metadata --locked --no-deps --format-version 1)
version=$(python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "wt-cli"))' <<< "$metadata")
target_dir=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' <<< "$metadata")
architecture=$(dpkg --print-architecture)

cargo build --release --locked -p wt-cli

output_dir=${1:-"$root/dist"}
mkdir -p -- "$output_dir"
output_dir=$(cd -- "$output_dir" && pwd)
package=$(mktemp -d)
trap 'rm -rf -- "$package"' EXIT

install -D -m 0755 "$target_dir/release/wt" "$package/usr/bin/wt"
install -D -m 0755 "$target_dir/release/watchtower" "$package/usr/bin/watchtower"
mkdir -p -- "$package/DEBIAN"
printf 'Package: wt\nVersion: %s\nArchitecture: %s\nMaintainer: Watchtower contributors\nDepends: libc6 (>= %s), libgcc-s1 | libgcc1\nDescription: Watchtower repository rule checker\n Local-first CLI for repository checks; provides wt and watchtower.\n' \
    "$version" "$architecture" "$libc_version" > "$package/DEBIAN/control"

output="$output_dir/wt_${version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$package" "$output"
printf 'Package ready: %s\n' "$output"
