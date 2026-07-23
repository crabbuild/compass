# Compass Documentation System Implementation Plan

> **For agentic workers:** Execute this plan inline, task by task. The user explicitly requested immediate authoring without a TDD approach. Use documentation validation and command verification instead of red-green-refactor steps.

**Goal:** Build a comprehensive, easy-to-digest Markdown documentation system for Compass and align the GitHub repository description with its independently evolving product identity.

**Architecture:** Keep the root README as a concise landing page and make `docs/README.md` the audience-layered navigation hub. Separate concepts, task guides, implementation internals, recipes, and exact references; preserve existing canonical policy and compatibility documents through links. Use inline ASCII for compact visuals and accessible, self-contained SVG files for architecture and pipeline diagrams.

**Tech Stack:** GitHub-flavored Markdown, hand-authored SVG 1.1, shell-based link/XML/content validation, Compass CLI help and Rust source as behavioral evidence.

## Global Constraints

- Serve new evaluators, developers integrating Compass, and Rust contributors.
- Use plain Markdown; do not add MkDocs, Docusaurus, or another site generator.
- Make documents comprehensive, clear, scannable, and understandable without prior graph-database knowledge.
- Use ASCII and SVG diagrams. Do not use Mermaid except for a genuine sequence diagram; the initial document set uses no Mermaid.
- Label roadmap material as **Available now**, **Planned**, or **Aspirational**.
- Describe Compass as inspired by Graphify and evolving independently beyond the frozen compatibility baseline.
- Preserve `COMPATIBILITY.md`, `CONTRIBUTING.md`, `MIGRATION.md`, `PERFORMANCE.md`, `SECURITY.md`, `SUPPORT.md`, `docs/COMPASSQL.md`, and `docs/COMPASSQL_SUPPORT.md` as canonical sources.
- Preserve all unrelated modified and untracked files in the shared worktree.
- Do not use a TDD workflow for documentation authoring.

---

### Task 1: Documentation Hub and First Success

**Files:**
- Create: `docs/README.md`
- Create: `docs/getting-started.md`
- Create: `docs/assets/diagrams/reader-journeys.svg`
- Create: `docs/assets/diagrams/first-graph.svg`

**Produces:** The canonical documentation index and a verified evaluator-to-first-query path used by every later section.

- [ ] Write `docs/README.md` with audience cards, topic index, document-type explanation, current/planned/aspirational legend, and links to all canonical top-level documents.
- [ ] Write `docs/getting-started.md` with installation choices, first graph, output tour, first queries, assistant setup, success checks, troubleshooting, and next steps.
- [ ] Create accessible SVGs for reader journeys and the first-graph flow; include `<title>` and `<desc>`.
- [ ] Verify install and query flags against `compass --help` and command-specific help.
- [ ] Run `git diff --check` for the four files and parse both SVGs with an XML parser.

### Task 2: Concepts and Product Mechanics

**Files:**
- Create: `docs/concepts/how-it-works.md`
- Create: `docs/concepts/graph-model.md`
- Create: `docs/concepts/provenance.md`
- Create: `docs/concepts/compassql.md`
- Create: `docs/assets/diagrams/graph-pipeline.svg`
- Create: `docs/assets/diagrams/provenance.svg`

**Consumes:** Navigation vocabulary and first-run artifact names from Task 1.

**Produces:** The conceptual vocabulary used by guides, implementation docs, and references.

- [ ] Explain discovery, parsing, extraction, resolution, graph construction, clustering, analysis, publication, and querying from simple model to technical detail.
- [ ] Define nodes, edges, direction, multiplicity, attributes, provenance, confidence, communities, god nodes, hyperedges, and graph snapshots with examples.
- [ ] Explain direct, inferred, and ambiguous evidence without implying that inference is an LLM guess.
- [ ] Introduce CompassQL as a deterministic read-only structural query language and route exact syntax to the canonical CompassQL references.
- [ ] Create accessible pipeline and provenance SVGs and validate them as XML.
- [ ] Cross-check terminology against `compass-model`, `compass-core`, `compass-resolve`, `compass-graph`, `compass-query`, and the canonical CompassQL documents.

### Task 3: User and Integration Guides

**Files:**
- Create: `docs/guides/exploring-a-codebase.md`
- Create: `docs/guides/integrating-compass.md`
- Create: `docs/guides/assistant-setup.md`
- Create: `docs/guides/versioned-history.md`
- Create: `docs/guides/operations.md`
- Create: `docs/assets/diagrams/integration-surfaces.svg`
- Create: `docs/assets/diagrams/history-materialization.svg`

**Consumes:** Commands and concepts from Tasks 1–2.

**Produces:** End-to-end procedures for the principal Compass workflows.

- [ ] Write a repeatable unfamiliar-codebase investigation workflow using report, query, explain, path, tree, and affected.
- [ ] Document stable machine-readable integration patterns, atomic output use, CompassQL parameters, and service/export boundaries.
- [ ] Document native assistant installation, project/global scope, verification, and safe removal without duplicating generated asset internals.
- [ ] Document enabling, building, querying, comparing, exporting, preferring, garbage-collecting, and disabling versioned history, including code-only versus semantic fingerprints.
- [ ] Document watch, serve, hooks, providers, graph database exports, and operational failure recovery.
- [ ] Create and validate the integration and history SVGs.
- [ ] Verify each documented command family against CLI source/help and canonical history contracts.

