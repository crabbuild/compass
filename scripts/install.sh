#!/bin/sh
set -eu

repository=${COMPASS_REPOSITORY:-crabbuild/compass}
release_base_url=${COMPASS_RELEASE_BASE_URL:-https://github.com/$repository/releases/latest/download}
install_dir=${COMPASS_INSTALL_DIR:-$HOME/.local/bin}

if [ "$(uname -s)" != Darwin ]; then
    echo "error: this installer currently supports macOS only" >&2
    exit 1
fi

case "$(uname -m)" in
    arm64|aarch64) target=aarch64-apple-darwin ;;
    x86_64|amd64) target=x86_64-apple-darwin ;;
    *)
        echo "error: unsupported macOS architecture: $(uname -m)" >&2
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

download "$archive" "$archive"
download "$checksum" "$checksum"
(
    cd "$temporary"
    shasum -a 256 -c "$checksum"
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
