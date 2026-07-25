#!/bin/sh
set -eu

if rg -n -i \
  'graphify|COMPASS_INTERNAL_GRAPHIFY_COMPAT|Frontend::Graphify|run_graphify_watch' \
  crates \
  --glob '*/src/**' \
  --glob '*/prompts/**' \
  --glob 'compass-cli/assets/compass-skill/**' \
  --glob 'compass-cli/assets/compass-integrations/**'
then
  echo "error: production Compass must not reference Graphify" >&2
  exit 1
fi

if rg -n \
  'GRAPHIFY_|\.graphify(ignore)?|merge\.graphify' \
  crates \
  --glob '*/src/**' \
  --glob '*/prompts/**' \
  --glob 'compass-cli/assets/compass-skill/**' \
  --glob 'compass-cli/assets/compass-integrations/**'
then
  echo "error: production Compass must use Compass-owned configuration and artifact names" >&2
  exit 1
fi

if rg -n -i \
  'compass-parity|Graphify-Labs/graphify|python-oracle|GRAPHIFY_REPO_ROOT|python[[:space:]]+-m[[:space:]]+graphify|run_graphify_watch' \
  .github scripts Makefile Cargo.toml \
  --glob '!check_product_boundary.sh'
then
  echo "error: Compass build, test, and CI automation must not execute or check out Graphify" >&2
  exit 1
fi
