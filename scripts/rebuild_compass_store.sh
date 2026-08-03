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

active_generation=
active_directory=
if [ -f "$output/.compass-active-generation" ]; then
    active_generation=$(tr -d '\r\n' < "$output/.compass-active-generation")
    case "$active_generation" in
        ''|.|..|*/*|*\\*)
            echo "error: invalid active Compass generation: $active_generation" >&2
            exit 1
            ;;
    esac
    active_directory="$output/.compass-generations/$active_generation"
fi

graph_path="$output/graph.json"
if [ -n "$active_directory" ]; then
    graph_path="$active_directory/graph.json"
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
        if [ -d "$output/.compass-store" ] && [ ! -e "$backup/failed-.compass-store" ]; then
            mv "$output/.compass-store" "$backup/failed-.compass-store"
        fi
        if [ -d "$backup/.compass-store" ] && [ ! -e "$output/.compass-store" ]; then
            mv "$backup/.compass-store" "$output/.compass-store"
        fi
        if [ -n "$active_directory" ] && [ -f "$backup/active-store.ref" ] \
            && [ ! -e "$active_directory/store.ref" ]; then
            mv "$backup/active-store.ref" "$active_directory/store.ref"
        fi
        for name in compass-store.sqlite3 store.ref compass-store.redb \
            compass-store.sqlite3-wal compass-store.sqlite3-shm
        do
            if [ -f "$backup/$name" ] && [ ! -e "$output/$name" ]; then
                mv "$backup/$name" "$output/$name"
            fi
        done
    fi
}
trap restore_on_failure EXIT HUP INT TERM

if [ -d "$output/.compass-store" ]; then
    mv "$output/.compass-store" "$backup/.compass-store"
    moved=1
fi
if [ -n "$active_directory" ] && [ -f "$active_directory/store.ref" ]; then
    printf '%s\n' "$active_generation" > "$backup/active-generation.txt"
    mv "$active_directory/store.ref" "$backup/active-store.ref"
    moved=1
fi

# Also preserve pre-shared-layout sidecars so this remains a hard-cut tool for
# development and pre-release stores.
for name in compass-store.sqlite3 store.ref compass-store.redb \
    compass-store.sqlite3-wal compass-store.sqlite3-shm
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
