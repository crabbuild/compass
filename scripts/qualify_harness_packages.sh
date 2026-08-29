#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --compass PATH [--codex PATH] [--claude PATH] [--opencode PATH]" >&2
}

compass_binary=""
codex_binary="codex"
claude_binary="claude"
opencode_binary="opencode"
while (($# > 0)); do
  case "$1" in
    --compass|--codex|--claude|--opencode)
      option="$1"
      shift
      if (($# == 0)); then
        usage
        exit 2
      fi
      case "$option" in
        --compass) compass_binary="$1" ;;
        --codex) codex_binary="$1" ;;
        --claude) claude_binary="$1" ;;
        --opencode) opencode_binary="$1" ;;
      esac
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

if [[ -z "$compass_binary" || ! -x "$compass_binary" ]]; then
  echo "error: --compass must name an executable" >&2
  exit 2
fi
for command in "$codex_binary" "$claude_binary" "$opencode_binary" node npm; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "error: required harness command is unavailable: $command" >&2
    exit 2
  fi
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/compass-harness-lifecycle.XXXXXX")"
cleanup() {
  if [[ -n "$scratch" && -d "$scratch" && "$scratch" == *compass-harness-lifecycle.* ]]; then
    rm -rf -- "$scratch"
  fi
}
trap cleanup EXIT

state="$scratch/state"
packages="$scratch/packages"
logs="$scratch/logs"
script_root="$(cd "$(dirname "$0")" && pwd)"
log_filter="$script_root/bounded_redacted_log.mjs"
mkdir -p "$state/user" "$packages" "$logs"
sentinel_content="user-owned instructions must survive managed package lifecycle"

resolve_command() {
  local resolved
  resolved="$(command -v "$1")"
  case "$resolved" in
    /*) printf '%s' "$resolved" ;;
    *) printf '%s/%s' "$(cd "$(dirname "$resolved")" && pwd)" "$(basename "$resolved")" ;;
  esac
}

compass_binary="$(resolve_command "$compass_binary")"
codex_binary="$(resolve_command "$codex_binary")"
claude_binary="$(resolve_command "$claude_binary")"
opencode_binary="$(resolve_command "$opencode_binary")"
export PATH="$(dirname "$compass_binary"):$PATH"

emit_log() {
  local stage="$1"
  echo "--- bounded redacted log: $stage ---" >&2
  sed -n '1,240p' "$logs/$stage.log" >&2
}

run_logged() {
  local stage="$1"
  shift
  set +e
  node "$log_filter" "$logs/$stage.log" "${COMPASS_HARNESS_STAGE_TIMEOUT_SECONDS:-300}" \
    "$scratch" "$state" "$packages" "$script_root" "${HOME:-}" "$PWD" -- "$@"
  local stage_status=$?
  set -e
  if [[ "$stage_status" -ne 0 ]]; then
    if [[ "$stage_status" -eq 124 ]]; then
      echo "error: lifecycle stage $stage exceeded its duration limit" >&2
    else
      echo "error: lifecycle stage $stage failed with status $stage_status" >&2
    fi
    emit_log "$stage"
    return "$stage_status"
  fi
}

require_log_contains() {
  local stage="$1"
  local pattern="$2"
  if ! grep -Fq -- "$pattern" "$logs/$stage.log"; then
    echo "error: lifecycle stage $stage did not report $pattern" >&2
    emit_log "$stage"
    return 1
  fi
}

require_skills_in_log() {
  local stage="$1"
  local skill
  for skill in \
    compass \
    compass-architecture \
    compass-change-impact \
    compass-debug \
    compass-index-maintenance \
    compass-mcp-setup \
    compass-navigate
  do
    require_log_contains "$stage" "$skill"
  done
}

require_equal() {
  local label="$1"
  local expected="$2"
  local actual="$3"
  if [[ "$actual" != "$expected" ]]; then
    echo "error: $label version mismatch: expected $expected, observed $actual" >&2
    return 1
  fi
}

extract_version() {
  awk 'match($0, /[0-9]+\.[0-9]+\.[0-9]+/) { print substr($0, RSTART, RLENGTH); exit }' "$1"
}

make_sentinel() {
  printf '%s\n' "$sentinel_content" >"$1"
}

require_sentinel() {
  if ! cmp -s "$1" <(printf '%s\n' "$sentinel_content"); then
    echo "error: harness modified user-owned sentinel" >&2
    return 1
  fi
}

installed_plugin_root() {
  local cache_root="$1"
  local marker="$2"
  local match=""
  local count=0
  while IFS= read -r candidate; do
    match="$candidate"
    count=$((count + 1))
  done < <(find "$cache_root" -maxdepth 8 -type f -path "*/$marker" -print | sort)
  if [[ "$count" -ne 1 ]]; then
    echo "error: expected one installed $marker below the isolated cache, observed $count" >&2
    return 1
  fi
  dirname "$(dirname "$match")"
}

