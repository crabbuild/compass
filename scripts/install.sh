#!/bin/sh
set -eu

repository=${COMPASS_REPOSITORY:-crabbuild/compass}
release_base_url=${COMPASS_RELEASE_BASE_URL:-https://github.com/$repository/releases/latest/download}
install_dir=${COMPASS_INSTALL_DIR:-$HOME/.local/bin}

os=$(uname -s)
arch=$(uname -m)
case "$os:$arch" in
    Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin ;;
    Darwin:x86_64|Darwin:amd64) target=x86_64-apple-darwin ;;
    Linux:arm64|Linux:aarch64) target=aarch64-unknown-linux-gnu ;;
    Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-gnu ;;
    *)
        echo "error: unsupported platform: $os $arch" >&2
        exit 1
        ;;
esac

name="compass-$target"
archive="$name.tar.gz"
checksum="$archive.sha256"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

download() {
    curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
        --output "$temporary/$2" "$release_base_url/$1"
}

verify_checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$1"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$1"
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

download "$archive" "$archive"
download "$checksum" "$checksum"
(
    cd "$temporary"
    verify_checksum "$checksum"
)

tar -C "$temporary" -xzf "$temporary/$archive"
test -x "$temporary/$name/compass"
mkdir -p "$install_dir"
install -m 0755 "$temporary/$name/compass" "$install_dir/compass"

printf 'Installed Compass to %s/compass\n' "$install_dir"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) printf 'Add %s to PATH before running compass.\n' "$install_dir" ;;
esac
