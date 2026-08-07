#!/bin/sh
set -eu

usage() {
    echo "usage: rebuild_compass_store.sh PROJECT_ROOT [--out OUTPUT] [--compass BINARY]" >&2
    exit 2
}

project=${1:-}
[ -n "$project" ] || usage
shift
output=
binary=${COMPASS_BIN:-compass}
while [ "$#" -gt 0 ]; do
    case "$1" in
        --out)
            [ "$#" -ge 2 ] || usage
            output=$2
            shift 2
            ;;
        --compass)
            [ "$#" -ge 2 ] || usage
            binary=$2
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

project=$(CDPATH= cd -- "$project" && pwd)
if [ -z "$output" ]; then
    output=${COMPASS_OUT:-$project/compass-out}
fi
mkdir -p "$(dirname -- "$output")"
output=$(CDPATH= cd -- "$(dirname -- "$output")" && pwd)/$(basename -- "$output")

current_snapshot=
current_directory=
if [ -f "$output/current-snapshot" ]; then
    current_snapshot=$(tr -d '\r\n' < "$output/current-snapshot")
    case "$current_snapshot" in
        ''|.|..|*/*|*\\*)
            echo "error: invalid active Compass snapshot: $current_snapshot" >&2
            exit 1
            ;;
    esac
    current_directory="$output/snapshots/$current_snapshot"
fi

graph_path="$output/graph.json"
if [ -n "$current_directory" ]; then
    graph_path="$current_directory/graph.json"
fi
if [ ! -f "$graph_path" ]; then
    echo "error: graph.json is required before rebuilding the store: $graph_path" >&2
    exit 1
fi

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup="$output/store-rebuild-backup-$timestamp-$$"
mkdir -p "$backup"
moved=0
restore_on_failure() {
    if [ "$moved" -eq 1 ]; then
        if [ -d "$output/store" ] && [ ! -e "$backup/failed-store" ]; then
            mv "$output/store" "$backup/failed-store"
        fi
        if [ -d "$backup/store" ] && [ ! -e "$output/store" ]; then
            mv "$backup/store" "$output/store"
        fi
        if [ -n "$current_directory" ] && [ -f "$backup/active-store.ref" ] \
            && [ ! -e "$current_directory/store.ref" ]; then
            mv "$backup/active-store.ref" "$current_directory/store.ref"
        fi
        for name in store.sqlite3 store.ref store.redb \
            store.sqlite3-wal store.sqlite3-shm
        do
            if [ -f "$backup/$name" ] && [ ! -e "$output/$name" ]; then
                mv "$backup/$name" "$output/$name"
            fi
        done
    fi
}
trap restore_on_failure EXIT HUP INT TERM

if [ -d "$output/store" ]; then
    mv "$output/store" "$backup/store"
    moved=1
fi
if [ -n "$current_directory" ] && [ -f "$current_directory/store.ref" ]; then
    printf '%s\n' "$current_snapshot" > "$backup/active-snapshot.txt"
    mv "$current_directory/store.ref" "$backup/active-store.ref"
    moved=1
fi

# Also preserve pre-shared-layout sidecars so this remains a hard-cut tool for
# development and pre-release stores.
for name in store.sqlite3 store.ref store.redb \
    store.sqlite3-wal store.sqlite3-shm
do
    if [ -e "$output/$name" ]; then
        mv "$output/$name" "$backup/$name"
        moved=1
    fi
done

if ! "$binary" update "$project" --out "$output" --force --store sqlite; then
    echo "error: store rebuild failed; the previous sidecar remains in $backup" >&2
    exit 1
fi

trap - EXIT HUP INT TERM
echo "rebuilt Compass store at $output"
echo "previous sidecar backup: $backup"