### Task 4: Design and Implementation Internals

**Files:**
- Create: `docs/design/principles.md`
- Create: `docs/design/architecture.md`
- Create: `docs/design/storage-and-history.md`
- Create: `docs/design/security-and-privacy.md`
- Create: `docs/implementation/workspace-tour.md`
- Create: `docs/implementation/extraction-pipeline.md`
- Create: `docs/implementation/query-engine.md`
- Create: `docs/implementation/semantic-pipeline.md`
- Create: `docs/implementation/extending-compass.md`
- Create: `docs/assets/diagrams/workspace-architecture.svg`
- Create: `docs/assets/diagrams/incremental-update.svg`

**Consumes:** Product mechanics and command boundaries from Tasks 1–3.

**Produces:** A source-backed architectural map and contributor onboarding path.

- [ ] Explain local-first boundaries, determinism, inspectability, atomic publication, resource bounds, and compatibility-versus-evolution principles.
- [ ] Map the CLI, orchestration, model, language, resolution, graph, query, semantic, history, integration, and output crate groups.
- [ ] Explain SQLite/Prolly history storage, immutable realizations, fingerprints, preferred pointers, jobs, leases, hooks, reconstruction, and garbage collection.
- [ ] Explain credential, network, file-system, historical-checkout, and generated-output trust boundaries while linking `SECURITY.md` as policy.
- [ ] Provide a crate-by-crate workspace tour with ownership, dependencies, extension points, and relevant tests.
- [ ] Explain cold and incremental extraction, query planning/execution, semantic orchestration, and extension workflows.
- [ ] Create and validate workspace and incremental-update SVGs.
- [ ] Cross-check all architectural claims against workspace manifests, crate public modules, CLI orchestration, tests, and qualification scripts.

### Task 5: Cookbook and Exact References

**Files:**
- Create: `docs/cookbook/README.md`
- Create: `docs/cookbook/impact-analysis.md`
- Create: `docs/cookbook/architecture-discovery.md`
- Create: `docs/cookbook/ci-and-automation.md`
- Create: `docs/cookbook/troubleshooting.md`
- Create: `docs/reference/commands.md`
- Create: `docs/reference/configuration.md`
- Create: `docs/reference/outputs.md`
- Create: `docs/reference/compatibility.md`

**Consumes:** Verified workflows and concepts from Tasks 1–4.

**Produces:** Copyable recipes and a lookup-oriented public contract index.

- [ ] Write scenario-driven recipes with problem, command, interpretation, variants, safety notes, and next step.
- [ ] Cover impact review, architecture discovery, CI caching and artifacts, deterministic JSON/JSONL use, and assistant context preparation.
- [ ] Build a symptom/cause/diagnosis/remedy troubleshooting matrix for install, extraction, query, semantic, history, and output failures.
- [ ] Inventory the public command families, shared inputs/outputs, format conventions, and exit-status pointers without inventing unsupported flags.
- [ ] Document configuration precedence and credential-safe examples based on current source.
- [ ] Document `compass-out/`, graph/result schemas, determinism, atomicity, and consumer guidance.
- [ ] Explain the frozen Graphify compatibility baseline and Compass-native divergence, linking canonical compatibility and migration ledgers.
- [ ] Verify commands, configuration names, paths, and schema-version strings against source and tests.

### Task 6: Roadmap, Landing Page, and Cross-Linking

**Files:**
- Create: `docs/roadmap.md`
- Modify: `README.md`
- Modify: all Markdown files created in Tasks 1–5 as needed for final links

**Consumes:** The complete documentation structure and current repository plans/specs.

**Produces:** A coherent landing-to-depth navigation system and explicitly qualified roadmap.

- [ ] Write the roadmap with separate **Available now**, **Planned**, and **Aspirational** sections, evidence links for planned work, and no delivery-date promises.
- [ ] Carefully merge a concise product-positioning and documentation-navigation section into the current root README without overwriting unrelated concurrent changes.
- [ ] Add related-page and next-step footers to every substantial new document.
- [ ] Check that every intended audience has a continuous path from the root README to exact reference or contribution material.
- [ ] Scan new docs for stale user-facing `graphify` commands, unlabeled speculative behavior, forbidden Mermaid, placeholders, and duplicated volatile claims.

### Task 7: Full Validation and Remote Metadata

**Files:**
- Validate: all created and modified Markdown and SVG files
- Remote metadata: `crabbuild/compass` GitHub repository description

**Consumes:** All prior tasks.

**Produces:** Verified, reviewable docs and confirmed GitHub metadata.

- [ ] Run a local relative-link and image checker across the complete Markdown set.
- [ ] Parse every new SVG as XML and render representative diagrams for visual inspection.
- [ ] Run `git diff --check`.
- [ ] Compare documented command names to the CLI command registry and inspect help for high-use commands.
- [ ] Audit the design specification requirement by requirement against current files and validation evidence.
- [ ] Update the GitHub description to: `Native, local-first knowledge graph engine for code and project artifacts—inspired by Graphify, built in Rust, and evolving beyond it.`
- [ ] Read the GitHub repository metadata back and confirm the exact description.
- [ ] Report the files authored, validation evidence, preserved unrelated changes, and remote metadata result.
