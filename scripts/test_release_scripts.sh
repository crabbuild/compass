#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test -f "$repo_root/scripts/install.sh"
test -f "$repo_root/scripts/install.ps1"
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 \
    > "$test_root/metadata.json"
python3 "$repo_root/scripts/publishable_crates.py" \
    < "$test_root/metadata.json" > "$test_root/publishable-crates.txt"
python3 - "$test_root/metadata.json" <<'PY'
import json
import pathlib
import sys

metadata = json.loads(pathlib.Path(sys.argv[1]).read_text())
packages = {package["name"]: package for package in metadata["packages"]}
internal = {"compass-transcribe", "compass-whisper"}

for name in internal:
    assert packages[name]["publish"] == [], f"{name} must set publish = false"

reachable = set()
pending = ["compass-cli"]
while pending:
    name = pending.pop()
    if name in reachable:
        continue
    reachable.add(name)
    pending.extend(
        dependency["name"]
        for dependency in packages[name]["dependencies"]
        if dependency["kind"] != "dev" and dependency["name"] in packages
    )

assert not reachable & internal, (
    "registry CLI graph reaches internal inference crates: "
    + ", ".join(sorted(reachable & internal))
)
PY
test "$(grep -Ec '^compass-(transcribe|whisper)$' "$test_root/publishable-crates.txt")" -eq 0

mkdir -p "$test_root/fake-checksum-bin"

cat > "$test_root/fake-checksum-bin/shasum" <<'EOF'
#!/bin/sh
echo "shasum must not be required when sha256sum is available" >&2
exit 99
EOF
chmod +x "$test_root/fake-checksum-bin/shasum"

cat > "$test_root/fake-checksum-bin/sha256sum" <<'EOF'
#!/bin/sh
if [ -x /usr/bin/sha256sum ]; then
    exec /usr/bin/sha256sum "$@"
fi
exec /usr/bin/shasum -a 256 "$@"
EOF
chmod +x "$test_root/fake-checksum-bin/sha256sum"

for release_case in \
    "aarch64-apple-darwin compass" \
    "x86_64-apple-darwin compass" \
    "aarch64-unknown-linux-gnu compass" \
    "x86_64-unknown-linux-gnu compass" \
    "aarch64-pc-windows-msvc compass.exe" \
    "x86_64-pc-windows-msvc compass.exe"
do
    set -- $release_case
    target=$1
    binary_name=$2
    fake_binary="$test_root/fake-$target-$binary_name"
    printf '#!/bin/sh\necho %s\n' "$target" > "$fake_binary"
    chmod +x "$fake_binary"
    dist="$test_root/dist-$target"
    PATH="$test_root/fake-checksum-bin:$PATH" \
        "$repo_root/scripts/package_release.sh" "$target" "$fake_binary" "$dist"
    archive="$dist/compass-$target.tar.gz"
    checksum="$archive.sha256"
    test -f "$archive"
    test -f "$checksum"
    (
        cd "$dist"
        PATH="$test_root/fake-checksum-bin:$PATH" \
            sha256sum -c "$(basename "$checksum")"
    )
    tar -tzf "$archive" | grep -Eq "(^|/)$binary_name$"
    test "$(tar -tzf "$archive" | grep -Ec "(^|/)$binary_name$")" -eq 1
done

if tar -tzf "$test_root/dist-aarch64-apple-darwin/compass-aarch64-apple-darwin.tar.gz" \
    | grep -Eiq '(^|/)(graph\.json|store\.(sqlite3|redb)|store\.ref|\.compass|credentials|\.env)(/|$|\.)'; then
    echo "release archive contains local store state or credentials" >&2
    exit 1
fi

release_dir="$test_root/release"
mkdir -p "$release_dir" "$test_root/fake-bin"
cp "$test_root/dist-aarch64-apple-darwin/"* "$release_dir/"
cp "$test_root/dist-x86_64-apple-darwin/"* "$release_dir/"
cp "$test_root/dist-aarch64-unknown-linux-gnu/"* "$release_dir/"
cp "$test_root/dist-x86_64-unknown-linux-gnu/"* "$release_dir/"

cat > "$test_root/fake-bin/curl" <<'EOF'
#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o|--output)
            output=$2
            shift 2
            ;;
        -* ) shift ;;
        * ) url=$1; shift ;;
    esac
done
cp "$FIXTURE_RELEASE/${url##*/}" "$output"
EOF
chmod +x "$test_root/fake-bin/curl"

cat > "$test_root/fake-bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
    -s) printf '%s\n' "$FIXTURE_OS" ;;
    -m) printf '%s\n' "$FIXTURE_ARCH" ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$test_root/fake-bin/uname"

for platform in \
    "Darwin arm64 aarch64-apple-darwin" \
    "Darwin x86_64 x86_64-apple-darwin" \
    "Linux aarch64 aarch64-unknown-linux-gnu" \
    "Linux x86_64 x86_64-unknown-linux-gnu"
do
    set -- $platform
    os=$1
    arch=$2
    target=$3
    install_dir="$test_root/install-$target"
    PATH="$test_root/fake-bin:$test_root/fake-checksum-bin:$PATH" \
        FIXTURE_OS="$os" \
        FIXTURE_ARCH="$arch" \
        FIXTURE_RELEASE="$release_dir" \
        COMPASS_RELEASE_BASE_URL="https://example.invalid/releases/latest/download" \
        COMPASS_INSTALL_DIR="$install_dir" \
        sh "$repo_root/scripts/install.sh"
    test -x "$install_dir/compass"
    test "$($install_dir/compass)" = "$target"
done

cp "$release_dir/compass-aarch64-apple-darwin.tar.gz.sha256" "$test_root/good.sha256"
printf '%064d  compass-aarch64-apple-darwin.tar.gz\n' 0 \
    > "$release_dir/compass-aarch64-apple-darwin.tar.gz.sha256"
if PATH="$test_root/fake-bin:$test_root/fake-checksum-bin:$PATH" \
    FIXTURE_OS=Darwin \
    FIXTURE_ARCH=arm64 \
    FIXTURE_RELEASE="$release_dir" \
    COMPASS_RELEASE_BASE_URL="https://example.invalid/releases/latest/download" \
    COMPASS_INSTALL_DIR="$test_root/checksum-must-fail" \
    sh "$repo_root/scripts/install.sh"; then
    echo "installer accepted a bad checksum" >&2
    exit 1
fi
test ! -e "$test_root/checksum-must-fail/compass"
mv "$test_root/good.sha256" "$release_dir/compass-aarch64-apple-darwin.tar.gz.sha256"

"$repo_root/scripts/tests/test_store_release.sh"
echo "release script tests passed"