verify_installed_skills() {
  local stage="$1"
  local installed_root="$2"
  local exported_root="$3"
  run_logged "$stage" diff -qr "$exported_root/skills" "$installed_root/skills"
}

for platform in codex claude opencode; do
  run_logged "export-$platform" "$compass_binary" agent export \
    --platform "$platform" \
    --transport stdio \
    --out "$packages/$platform"
  run_logged "validate-$platform" "$compass_binary" agent validate \
    --platform "$platform" \
    --path "$packages/$platform"
done

manifest_harness_version() {
  node -e '
    const fs = require("node:fs")
    const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"))
    if (typeof manifest.harness_version !== "string" || manifest.harness_version.length === 0) process.exit(2)
    process.stdout.write(manifest.harness_version)
  ' "$1/manifest.json"
}

expected_codex_version="$(manifest_harness_version "$packages/codex")"
expected_claude_version="$(manifest_harness_version "$packages/claude")"
expected_opencode_version="$(manifest_harness_version "$packages/opencode")"
run_logged codex-version "$codex_binary" --version
run_logged claude-version "$claude_binary" --version
run_logged opencode-version "$opencode_binary" --version
actual_codex_version="$(extract_version "$logs/codex-version.log")"
actual_claude_version="$(extract_version "$logs/claude-version.log")"
actual_opencode_version="$(extract_version "$logs/opencode-version.log")"
require_equal Codex "$expected_codex_version" "$actual_codex_version"
require_equal "Claude Code" "$expected_claude_version" "$actual_claude_version"
require_equal OpenCode "$expected_opencode_version" "$actual_opencode_version"

codex_home="$state/codex"
mkdir -p "$codex_home"
codex_sentinel="$codex_home/USER-INSTRUCTIONS.md"
make_sentinel "$codex_sentinel"
run_logged codex-marketplace env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin marketplace add "$packages/codex" --json
run_logged codex-available env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin list --available --json
require_log_contains codex-available '"compass"'
run_logged codex-install env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin add compass@compass-plugins --json
require_sentinel "$codex_sentinel"
run_logged codex-list env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin list --json
require_log_contains codex-list '"compass"'
codex_plugin_root="$(installed_plugin_root "$codex_home/plugins/cache" ".codex-plugin/plugin.json")"
verify_installed_skills codex-installed-skills "$codex_plugin_root" "$packages/codex"
run_logged codex-skill-discovery env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" -C "$state/user" debug prompt-input
require_skills_in_log codex-skill-discovery
run_logged codex-mcp-list env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" mcp list
require_log_contains codex-mcp-list compass
# Codex documents plugin add as the reinstall/update operation for local marketplaces.
run_logged codex-upgrade env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin add compass@compass-plugins --json
require_sentinel "$codex_sentinel"
run_logged codex-uninstall env CODEX_HOME="$codex_home" HOME="$state/user" \
  "$codex_binary" plugin remove compass@compass-plugins --json
require_sentinel "$codex_sentinel"

