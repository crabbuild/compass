# Plan 002: Restore incoming evidence in directed wiki exports

> **Executor instructions:** Follow every step and verification gate. Do not
> push or open a pull request. Update `advisor-plans/README.md` when complete.
>
> **Drift check:** Run
> `git diff --stat 3837b411..HEAD -- crates/compass-output/src/wiki.rs crates/compass-output/tests`
> and stop if the wiki incidence model has materially changed.

## Status

- **Priority:** P1
- **Effort:** S
- **Risk:** LOW
- **Depends on:** plan 001
- **Category:** bug
- **Planned at:** `3837b411`, 2026-07-23

## Why this matters

Normal Compass graphs are directed, but the wiki's incident-edge index includes
target-side edges only for undirected graphs. Wiki-only navigation can therefore
omit callers, dependents, inbound community bridges, and half of a god node's
useful neighborhood. Graphify main uses an undirected NetworkX graph and does
not lose that evidence.

## Current state

`crates/compass-output/src/wiki.rs:158-164` currently contains:

```rust
incident.entry(edge.source.as_str()).or_default().push(edge);
if !document.directed || edge.target != edge.source {
    incident.entry(edge.target.as_str()).or_default().push(edge);
}
```

`WikiGraph::neighbors` at `wiki.rs:177-191` returns the target endpoint for an
outgoing edge, but only returns the source endpoint for an incoming edge when
the entire document is undirected.

Direction is a public graph invariant; `docs/concepts/graph-model.md:94-104`
explains that incoming `CALLS` traversal is how callers are found.

## Commands

| Purpose | Command | Expected result |
|---|---|---|
| Target tests | `cargo test -p compass-output wiki --locked` | all wiki tests pass |
| Full crate | `cargo test -p compass-output --locked` | all tests pass |
| Lint | `cargo clippy -p compass-output --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope:**

- `crates/compass-output/src/wiki.rs`
- Create `crates/compass-output/tests/wiki_direction.rs` or add equivalent
  focused tests to an established wiki test module.

**Out of scope:**

- Changing graph direction or edge identity.
- Changing natural-query traversal.
- Reproducing Graphify's undirected graph globally.
- Changing Obsidian, HTML, GraphML, or history semantics.

## Git workflow

- Branch: `advisor/002-directed-wiki-evidence`
- Commit: `Preserve incoming evidence in directed wiki exports`
- Do not push or open a PR.

## Steps

### Step 1: Add failing directed fixtures

Create a directed graph with:

- `caller -> target` using `CALLS`;
- `external -> target` crossing communities;
- `target -> dependency`;
- a self-loop.

Export a wiki where `target` is both a community member and a god node. Assert
that:

- incoming caller and external evidence is present;
- outgoing dependency evidence is present;
- the inbound cross-community count is included;
- self-loops are emitted once;
- deterministic output is unchanged across two runs.

**Verify:** the new test fails on current code specifically because incoming
target-side evidence is absent.

### Step 2: Make incidence direction-complete

Index every non-self-loop edge under both endpoints regardless of graph
direction. Return a small typed neighbor record containing:

- other node ID;
- edge;
- orientation (`Incoming`, `Outgoing`, or `SelfLoop`).

Do not infer direction from relation names. Use stored source and target.

**Verify:** the new directed fixtures pass.

### Step 3: Render direction without losing compatibility

In relationship sections, expose direction using stable arrows or explicit
incoming/outgoing labels. Keep headings, filenames, escaping, truncation, and
audit-trail behavior unchanged.

If an exact Graphify-compatible wiki projection is required, make it an
explicit option; do not make the authoritative Compass wiki lossy.

**Verify:** full `compass-output` tests and clippy pass.

## Test plan

- Directed incoming, outgoing, cross-community, and self-loop cases.
- Undirected regression fixture.
- Duplicate parallel-edge behavior.
- Hostile label/path escaping.
- Byte-determinism across repeat exports.

Model tests after `crates/compass-output/tests/coverage_paths.rs` and the private
wiki helpers in `crates/compass-output/src/wiki.rs`.

## Done criteria

- [ ] Directed wiki articles include incoming and outgoing evidence.
- [ ] Orientation is visible and source/target semantics remain exact.
- [ ] Self-loops are not duplicated.
- [ ] Full `compass-output` tests, clippy, and format checks pass.
- [ ] No files outside the scope changed.

## STOP conditions

- Wiki output is a documented byte-compatibility contract with Graphify main.
- Fixing the bug requires changing authoritative graph direction.
- Parallel-edge behavior cannot be represented without resolving plan 003 or
  the deferred edge-multiplicity design.

## Maintenance notes

Reviewers should check that inbound evidence affects both article content and
cross-community counts. Future relation renderers must use stored orientation,
not English relation-name heuristics.
