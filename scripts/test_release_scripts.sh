#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

fake_binary="$test_root/fake-compass"
printf '#!/bin/sh\necho compass 0.1.0\n' > "$fake_binary"
chmod +x "$fake_binary"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    dist="$test_root/dist-$target"
    "$repo_root/scripts/package_macos.sh" "$target" "$fake_binary" "$dist"
    archive="$dist/compass-$target.tar.gz"
    checksum="$archive.sha256"
    test -f "$archive"
    test -f "$checksum"
    (cd "$dist" && shasum -a 256 -c "$(basename "$checksum")")
    tar -tzf "$archive" | grep -Eq '(^|/)compass$'
    test "$(tar -tzf "$archive" | grep -Ec '(^|/)compass$')" -eq 1
done

release_dir="$test_root/release"
mkdir -p "$release_dir" "$test_root/fake-bin"
cp "$test_root/dist-aarch64-apple-darwin/"* "$release_dir/"
cp "$test_root/dist-x86_64-apple-darwin/"* "$release_dir/"

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
    -s) printf '%s\n' Darwin ;;
    -m) printf '%s\n' "$FIXTURE_ARCH" ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$test_root/fake-bin/uname"

for arch in arm64 x86_64; do
    install_dir="$test_root/install-$arch"
    PATH="$test_root/fake-bin:$PATH" \
        FIXTURE_ARCH="$arch" \
        FIXTURE_RELEASE="$release_dir" \
        COMPASS_RELEASE_BASE_URL="https://example.invalid/releases/latest/download" \
        COMPASS_INSTALL_DIR="$install_dir" \
        sh "$repo_root/scripts/install.sh"
    test -x "$install_dir/compass"
    test "$($install_dir/compass)" = "compass 0.1.0"
done

cp "$release_dir/compass-aarch64-apple-darwin.tar.gz.sha256" "$test_root/good.sha256"
printf '%064d  compass-aarch64-apple-darwin.tar.gz\n' 0 \
    > "$release_dir/compass-aarch64-apple-darwin.tar.gz.sha256"
if PATH="$test_root/fake-bin:$PATH" \
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

echo "release script tests passed"
