#!/bin/sh
set -eu

if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  echo "CARGO_TARGET_DIR must name this checkout's external target directory" >&2
  exit 2
fi

cargo build \
  -p compass-query \
  --example text_ranker_profile_qualification \
  --release \
  --locked

executable="$CARGO_TARGET_DIR/release/examples/text_ranker_profile_qualification"
if [ "$(uname -s)" = "Windows_NT" ]; then
  executable="${executable}.exe"
fi

if [ ! -x "$executable" ]; then
  echo "qualification executable is missing: $executable" >&2
  exit 1
fi

run_profile() {
  profile="$1"
  echo "text ranker qualification: $profile" >&2
  case "$(uname -s)" in
    Darwin)
      /usr/bin/time -l "$executable" "--$profile"
      ;;
    Linux)
      /usr/bin/time -v "$executable" "--$profile"
      ;;
    *)
      "$executable" "--$profile"
      echo "peak resident memory was not measured on this platform" >&2
      ;;
  esac
}

run_profile full-scan
run_profile bm25
