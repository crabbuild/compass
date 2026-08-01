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

The section above is the pre-cutover baseline. It does not authorize a
dual-run or translation path.

## Post-cutover qualification

Java's hard-cut version-1 universal path was qualified on 2026-08-01 from the
working tree based at Compass commit `dd14b3ce99b02fd82aa5df199cd5769869916c97`.
The pinned Spring and Graphify revisions are unchanged from the baseline. Java
remains `UniversalCandidate`; this review does not promote it to
`UniversalComplete`.

The six checked Java fixtures were extracted three times with the final release
binary. All three graph files were byte-identical, their canonical graph
digests matched, and their occurrence digests matched:

- graph bytes SHA-256:
  `40524a2d9ce8169348572ba7eb082338c648c740688a9a03e31648b2f4d82af0`
- canonical graph SHA-256:
  `43c72e8038181fdf44a635a93eb9f5be27813903a930c35de979faf0bf00ebcd`
- occurrence SHA-256:
  `3e1916d0cc1f11db3ecd5c9b4f7e486531b265d3e8092250c3b201fbad54f1a3`
- each run: 45 nodes, 60 edges, and no graph diagnostics

The candidate handled all 28 established fixture relationships and all 41
valid Graphify fixture relationships. The Graphify fixture output contained
one dangling import to `RequestMethod`; it was recorded and removed before
strict comparison. Candidate fixture retention is therefore 100%, above the
95% baseline-retention gate, with no missing or ambiguous relationship in any
checked family. Imports handled 10/10 Graphify facts and references handled
13/13.

### Final pinned Spring quality

The final strict index contained 168,475 Compass nodes and 650,782 canonical
Compass edges. It compared them with 138,830 Graphify nodes and 477,066
Graphify edges. Both inputs had zero validation errors and zero dangling
edges. The same final classifier was also run over the established Compass
graph, so the family comparison below is apples-to-apples.

| Relation | Graphify | Established handled | Candidate exact | Candidate dominated | Candidate rejected | Candidate missing | Candidate ambiguous | Candidate handled |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| calls | 112,195 | 48.51% | 15,016 | 65,581 | 7,415 | 24,183 | 0 | 78.45% |
| case_of | 872 | 0.00% | 0 | 0 | 0 | 872 | 0 | 0.00% |
| contains | 102,310 | 95.19% | 32,694 | 65,506 | 0 | 4,103 | 7 | 95.98% |
| extends | 4,454 | 86.06% | 2,130 | 1,630 | 144 | 537 | 13 | 87.65% |
| implements | 4,379 | 95.39% | 1,988 | 2,054 | 202 | 132 | 3 | 96.92% |
| imports | 126,118 | 59.13% | 31,055 | 38,021 | 56,183 | 859 | 0 | 99.32% |
| references | 126,738 | 81.94% | 12,009 | 86,429 | 14,361 | 13,931 | 8 | 89.00% |

Overall handled coverage is 432,418/477,066, or **90.64%**, compared with
338,261/477,066, or **70.90%**, for the established graph under the final
classifier. Candidate coverage is 127.84% of the established baseline and
exceeds it by 19.74 percentage points. No relationship family regressed;
`case_of` is unchanged and every other family improved.

The largest implementation gaps discovered during qualification were in
calls, imports, and containment. The final candidate adds source-anchored,
arity-constrained ownership for Java methods and constructors, preserves
annotation types through Code Graph v1 normalization and endpoint validation,
and makes the comparator distinguish Java owner spelling and annotation-shifted
declaration anchors without selecting arbitrary candidates.

### Final pinned Spring performance

The three-sample candidate measurements before the final ownership correction
had medians of 82.86 seconds cold and 3.23 seconds warm. The final binary was
then confirmed at 85.46 seconds cold and 3.55 seconds warm on the same pinned
Spring checkout (`compass --timing` reported 79.41 and 3.53 seconds inside
those process-wall measurements). Its cold and warm graph files were
byte-identical with SHA-256
`5d312789a82d8763173e496a76af055d17ffef17e3aad46de1e6a4e1fe51cd69`.
The pinned Graphify medians were 171.07 seconds cold and 64.70 seconds warm.
Compass therefore remained lower-latency for both blocking workloads; final
cold was 2.00x faster and final warm was 18.23x faster. Peak RSS remained
non-blocking and was recorded at 8.61 GiB cold and 51.72 MiB warm for the final
confirmation.

### Remaining scope

This closes Java's post-cutover candidate qualification without promoting Java
past `UniversalCandidate`. The former Spring follow-up is now implemented by
the production `spring-java` universal framework pack at Compass commit
`1946012aa67bba474f4016aa2d9f79010a3c1476`; its independent evidence is
recorded in the 2026-08-01 Spring universal-pack review. Java Spring no longer
runs through the legacy source detector. Kotlin Spring remains on the
established framework path and is not covered by the Java hard cut.
