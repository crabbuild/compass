# Configuration reference

Compass configuration comes from explicit command options, environment
variables, provider registries, repository history configuration, and generated
integration files. This page explains ownership and safe use.

## Precedence rule

For a specific command, use:

```text
explicit CLI option
    before
documented environment fallback
    before
stored provider/repository configuration
    before
built-in default
```

Not every option follows one universal resolver. The command's help and source
remain authoritative. In automation, prefer explicit non-secret options and
record them.

## Architecture overlay

Architecture exports work without configuration. When repository-owned domain
vocabulary is more precise than automatic path and declaration evidence, pass
a bounded overlay explicitly:

```bash
compass export callflow-html \
  --architecture-overlay .compass/architecture.toml
```

The first schema is `compass.architecture-overlay/1`. It supports source-scope
rules and named groups selected by normalized path prefixes or numeric
community IDs. IDs and selectors must be unique; empty or overlapping path
prefixes and communities claimed by multiple groups fail before output is
published. Overlay names affect presentation, while stable group identity and
graph relationships remain separate. Source-rule values are `production`,
`test`, `generated`, `vendor`, `documentation`, and `unknown`.

```toml
schema = "compass.architecture-overlay/1"

[[sourceRules]]
pathPrefix = "generated/client"
scope = "generated"

[[groups]]
id = "billing-ledger"
name = "Billing Ledger"
pathPrefixes = ["crates/billing"]
pin = true
```

For the canonical current-project artifact
`<project>/compass-out/graph.json`, Compass discovers
`<project>/.compass/architecture.toml`. Arbitrary or historical graph paths do
not inspect a live checkout; pass their matching overlay explicitly with
`--architecture-overlay`. An explicit option always wins. `--sections` is a
deprecated compatibility spelling; legacy JSON sections are converted to
overlay community selectors.

## Output root

Default:

```text
compass-out/
```

Several command families honor:

```bash
COMPASS_OUT=custom-output compass update .
```

Where available, `--out DIR` is clearer:

```bash
compass update . --out custom-output
```

Do not point two concurrent writers at one output directory.

Successful builds materialize `graph.json`, `manifest.json`,
`GRAPH_REPORT.md`, and optional `graph.html` directly under this root. These
stable paths are the portable integration surface. Visible snapshot
directories, store references, and the encoding beneath `cache/` remain
Compass-owned operational state. Their visible names disclose what Compass
created; consumers should still use documented commands and stable root
artifacts instead of parsing implementation files directly.

## Compass Store configuration

The local build publishes `graph.json` under the selected output root. SQLite
query storage is enabled by default; passing `--store json` opts out. A SQLite
build additionally publishes:

```text
DIR/store/store.sqlite3
DIR/snapshots/<active>/store.ref
```

`DIR` is `compass-out/` by default and can be set with `--out DIR` or the
documented `COMPASS_OUT` fallback. The CLI's default build and query engine
uses the validated sidecar when it is present, falling back to JSON for
output-only builds. `--engine json` forces the portable reader and
`--engine store` requires the validated sidecar for a typed query. The
`compass-store-redb` crate is a separate
library adapter and is not a CLI setting. PostgreSQL and DynamoDB are deferred
service adapters, so no endpoint, credential, TLS, or cloud SDK configuration
is read by local store commands.

SQLite uses one shared local WAL-backed file and checkpoints it before
publication and backup; complete snapshots are not database copies. Do not
run two writers against one output root. JSON query indexes beneath the cache
root are disposable and may be deleted; the output root, including the shared
database and snapshot references, must be kept together. Use
`compass store status|validate` for
health, `compass store backup` for a digest-bound copy, and `compass store
restore` into a new directory for recovery. The store API enforces bounded
namespace, partition, key, value, transaction, scan, and graph sizes. Local
publication retains and collects two complete snapshots; distributed leases
and hosted quotas are deferred. Local disk availability remains an operational
limit. See the [operations guide](../guides/operations.md)
for the support window and rebuild procedure.

## Build configuration

Initialize a reviewable repository scope with:

```bash
compass init . --include src --exclude '**/generated/**' --yes
```

### Inference policy

Select graph breadth per build with:

```bash
compass update . --inference-level medium
```

Supported values are `low`, `medium`, `high`, and `max`; the default is `low`.
The option is available on `init`, `update`, `extract`, and `watch`. Use
explicit `max` when the former complete-inference breadth is required. It is a
build-profile input, so changing it causes a coherent republish even when
source files are unchanged. Extraction caches keep the complete normalized
evidence and can be reused across levels.

Compass writes:

```toml
version = 1

[build]
include = ["src/"]
exclude = ["**/generated/**"]
```

An empty include list means the whole eligible repository. Paths are
project-root-relative; absolute paths and root escapes are rejected.
`update`, `extract`, and `watch` load this file automatically. Filtering is
applied as built-in safety skips, Git ignores, configured includes, configured
excludes, then command-line exclusions. Invalid configuration stops the build
instead of silently widening its scope.

