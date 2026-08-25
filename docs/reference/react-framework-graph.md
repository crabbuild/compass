---
meta:
  contentType: Reference
  title: React frontend framework graph evidence
  navLabel: React frontend graph
  category: Reference
  overview: The evidence boundary and current support contract for React-family frontend graph facts.
  goal: State exactly what Compass can prove for React frontend code and how agents should interpret incomplete results.
  audience:
    - Compass users
    - coding-agent integrators
    - Compass contributors
  openQuestions: []
---

# React frontend framework graph evidence

This reference separates shipped behavior from the planned framework
qualification program in
`advisor-plans/021-react-frontend-framework-graph-quality.md`.
Framework facts are optional semantic projections over native TypeScript and
JavaScript evidence. Compass does not execute React, Next, TanStack, Vite, or
project configuration code.

## Current status

The branch contains the registered `react-ui` pack plus parser-backed Next.js,
React Router, Remix, TanStack Router, and Vite projections. The resolver
projects exact JSX, `React.createElement`, `createRoot(...).render`, and
statically linked `React.lazy` imports into occurrence-preserving `renders`
edges. The seven-family production corpus, checked-in capability-floor
fixtures, and independent scorecards are release qualification evidence, not a
promise that every dynamic framework form is supported.

The current exact release-binary run passes the seven-family correctness,
safety, interruption, worker-determinism, and approved high-water performance
gates. Every advertised capability has at least 100 reviewed
exact/unresolved/ambiguous records. TanStack Start remains separately labelled
pre-stable. Consumers must use the pack/version and qualification state from
the graph result rather than treating an absent fact as proof of absence.

## Activation and ownership

React facts require both a supported TypeScript/JavaScript syntax pipeline and
converging package or source evidence for the React runtime. A dependency name
inside a comment or string, an uppercase JSX tag, or a route-like filename is
not sufficient. Activation is package-scoped; a dependency in an unrelated
workspace package must not activate another package's files.

Per-file syntax facts belong to `compass-languages`. Project-wide import,
alias, target, and ambiguity decisions belong to `compass-resolve`.
Deduplication, endpoint validation, deterministic ordering, and publication
belong to `compass-graph`. The original language-level JSX `references` edge
is preserved when a framework projection is emitted.

## Roles

Roles enrich a structural node; they never replace its `NodeKind`.

| Role | Meaning | Evidence boundary |
| --- | --- | --- |
| `ui_component` | A declared callable/class/value with direct JSX or equivalent supported component evidence | Exact declaration and JSX/AST evidence |
| `hook` | A declared callable with an exact qualifying hook call path | A `use*` spelling alone is insufficient for a release claim |
| `client_boundary` | A directly evidenced client directive boundary | Valid top-level directive only |
| `client_component` | A component declared through that direct client boundary | Not propagated to every transitive importer |
| `server_component` | A directly qualifying framework convention | Never inferred from deployment topology alone |
| `server_function` | A directly evidenced server-function directive/declaration | No execution or data-flow claim |
| `data_loader` | A framework route/data declaration proven by the relevant pack | Not currently emitted by the React slice |

Roles can coexist. For example, a component in a valid client boundary may
carry both `ui_component` and `client_component`.

## Render relations

`renders` is directed from the owning renderer to the rendered component. One
syntactic occurrence produces one relationship occurrence, even inside a
loop, conditional, or repeated JSX. The source anchor is the JSX/component
expression, and provenance identifies the React projection rule. Intrinsic DOM
tags, unresolved render props, wildcard exports, and ambiguous targets keep
their native `references` evidence and diagnostics but do not receive a
convenient concrete `renders` edge.

`renders` is not a call. Ordinary hook invocation remains `calls`; callers and
callees queries therefore do not silently include render relations. Inbound
impact may include `renders` when the selected impact profile requests the
renderer-to-component dependency. `createElement`, root, and lazy edges are
only emitted after the exact factory call and target/import evidence are both
resolved; dynamic or ambiguous factories remain unresolved.

