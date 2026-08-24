#!/usr/bin/env bash
set -euo pipefail

# Qualification and release builds intentionally run with TSLP_OFFLINE=1. Keep
# the parser-source acquisition explicit, pinned, and checksum-verified at the
# CI boundary instead of allowing the Rust build to download at an arbitrary
# point in the compilation.
PARSER_VERSION="1.13.1"
PARSER_SHA256="411bc912ca9b6fa43f75aeff0ad9df419b2b425e99795c66ab253f2970e2afd3"
PARSER_URL="https://github.com/xberg-io/tree-sitter-language-pack/releases/download/v${PARSER_VERSION}/parser-sources-${PARSER_VERSION}.tar.zst"

usage() {
  echo "usage: $0 DESTINATION" >&2
  exit 2
}

[[ "$#" -eq 1 && -n "$1" ]] || usage
DESTINATION="$1"

if [[ -f "$DESTINATION/sources/language_definitions.json" && -d "$DESTINATION/parsers" ]]; then
  echo "[parser-sources] using existing bundle at $DESTINATION"
  exit 0
fi
[[ ! -e "$DESTINATION" || -d "$DESTINATION" ]] || {
  echo "[parser-sources] destination is not a directory: $DESTINATION" >&2
  exit 1
}

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/compass-parser-sources.XXXXXX")"
trap 'chmod -R u+w "$TMP_ROOT" 2>/dev/null || true; rm -rf -- "$TMP_ROOT"' EXIT
ARCHIVE="$TMP_ROOT/parser-sources.tar.zst"
EXTRACTED="$TMP_ROOT/extracted"

echo "[parser-sources] download pinned parser bundle v$PARSER_VERSION"
curl --fail --silent --show-error --location --retry 3 --retry-delay 2 \
  "$PARSER_URL" --output "$ARCHIVE"

actual_sha256="$(
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$ARCHIVE" | awk '{print $1}'
  else
    shasum -a 256 "$ARCHIVE" | awk '{print $1}'
  fi
)"
[[ "$actual_sha256" == "$PARSER_SHA256" ]] || {
  echo "[parser-sources] checksum mismatch for $PARSER_URL: expected $PARSER_SHA256, got $actual_sha256" >&2
  exit 1
}

mkdir -p "$EXTRACTED"
tar --extract --zstd --file "$ARCHIVE" --directory "$EXTRACTED"

SOURCE_ROOT=""
if [[ -f "$EXTRACTED/sources/language_definitions.json" && -d "$EXTRACTED/parsers" ]]; then
  SOURCE_ROOT="$EXTRACTED"
else
  definition_path="$(find "$EXTRACTED" -type f -path '*/sources/language_definitions.json' -print -quit)"
  [[ -n "$definition_path" ]] || {
    echo "[parser-sources] archive does not contain sources/language_definitions.json" >&2
    exit 1
  }
  SOURCE_ROOT="${definition_path%/sources/language_definitions.json}"
fi
[[ -d "$SOURCE_ROOT/parsers" ]] || {
  echo "[parser-sources] archive does not contain a parsers directory" >&2
  exit 1
}

mkdir -p "$DESTINATION"
cp -R "$SOURCE_ROOT/." "$DESTINATION/"
[[ -f "$DESTINATION/sources/language_definitions.json" && -d "$DESTINATION/parsers" ]] || {
  echo "[parser-sources] failed to install a complete bundle at $DESTINATION" >&2
  exit 1
}
echo "[parser-sources] installed verified bundle at $DESTINATION"
