#!/bin/sh
set -eu

workspace_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$workspace_root"

check_tree() {
    package=$1
    tree=$(cargo tree -p "$package" --locked --prefix none)
    if printf '%s\n' "$tree" | rg -i '^surrealdb[^ ]*( |$)' >/dev/null; then
        printf 'error: SurrealDB reached default dependency tree for %s\n' "$package" >&2
        exit 1
    fi
}

check_tree compass-cli
check_tree compass-mcp
check_tree compass-core

default_projection_tree=$(cargo tree -p compass-graphdb-surreal --locked --prefix none)
if printf '%s\n' "$default_projection_tree" | rg -i '^surrealdb[^ ]*( |$)' >/dev/null; then
    printf 'error: SurrealDB reached the projection crate without an engine feature\n' >&2
    exit 1
fi

printf 'SurrealDB feature isolation: PASS\n'

if [ "${1:-}" = "--binary" ]; then
    target_root=${CARGO_TARGET_DIR:-target}
    case "$target_root" in
        /*) ;;
        *) target_root="$workspace_root/$target_root" ;;
    esac
    cargo build -p compass-cli --locked
    binary_path="$target_root/debug/compass"
    if [ -f "$target_root/debug/compass.exe" ]; then
        binary_path="$target_root/debug/compass.exe"
    fi
    if [ ! -f "$binary_path" ]; then
        printf 'error: default compass binary was not produced under %s\n' "$target_root/debug" >&2
        exit 1
    fi
    printf 'Default compass binary built from a SurrealDB-free dependency closure: PASS\n'
fi
