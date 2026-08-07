#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

project="$test_root/project"
output="$project/compass-out"
old_snapshot=snapshot-old
mkdir -p "$output/snapshots/$old_snapshot" "$output/store"
printf '%s\n' "$old_snapshot" > "$output/current-snapshot"
printf '%s\n' '{"graph":{"schema":"compass.graph/1"},"nodes":[],"links":[]}' \
    > "$output/snapshots/$old_snapshot/graph.json"
printf 'old-sqlite\n' > "$output/store/store.sqlite3"
printf 'old-reference\n' > "$output/snapshots/$old_snapshot/store.ref"

cat > "$test_root/fake-compass" <<'EOF'
#!/bin/sh
set -eu
if [ "$FAKE_COMPASS_FAIL" = 1 ]; then
    exit 17
fi
test "$1" = update
shift
store=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --out) output=$2; shift 2 ;;
        --store) store=$2; shift 2 ;;
        *) shift ;;
    esac
done
test "$store" = sqlite
new_snapshot=snapshot-new
mkdir -p "$output/store" "$output/snapshots/$new_snapshot"
printf 'new-sqlite\n' > "$output/store/store.sqlite3"
printf '%s\n' '{"graph":{"schema":"compass.graph/1"},"nodes":[],"links":[]}' \
    > "$output/snapshots/$new_snapshot/graph.json"
printf 'new-reference\n' > "$output/snapshots/$new_snapshot/store.ref"
printf '%s\n' "$new_snapshot" > "$output/current-snapshot"
EOF
chmod +x "$test_root/fake-compass"

FAKE_COMPASS_FAIL=0 sh "$repo_root/scripts/rebuild_compass_store.sh" "$project" --compass "$test_root/fake-compass" > "$test_root/success.txt"
backup=$(sed -n 's/^previous sidecar backup: //p' "$test_root/success.txt")
test -n "$backup"
test "$(cat "$output/store/store.sqlite3")" = new-sqlite
test "$(cat "$backup/store/store.sqlite3")" = old-sqlite
test "$(cat "$backup/active-store.ref")" = old-reference

printf 'replacement-sqlite\n' > "$output/store/store.sqlite3"
printf 'replacement-reference\n' \
    > "$output/snapshots/snapshot-new/store.ref"
if FAKE_COMPASS_FAIL=1 sh "$repo_root/scripts/rebuild_compass_store.sh" \
    "$project" --compass "$test_root/fake-compass" > "$test_root/failure.txt" 2>&1; then
    echo "expected failed rebuild" >&2
    exit 1
fi
test "$(cat "$output/store/store.sqlite3")" = replacement-sqlite
test "$(cat "$output/snapshots/snapshot-new/store.ref")" = replacement-reference

echo "store release script tests passed"
