# Upstream contribution plan for the SurrealDB, MCP, and agent ecosystem work

> **Status:** contributor planning notes, not an accepted Compass roadmap.
>
> These notes explain how to use the repository's existing KBD and Prometheus
> tooling without turning local orchestration state into the product design. The
> upstream maintainers remain the authority on scope, sequencing, dependency
> policy, compatibility, and whether any proposal should proceed.

## Purpose

The companion [research report](compass-surrealdb-mcp-skills-research.md)
contains several related recommendations:

- modernize the existing Rust MCP server for MCP 2026;
- evaluate a graph-native SurrealDB projection and a separate Store adapter;
- evolve Compass's existing Agent Skills and skill-aware CLI installation; and
- publish native Codex, Claude Code, and OpenCode packages.

Those recommendations should not become one pull request. They cross storage,
query, transport, security, licensing, CLI, documentation, and packaging
contracts. Compass's contribution guide explicitly asks for focused changes and
warns that maintainers may split broad pull requests.

## Start with upstream agreement, not implementation

Before opening a code pull request:

1. Search Compass Issues and Discussions for SurrealDB, graph backends, MCP
   2026, `rmcp` 3.x, Agent Skills, Codex plugins, Claude plugins, and OpenCode.
2. Use a Discussion for the combined direction because it is still open-ended.
3. Convert only an agreed, observable outcome into a focused feature request.
4. Ask maintainers which slice they would review first and whether they want a
   design document or proof-of-concept branch before code.
5. Treat silence as no authorization to introduce a large dependency or public
   contract.

The proposal should lead with a user need, not a technology:

```text
Agents using Compass need versioned, bounded code-graph context through a
current MCP contract. Some users also need an optional graph-native persistence
and query target that can run embedded or remotely. The default structural
workflow must remain local, deterministic, credential-free, and independent of
that optional target.
```

State these non-goals explicitly:

- no replacement of `graph.json`, SQLite, or redb as the default;
- no mandatory network, credentials, embeddings, vector database, or model;
- no raw write-capable SurrealQL tool for agents by default;
- no Prometheus runtime dependency in Compass;
- no Graphify compatibility or fallback path;
- no claim that research, an MCP protocol draft, or a future document is
  shipped behavior; and
- no combined “database + MCP + skills + three marketplaces” mega-PR.

## Preserve the current KBD work before starting

As of 2026-08-28, this checkout reports:

```text
active phase: compass-scoping-and-bounds
status: plan_ready
next change: C-001 / stream-snapshot-read
exact next command: /opsx:new stream-snapshot-read
KBD runtime mode: legacy; typed runtime not initialized yet
```

Do not append the ecosystem work to that phase merely because
`read_snapshot` may later help a remote adapter. Finish, pause, or formally
supersede the current phase first. Use the typed KBD command surface and never
edit `progress.json`, the waypoint, or event counters by hand.

Check state before every new session:

```bash
prometheus kbd status --json
git status --short
```

The current checkout also contains unrelated tracked modifications and many
untracked harness/generated paths. Create a clean branch or worktree from the
upstream base for each proposed PR. Do not use destructive cleanup commands on
this checkout.

## Suggested KBD structure

After the current phase is resolved, create a new top-level phase such as
`compass-agent-graph-ecosystem`. Its goal should be decision-quality evidence
and independently reviewable changes, not “implement the research report.”

A practical sequence is:

```text
Assess
  verify current contracts and existing implementation
Analyze
  compare status quo, MCP migration, and optional backend candidates
Plan
  define one OpenSpec change per intended upstream PR
Execute
  apply one task at a time through KBD
Reflect
  record measured outcomes and revise the next change
```

Harness commands:

```text
/kbd-new-phase compass-agent-graph-ecosystem
/kbd-assess compass-agent-graph-ecosystem
/kbd-plan compass-agent-graph-ecosystem
/opsx:new <focused-change-id>
/kbd-apply <focused-change-id>
/opsx:verify <focused-change-id>
/opsx:archive <focused-change-id>
/kbd-reflect compass-agent-graph-ecosystem
```

Use `/kbd-apply`, not bare `/opsx:apply`; the KBD wrapper advances one task at a
time and keeps the append-only journal, projection, hooks, and waypoint
consistent. If the installed command names differ, run the relevant skill help
instead of guessing flags.

For each change, make the OpenSpec proposal answer:

- What observable problem does this solve?
- Which crate owns the behavior?
- Which current machine contract changes, if any?
- What remains the supported fallback?
- What resource, security, and network bounds apply?
- Which test falsifies the approach?
- Which documentation becomes authoritative only after release?

## Recommended PR sequence

Each PR below should be independently useful and revertible. Do not promise all
later PRs in order to justify the first.

### PR 0 — maintainer-approved decision record

