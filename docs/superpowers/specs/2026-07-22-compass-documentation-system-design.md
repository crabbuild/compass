# Compass Documentation System Design

**Date:** 2026-07-22
**Status:** Approved for implementation planning
**Repository:** `crabbuild/compass`

## Summary

Compass needs a comprehensive, repository-native documentation system that
serves three audiences without forcing any of them to read the entire manual:

1. new users evaluating Compass;
2. developers using or integrating Compass;
3. contributors extending the Rust implementation.

The documentation will use plain Markdown, inline ASCII diagrams, and
checked-in SVG diagrams. Mermaid diagrams are prohibited except when a true
sequence diagram is materially clearer than prose, ASCII, or SVG. The initial
implementation should not require Mermaid.

Compass will be positioned as an independently evolving, native local-first
knowledge graph engine. It is inspired by Graphify and informed by compatibility
work with Graphify, but its current and future product identity is not limited
to being a port. Compass already contains native features that go beyond the
frozen Graphify compatibility baseline, and its feature set is expected to
diverge further.

The GitHub repository description will be:

> Native, local-first knowledge graph engine for code and project
> artifacts—inspired by Graphify, built in Rust, and evolving beyond it.

## Goals

- Give a new evaluator a clear understanding of Compass within five minutes.
- Let a new user install Compass, build a graph, and run useful queries without
  needing to understand the implementation.
- Give integrators task-oriented guidance for automation, assistant setup,
  CompassQL, graph output consumption, history, and service integrations.
- Give contributors a reliable architectural map of the workspace, major
  pipelines, extension points, invariants, and verification practices.
- Make long-form documentation easy to scan through progressive disclosure,
  consistent page openings, cross-links, examples, tables, ASCII diagrams, and
  SVG diagrams.
- Preserve existing authoritative documents and remove the need to duplicate
  contracts across multiple pages.
- Distinguish current, planned, and aspirational behavior unambiguously.
- Keep GitHub repository metadata aligned with the documentation's product
  positioning.

## Non-goals

- Adding a documentation-site generator, generated website, theme, or hosting
  configuration.
- Translating the new documentation in the initial implementation.
- Replacing source code, CLI help, tests, compatibility evidence, or security
  policy as authoritative contracts.
- Promising delivery dates for planned or aspirational roadmap items.
- Re-documenting every internal function or producing API documentation that
  belongs in Rustdoc.
- Redesigning Compass behavior or changing public CLI contracts as part of the
  documentation work.
- Rewriting unrelated legal, governance, or release-policy documents.

## Audience Model

### Evaluators

Evaluators want to know what Compass does, why it exists, how it differs from
alternatives, what runs locally, what it produces, how mature it is, and whether
it fits their workflow. They should start at the root README and move through
the overview, getting started, how-it-works, privacy, compatibility, and
performance material.

### Users and integrators

Users and integrators want repeatable procedures and exact contracts. They
should move from getting started into task guides, cookbook recipes, CompassQL,
and command/configuration/output references.

### Contributors

Contributors need system boundaries, data flow, crate ownership, extension
points, invariants, test commands, and contribution policies. They should move
from design principles into architecture, the workspace tour, implementation
pipelines, extension guides, and the existing contributing documents.

## Information Architecture

The documentation uses an audience-layered learning hub. The root README is a
concise product landing page; `docs/README.md` is the full navigation hub.

```text
README.md
docs/
├── README.md
├── getting-started.md
├── guides/
│   ├── exploring-a-codebase.md
│   ├── integrating-compass.md
│   ├── assistant-setup.md
│   ├── versioned-history.md
│   └── operations.md
├── concepts/
│   ├── how-it-works.md
│   ├── graph-model.md
│   ├── provenance.md
│   └── compassql.md
├── design/
│   ├── principles.md
│   ├── architecture.md
│   ├── storage-and-history.md
│   └── security-and-privacy.md
├── implementation/
│   ├── workspace-tour.md
│   ├── extraction-pipeline.md
│   ├── query-engine.md
│   ├── semantic-pipeline.md
│   └── extending-compass.md
├── cookbook/
│   ├── README.md
│   ├── impact-analysis.md
│   ├── architecture-discovery.md
│   ├── ci-and-automation.md
│   └── troubleshooting.md
├── reference/
│   ├── commands.md
│   ├── configuration.md
│   ├── outputs.md
│   └── compatibility.md
├── roadmap.md
└── assets/
    └── diagrams/
```

The precise set of files may be consolidated during implementation when two
planned pages would be too thin to justify separate maintenance. Consolidation
must preserve the audience journeys and conceptual boundaries.

## Reader Journeys

```text
Evaluating Compass
    |
    +--> Product overview
    +--> Getting started
    +--> How it works
    `--> Compatibility, privacy, and performance