claude_home="$state/claude-home"
mkdir -p "$claude_home"
claude_sentinel="$claude_home/USER-INSTRUCTIONS.md"
make_sentinel "$claude_sentinel"
run_logged claude-validate env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin validate "$packages/claude"
run_logged claude-marketplace env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin marketplace add "$packages/claude" --scope user
run_logged claude-install env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin install compass@compass-plugins --scope user --yes
require_sentinel "$claude_sentinel"
run_logged claude-list env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin list
require_log_contains claude-list compass
claude_plugin_root="$(installed_plugin_root "$claude_home/plugins/cache" ".claude-plugin/plugin.json")"
verify_installed_skills claude-installed-skills "$claude_plugin_root" "$packages/claude"
run_logged claude-skill-discovery env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin details compass@compass-plugins
require_skills_in_log claude-skill-discovery
run_logged claude-mcp-list env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" mcp list
require_log_contains claude-mcp-list compass
run_logged claude-upgrade env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin update compass@compass-plugins --scope user --yes
require_sentinel "$claude_sentinel"
run_logged claude-uninstall env CLAUDE_CONFIG_DIR="$claude_home" HOME="$state/user" \
  "$claude_binary" plugin uninstall compass@compass-plugins --scope user --yes
require_sentinel "$claude_sentinel"

opencode_project="$state/opencode-project"
opencode_config="$state/opencode-config"
mkdir -p "$opencode_project" "$opencode_config"
opencode_sentinel="$opencode_project/USER-INSTRUCTIONS.md"
make_sentinel "$opencode_sentinel"
cp "$packages/opencode/opencode.json" "$opencode_project/opencode.json"
run_logged opencode-pack npm pack "$packages/opencode" --pack-destination "$scratch" --json
tarball=""
tarball_count=0
while IFS= read -r candidate; do
  tarball="$(basename "$candidate")"
  tarball_count=$((tarball_count + 1))
done < <(find "$scratch" -maxdepth 1 -type f -name '*.tgz' -print | sort)
if [[ "$tarball_count" -ne 1 ]]; then
  echo "error: expected one packed OpenCode artifact, observed $tarball_count" >&2
  exit 1
fi
run_logged opencode-install npm install --ignore-scripts --prefix "$opencode_project" "$scratch/$tarball"
require_sentinel "$opencode_sentinel"
opencode_plugin_root="$opencode_project/node_modules/@compass/opencode-plugin"
verify_installed_skills opencode-installed-skills "$opencode_plugin_root" "$packages/opencode"
(
  cd "$opencode_project"
  run_logged opencode-mcp-list env OPENCODE_CONFIG_DIR="$opencode_config" HOME="$state/user" \
    "$opencode_binary" mcp list
  run_logged opencode-skill-discovery env OPENCODE_CONFIG_DIR="$opencode_config" HOME="$state/user" \
    "$opencode_binary" debug skill
)
require_log_contains opencode-mcp-list compass
require_skills_in_log opencode-skill-discovery
run_logged opencode-plugin-load node --input-type=module - "$opencode_project" <<'NODE'
import { pathToFileURL } from "node:url"
import path from "node:path"
const project = process.argv[2]
if (typeof project !== "string" || project.length === 0) throw new Error("missing project path")
const pluginPath = path.join(project, "node_modules", "@compass", "opencode-plugin", "src", "index.js")
const plugin = (await import(pathToFileURL(pluginPath).href)).default
const hooks = await plugin({ $: () => { throw new Error("unexpected subprocess") } })
const rendered = await hooks.tool.compass_mcp_config.execute({ transport: "http" }, {})
const parsed = JSON.parse(rendered)
if (parsed.mcp?.compass?.url !== "http://127.0.0.1:8080/mcp") process.exit(1)
NODE
run_logged opencode-upgrade npm install --ignore-scripts --force --prefix "$opencode_project" "$scratch/$tarball"
require_sentinel "$opencode_sentinel"
run_logged opencode-uninstall npm uninstall --ignore-scripts --prefix "$opencode_project" @compass/opencode-plugin
require_sentinel "$opencode_sentinel"

echo "qualified installed Codex $actual_codex_version, Claude Code $actual_claude_version, and OpenCode $actual_opencode_version packages"