Prefer a Discussion or feature request first. If maintainers want a repository
document, submit a short Compass-native design proposal distilled from the
research. Do not submit the entire research package, local absolute paths,
review packets, or Prometheus internals.

Decision required:

- Is MCP 2026 support desired now?
- Is an optional BSL dependency acceptable in any published artifact?
- Does upstream want SurrealDB specifically, or a backend capability with
  SurrealDB as one candidate?
- Which harness packages belong in the Compass repository?

### PR 1 — `rmcp 3.1.4` and MCP 2026 compatibility

Scope only the dependency and protocol migration. Do not add SurrealDB or new
plugin packages.

Before merge:

- document the `rmcp 2.2.0` to `3.1.4` API and transitive-dependency delta;
- confirm Rust 1.97.1, license, unsafe-code, and `deny.toml` policy;
- add `server/discover`, stateless request metadata, `resultType`, deterministic
  tool ordering, and the new subscription model as required;
- run reference conformance on a branch;
- test stdio and HTTP against named Codex, Claude Code, and OpenCode versions;
- state whether legacy MCP support exists, its exact cost, and its removal date;
- update MCP reference, compatibility, migration, changelog, security, and
  operations documentation where required.

If the clients do not interoperate, do not merge merely because the Rust code
compiles.

### PR 2 — structured task-level MCP results

Keep database implementation out. Define a common versioned result envelope
and improve a small number of existing tools first—prefer `search_symbols`,
`get_callers`, `get_callees`, and `get_impact`.

Measure a checked-in task corpus against the current bounded raw-traversal
baseline. Preserve exact evidence, direction, multiplicity, ambiguity, bounds,
pagination, and deterministic ordering. Do not use a product schema name as the
MCP `resultType`; keep `resultType: "complete"` and carry the Compass schema in
a separate field.

### PR 3 — persistent SurrealDB evaluation only

Do not begin with a production adapter. Create a disposable evaluation that
uses the persistent engines under consideration, not only in-memory mode. It
must test:

- parallel directed relations with stable IDs;
- source provenance and confidence round trips;
- generation-scoped reads and atomic activation;
- kill-during-write recovery;
- deterministic ordering and pagination at scale;
- dependency/build time, compressed binary size, cold start, peak RSS, and
  query latency; and
- the exact SurrealDB 3.2.4 license text and downstream distribution effect.

This evidence can live in an issue, benchmark artifact, or maintainer-approved
qualification document. A failed evaluation should leave no production
dependency in the workspace.

### PR 4 — optional graph projection/query adapter

Proceed only after PR 3 and a maintainer decision. Keep it in a focused
integration crate such as `compass-graphdb-surreal`; do not import SurrealDB
into `compass-model`, `compass-store`, `compass-query`, or the default CLI path
without an ownership reason.

The projection must be derived from one immutable Compass generation. The
current JSON/SQLite path remains authoritative and supported. Native queries
must pass dual-engine semantic equivalence before being exposed to MCP or CLI.

### PR 5 — optional `Store` adapter

Treat this as a separate decision. Pass the unchanged
`compass-store-qualification` contract, including ordered scans, cursors,
conditional writes, error taxonomy, interruption, reopen, and bounded work.
The synchronous/async boundary belongs in the adapter crate; no executor should
leak into the core Store contract.

If this adapter does not solve an independently stated user problem, do not add
it merely because the graph projection exists.

### PR 6 — Agent Skills and skill-aware CLI evolution

First preserve the existing umbrella skill and `compass install` behavior.
Add focused skills only as additive artifacts with trigger-discrimination and
complete-tree installation tests. A possible CLI namespace is
`compass agent list|install|doctor|export|validate|mcp-config`, but command names
must be approved through normal CLI contract review.

Tests must cover project/user scope, copied files, checksums, user-authored
instruction preservation, idempotent upgrade, uninstall, path containment, and
the absence of credentials or machine paths.

### PR 7 — native harness distribution

Generate platform-specific artifacts from a Compass-owned inventory:

- Codex: `.codex-plugin/plugin.json`, skills, `.mcp.json`, and a repository
  `.agents/plugins/marketplace.json` only if maintainers want a marketplace;
- Claude Code: `.claude-plugin/plugin.json`, skills, `.mcp.json`, and
  `.claude-plugin/marketplace.json` when distribution is approved; and
- OpenCode: a real TypeScript/npm or project plugin, not an invented common
  marketplace format.

Clean-install, discovery/load, MCP invocation, upgrade, and uninstall must pass
in each named harness. Generated files must be reproducible and must not contain
local paths or secrets.

## Prometheus skills to use during the work

Prometheus should improve the contribution process, not become a Compass
runtime dependency.

