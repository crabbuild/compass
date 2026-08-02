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

if [ ! -f "$output/graph.json" ]; then
    echo "error: graph.json is required before rebuilding the store: $output/graph.json" >&2
    exit 1
fi

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup="$output/store-rebuild-backup-$timestamp-$$"
mkdir -p "$backup"
moved=0
restore_on_failure() {
    if [ "$moved" -eq 1 ]; then
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

for name in compass-store.sqlite3 store.ref compass-store.redb \
    compass-store.sqlite3-wal compass-store.sqlite3-shm
do
    if [ -e "$output/$name" ]; then
        mv "$output/$name" "$backup/$name"
        moved=1
    fi
done

if ! "$binary" update "$project" --out "$output" --force; then
    echo "error: store rebuild failed; the previous sidecar remains in $backup" >&2
    exit 1
fi

trap - EXIT HUP INT TERM
echo "rebuilt Compass store at $output"
echo "previous sidecar backup: $backup"