`vendor/` is deliberately not a built-in skip. Vendored directories can contain
real Go source and can also be explicit workspace members, as Compass's vendored
parser pack is. A language-neutral discovery layer cannot infer that this source
is disposable from the directory name alone. Repositories that do not want
vendored source in a graph should exclude it explicitly and reviewably:

```bash
compass init . --exclude 'vendor/**' --yes
```

The same pattern may be placed in `.compassignore`. Keeping this opt-out explicit
preserves existing discovery behavior and applies consistently to builds and
filesystem watching.

Common explicit options:

| Concern | Options |
| --- | --- |
| scope | positional `PATH`, `--exclude PATTERN` |
| ignore | default Git ignore or `--no-gitignore` |
| rebuild | `--force` |
| outputs | `--out`, `--no-viz`, `--no-cluster`, `--no-program` |
| analysis | `--resolution`, `--exclude-hubs` |
| code metadata | `--cargo`, `--postgres`, `--google-workspace` |
| semantics | `--code-only`, `--backend`, `--model`, `--mode` |
| resources | `--token-budget`, `--max-workers`, `--max-concurrency`, `--api-timeout` |
| completeness | `--allow-partial` |

`--code-only` is an explicit semantic choice, not merely a performance flag.

## Provider environment families

Current built-in backend code recognizes families including:

| Backend | Key variables | Endpoint/model examples |
| --- | --- | --- |
| Anthropic/Claude | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL` |
| Kimi/Moonshot | `MOONSHOT_API_KEY` | `KIMI_BASE_URL` |
| Gemini | `GEMINI_API_KEY`, `GOOGLE_API_KEY` | `GEMINI_BASE_URL`, `COMPASS_GEMINI_MODEL` |
| OpenAI | `OPENAI_API_KEY` | `OPENAI_BASE_URL`, `OPENAI_MODEL`, `COMPASS_OPENAI_MODEL` |
| DeepSeek | `DEEPSEEK_API_KEY` | `DEEPSEEK_BASE_URL`, `COMPASS_DEEPSEEK_MODEL` |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` | `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_API_VERSION`, `AZURE_OPENAI_DEPLOYMENT` |
| Ollama-compatible | optional `OLLAMA_API_KEY` | `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| Bedrock | AWS credential chain | `COMPASS_BEDROCK_MODEL` |
| Claude CLI | local `claude` login | `COMPASS_CLAUDE_CLI_MODEL` |

Some Compass variables retain `COMPASS_` names. Their presence does not
change the public executable name.

Choose a provider explicitly when more than one credential is present:

```bash
COMPASS_BACKEND=openai OPENAI_API_KEY="$OPENAI_API_KEY" \
  COMPASS_MODEL=gpt-4.1-mini compass extract .
```

`--backend` and `--model` take precedence over `COMPASS_BACKEND` and
`COMPASS_MODEL`. If neither is supplied, Compass detects the first configured
built-in provider in this order: Gemini, Kimi, Claude, OpenAI, DeepSeek, Azure,
Bedrock, then Ollama; custom providers are considered after built-ins. The
selector and model are not secrets; `COMPASS_MODEL` also takes precedence over
a provider-specific model variable. Provider credentials remain in the
provider-specific environment variable or secret store and are never written
to graph artifacts or history profiles. Use `compass extract --help` and
current provider documentation before deployment; backend support and model
defaults can evolve.

## Custom provider registry

Add:

```bash
compass provider add internal \
  --base-url https://models.example.test/v1 \
  --default-model approved-model \
  --env-key INTERNAL_MODEL_API_KEY
```

The registry stores:

```json
{
  "internal": {
    "base_url": "https://models.example.test/v1",
    "default_model": "approved-model",
    "env_key": "INTERNAL_MODEL_API_KEY",
    "pricing": {"input": 0.0, "output": 0.0},
    "temperature": 0
  }
}
```

It stores the environment-variable name, not its secret value. The
compatibility registry path is under the user's Compass config
directory (`~/.compass/providers.json` on common Unix setups).

Inspect:

```bash
compass provider list
compass provider show internal
```

Use the registered provider without putting its secret in the registry:

```bash
COMPASS_BACKEND=internal INTERNAL_MODEL_API_KEY="$INTERNAL_MODEL_API_KEY" \
  compass extract .
```

Remove:

```bash
compass provider remove internal
```

Unsafe endpoints are rejected or warned according to endpoint checks.

## Credential rules

```text
Do:
  inject secrets through approved environment/secret stores
  scope keys to the provider and environment
  redact logs
  rotate exposed keys

Do not:
  commit .env files with keys
  pass keys as query parameters
  put keys in Git remote/URL strings
  include keys in history profiles or docs
  print environment values for diagnosis
```

History fingerprints include meaning-affecting provider/model configuration but
exclude credential values.

## Semantic concurrency and timeout

Use explicit bounds:

```bash
compass extract . \
  --backend internal \
  --model approved-model \
  --max-concurrency 4 \
  --api-timeout 60 \
  --token-budget 200000