| Stage | Useful skill | Expected artifact |
| --- | --- | --- |
| Assess current state | `deep-research`, Compass queries | Source-grounded assessment pinned to a commit |
| Verify current libraries | Context7 documentation lookup | Named versions and primary documentation |
| Challenge a decision | `adversarial-review` in decision mode | Findings with falsifiers before implementation |
| Review a new dependency | `dependency-pin-discipline` | Exact pin, MSRV/license/transitive delta, update policy |
| Design MCP/CLI contracts | `api-and-interface-design` | Versioned request/result/error and compatibility contract |
| Design remote trust boundaries | `agent-runtime-security` | Authentication, authorization, path, query, secret, and DoS limits |
| Specify behavior | `bdd-scenarios` or `bdd-testing` | Reviewable scenarios for success, ambiguity, bounds, and failure |
| Record accepted architecture | `documentation-and-adrs` | Maintainer-approved design/ADR and consequences |
| Pre-PR review | `code-review-and-quality`, `adversarial-review` | Actionable findings before maintainer review |
| Close a KBD phase | `/kbd-reflect` | Measured outcomes, rejected assumptions, next-phase seed |

When a skill produces local state, separate it from the proposed product patch.
The repository currently tracks selected `.kbd-orchestrator/**`,
`.prometheus/knowledge/wiki/**`, and harness files, but that does not mean every
session transcript, event, generated configuration, or cache belongs in every
feature PR. Ask maintainers which orchestration artifacts they want. If they do
want them, keep them in a separate commit so they can review or drop them
without touching implementation.

Never add Prometheus package imports, executable requirements, MCP endpoints,
user-home paths, model policy, or skill-cache paths to Compass production code.
The acceptable integration boundary is the contributor workflow plus public
standards such as MCP and Agent Skills.

## Upstream acceptance checklist

Before requesting review, verify all of the following:

- [ ] An Issue or Discussion records the maintainer-approved scope.
- [ ] The PR solves one observable problem and has a narrow title.
- [ ] The branch starts from the intended upstream base in a clean worktree.
- [ ] No unrelated KBD events, wiki transcripts, generated graphs, harness
      configs, formatting, or user changes are included.
- [ ] New dependencies have an exact justification, license analysis,
      transitive-dependency review, and lockfile update.
- [ ] Default Compass remains native, local-first, deterministic, bounded, and
      credential-free.
- [ ] Graph identity, direction, multiplicity, provenance, ambiguity, and source
      anchors are preserved.
- [ ] Network, authentication, secrets, timeouts, retries, and response limits
      are explicit and tested with local mocks.
- [ ] The change lives in the lowest owning crate; CLI and MCP remain thin.
- [ ] The lowest useful unit/regression tests and public contract tests exist.
- [ ] `COMPATIBILITY.md`, `MIGRATION.md`, `CHANGELOG.md`, `SECURITY.md`,
      `PERFORMANCE.md`, and relevant docs are updated when their contract changes.
- [ ] Documentation distinguishes current behavior from future evaluation.
- [ ] `git diff --check` and all targeted gates pass.
- [ ] The PR description follows `.github/pull_request_template.md` and lists
      every check not run.
- [ ] The contribution remains `MIT OR Apache-2.0`; bundled third-party license
      notices are complete.

For Rust changes, the repository's normal completion baseline is:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
```

Add the surface-specific gates from `AGENTS.md`. For a documentation-only
proposal, run `git diff --check` and verify every changed link and command.

## How to present the first upstream request

Keep the opening short:

```markdown
### Problem

Compass already has an MCP server, but its current Rust SDK/protocol line does
not implement the latest stateless discovery contract. Agents also lack one
uniform structured result envelope across the safest typed code-navigation
tools.

### Desired outcome

Bring the existing server to a maintainer-approved MCP 2026 contract while
preserving the current stdio/HTTP security bounds, deterministic tool order,
typed evidence, and a documented compatibility path for current clients.

### Non-goals

This proposal does not add SurrealDB, change the default graph/store engine,
add embeddings, or publish new harness marketplaces.

### Contribution

I can submit a focused migration PR with conformance tests and named
Codex/Claude/OpenCode interoperability results after the protocol scope is
approved.
```

That first request is much easier to evaluate than asking maintainers to accept
the entire ecosystem direction at once.

## Documentation lifecycle

Keep both documents in `docs/future/` while they are unaccepted planning work.
If maintainers accept a decision:

1. distill the accepted rationale and alternatives into a short design document
   or ADR in the location they prefer;
2. create an implementation plan under `docs/implementation/` only for approved
   engineering work;
3. update concepts, guides, cookbook, reference, compatibility, migration,
   security, performance, and changelog documentation only as behavior ships;
4. leave rejected or superseded decisions visible with their status rather than
   rewriting history; and
5. remove local paths, model names, KBD mechanics, and research-package metadata
   from user-facing Compass documentation.

## Recommended next step

Finish or pause the active `compass-scoping-and-bounds` phase, create a clean
worktree, and open a Discussion limited to MCP 2026 modernization. Use the
larger SurrealDB and agent-distribution research as context, not as the scope of
the first implementation request.
