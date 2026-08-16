# Pull-request intelligence contract

Compass pull-request intelligence turns one frozen change request, two
comparable immutable graph realizations, and the canonical semantic diff into
one strict report. The machine contract is
`compass.pr_intelligence.report/1`. CLI text, Markdown, SARIF, MCP, and the
GitHub Action are projections of that report; they do not calculate risk.

## Exact identity

Every report binds all of these values:

- forge, host, owner, repository, and optional positive pull-request number;
- full merge-base, pull-request-head, and target-head Git object IDs;
- either the full deterministic synthetic-merge object ID, a conflict evidence
  digest, or an explicit unavailable reason;
- graph schema, extractor version, configuration/profile digest, and policy
  pack digest;
- the immutable evidence-manifest digest.

Local capture resolves commits and creates a synthetic merge with `git
merge-tree` plus a deterministic commit object. It does not switch the working
tree or fetch. GitHub capture reads paginated metadata, validates the local
objects and changed-file set, and rejects a base/head change observed during
capture.

## Canonical report

The top-level object contains:

| Field | Contract |
| --- | --- |
| `schema` | Exactly `compass.pr_intelligence.report/1` |
| `identity` | Repository, revisions, graph profile, policy, and manifest identity |
| `completeness` | Evidence coverage for this conclusion |
| `findings` | Strictly ordered canonical findings |
| `risk_factors` | Versioned integer-rubric inputs and points |
| `advisory_risk` | Optional score and advisory band |
| `gates` | Independent typed deterministic results |
| `omissions` | Exact canonical omission counts and reasons |
| `report_digest` | SHA-256 of canonical JSON without this field |

Unknown fields and enum values fail. Unknown report majors fail. Strings,
collections, hunks, findings, witnesses, locations, and rendered bytes have
hard bounds. A limit failure is not an empty or complete report.

Canonical JSON recursively orders object keys. Arrays whose order is part of
the contract are normalized by the producer. A finding fingerprint has the
form `cmpprv1:<sha256>` and includes typed finding kind, classifier version,
source/target entity identities, the selected shortest witness relationships,
and stable scalar evidence. Display text, source coordinates, timestamps, and
durations do not affect it. Entity or witness identity changes do.

## Completeness

| Value | Meaning |
| --- | --- |
| `local_exact` | Exact evidence for the selected local repository realizations |
| `downstream_complete` | The authorized downstream evidence set completed |
| `downstream_partial` | Some authorized downstream evidence was omitted or bounded |
| `downstream_unavailable` | Required downstream evidence could not be obtained |

Incomplete evidence can add uncertainty points or make a conclusion
unavailable. It never subtracts points. A non-clean synthetic merge makes the
advisory band unavailable and merge-dependent gates indeterminate.

## Advisory rubric version 1

The score is a deterministic bounded integer, capped at 100:

| Factor | Points per finding | Cap |
| --- | ---: | ---: |
| Public contract change | 20 | 40 |
| Affected caller or consumer | 4 | 24 |
| Cross-community dependency impact | 10 | 20 |
| Typed cycle evidence | 20 | 20 |
| Non-exact witness confidence | 4 | 16 |
| Verification gap | 12 | 36 |
| Incomplete evidence | 20 | 20 |
| Merge conflict | 30 | 30 |

Bands are `low` (0–19), `moderate` (20–44), `high` (45–69), and `critical`
(70–100). `unavailable` has no score. Risk is advisory only: neither the CLI
nor the Action turns a band into a merge gate.

Cross-boundary points require typed source and target community identities that
differ; an ordinary dependency change is not treated as a boundary crossing.
Cycle points require bounded, directed topology evidence that the changed
dependency participates in a cycle. Neither factor is inferred from finding
prose.

## Deterministic gates

