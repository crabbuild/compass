#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUALIFY_TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-python-frameworks.XXXXXX")"
trap 'chmod -R u+w "$QUALIFY_TMP" 2>/dev/null || true; rm -rf -- "$QUALIFY_TMP"' EXIT

usage() {
  echo "usage: $0 --fixtures-only | --pinned --baseline PATH" >&2
  exit 2
}

cd "$QUALIFY_ROOT"
case "${1:-}" in
  --fixtures-only)
    [[ "$#" -eq 1 ]] || usage
    arguments=(--fixtures-only)
    ;;
  --pinned)
    [[ "$#" -eq 3 && "${2:-}" == "--baseline" && -n "${3:-}" ]] || usage
    arguments=(--pinned --baseline "$3")
    ;;
  *)
    usage
    ;;
esac

python3 scripts/qualify_python_frameworks.py "${arguments[@]}" --output "$QUALIFY_TMP/first.json"
python3 scripts/qualify_python_frameworks.py "${arguments[@]}" --output "$QUALIFY_TMP/second.json"
cmp "$QUALIFY_TMP/first.json" "$QUALIFY_TMP/second.json"
python3 - "$QUALIFY_TMP/first.json" <<'PY'
import hashlib
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
raw = path.read_bytes()
report = json.loads(raw)
print(
    json.dumps(
        {
            "mode": report["mode"],
            "productionQualified": report["productionQualified"],
            "reportSha256": hashlib.sha256(raw).hexdigest(),
            "schema": report["schema"],
            "status": report["status"],
        },
        sort_keys=True,
        separators=(",", ":"),
    )
)
PY
