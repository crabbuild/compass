# Operate Compass

This guide covers the long-running and optional operational surfaces around the
core build/query loop: watch mode, MCP serving, hooks, providers, global
registries, ingestion, PR workflows, and exports.

> **Who this guide is for:** maintainers, platform engineers, editor/assistant
> integrators, and developers running Compass beyond one-shot local commands.
>
> **You will learn:** lifecycle, safety boundaries, observability, and recovery
> patterns for operational commands.
>
> **Prerequisites:** [Getting started](../getting-started.md) and a working
> `compass update .`.
>
> **Reading time:** 15–20 minutes.

## Operational principle: start one-shot, then automate

Before enabling a watcher, hook, service, or provider-backed workflow:

1. run the equivalent one-shot command;
2. confirm inputs and outputs;
3. record expected exit behavior;
4. decide where logs and credentials live;
5. add lifecycle management and resource limits;
6. test shutdown and failure recovery.

Automation should reproduce known behavior, not become the first place you
discover configuration.

## Watch mode

Start:

```bash
compass watch .
```

Current options include:

```text
--debounce SECONDS
--out DIR
--no-cluster
--no-viz
--no-gitignore
--exclude PATTERN
--poll
```

Watch mode observes relevant changes, debounces bursts, and refreshes the
output. Choose polling only when native filesystem events are unavailable or
unreliable.

### Good use cases

- local editor sessions;
- live architecture exploration;
- a development container with an explicit process supervisor.

### Avoid

- starting multiple watchers for the same output directory;
- using watch as an unbounded CI step;
- watching dependency caches or generated directories;
- assuming the output is current after the watcher has failed.

### Operate it

Capture logs and stop it through your process supervisor or foreground terminal.
After a failure, run one manual update:

```bash
compass update .
```

That separates watcher/event problems from extraction problems.

## MCP service

Start by inspecting:

```bash
compass serve --help
```

Use stdio for a local assistant when possible. It has a smaller exposure
surface than a listening HTTP service.

For HTTP:

- bind explicitly;
- require the supported authentication mechanism;
- limit request and graph sizes;
- do not expose a source graph to untrusted tenants;
- keep provider secrets out of service arguments and logs;
- health-check through a real MCP client;
- stop accepting requests before replacing or removing graph files.

Compass's MCP implementation is tested against an official client oracle for
its tools, resources, transports, authentication, and limits. Your deployment
still owns TLS termination, process isolation, and network access policy.

## Git hooks

Inspect status:

```bash
compass hook status
```

Install managed refresh hooks:

```bash
compass hook install
```

Remove managed sections:

```bash
compass hook uninstall
```

The hook integration:

- launches background refresh work after relevant changes;
- avoids recursive rebuilds;
- guards rebases, merges, cherry-picks, and linked-worktree behavior;
- can enqueue exact history commits where history is enabled;
- writes logs to its configured/default cache path.

Use `GRAPHIFY_SKIP_HOOK=1` only as the documented compatibility control for a
specific operation. Do not permanently mask failing hooks without fixing or
uninstalling them.

When moving or reinstalling the Compass binary, reinstall hooks so embedded
invocation paths remain valid.

## Semantic providers

Built-in backends and custom OpenAI-compatible providers are configured
explicitly.

Provider registry lifecycle:

```bash
compass provider list
compass provider add NAME \
  --base-url URL \
  --default-model MODEL \
  --env-key ENVIRONMENT_VARIABLE_NAME
compass provider show NAME
compass provider remove NAME
```

The registry stores provider metadata and the *name* of the environment
variable containing a key—not the secret value itself.

Before sending a corpus:

- identify the exact endpoint;
- verify TLS and organizational approval;
- understand retention and training terms;
- set the key in process environment or an approved secret store;
- use a small non-sensitive corpus for the first call;
- set timeouts and concurrency;
- decide whether partial semantic results are permitted.

An Ollama-compatible endpoint on a non-loopback host is a network transfer even
if the product is commonly described as local. Compass warns about unexpected
or metadata-address endpoints; deployment policy should fail closed too.

## Build with a provider

Example shape:

