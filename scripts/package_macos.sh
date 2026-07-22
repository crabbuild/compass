#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: package_macos.sh <target> <compass-binary> <dist-directory>" >&2
    exit 2
fi

target=$1
binary=$2
dist=$3
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *)
        echo "error: unsupported macOS target: $target" >&2
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

install -m 0755 "$binary" "$bundle/compass"
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
    shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256"
)

printf '%s\n' "$archive"
