# Security and privacy design

Compass analyzes source repositories and can optionally contact external
systems. Its security model begins by separating fully local structural work
from explicit network, credential, subprocess, and historical-checkout
boundaries.

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

## Agent Graph write boundary

Agent Graph writes are denied by default. A trusted adapter mints a bounded,
expiring grant for one canonical repository, overlay, Base Generation,
expected revision, principal, permission set, and mask policy. Change requests
cannot self-assign any of those values. Every batch is validated and Grounded
before immutable objects are published; selector activation is conditional, so
competing writers receive a conflict rather than losing an update.

HTTP requires a distinct write credential in addition to normal API
authentication. Project allowlists are canonicalized at startup, a non-Git
state root may serve only one project, and the write tool is not advertised
when disabled. Audit records contain bounded digests and trusted adapter/model
labels only. Prompts, model responses, chain-of-thought, credentials, tokens,
and source excerpts are outside the audit contract.

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

### Local query-feedback review

Compass does not transmit query telemetry. Operators may separately collect an
approved local JSONL query export and run
`scripts/prepare_query_relevance_review.py` to create a bounded review queue for
relevance qualification. The importer reads at most 16 MiB/10,000 JSONL lines,
applies deterministic best-effort redaction, omits responses and repository
paths, and emits no more than 256 candidates. It makes no network requests and
never modifies the shipped ranker or judgment corpus.

MCP `query_graph` logging remains disabled by default. Setting
`COMPASS_QUERY_LOG` opts into a local `compass.query-log/1` JSONL file capped at
16 MiB; both typed and compatibility-traversal questions are represented.
Typed responses are never logged. Compatibility-traversal response logging is a
separate opt-in and should remain disabled for relevance sampling.

Query text can reveal internal symbols, business terms, paths, identities, or
secrets, and best-effort redaction is not anonymization. Store both raw logs and
review queues outside the repository with source-equivalent access controls,
minimize collection, define a retention period, and require human review before
committing a de-identified case. Do not enable response capture for relevance
sampling.

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

### GitHub PR review and Action delivery

`compass review --base/--head` treats the checkout and Git objects as
untrusted data. It passes arguments separately, bounds Git duration/output,
uses `git merge-tree`, writes only a deterministic synthetic commit object, and
does not switch branches, fetch, run hooks, load submodules, invoke package
managers, or execute repository code. GitHub mode additionally uses bounded
`gh api` metadata and file pagination, then rejects revision drift.

The reusable Action downloads one exact release and checksum from its fixed
release repository, verifies both checksum and archive layout, and analyzes
without placing the comment token in the analysis environment. Artifact and
job summary publication precede comment delivery. The token is passed only to
the pinned delivery step, which validates report schema, digest, exact
repository/PR/revision identity, and comment size before bounded API calls.

Use the Action in a dedicated job that has not executed contributor-controlled
scripts or binaries. Merely isolating the token in a later step does not make a
runner safe after untrusted code has run; a background process could survive
between steps. Keep tests/builds in a separate read-only job. Fork PRs never
receive comment delivery, even when a token input is present. Do not use
`pull_request_target` to check out or execute a contributor head.

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

Markdown frontmatter is untrusted source. Compass validates it within byte,
key, item, depth, and graph-node budgets; rejects YAML aliases and tags; and
requires parser-backed source ranges before publication. Public ConfigKey
labels include values only for a conservative set of content metadata such as
title, tags, aliases, authors, dates, layout, and status. Generic values,
including credential-shaped fields, remain out of graph artifacts. This is a
disclosure reduction, not permission to store credentials in frontmatter.

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

## Local document and OCR boundary

PDF and Office bytes, ZIP members, XML, raster dimensions, OCR observations,
downloaded model bytes, and document-cache JSON are untrusted. Compass checks
raw and expanded sizes before allocation and keeps raw file/cache reads bounded
at the stream even if a file grows after its metadata check. It rejects archive
traversal, unsafe relationships, unknown cache fields, and incoherent OCR
origin, profile, completeness, or geometry. Formulas/macros/OLE stay inert,
document URLs are never followed, and every OCR request/result is validated
against the normalized raster.

OCR extraction is network-disabled by construction. `compass models install`
is the sole download command; it uses fixed HTTPS release hosts, at most three
validated redirects, immutable `v0.7.0` artifact names, exact sizes and
SHA-256, temporary same-directory files, and atomic publication with parent
directory synchronization. A bounded per-profile lock serializes concurrent
installers. Symlinked model artifacts, verified markers, and install locks are
rejected. Extraction, inspection, watch, and history verify local artifacts and
fail explicitly when a selected profile is absent. They never downgrade to
native-only processing after OCR was requested.

The ONNX runtime and pure-Rust PDF renderer are linked into Compass. Users do
not configure an executable or install Python, Tesseract, an office suite,
Poppler, Java, or a system ONNX library. Native text remains authoritative;
model output is separately identified derived evidence and cannot execute or
replace source content.

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

### GitHub PR review

- [ ] Pin checkout and Compass Action references to reviewed full commit SHAs.
- [ ] Give the review job `contents: read` and only the comment permission it needs.
- [ ] Do not execute contributor-controlled code in the token-bearing review job.
- [ ] Keep `fail-on` at `none` or `deterministic`; advisory risk is not policy.
- [ ] Protect report, SARIF, Markdown, and logs like source artifacts.

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