Gate states are `pass`, `fail`, `indeterminate`, or `error`. The initial gate
is `proven-contract-break` rule version 1. It fails only when the semantic
classifier reports a proven break with exact confidence, the synthetic merge
is clean, and required evidence is complete. Conflicts and incomplete evidence
produce `indeterminate`, never a false pass or fail.

Advisory factors and gate rules are separate versioned contracts. Consumers
must decide policy from `gates[].state`, not from the score, band, finding
count, SARIF level, or prose.

## CLI

```text
compass review --base REV --head REV
  [--repo OWNER/REPO] [--host HOST] [--pull-request-number N]
  [--fingerprint SHA256]
  [--format text|json|markdown|sarif]
  [--output PATH]
  [--max-findings N --max-output-bytes N]

compass review --pr NUMBER --repo OWNER/REPO [--host HOST]
  [--fingerprint SHA256]
  [--format text|json|markdown|sarif]
  [--output PATH]
```

Local mode never fetches. `--repo`, `--host`, and
`--pull-request-number` can bind forge identity supplied by a frozen CI event
without calling GitHub again. GitHub mode uses `gh api`, freezes full IDs, and
requires the corresponding objects locally. `--output` uses atomic writing.
Markdown bounds report the exact projection omission count and do not mutate
the canonical digest.

On a fresh checkout with non-code files and no existing history profile, build
the target realization explicitly with `compass history build BASE --code-only`
before running review. This is the local structural path; semantic profiles
remain explicit and are never silently downgraded. The reusable GitHub Action
performs this preparation automatically.

Usage errors exit 2. Capture, history, profile, semantic, limit, and output
errors exit 1. A valid report exits 0 even when advisory risk is critical or a
deterministic gate reports `fail`; merge policy belongs to the Action or the
calling automation.

## MCP

`review_pull_request` accepts `base`, `head`, optional `fingerprint`, and the
standard optional `project_path`. It never fetches or builds. Both exact Git
objects and matching preferred immutable realizations must already exist. The
tool returns the canonical report in the `result` field of
`compass.mcp.tool-result/1`.

The structured response is bounded at 16 MiB. If the report does not fit, MCP
returns an explicit transport-limit error; it never truncates findings or
substitutes an empty report.

## Additive readiness envelope

`compass review --base REV --head REV --readiness --format json` emits
`compass.pr-readiness/1`. It references, but does not modify or embed, the
canonical report by `reportDigest` and repeats exact revision,
graph/extractor/configuration, base/comparison extraction fingerprints, and
evidence-manifest identities.

Facets cover signature/body findings, direct and transitive impact, static
related-test mappings and gaps, advisory documentation drift, and bounded
local Git ownership. Missing test evidence is `unknown`, never “untested.”
Documentation drift is advisory-only and includes exact `documents`
relationships when the canonical finding witnesses contain them. Ownership
uses bounded local history at the exact PR head and never contacts a forge.

MCP `pr_readiness` accepts the same `base`, `head`, optional `fingerprint`, and
optional `project_path` as `review_pull_request`. Its domain digest is also the
transport `semanticResultDigest`.

## Projections

- JSON is the canonical report and round-trips through the strict schema.
- Text and Markdown include exact identity, completeness, factors, gates,
  findings, witness paths/locations, verification gaps, and omissions. Finding
  statements in the canonical JSON and human-facing projections resolve
  retained entity identities to human-readable names; stable entity identities
  remain in `source_entities`, `target_entities`, and fingerprints for machine
  traceability.
- SARIF 2.1.0 preserves Compass fingerprints in `partialFingerprints` and
  carries the report digest, completeness, advisory result, factors, gates,
  witness evidence, and omissions in typed properties.

## Related pages

- [GitHub PR review guide](../guides/github-pr-review.md)
- [Commands](commands.md)
- [Outputs](outputs.md)
- [Security and privacy](../design/security-and-privacy.md)

**Next step:** generate JSON locally, validate its schema and digest, then add
the GitHub Action with `fail-on: none` before considering a deterministic gate.
