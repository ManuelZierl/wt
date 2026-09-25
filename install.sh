#!/bin/sh
# Install the latest wt release for Ubuntu 22.04+ x86-64 without Rust or sudo.
set -eu

if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    printf 'wt installer supports Linux x86-64 only.\n' >&2
    exit 1
fi

libc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || {
    printf 'wt installer requires glibc 2.35 or newer (Ubuntu 22.04+).\n' >&2
    exit 1
}
case "$libc" in
    'glibc 2.'*)
        minor=${libc#glibc 2.}
        minor=${minor%%.*}
        if [ "$minor" -lt 35 ]; then
            printf 'wt requires glibc 2.35 or newer (found %s).\n' "$libc" >&2
            exit 1
        fi ;;
    'glibc 3.'*) ;;
    *) printf 'wt installer requires glibc 2.35 or newer (found %s).\n' "$libc" >&2; exit 1 ;;
esac

for tool in curl sha256sum tar mktemp install; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'Missing required command: %s\n' "$tool" >&2
        exit 1
    }
done

asset=wt-linux-x86_64.tar.gz
if [ -n "${WT_RELEASE_BASE_URL:-}" ]; then
    base=${WT_RELEASE_BASE_URL%/}
else
    case "${WT_VERSION:-latest}" in
        latest) base=https://github.com/ManuelZierl/wt/releases/latest/download ;;
        v[0-9]*) base="https://github.com/ManuelZierl/wt/releases/download/$WT_VERSION" ;;
        *) printf 'WT_VERSION must be a release tag such as v0.0.1.\n' >&2; exit 1 ;;
    esac
fi

temp=$(mktemp -d)
trap 'rm -rf -- "$temp"' EXIT HUP INT TERM
curl -fLsS --retry 2 "$base/$asset" -o "$temp/$asset"
curl -fLsS --retry 2 "$base/$asset.sha256" -o "$temp/$asset.sha256"
(cd "$temp" && sha256sum -c "$asset.sha256")
mkdir "$temp/unpacked"
tar -xzf "$temp/$asset" -C "$temp/unpacked" wt watchtower

destination=${WT_INSTALL_DIR:-"${HOME:?HOME is required unless WT_INSTALL_DIR is set}/.local/bin"}
mkdir -p -- "$destination"
staging=$(mktemp -d "$destination/.wt-install.XXXXXX")
trap 'rm -rf -- "$temp" "$staging"' EXIT HUP INT TERM
install -m 755 "$temp/unpacked/wt" "$staging/wt"
install -m 755 "$temp/unpacked/watchtower" "$staging/watchtower"
mv -f -- "$staging/wt" "$destination/wt"
mv -f -- "$staging/watchtower" "$destination/watchtower"
printf 'Installed wt and watchtower in %s\n' "$destination"
case ":$PATH:" in
    *":$destination:"*) ;;
    *) printf 'Add %s to your PATH to run wt from any directory.\n' "$destination" ;;
esac
