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

## Compass Store configuration

The local build publishes `graph.json` under the selected output root. Passing
`--store sqlite` additionally publishes:

```text
DIR/.compass-store/compass-store.sqlite3
DIR/.compass-generations/<active>/store.ref
```

`DIR` is `compass-out/` by default and can be set with `--out DIR` or the
documented `COMPASS_OUT` fallback. The CLI's default build and query engine is
JSON. `--store sqlite` opts a build into the local sidecar, and
`--engine store` requires that validated sidecar for a typed query. The
`compass-store-redb` crate is a separate
library adapter and is not a CLI setting. PostgreSQL and DynamoDB are deferred
service adapters, so no endpoint, credential, TLS, or cloud SDK configuration
is read by local store commands.

SQLite uses one shared local WAL-backed file and checkpoints it before
publication and backup; complete generations are not database copies. Do not
run two writers against one output root. JSON query indexes beneath the cache
root are disposable and may be deleted; the output root, including the shared
database and generation references, must be kept together. Use
`compass store status|validate` for
health, `compass store backup` for a digest-bound copy, and `compass store
restore` into a new directory for recovery. The store API enforces bounded
namespace, partition, key, value, transaction, scan, and graph sizes. Local
publication retains and collects two complete generations; distributed leases
and hosted quotas are deferred. Local disk availability remains an operational
limit. See the [store operations guide](../guides/compass-store-operations.md)
for the support window and rebuild procedure.

## Build configuration

Initialize a reviewable repository scope with:

```bash
compass init . --include src --exclude '**/generated/**' --yes
```

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
| Gemini | `GEMINI_API_KEY`, `GOOGLE_API_KEY` | `GEMINI_BASE_URL`, `COMPASS_GEMINI_MODEL` |
| OpenAI | `OPENAI_API_KEY` | `OPENAI_BASE_URL`, `OPENAI_MODEL`, `COMPASS_OPENAI_MODEL` |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` | `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_API_VERSION`, `AZURE_OPENAI_DEPLOYMENT` |
| Ollama-compatible | optional `OLLAMA_API_KEY` | `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| Bedrock | AWS credential chain | `COMPASS_BEDROCK_MODEL` |

Some Compass variables retain `COMPASS_` names. Their presence does not
change the public executable name.

Use `compass extract --help` and current provider documentation before
deployment; backend support and model defaults can evolve.

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
--budget N
--graph PATH | --at REV
```

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
use stdio when one local client is sufficient.

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