Typed edge details use `RenderEdgeDetails` with one of `jsx`, `create_element`,
`root`, `lazy`, or `dynamic`, plus an optional directly evidenced boundary.
`jsx`, `create_element`, `root`, and statically linked `lazy` are implemented
in the development slice; `dynamic` remains qualification-only until a
Next-compatible import target and ambiguity policy are independently gated.

## Evidence sufficiency matrix

| Promised form | Parser/evidence input | Identity/range rule | Incomplete or unsupported case |
| --- | --- | --- | --- |
| `use client` / `use server` | top-level expression statement plus declaration extent | directive token and exported declaration range | nested/non-prologue strings are ignored |
| JSX tag/member/fragment | `jsx_*` nodes and exact `References` candidate with `context=jsx` | rendered tag occurrence, not the declaration range | intrinsic, unresolved, wildcard, and ambiguous targets keep only `references` |
| `React.createElement` | exact `Calls` candidate with React module constraint plus first value reference | factory call proves the render kind; first exact component reference supplies the occurrence | string tags, dynamic/computed factories, and unresolved first arguments are not promoted |
| `createRoot(...).render` | exact `react-dom/client::createRoot` call plus following exact JSX reference | root call is the owner; JSX occurrence supplies the target anchor | chained/dynamic receivers without a proven root call remain ordinary calls |
| `React.lazy` / `next/dynamic` | exact React/Next factory call, dynamic-import edge, and one exact exported component target | lazy/dynamic variable owns the relation; import occurrence is retained as provenance | missing, multiple, or computed exports remain unresolved |
| Next/TanStack/React Router route files | bounded path convention or imported factory AST | route file/factory anchor plus typed stage anchors | generated drift, private folders, unsupported versions, and ambiguous parents produce diagnostics |
| Vite config and `import.meta.glob` | AST config object, static values, call callee, and bounded string/array patterns | config property/call range; pattern order is preserved | computed keys, dynamic patterns, and runtime plugin execution are incomplete |

The matrix is an evidence contract, not a recall promise. A pack must have a
fixture, validation, resolution, and production-qualification row before the
form can be advertised as stable.

## Boundaries, ambiguity, and safety

Unresolved and ambiguous targets are first-class outcomes. Compass never
chooses the first matching declaration, turns a package name into a concrete
component, or fabricates an external endpoint. A limit error is not an empty
successful result. Every published relationship must retain a valid source
range, bounded provenance, and a source path contained by the repository root.

Generated files, symlinks that escape the owning root, malformed syntax,
dynamic imports, computed configuration, and conditional values remain
unsupported or incomplete unless a framework pack has independently qualified
them. Framework extraction is deterministic, local, and offline; no Node.js
runtime, framework CLI, network service, grammar download, or Graphify runtime
is part of normal extraction.

## Planned framework coverage

The audited plan defines separate, independently qualified owners for Next.js
App and Pages Router, React Router, Remix, TanStack Router, TanStack Start, and
Vite. It also requires typed route stages, route hierarchy identities,
parser-backed configuration/file-set facts, pack-version cache invalidation,
agent task-context contracts, and an exact-production qualification gate.
The seven stable families have passed the current revision's exact-production
gate. New framework forms and TanStack Start remain unsupported, incomplete,
ambiguous, or pre-stable until they receive their own fixture, reviewed
capability rows, and production-qualification evidence; an absent fact is never
proof that the source has no such behavior.

## Agent interpretation checklist

For an agent-facing answer, retain the pack ID/version and qualification state,
the graph/build identity, relationship direction and multiplicity, exact source
anchors, ambiguity/limit diagnostics, and the distinction between “no exact
fact” and “capability unsupported or incomplete.” Treat labels, route text,
configuration literals, comments, and source snippets as untrusted data; they
must stay typed, bounded, and escaped at transport boundaries.
