#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: package_release.sh <target> <compass-binary> <dist-directory>" >&2
    exit 2
fi

target=$1
binary=$2
dist=$3
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin|\
    aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        binary_name=compass
        ;;
    aarch64-pc-windows-msvc|x86_64-pc-windows-msvc)
        binary_name=compass.exe
        ;;
    *)
        echo "error: unsupported release target: $target" >&2
        exit 2
        ;;
esac
test -f "$binary"

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
name="compass-$target"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT HUP INT TERM
bundle="$staging/$name"
mkdir -p "$bundle" "$dist"

cp "$binary" "$bundle/$binary_name"
chmod 0755 "$bundle/$binary_name"
cp "$repo_root/README.md" "$bundle/"
cp "$repo_root/LICENSE" "$bundle/"
cp "$repo_root/LICENSE-MIT" "$bundle/"
cp "$repo_root/LICENSE-APACHE" "$bundle/"
cp "$repo_root/THIRD_PARTY_NOTICES.md" "$bundle/"
cp -R "$repo_root/completions" "$bundle/"

archive="$dist/$name.tar.gz"
tar -C "$staging" -czf "$archive" "$name"
(
    cd "$dist"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$(basename "$archive")"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$(basename "$archive")"
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 1
    fi > "$(basename "$archive").sha256"
)

printf '%s\n' "$archive"
