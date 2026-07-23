# Security and privacy design

Compass analyzes source repositories and can optionally contact external
systems. Its security model begins by separating fully local structural work
from explicit network, credential, subprocess, and historical-checkout
boundaries.

> **Who this page is for:** security reviewers, operators, integrators, and
> contributors changing an input or network boundary.
>
> **You will learn:** what stays local, when data can leave the machine, how
> untrusted input is bounded, and which operational practices remain the
> deployer's responsibility.
>
> **Prerequisites:** [Design principles](principles.md).
>
> **Reading time:** 12–15 minutes.

> This page explains architecture. The authoritative vulnerability-reporting
> and supported-version policy is [SECURITY.md](../../SECURITY.md).

## Trust-boundary map

```text
Repository files
     |
     v
Local structural pipeline -----------------------+
  parsers · resolution · graph · local queries   |
     |                                            |
     v                                            |
compass-out/ and local history                    |
                                                  |
Explicit optional boundaries                      |
  +--> semantic provider endpoint                 |
  +--> GitHub / remote clone / URL ingestion      |
  +--> PostgreSQL / Google Workspace              |
  +--> Neo4j / FalkorDB                           |
  +--> MCP HTTP clients                           |
  `--> bounded helper subprocesses                |
```

The default code-only graph path does not need a network or model key.

## Data that remains local by default

For structural source analysis:

- file discovery happens locally;
- tree-sitter parsing happens locally;
- language extraction and resolution happen locally;
- graph construction, clustering, reporting, and querying happen locally;
- outputs are written to local `compass-out/`;
- no embedding store is created;
- Python is not launched by the released executable.

Your operating environment can still copy, back up, index, or monitor these
files. “Local” describes Compass's data path, not the entire host.

## When data can leave the machine

### Semantic providers

When a semantic backend is configured, supported document/media content or
derived chunks can be sent to that provider.

Before use:

- approve endpoint and provider;
- understand retention and training policy;
- scope the corpus;
- configure timeouts and concurrency;
- keep secrets in an approved store/environment;
- decide how partial results are handled.

An Ollama-compatible URL is local only when it actually points to an approved
local endpoint. Compass checks URL schemes, warns on non-loopback transfer, and
rejects link-local/metadata targets in relevant paths.

### Remote ingestion and cloning

`compass add` and `compass clone` fetch remote content. URL ingestion is
bounded and SSRF-resistant, but fetched content remains untrusted.

Do not automatically execute build scripts from an untrusted clone. Use a
dedicated directory and sandbox according to your organization.

### External services

GitHub PR workflows, PostgreSQL introspection, Google Workspace export, and
Neo4j/FalkorDB push all cross explicit service boundaries. Consult command help
and network policy.

### MCP HTTP

HTTP service mode can expose graph and source-location information. Bind
narrowly, authenticate, limit requests, and terminate TLS appropriately.
Prefer stdio for a single local assistant.

## Credentials

Built-in providers use documented environment variables. Custom provider
metadata stores an environment-variable *name*, not the secret itself.

Rules:

- never pass keys as positional query text;
- never commit keys in provider config, docs, fixtures, or agent instructions;
- redact authorization headers and secret URLs from logs;
- use separate credentials for development and production;
- rotate credentials after accidental disclosure;
- do not include secret values in extraction fingerprints.

History fingerprints include provider/model configuration because it affects
meaning, while excluding credentials because they do not.

## Untrusted source and graph input

Repository content can be adversarial:

- deeply nested syntax;
- oversized files;
- malformed JSON/XML/archives;
- decompression bombs;
- path traversal attempts;
- prompt-injection text;
- malicious URLs;
- huge query expansions;
- invalid graph endpoints.

Compass uses:

- raw and decompressed size caps;
- archive member and compression-ratio limits;
- parser/JSON depth and record limits;
- source and output extension checks;
- canonical/root-bound path handling;
- URL scheme/host/address validation;
- query depth/row/expansion/memory/deadline limits;
- subprocess timeouts and output caps;
- semantic fragment validation and injection-sentinel neutralization.

Limits should fail explicitly. A caller must not reinterpret a limit failure as
empty or complete data.

## Historical checkout threats

Checking out an old Git commit can execute code through hooks, filters, LFS,
submodules, credential helpers, or network fetches if done naively.

Compass historical materialization:

- creates a detached offline worktree;
- does not run hooks;
- rejects external-code checkout filters;
- does not fetch or prompt;
- does not smudge LFS;
- does not recurse submodules;
- reports Gitlinks/LFS pointers as limitations;
- excludes caller-local and global ignore state.

This reduces both nondeterminism and repository-controlled code execution.

## Output sensitivity

`graph.json` and reports can reveal:

- file paths and source locations;
- internal type/function names;
- architecture and dependencies;
- database/schema names;
- document concepts;
- external service relationships;
- potential high-value hubs.

Treat graph artifacts with the same or higher classification as the source
corpus. Do not upload them to a public artifact store merely because they
contain less text than the repository.

HTML and SVG exports must remain self-contained and avoid loading untrusted
external scripts/fonts/resources.

## Atomicity and integrity

An attacker or concurrent process may try to make a consumer read an incomplete
artifact.

Compass:

- writes through atomic helpers;
- validates graph structures;
- uses build guards;
- signs binary cache reuse with graph file metadata;
- validates history realizations before read/export/preference;
- uses SQLite durability and content-addressed roots.

Consumers should:

- wait for a successful producing process;
- open files with least privilege;
- reject unknown structured-output major versions;
- validate hashes/signatures when transferring artifacts;
- prevent untrusted users from replacing a graph path.

## Service and query isolation

CompassQL is read-only and rejects mutations, procedures, `LOAD CSV`, dynamic
execution, and unbounded paths.

That reduces query-driven side effects. It does not replace process isolation:
a service still reads graph files, uses memory/CPU, and may expose sensitive
results.

Set per-request budgets and avoid serving multiple trust domains from one
unpartitioned graph.

## Subprocess boundaries

Some optional workflows use controlled subprocesses such as Git, GitHub CLI,
or `gws`.

Safe patterns include:

- argument arrays rather than shell concatenation;
- explicit timeouts;
- captured-output caps;
- restricted environment;
- validated paths/URLs;
- stable executable discovery;
- no secrets echoed in diagnostics.

When adding a subprocess, test timeout, nonzero exit, oversized output,
malformed UTF-8, and missing executable behavior.

## Threat-informed operating checklist

### Fully local code graph

- [ ] Use `--code-only` when non-code inputs should be excluded.
- [ ] Keep `compass-out/` private like the source.
- [ ] Exclude unneeded generated/vendor directories.
- [ ] Run as a non-privileged user.

### Semantic graph

- [ ] Approve endpoint, model, and retention policy.
- [ ] Store key outside the repository.
- [ ] Test on non-sensitive content.
- [ ] Set time/concurrency/size limits.
- [ ] Surface partial status.

### MCP HTTP

- [ ] Bind to intended interface only.
- [ ] Configure supported authentication.
- [ ] Terminate TLS appropriately.
- [ ] Limit graph/request/result size.
- [ ] Separate trust domains.

### History

- [ ] Back up SQLite and WAL coherently.
- [ ] Do not edit preferred pointers or Prolly keys.
- [ ] Do not copy/delete live resources piecemeal.
- [ ] Use explicit recovery commands.

### External export

- [ ] Confirm target and write semantics.
- [ ] Use environment-provided secrets.
- [ ] Test on disposable target.
- [ ] Verify counts and known paths.

## Vulnerability reporting

Do not open a public issue for a suspected vulnerability. Follow
[SECURITY.md](../../SECURITY.md) for the current supported versions and private
reporting channel.

Include:

- Compass version and platform;
- minimal reproduction;
- trust boundary crossed;
- impact and required preconditions;
- logs with credentials/source content removed;
- whether the issue affects current-tree, history, service, provider, or
  integration paths.

## Related pages

- [Security policy](../../SECURITY.md)
- [Operations](../guides/operations.md)
- [Storage and history](storage-and-history.md)
- [Configuration reference](../reference/configuration.md)

**Next step:** identify which optional boundaries your deployment enables and
complete the matching checklist before processing a sensitive repository.