Using or integrating Compass
    |
    +--> Task guides
    +--> Cookbook recipes
    +--> CompassQL
    `--> Command, configuration, and output references

Extending Compass
    |
    +--> Design principles
    +--> System architecture
    +--> Workspace and crate tour
    +--> Implementation pipelines
    `--> Contribution workflow
```

Each substantial page begins with:

- the intended audience;
- what the reader will learn;
- prerequisites;
- an estimated reading or completion time.

Each substantial page ends with:

- related documents;
- the most useful next step.

## Document Types

The documentation follows four explicit content types:

| Type | Reader question | Required characteristics |
| --- | --- | --- |
| Concept | “What is this and why does it work this way?” | Plain-language model, terminology, diagrams, links to exact references |
| Guide | “How do I complete this task?” | Prerequisites, ordered procedure, verification, failure recovery |
| Cookbook | “How do I solve this concrete scenario?” | Short problem statement, copyable recipe, variations, caveats |
| Reference | “What is the exact contract?” | Complete syntax/schema/options, defaults, limits, exits, compatibility notes |

Design and implementation documents may combine concept and reference elements,
but must state which behavior is a public contract and which is an internal
implementation detail.

## Root README Design

The root README will become a concise landing page. It will contain:

1. the product name and one-sentence value proposition;
2. the relationship to Graphify and Compass's independent direction;
3. a small “source to graph to answers” visual;
4. an installation path;
5. one end-to-end example;
6. a capability summary;
7. links for evaluators, users/integrators, and contributors;
8. platform, maturity, licensing, support, and security links.

Long reference material currently in the README will move to or be summarized
by focused documents. The README must remain useful by itself and must not
become a bare link directory.

## Content Scope

### Getting started

The getting-started guide covers:

- choosing a supported installation path;
- verifying the binary;
- creating a graph in a sample or existing repository;
- understanding `compass-out/`;
- running natural-language discovery, `explain`, `path`, and `affected`;
- opening the HTML graph when available;
- installing assistant integration;
- cleaning up or regenerating output;
- common first-run errors and their remedies.

### Guides

Guides cover:

- exploring an unfamiliar codebase;
- using Compass in assistant workflows;
- integrating graph output and JSON into tooling;
- running CompassQL safely and deterministically;
- enabling, querying, exporting, and maintaining versioned history;
- operating watch, serve, hooks, providers, and optional integrations;
- using Compass in automation and CI.

### Concepts and how it works

Concept pages cover:

- nodes, edges, direction, provenance, confidence, communities, and god nodes;
- deterministic structural extraction;
- resolution and inferred relationships;
- optional semantic extraction;
- incremental rebuilds and manifests;
- graph analysis, clustering, and output generation;
- query, traversal, CompassQL, and result limits;
- versioned realizations, fingerprints, preferred realizations, and Prolly
  storage;
- the boundary between current Graphify compatibility and native Compass
  evolution.

### Design

Design documents cover:

- local-first operation and explicit semantic-provider boundaries;
- deterministic and inspectable results;
- conservative failure behavior and atomic publication;
- stable machine-readable contracts;
- bounded resource usage;
- workspace boundaries;
- storage/history architecture;
- security and privacy threat boundaries.

### Implementation

Implementation documents cover:

- the role of each workspace crate;
- the CLI-to-pipeline data flow;
- file discovery, language detection, parsing, extraction, resolution, merge,
  analysis, and rendering;
- query loading, indexing, planning, execution, and rendering;
- semantic orchestration and provider boundaries;
- versioned history storage, materialization, hooks, leases, jobs, and garbage
  collection;
- adding a language, relation, extractor, integration, query capability, or
  output surface;
- relevant tests and qualification commands for each extension.

### Cookbook

Recipes cover representative tasks:

- finding an authentication or authorization path;
- mapping a subsystem;
- finding callers and downstream impact;
- comparing two commits;
- producing machine-readable query output;
- adding Compass to CI;
- providing focused graph context to a coding assistant;
- diagnosing missing symbols, unexpected edges, stale output, provider
  failures, history corruption, and oversized graphs.

### References

References cover:

- the current public command surface and global conventions;
- configuration sources, precedence, environment variables, and provider
  settings;
- `compass-out/` artifacts and graph/result schemas;
- exits, diagnostics, limits, atomicity, and deterministic ordering;
- compatibility and migration links;
- supported platforms, formats, languages, and optional integrations.

### Roadmap

The roadmap has three labeled sections:

1. **Available now** — verified shipped behavior;
2. **Planned** — work supported by committed specs or implementation plans;
3. **Aspirational** — ideas that are not commitments and have no promised
   release or compatibility guarantee.

Every planned item links to repository evidence. Aspirational ideas state the
problem or opportunity, not a fictional delivery date. The roadmap explains
that Compass may diverge from Graphify when native design goals, performance,
correctness, or user needs justify it.

## Existing Canonical Documents

The following remain authoritative and must be linked rather than copied:

- `COMPATIBILITY.md`
- `CONTRIBUTING.md`
- `MIGRATION.md`
- `PERFORMANCE.md`
- `SECURITY.md`
- `SUPPORT.md`
- `docs/COMPASSQL.md`
- `docs/COMPASSQL_SUPPORT.md`

New pages may summarize these documents for navigation or teaching, but each
summary must link to its canonical source and avoid restating volatile numeric
or compatibility details unless those details are verified during
implementation.

Approved specs and plans are implementation evidence, not end-user
documentation. Public docs may derive explanations from them but must not
expose speculative details as shipped behavior.

## Visual System

### ASCII diagrams

Use inline ASCII for compact relationships, decisions, directory trees, and
small pipelines. Diagrams must render correctly in monospaced Markdown blocks
without color.

### SVG diagrams

Use checked-in SVGs for:

- end-to-end graph construction;
- query and traversal flow;
- workspace/crate architecture;
- incremental update pipeline;
- history storage and materialization;
- integration surfaces;
- provenance and confidence;
- reader journeys when an SVG adds meaningful clarity.

Each SVG must:

- include `<title>` and `<desc>` elements;
- use semantic group labels;
- remain readable against GitHub light and dark backgrounds;
- avoid relying on color as the only carrier of meaning;
- use legible text at normal GitHub content width;
- have nearby Markdown prose that communicates the same essential information;
- avoid external fonts, scripts, images, or network resources.

### Mermaid

Mermaid is not used in the initial documentation set. A future document may use
Mermaid only for a genuine sequence diagram where time-ordered interactions are
materially clearer than ASCII, SVG, or prose.

## Technical Accuracy Policy

Every statement about current behavior must be supported by at least one of:

- CLI command definitions or generated `--help`;
- public Rust interfaces and workspace boundaries;
- automated tests or qualification scripts;
- existing canonical release, compatibility, performance, or security
  evidence.

Claims derived from source rather than a public contract must be labeled as
implementation details. Claims inferred from repository plans must be labeled
planned. Uncommitted product ideas must be labeled aspirational.

The docs must use the `compass` executable. References to `graphify` belong
only in historical, compatibility, migration, or explicit comparison contexts.

## Error Handling in Guides

Task guides must not present only the happy path. Each procedure includes, as
applicable:

- how the reader confirms success;
- expected output artifacts;
- common failure messages or failure categories;
- whether retrying is safe;
- cleanup or recovery steps;
- relevant diagnostic commands;
- links to deeper troubleshooting or support material.

Security-sensitive examples use placeholders and never encourage putting
credentials in command history, tracked configuration, screenshots, or example
output.

## Verification Strategy

### Source and command verification

- Generate or inspect CLI help for documented commands and flags.
- Run practical quickstart and cookbook commands against a temporary sample
  repository.
- Confirm output paths and representative output structure.
- Cross-check current-feature claims against source and tests.
- Cross-check planned items against committed specs or plans.

### Documentation integrity

- Check all relative Markdown links.
- Check every referenced image exists.
- Parse every SVG as XML.
- Render and visually inspect SVGs.
- Scan for `TODO`, `TBD`, placeholders, unsupported claims, accidental Mermaid,
  and stale end-user `graphify` command examples.
- Check headings and navigation for duplicate or orphaned topics.
- Ensure every substantial page declares audience, outcomes, prerequisites,
  estimated time, related pages, and next step.

### Repository verification

- Run formatting and test commands proportional to documentation-only changes.
- Preserve the user's existing untracked
  `docs/superpowers/plans/2026-07-22-compass-native-skill.md`.
- Run `graphify update .` in the parent Graphify repository if implementation
  modifies code files, as required by the workspace instructions.
- Review `git diff --check` and final status before handoff.

### Remote metadata verification

After the final wording exists in the committed documentation:

1. inspect the current `crabbuild/compass` metadata;
2. update the GitHub description to the approved text;
3. read the repository metadata back;
4. report the confirmed description to the user.

## Change Boundaries

- Documentation changes are made in the Compass repository.
- Existing user changes and untracked files are preserved.
- The design-spec commit contains only this specification.
- The later documentation implementation should use focused commits or a
  clearly reviewable documentation commit, without bundling unrelated code
  changes.
- Remote repository metadata is changed only after local documentation and
  verification are complete.

## Success Criteria

The work is complete when:

- the root README clearly positions Compass and routes all three audiences;
- the Markdown documentation hub and its primary learning paths exist;
- getting started, guides, how-it-works, design, implementation, cookbook,
  reference, and roadmap content are comprehensive and cross-linked;
- current, planned, and aspirational features are visibly distinct;
- major architecture and data-flow concepts have accessible ASCII or SVG
  diagrams;
- no non-sequence Mermaid diagram is introduced;
- command examples and internal claims have been verified;
- all local links and SVGs validate;
- existing canonical documents remain authoritative and discoverable;
- no unrelated user file is overwritten;
- the GitHub repository description matches the approved positioning and is
  confirmed after update.