```

Lower concurrency when provider rate limits or corpus sensitivity demand it.
`--allow-partial` changes the completeness contract and should be recorded in
automation.

Environment variables for Ollama parallelism/context may exist
in the current source, including `COMPASS_OLLAMA_PARALLEL`,
`COMPASS_OLLAMA_NUM_CTX`, and `COMPASS_OLLAMA_KEEP_ALIVE`. Prefer documented
CLI options when available; treat Compass variables as exact,
version-specific interfaces.

## History configuration

```bash
compass history enable --code-only
```

or:

```bash
compass history enable \
  --backend internal \
  --model approved-model \
  --exclude 'vendor/**' \
  --cargo
```

The stored repository profile governs eager and lazy historical
materialization. Disable:

```bash
compass history disable
```

This stops eager enqueueing but preserves data and explicit/lazy history
commands.

Do not edit history configuration or preferred pointers by hand.

## Query configuration

Natural-language discovery:

```text
--dfs
--context VALUE
--direction auto|incoming|outgoing|both
--scope KIND:VALUE
--text-budget N
--cursor TOKEN
--graph PATH | --at REV
--max-nodes N
--max-edges N
```

`--context VALUE` filters stored relationship evidence contexts such as `call`,
`import`, or `route`; it does not scope retrieval to a node, file, package,
community, or subsystem. Use repeatable `--scope KIND:VALUE` for an explicit OR
scope over `community`, `source`, `package`, or `node`.

`--text-budget` controls approximate rendered tokens per discovery page
(default 2,000). Follow the opaque `next` cursor with the same semantic query;
the presentation-only text budget may change. `--traverse`, `--budget`, and
`--page` explicitly select the bounded legacy compatibility renderer.
The default semantic neighborhood contains at most 64 nodes and 128 edges.
`--max-nodes` and `--max-edges` may raise those bounds to the hard ceilings of
500 nodes and 1,000 edges when a wider response is intentional.

CompassQL:

```text
--param NAME=VALUE
--params-file PATH
--format table|json|jsonl
--output PATH
--timeout-ms N
--max-rows N
--max-path-depth N
--max-expanded-relationships N
--max-memory-bytes N
```

Query limits are per invocation and part of the result contract.

## MCP configuration

The current service surface includes:

```text
--transport stdio|http
--host HOST
--port PORT
--api-key KEY
--path PATH
--json-response
--stateless
--session-timeout SECONDS
```

Avoid literal API keys in command history. Bind to loopback for local use and
use stdio when one local client is sufficient. HTTP is always stateless MCP
2026-07-28 and does not use `Mcp-Session-Id`; `--stateless` remains accepted for
script compatibility. `--session-timeout` is deprecated, validated and ignored
with a warning through 0.4.x, and is removed in 0.5.0.

## Graph database configuration

Native exporters support Neo4j/FalkorDB connection information. Current code
recognizes password environment variables including:

```text
NEO4J_PASSWORD
FALKORDB_PASSWORD
```

Use command help for URI/user/database/graph options. Confirm target and write
semantics before export.

## Hook configuration

Managed hooks recognize controls such as:

```text
COMPASS_SKIP_HOOK
COMPASS_REBUILD_LOG
```

The output root can be influenced by `COMPASS_OUT`.

Strict assistant hook mode uses:

```text
COMPASS_HOOK_STRICT
```

Reinstall hooks after moving/upgrading the binary so embedded invocation paths
remain correct.

Codex project hooks are not active until their exact definition is reviewed and
trusted in Codex. After `compass install --project --platform codex`, use
`/hooks` to review the bounded `hook-guard search` command. New or changed hook
content requires review again.

## Assistant configuration

```bash
compass install --platform codex
compass install --project --platform codex
```

Project scope writes reviewable repository files. Global scope writes
platform-specific user configuration. The platform list and exact destinations
come from `compass install --help`.

## Agent Graph MCP configuration

Agent Graph tools are opt-in server configuration:

```text
--agent-graph-project PATH       repeatable canonical project allowlist
--agent-graph-principal ID      trusted owner identity
--agent-graph-state-root PATH   explicit state for one non-Git project
--agent-graph-writes            advertise and authorize write batches
--agent-graph-masks             additionally authorize curated masks
--write-api-key KEY             HTTP write credential, distinct from --api-key
```

Setting a state root for multiple projects is rejected. Masks require writes.
HTTP write startup requires non-empty, distinct read and write keys; the same
values may be supplied through `COMPASS_API_KEY` and
`COMPASS_WRITE_API_KEY`. Stdio still requires explicit write enablement but
does not create a network credential boundary.

## Reproducibility record

For a reproducible job, record:

```text
Compass version
source commit / dirty state
root and excludes
code-only or semantic profile
provider/model name (not key)
analysis and output options
query limits and schema version
history realization/fingerprint where applicable
```

Environment-only configuration that affects meaning must be captured in your
job metadata even if Compass's own history fingerprint already includes it.

## Related pages

- [Command reference](commands.md)
- [Operations guide](../guides/operations.md)
- [Assistant setup](../guides/assistant-setup.md)
- [Versioned history](../guides/versioned-history.md)

**Next step:** replace implicit defaults in one automation workflow with
explicit non-secret options and record the selected profile/version.
