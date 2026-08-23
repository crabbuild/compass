#!/bin/sh
set -eu

mode=fixtures
case "${1:-}" in
  ""|--fixtures-only) mode=fixtures ;;
  --with-installed-model) mode=model ;;
  *)
    echo "usage: scripts/qualify_document_ocr_v1.sh [--fixtures-only|--with-installed-model]" >&2
    exit 2
    ;;
esac

if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  echo "CARGO_TARGET_DIR must name this checkout's directory under /Volumes/Workspace/crabbuild-target" >&2
  exit 2
fi
case "$CARGO_TARGET_DIR" in
  /Volumes/Workspace/crabbuild-target/*) ;;
  *)
    echo "CARGO_TARGET_DIR must be below /Volumes/Workspace/crabbuild-target" >&2
    exit 2
    ;;
esac

cargo test -p compass-ocr -p compass-media --lib --locked
cargo test -p compass-core document --lib --locked
cargo test -p compass-semantic --test orchestration_coverage --locked
cargo test -p compass-cli --test document_cli --locked
python3 scripts/validate_document_ocr_manifest.py

if [ "$mode" = model ]; then
  cargo test -p compass-ocr --test model_acceptance --locked -- --ignored --nocapture
fi

printf '{"schema":"compass.document-ocr.qualification-result/1","mode":"%s","status":"passed","network_during_gate":false}\n' "$mode"
