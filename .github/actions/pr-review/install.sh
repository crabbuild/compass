#!/bin/sh
set -eu

version=${COMPASS_ACTION_VERSION:?missing Compass version}
repository=${COMPASS_ACTION_RELEASE_REPOSITORY:?missing release repository}

if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$'; then
    echo "error: compass-version must be an exact semantic version" >&2
    exit 2
fi
if ! printf '%s\n' "$repository" | grep -Eq '^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$'; then
    echo "error: release-repository must be OWNER/REPO" >&2
    exit 2
fi

runner_os=${RUNNER_OS:-}
runner_arch=${RUNNER_ARCH:-}
case "$runner_os:$runner_arch" in
    Linux:X64) target=x86_64-unknown-linux-gnu; binary=compass ;;
    Linux:ARM64) target=aarch64-unknown-linux-gnu; binary=compass ;;
    macOS:X64) target=x86_64-apple-darwin; binary=compass ;;
    macOS:ARM64) target=aarch64-apple-darwin; binary=compass ;;
    Windows:X64) target=x86_64-pc-windows-msvc; binary=compass.exe ;;
    Windows:ARM64) target=aarch64-pc-windows-msvc; binary=compass.exe ;;
    *)
        echo "error: unsupported GitHub runner: $runner_os $runner_arch" >&2
        exit 1
        ;;
esac

name="compass-$target"
archive="$name.tar.gz"
checksum="$archive.sha256"
base="https://github.com/$repository/releases/download/compass-v$version"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --output "$temporary/$archive" "$base/$archive"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --output "$temporary/$checksum" "$base/$checksum"
(
    cd "$temporary"
    fields=$(awk 'NF { count += 1; if (NF != 2 || $2 != "'"$archive"'") bad = 1 } END { if (count != 1 || bad) exit 1; print count }' "$checksum")
    test "$fields" = 1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$checksum"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$checksum"
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 1
    fi
)
if ! tar -tzf "$temporary/$archive" | awk -v root="$name" -v executable="$binary" '
    BEGIN { found = 0 }
    $0 == root "/" { next }
    $0 ~ /(^|\/)\.\.(\/|$)/ || $0 ~ /\\/ { bad = 1 }
    index($0, root "/") != 1 { bad = 1 }
    seen[$0]++ { if (seen[$0] > 1) bad = 1 }
    $0 == root "/" executable { found += 1 }
    END { if (bad || found != 1) exit 1 }
'; then
    echo "error: release archive has an unsafe or unexpected layout" >&2
    exit 1
fi
tar -C "$temporary" -xzf "$temporary/$archive"
test -f "$temporary/$name/$binary"
test ! -L "$temporary/$name/$binary"
if find "$temporary/$name" -type l -print -quit | grep -q .; then
    echo "error: release archive contains a symbolic link" >&2
    exit 1
fi
install_dir="$RUNNER_TEMP/compass-pr-review-bin"
mkdir -p "$install_dir"
cp "$temporary/$name/$binary" "$install_dir/$binary"
chmod 0755 "$install_dir/$binary"
printf '%s\n' "$install_dir" >> "$GITHUB_PATH"
