# Extending Compass

This guide turns common extension goals into concrete crate, contract, and
verification checklists.

> **Who this guide is for:** contributors adding a language, relation,
> integration, query capability, output, command, or provider.
>
> **You will learn:** where an extension belongs, which public contracts it can
> affect, and what evidence is required before it is complete.
>
> **Prerequisites:** [Workspace tour](workspace-tour.md) and
> [Contributing](../../CONTRIBUTING.md).
>
> **Reading time:** 15 minutes.

## Before editing

1. Read the relevant crate `src/lib.rs`.
2. Find the nearest existing implementation.
3. Find its unit and CLI/integration tests.
4. Read [COMPATIBILITY.md](../../COMPATIBILITY.md) for the affected command
   family.
5. Decide whether the change is:
   - compatibility behavior;
   - intentional divergence with migration impact;
   - Compass-native behavior.
6. Define direction, provenance, limits, failure, and output contract.

## Add a language or file format

### Ownership

```text
classification/extension        compass-files + language registry
parser/query selection          vendored language pack
per-file facts                  compass-languages
cross-file target resolution    compass-resolve
graph/result verification       compass-graph / CLI fixtures
```

### Implementation checklist

- add deterministic extension/filename recognition;
- define parser/language spec;
- extract file/module and supported entities;
- emit containment and dependency relations;
- include source file/location;
- emit unresolved raw call/member facts when needed;
- add project/config metadata support if resolution needs it;
- implement resolver logic conservatively;
- preserve ambiguous targets;
- update coverage/support documentation.

### Evidence

- minimal syntax fixture;
- nested scopes;
- imports and cross-file calls;
- duplicate names;
- inheritance/members;
- malformed source;
- ignored/generated paths;
- stable IDs and ordering;
- direction, relation, context, provenance;
- incremental edit/rename/delete;
- multilingual corpus qualification.

## Add a relation

Define:

```text
name          normalized persisted string
direction     what source and target mean
context       call/import/declaration/etc.
provenance    direct, resolved, ambiguous
multiplicity  whether parallel edges are meaningful
impact        whether affected should traverse it
CompassQL     normalized relationship type
```

Do not reuse `uses` when a more precise stable relation is justified, and do
not introduce a narrowly named relation with no consumer or documentation.

Update:

- extractor/resolver;
- graph dedup key if appropriate;
- affected default relations only with impact justification;
- reports/renderers;
- query/CompassQL fixtures;
- output reference.

## Add a CompassQL feature

Work through the full vertical slice:

1. token and span;
2. AST;
3. parser with precise diagnostic;
4. semantic scope/type validation;
5. logical operator/value;
6. optimizer rule if beneficial;
7. bounded executor;
8. explain/profile representation;
9. JSON/JSONL typed result;
10. support matrix and language version decision.

Evidence includes unit syntax cases, negative diagnostics, scope/type cases,
TCK feature, CLI source modes, limits/cancellation, differential behavior where
portable, and benchmark regression.

Mutation and unbounded execution remain outside CompassQL's read-only design.

## Add a CLI command

Keep the binary entry small. Add command parsing/rendering in a focused
`compass-cli` module and put reusable behavior in the owning crate.

Define before implementation:

- usage string and examples;
- positional/optional argument grammar;
- mutual exclusions and defaults;
- stdout versus stderr;
- text and machine formats;
- exit codes;
- filesystem/network side effects;
- idempotence and cleanup;
- help visibility and compatibility status.

CLI tests should execute the binary boundary and assert:

- help;
- success output;
- usage errors;
- runtime errors;
- no partial file on failure;
- project/global path behavior;
- JSON schema/version;
- platform path/encoding where relevant.

## Add an output format

Add to `compass-output` when it is a representation of an already complete
graph.

Requirements:

- self-contained where possible;
- safe escaping;
- no external scripts/fonts by default;
- direction and parallel-edge preservation;
- source location and provenance where the format supports them;
- deterministic ordering;
- atomic write;
- size/complexity guard;
- round-trip or semantic-equivalence test;
- renderer version if history reconstruction depends on it.

Do not implement topology changes inside a renderer.

## Add a semantic provider

Add a backend only when:

- endpoint and authentication shape are well defined;
- request/response size and timeout are bounded;
- secret variables are documented but never stored;
- response normalization maps into the shared untrusted-fragment validator;
- context-limit and retry behavior are finite;
- base URL safety checks apply;
- model selection enters history fingerprints;
- code-only paths do not load/call it.

Tests use a local mock server and no real key.

## Add an external integration

Choose or create a focused crate. Define:

```text
input trust       URL/path/connection/credentials
network protocol  TLS/auth/redirects/timeouts
bounds            bytes/rows/process output/duration
graph mapping     IDs, relations, provenance, multiplicity
side effects      read-only, append, replace, files written
recovery          retry/idempotence/rollback
CLI surface       explicit opt-in and diagnostics
```

For subprocess integrations, avoid shell concatenation and cap both duration
and captured output.

For remote write integrations, test against a disposable endpoint and verify
counts.

## Add history data

History changes are compatibility-sensitive. Determine:

- artifact class;
- typed key encoding;
- canonical value encoding;
- size/count/depth limit;
- fingerprint input;
- structural-sharing behavior;
- diff representation;
- reconstruction/export;
- validation;
- schema/canonical version migration.

Published realizations are immutable. Never “fix” them in place.

Add round-trip, reopen, publication, corruption, diff symmetry, GC reachability,
and SQLite contract tests.

## Add assistant integration

Generated skill assets live in the native CLI package and must be tested as
complete file trees.

Requirements:

- use `compass` and `compass-out/`;
- support global/project scope as appropriate;
- preserve user-authored instruction content;
- idempotent install;
- safe uninstall and explicit purge;
- no machine paths or secrets;
- platform-specific discovery layout;
- actual workflow instructions, not only a command list.

## Compatibility and divergence

If Graphify has equivalent behavior:

- add/update differential evidence;
- preserve approved normalizations;
- update the compatibility ledger.

If Compass intentionally differs:

- document why;
- add native contract tests;
- update migration guidance;
- avoid exposing a half-compatible internal frontend behavior as public
  product identity.

If Compass-native:

- document the new contract independently;
- do not invent a Graphify command solely for symmetry.

## Verification matrix

| Extension | Required broad evidence |
| --- | --- |
| Language | multilingual extraction + incremental + parity where applicable |
| Relation | direction/provenance/multiplicity + query/impact |
| Query | compiler + executor + TCK + CLI + limits + benchmark |
| Command | help/args/streams/exits/files + platform cases |
| Output | escaping/determinism/semantic equivalence/atomic write |
| Provider | mock server + endpoint/key/timeout/size/partial/cache |
| Integration | protocol failure + bounds + mapping + idempotence |
| History | canonical round trip + reopen + corruption + diff + GC |
| Assistant | exact generated trees + idempotent install/uninstall |

## Documentation checklist

- concept page if a new idea is introduced;
- guide or cookbook recipe if users perform a new task;
- reference update for flags/schema/config;
- roadmap update if status changes;
- compatibility/migration/security update where affected;
- diagram update only when it improves understanding.

## Related pages

- [Contributing](../../CONTRIBUTING.md)
- [Workspace tour](workspace-tour.md)
- [Design principles](../design/principles.md)
- [Compatibility reference](../reference/compatibility.md)

**Next step:** write a one-paragraph extension contract covering ownership,
inputs, outputs, provenance, bounds, and failure, then map it to the relevant
checklist above.