```bash
compass extract . \
  --backend openai \
  --model your-approved-model \
  --max-concurrency 4 \
  --api-timeout 60
```

Use `compass extract --help` for the exact backend and limit surface. Missing
credentials should cause a clear failure. Use `--code-only` when your intent is
to skip semantic sources entirely.

`--allow-partial` changes completeness expectations and should never be added
to automation without deciding how partial status is surfaced to consumers.

## Global graph registry

Register:

```bash
compass global add path/to/graph.json --as repository-tag
```

Inspect:

```bash
compass global list
compass global path
```

Remove:

```bash
compass global remove repository-tag
```

Operational ownership questions:

- who refreshes the registered graph?
- how is its source revision recorded?
- what happens when a repository moves?
- are tags unique and stable?
- who removes stale entries?

Treat the registry as an index, not an automatic freshness guarantee.

## Remote ingestion and cloning

Compass exposes native project workflows including:

```bash
compass clone https://github.com/OWNER/REPOSITORY \
  --branch BRANCH \
  --out DIRECTORY

compass add URL --author "Name" --dir ./raw
```

These commands cause network and filesystem changes. In automation:

- validate the host and URL;
- use a dedicated output directory;
- avoid embedding credentials in URLs;
- set repository size and trust policies externally;
- scan or sandbox untrusted checkout content;
- record source URL and revision.

Do not assume cloned content is safe to build or execute.

## Pull-request intelligence

The `prs` command can inspect repository PR context:

```bash
compass prs --triage --repo OWNER/REPO
```

Optional modes include worktree, conflict, wrong-base, base-branch, repository,
and graph controls. Run `compass prs --help` for the current contract.

PR workflows may require GitHub credentials and network access. Keep this
separate from the fully local structural graph claim.

## Graph database export

Inspect:

```bash
compass export --help
```

Before writing to Neo4j or FalkorDB:

- confirm endpoint, database/graph, and write semantics;
- supply passwords through supported environment variables;
- test against a disposable target;
- verify counts and a known path after export;
- retain source `graph.json` and revision;
- plan idempotence and rollback.

The existence of a native exporter does not make the remote database local.

## Merge and maintenance commands

Compass exposes lower-level operations such as:

```text
diagnose multigraph
merge-graphs
merge-driver
merge-chunks
merge-semantic
cache-check
cluster-only
label
save-result
reflect
check-update
```

Use them when a guide or command help identifies the correct workflow. Several
are pipeline/integration surfaces, not everyday first-run commands.

Rules:

- keep original inputs until output validates;
- write to a new path where the command supports it;
- inspect multigraph direction and duplicate-edge diagnostics before merging;
- do not hand-edit incremental caches to recover them;
- treat semantic merge completeness as a publication boundary.

## Logs and observability

For any long-running operation, record:

```text
Compass version
working directory
resolved source revision
command with secrets removed
output directory
profile/provider name (not key)
start/end time and exit status
artifact counts or hashes
diagnostic codes
```

Do not log:

- API keys;
- authorization headers;
- full secret-bearing URLs;
- query parameter values that your application classifies as sensitive;
- source content merely to make debugging easier.

## Recovery ladder

Use the smallest recovery that matches the failure:

```text
query problem
  -> rerun exact query with smaller scope / inspect diagnostic

stale current graph
  -> compass update .

watcher problem
  -> stop watcher, run one-shot update, restart deliberately

provider problem
  -> verify endpoint/key/model on a small source; do not publish incomplete result

hook problem
  -> inspect status/log, uninstall managed hook, verify manual update

history problem
  -> use history status/list/show/validate/rebuild flows

output corruption
  -> retain evidence, rebuild to a new location, compare before replacement
```

Avoid deleting a live SQLite/WAL store, lock, or lease as a first response.

## Related pages

- [Integrating Compass](integrating-compass.md)
- [Versioned graph history](versioned-history.md)
- [Security and privacy](../design/security-and-privacy.md)
- [Troubleshooting cookbook](../cookbook/troubleshooting.md)

**Next step:** choose one operational surface, run its one-shot equivalent, and
write down lifecycle, credential, log, and recovery ownership before enabling
automation.
