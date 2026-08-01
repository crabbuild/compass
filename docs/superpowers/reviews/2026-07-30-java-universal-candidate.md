# Java Universal Candidate Quality Review

## Established-algorithm baseline

This review calls Java's pre-cutover implementation the **established
algorithm**. It is active production behavior, not a deprecated or translated
path. The Java increment will replace it in one hard cut only after the
version-1 universal candidate improves this baseline without a relation-family
regression.

The baseline was captured from clean Compass commit `6873268` before any Java
adapter or Java resolver production code changed. The commit differs from the
qualified Rust candidate only by a source-agnostic benchmark fix that streams
nested graph diagnostics one record at a time; it does not change extraction,
resolution, projection, or publication.

The checked Java vertical remained green:

- `fixtures/code-graph/routes/jvm/SpringController.java`
- `fixtures/code-graph/routes/jvm/NearMatches.java`
- `fixtures/code-graph/routes/jvm/PlayController.java`
- `fixtures/code-graph/domain/jobs/spring.java`
- `fixtures/code-graph/domain/messaging/spring.java`
- `fixtures/code-graph/domain/orm/jpa.java`

Release runs of `php_ruby_jvm_routes` and `domain_resolution` passed all 13
tests. They cover exact Spring route composition, near-match rejection, Play
Java/Scala handler resolution, scheduled jobs, message direction/transport,
JPA table mapping, and missing-target diagnostics. These checked behaviors are
the fixture regression contract for the candidate.

## Pinned Spring comparison

The full baseline used immutable inputs:

- Spring Framework: `eceebb3077dda9e1b19d73c0398ef022cd91f99c`
- Graphify: `4fe11092ccbe9f543608f140c790f68d5d83cae4`
- Compass: `6873268`
- Samples: three cold, warm, incremental, and restore runs per tool
- Harness run: `20260731T214529Z`

Every Compass and Graphify sample was eligible. Both tools produced stable
cold, warm, incremental, and restore canonical graph digests on this Spring
revision.

| Tool/workload | Eligible | p50 | p95 | Peak RSS |
|---|---:|---:|---:|---:|
| Compass cold | 3/3 | 70.143 s | 71.858 s | 6,808.03 MiB |
| Graphify cold | 3/3 | 167.905 s | 171.540 s | 1,490.92 MiB |
| Compass warm | 3/3 | 1.848 s | 2.337 s | 54.78 MiB |
| Graphify warm | 3/3 | 61.097 s | 61.776 s | 1,979.48 MiB |
| Compass incremental | 3/3 | 1.828 s | 1.853 s | 54.41 MiB |
| Graphify incremental | 3/3 | 61.718 s | 62.736 s | 1,975.08 MiB |

Compass was **2.394x faster cold**, **33.059x faster warm**, and **33.768x
faster incremental**. The generic 5x cold gate failed, and Compass's cold RSS
was higher. Those measurements remain visible but are not the Java candidate's
approved blocking gates. The candidate must keep lower cold and warm medians
than Graphify; RSS is recorded but non-blocking.

## Established graph quality

Compass published 152,516 nodes, 305,918 raw edges, and 9,134 communities.
Best-effort publication deterministically omitted 32,070 invalid or incident
nodes and 87,088 invalid or incident edges. The strict comparison index
deduplicated relationship payloads to 304,084 Compass edges and compared them
with 138,830 Graphify nodes and 477,066 Graphify edges.

Node classification handled 136,089 of 138,830 Graphify nodes (98.03%):
113,073 exact, 263 dominated, 22,753 rejected, 2,729 missing, and 12
ambiguous.

Edge classification handled 338,885 of 477,066 Graphify edges (**71.04%**).
Handled means exact, dominated by more precise Compass evidence, or explicitly
rejected because stronger evidence contradicts the Graphify binding. Missing
and ambiguous facts do not count as handled.

| Relation | Graphify | Exact | Dominated | Rejected | Missing | Ambiguous | Handled |
|---|---:|---:|---:|---:|---:|---:|---:|
| calls | 112,195 | 45,154 | 3,254 | 6,132 | 57,655 | 0 | 48.61% |
| case_of | 872 | 0 | 0 | 0 | 872 | 0 | 0.00% |
| contains | 102,310 | 93,524 | 4,018 | 0 | 4,747 | 21 | 95.34% |
| extends | 4,454 | 2,227 | 1,393 | 245 | 582 | 7 | 86.78% |
| implements | 4,379 | 1,221 | 2,414 | 565 | 176 | 3 | 95.91% |
| imports | 126,118 | 11,469 | 20,062 | 43,048 | 51,463 | 76 | 59.13% |
| references | 126,738 | 8,993 | 27,985 | 67,181 | 22,577 | 2 | 82.18% |

The Spring corpus contains Java, Kotlin, build metadata, documentation, and
package dependency facts, so the full comparison is deliberately broader than
Java syntax alone. The candidate gate nevertheless uses the same corpus and
classifier: overall strict coverage must exceed 71.04%, no relation-family
handled percentage may decrease, every Compass workload must remain eligible
and deterministic, and Compass cold/warm medians must remain below Graphify.

This is baseline evidence only. It does not qualify Java as a universal
candidate and does not authorize a dual-run or translation path.
