#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

project="$test_root/project"
output="$project/compass-out"
mkdir -p "$output"
printf '%s\n' '{"graph":{"schema":"compass.graph/1"},"nodes":[],"links":[]}' > "$output/graph.json"
printf 'old-sqlite\n' > "$output/compass-store.sqlite3"
printf 'old-reference\n' > "$output/store.ref"

cat > "$test_root/fake-compass" <<'EOF'
#!/bin/sh
set -eu
if [ "$FAKE_COMPASS_FAIL" = 1 ]; then
    exit 17
fi
test "$1" = update
shift
while [ "$#" -gt 0 ]; do
    case "$1" in
        --out) output=$2; shift 2 ;;
        *) shift ;;
    esac
done
printf 'new-sqlite\n' > "$output/compass-store.sqlite3"
printf 'new-reference\n' > "$output/store.ref"
EOF
chmod +x "$test_root/fake-compass"

FAKE_COMPASS_FAIL=0 sh "$repo_root/scripts/rebuild_compass_store.sh" "$project" --compass "$test_root/fake-compass" > "$test_root/success.txt"
backup=$(sed -n 's/^previous sidecar backup: //p' "$test_root/success.txt")
test -n "$backup"
test "$(cat "$output/compass-store.sqlite3")" = new-sqlite
test "$(cat "$backup/compass-store.sqlite3")" = old-sqlite
test "$(cat "$backup/store.ref")" = old-reference

printf 'replacement-sqlite\n' > "$output/compass-store.sqlite3"
printf 'replacement-reference\n' > "$output/store.ref"
if FAKE_COMPASS_FAIL=1 sh "$repo_root/scripts/rebuild_compass_store.sh" \
    "$project" --compass "$test_root/fake-compass" > "$test_root/failure.txt" 2>&1; then
    echo "expected failed rebuild" >&2
    exit 1
fi
test "$(cat "$output/compass-store.sqlite3")" = replacement-sqlite
test "$(cat "$output/store.ref")" = replacement-reference

echo "store release script tests passed"
