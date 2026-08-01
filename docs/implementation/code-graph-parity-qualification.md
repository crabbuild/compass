# Code-graph parity qualification

This report records the 2026-08-01 Compass-versus-Graphify qualification for
Python, Rust, Go, Java, TypeScript, and JavaScript. It is an implementation
checkpoint, not a claim that Graphify defines the upper bound for Compass.

> **Who this page is for:** contributors improving structural extraction and
> cross-file resolution.
>
> **You will learn:** which repositories were pinned, how parity was measured,
> which improvements are now protected, and which gaps remain open.

## Pinned repository matrix

| Language | Repository | Revision | Source size |
| --- | --- | --- | ---: |
| Python | encode/httpx | `b5addb64f0161ff6bfe94c124ef76f6a1fba5254` | 60 Python files, 17,753 lines |
| Rust | BurntSushi/ripgrep | `435f59fc4b43af3ab32f34d53fa34978f393fe52` | 110 Rust files, 56,376 lines |
| Go | etcd-io/bbolt | `55cb34b031c9855defb6c52db560a610f85bf5c3` | 121 Go files, 25,045 lines |
| Java | google/gson | `ff521d70b2ecf2ed85dd1012081493ad7d8c0ba6` | 264 Java files, 56,240 lines |
| TypeScript | date-fns/date-fns | `4098115cf705e3af7f663d8e5b0686e39a9f478a` | 1,589 TS/TSX files, 106,743 lines |
| JavaScript | axios/axios | `c3f553c740ebf3dff5e22dae24e9caaafafddd2d` | 187 JS-family files, 32,981 JavaScript lines |

The repositories are read-only qualification inputs. The Graphify observation
uses the bundled 0.9.31 checkout at revision
`3b243d6b3d17b4397f7d0f233fd8950984915e8d` in an isolated environment.
Neither Graphify nor that environment is a Compass runtime or test dependency.

## Method

Each repository was built three times into independent output directories with
the release `compass` binary using `--no-program --no-cluster --no-viz`. This
isolates the structural code graph from the separate Program IR product. The
comparison requires byte-identical `graph.json` and an identical canonical
occurrence digest across all three runs; no run may publish `program.json`.
Compass graphs must report zero validation errors. Default Program IR output
retains its own native cold/incremental byte-determinism coverage.

The shared correctness classifier compares source-grounded node and edge facts,
qualified ownership, source sites, relation normalization, and repeated
occurrences. It classifies Graphify facts as exact, dominated by stronger
Compass evidence, rejected as unverifiable or incompatible, ambiguous, or
missing.

Graphify emitted edges whose source or target node does not exist in every
repository. The comparator reports those dangling edges and excludes only those
well-formed-but-dangling records before semantic comparison. It continues to
fail on malformed records, conflicting node identities, Compass dangling
edges, or Compass validation errors.

## Final structural results

| Language | Compass nodes | Compass edges | Graphify nodes | Comparable Graphify edges | Raw dangling Graphify edges |
| --- | ---: | ---: | ---: | ---: | ---: |
| Python | 2,878 | 5,750 | 1,669 | 3,710 | 231 |
| Rust | 10,333 | 27,085 | 4,235 | 10,818 | 211 |
| Go | 3,099 | 11,315 | 1,364 | 3,602 | 686 |
| Java | 10,254 | 26,064 | 4,619 | 14,904 | 1,617 |
| TypeScript | 19,275 | 27,094 | 7,288 | 18,036 | 919 |
| JavaScript | 5,387 | 5,443 | 1,779 | 2,356 | 393 |

Node parity is complete for Python, Go, and Java. Remaining Graphify node facts
are 12 Rust, 97 TypeScript, and 114 JavaScript nodes, plus 6 ambiguous
TypeScript and 5 ambiguous JavaScript identities.

| Language | Exact edges | Dominated | Rejected | Ambiguous | Missing |
| --- | ---: | ---: | ---: | ---: | ---: |
| Python | 2,146 | 461 | 937 | 0 | 166 |
| Rust | 4,729 | 2,057 | 3,408 | 0 | 624 |
| Go | 2,525 | 710 | 331 | 0 | 36 |
| Java | 2,871 | 10,024 | 1,160 | 0 | 849 |
| TypeScript | 15,876 | 105 | 778 | 8 | 1,269 |
| JavaScript | 1,784 | 226 | 3 | 5 | 338 |

Compass already publishes substantially more validated topology, but the
remaining edge misses are real qualification work. In particular, raw edge
count alone is not evidence that every useful Graphify relationship has an
equal or stronger Compass fact.

## Improvements protected by this qualification

- Rust universal evidence now publishes explicit `type_of` and `returns`
  relationships and retains repeated call occurrences. The comparator credits
  an exact-endpoint, exact-occurrence typed relationship as stronger than a
  generic Graphify reference; 523 ripgrep return facts meet that stricter rule.
- Rust module identity retains the source-backed crate name for packages rooted
  directly under `crates/<name>` without a conventional `src/` directory.
  `crate`, `self`, and `super` imports therefore meet the same declarations
  across ripgrep's custom binary layout. Local generic impl types are matched
  through their outer nominal path only; reference, foreign, and unresolved
  generic owners remain unowned. Together these corrections add 1,706
  normalized ripgrep edges, reduce call misses from 166 to 56, containment
  misses from 374 to 55, and implementation misses from 126 to 8. Rust adapter
  evidence advances to version 2 so cached version-1 facts refresh.
- Comparator adjudication rejects an anchored behavioral or type edge only
  when its exact Compass endpoints prove a cross-language target, and credits
  a field declaration's exact `type_of` occurrence over Graphify's flat
  owner-level reference. On ripgrep this rejects 494 Graphify Rust references
  to an unrelated Python `Result` class and recognizes 124 more precise field
  relationships. Named generic impl nodes are mapped only to one same-file,
  source-backed code type. Rust's total missing count falls from 2,134 to 624
  without treating Graphify as an oracle.
- Go resolution uses lexical receiver types, explicit variables, method return
  types, and imported local package types without guessing through ambiguity.
- Go interfaces publish their method declarations at the exact interface
  source sites with interface ownership, signatures, bindings, and return
  evidence. This adds 29 source-grounded nodes and 65 edges on bbolt and makes
  six Graphify interface-to-concrete-method ownership claims explicitly
  rejectable instead of silently omitting the real interface declarations.
- Go call-form expressions resolve against both callable and named-type
  namespaces: functions remain `calls`, while conversions become exact
  `references`. Package-qualified lookup excludes receiver methods before
  resolving package members, chained receivers follow exact local return
  types, and named result parameters retain their declared receiver type. On
  bbolt this adds another 140 validated edges, raises exact call coverage by 7,
  identifies 52 Graphify call edges as type conversions, and cuts total Go
  misses from 128 to 90.
- Go range values inherit an element type only from a source-grounded slice,
  array, map, or channel return/member type, and only for the second variable
  in a two-variable range. One-variable indexes/keys remain untyped. This adds
  five exact bbolt calls for `Bucket.inlineable/free/write` and
  `node.size/write`, reducing total Go misses again from 90 to 85.
- Go index expressions reuse that exact collection-element channel for method
  receivers. Exact collection owners keep same-named element methods distinct;
  on bbolt this covers five more calls (two exact and three dominated by more
  precise owner evidence) and reduces total Go misses from 85 to 80.
- Local named collection declarations preserve their exact element type for
  indexed parameters and built-in `make` initializers. This adds 11 exact
  bbolt calls, including `Inode` accessors and mutators, and reduces total Go
  misses from 80 to 69 without inferring through imported aliases.
- Owner-qualified Go field metadata now follows direct selector chains without
  crossing same-named fields on unrelated owners. Exact return/range handoff
  also survives a same-named package declaration shadowing the range value. On
  bbolt this adds five calls and reduces total Go misses from 69 to 68.
- Full Go import paths are joined to the longest exact repository directory
  suffix before external fallback. On bbolt, 60 call occurrences now target
  declarations under `internal/common` or `internal/freelist`, removing 34
  inferred external identities without changing call multiplicity or Graphify
  coverage counts.
- Go result annotations publish the stronger `returns` contract rather than a
  generic reference. This includes APIs such as `Cursor() *Cursor`, whose
  method and result names coincide. Bbolt gains 200 typed return edges,
  recovers 19 previously suppressed result facts, and removes 27 false calls
  to `time.Duration` and `unsafe.Pointer` conversions. Nine more Graphify
  reference misses become covered, reducing total Go misses from 68 to 59.
- Unique Go call results now carry the exact callable identity across file and
  import boundaries. The resolver follows the callable's typed `returns`
  evidence only when both are unique; unpositioned multi-result calls, foreign
  packages with the same terminal name, and ambiguous declarations fail closed. Exact
  root-module imports are proven against the bounded `go.mod` module directive.
  On bbolt this raises exact call coverage by 54 and exact reference coverage
  by 20, replaces false root-package targets with `internal/btesting` and
  `internal/common` declarations, and removes inferred full-module-path
  placeholders. Total Go misses fall from 59 to 45 while normalized graph
  edges rise from 10,117 to 11,294.
- Embedded Go fields retain their selector-visible member type in addition to
  embedding evidence. Explicit selectors can therefore resolve to the
  embedded method declaration instead of degrading into a reference to the
  outer receiver type; bbolt's surgery command options are the qualification
  cases. Four Graphify self-targeted override calls are now rejected as
  incorrect, two explicit `log.Logger` calls replace references to the outer
  logger type, and total bbolt misses fall from 45 to 41 without adding an
  ambiguous result.
- Go package identity now combines the repository-relative directory with the
  parser-proven package clause. External `command_test` helpers no longer
  collide with production `command` helpers in the same directory. On bbolt,
  five calls become exact (`readMetaPage` in three sites and `fileSize` in two),
  normalized edges rise from 11,294 to 11,306, and total misses fall from 41
  to 36 with zero ambiguity.
- Positional Go assignments such as `page, node := bucket.pageNode(id)` retain
  the exact selected result index in their call-result binding. Ordered return
  evidence recovers nine additional bbolt calls that Graphify does not model,
  including `Page.IsBranchPage`, `Page.Count`, `Page.BranchPageElement`,
  `Page.Id`, `Page.Flags`, and `Meta` accessors. Normalized edges rise from
  11,306 to 11,315 with no new node or ambiguous resolution.
- Directly observed declaration ownership carries the exact child identity,
  including same-named overloads and Python getter/setter pairs, and always
  publishes the graph-contract `contains` relation. This removes all 13 HTTPX
  containment misses, 30 bbolt containment misses, and another 9 ripgrep
  containment misses without changing node identities.
- Comparator ownership adjudication rejects a cross-type Graphify containment
  only when Compass has one exact, source-anchored code-type owner. It rejects
  the six bbolt claims above and nine Gson nested-type identity collisions,
  while retaining configuration-object containment as unresolved evidence.
- Java callable ownership now carries the exact declaration fact emitted from
  the same syntax node instead of re-resolving by owner, name, and arity. On
  Gson this publishes 103 additional normalized ownership edges, preserves
  same-name/same-arity overloads, and advances the Java adapter to version 2.
  The comparator also normalizes Graphify's legacy `case_of` spelling to
  canonical `contains` and recognizes only a unique, cycle-safe containment
  path within depth 8 and 4,096 explored states. All 3,753 comparable Gson
  containment claims are now accounted for: 1,056 exact, 2,688 dominated by
  richer Compass hierarchy, and 9 rejected owner conflicts, with no missing
  or ambiguous result.
- Java parameter declarations and call arguments now retain bounded canonical
  type vectors. The resolver uses them only when every argument is statically
  known and exactly one same-arity overload has the identical vector. It also
  treats multiple same-named member bindings as an overload set instead of
  silently selecting the first declaration. On Gson this adds 893 normalized
  edges and two correctly scoped `java.lang` external call targets, reduces
  call misses from 1,313 to 705, and explicitly rejects 164 additional
  Graphify overload targets. Representative corrections include selecting
  `newFactory(Class, TypeAdapter)` at line 993 rather than the unrelated
  line-981 overload; nested-call arguments and incomplete vectors still fail
  closed.
- Clean Java type declarations mark their direct-base set complete, including
  `interface extends` relationships that were previously omitted. After exact
  type-vector matching, the resolver may prove primitive widening,
  boxing/unboxing, array, complete local hierarchy, and a bounded set of
  stable core-Java conversions. It selects only a unique most-specific
  parameter vector and treats unknown external hierarchy as a competing
  possibility. This adds 398 previously unresolved Gson call edges without
  removing or retargeting an existing call, reduces call misses from 705 to
  429 and total misses from 1,125 to 849, and advances the Java adapter to
  version 3.
- Java preserves anonymous enum/object bodies, direct generic supertypes, and
  distinct evidence candidates for exact targets and hierarchy paths.
- JavaScript and TypeScript decompose object, array, rest, and assignment
  bindings into explicit variable facts.
- TypeScript masks the unsupported `type` modifier in `export type * from`
  only for parsing, retaining byte-exact anchors and all source-grounded barrel
  imports and re-exports. On date-fns this raises exact export coverage from
  17 of 764 facts to 758 of 764 and cuts total TypeScript misses by 1,515.
- JavaScript-family named imports retain package specifiers and local/imported
  binding identities even when the specifier is not relative. The resolver
  reads only bounded, inventoried `package.json` files, follows repository-local
  `exports` targets and bounded wildcard barrels, recognizes NodeNext `.js` to
  `.ts` source aliases, and requires a unique package name, export target, and
  declaration. Duplicate package names and conditional targets remain
  unresolved. On date-fns this adds 855 validated edges, cuts import misses
  from 787 to 24, and rejects 191 Graphify imports that conflict with exact
  repository declarations.
- Imported JavaScript-family function values used as call arguments or object
  and array members publish exact `references` occurrences to their resolved
  declarations. They are not mislabeled as calls. The comparator rejects an
  inferred Graphify `indirect_call` only when an exact Compass value-reference
  occurrence proves the relationship or target wrong. Date-fns adds 878 such
  references, rejects 587 false indirect calls (including fp wrapper aliases
  incorrectly rebound to an unrelated test helper), and reduces call misses
  from 593 to 6. Extraction semantics version 4 invalidates older cached facts.
- JavaScript and TypeScript now publish source-anchored `extends` and
  `implements` relationships, resolving named and aliased imports to exact
  definitions. Exact date-fns inheritance coverage rises from 0 to 255 facts;
  another 50 multiline bases are dominated by Compass's more precise per-base
  anchors. Axios's six source-code inheritance facts are all exact, while its
  remaining Graphify `extends` facts come from configuration data.
- An offset-preserving parser mask for indexed `typeof import(...)` type
  queries prevents one valid TypeScript namespace file from being quarantined
  by the pinned grammar. The original source remains authoritative for names,
  hashes, and anchors.
- Markdown document identities remain distinct across same-named files, and
  portable file-stem aliases retain `documents` edges only when the target is
  unique.
- External placeholders are scoped by wiring evidence and cannot retain
  trusted provenance without source anchors.
- MCP configuration accepts the top-level `servers` shape used by current
  editor configurations.

The native fixture qualification validates the strict manifest and passes the
in-process scale ceilings. Clean, warm, forced-clean, restored-source, and
alternate-checkout graph bytes agree. The fixture-only gate covers 57
languages, 24 frameworks, 24 exact flows, and 1,427 invariants.

## Performance checkpoint

The table compares a single prior cold Graphify observation with the median of
three independent cold Compass structural-only runs. Go and Rust use six runs
because their observed wall-time variance was material. Times are process wall time;
memory is peak resident set size. They are useful directional evidence, not a
controlled cross-machine benchmark.

| Language | Graphify wall | Compass wall | Graphify peak RSS | Compass peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Python | 2.58 s | 0.92 s | 92 MiB | 126 MiB |
| Rust | 2.62 s | 2.89 s | 57 MiB | 336 MiB |
| Go | 1.17 s | 1.31 s | about 51 MiB | 167 MiB |
| Java | 4.71 s | 2.83 s | 73 MiB | 470 MiB |
| TypeScript | 13.19 s | 3.64 s | 250 MiB | 439 MiB |
| JavaScript | 2.12 s | 0.85 s | 120 MiB | 172 MiB |

Compass is faster in four of the six external structural process measurements
at this checkpoint. Go's latest sample has a 1.31-second median versus the
single prior 1.17-second Graphify observation. Rust's latest six-run wall-time
median is 2.89 seconds versus Graphify's prior 2.62 seconds. Interleaved runs
show lower Compass user CPU time than the preceding checkpoint, but material
host scheduling variance makes the wall result inconclusive; the measured gap
therefore remains open rather than being hidden by the additional exact
topology. The explicit
`--no-program` boundary still lowers Compass
peak RSS by 24–34% versus the earlier combined graph-and-Program measurements.
It does not yet surpass Graphify on peak RSS in any selected repository.

The TypeScript workspace-package and exact value-reference passes increase the
date-fns median from the earlier 2.29-second checkpoint to 3.64 seconds and
peak RSS from 426 MiB to 439 MiB while adding 1,733 validated edges. It remains
more than three times faster than the 13.19-second Graphify observation, but
the time and memory increase is a measured regression to address rather than
hide behind the correctness gain.

The resolver now consumes semantic-evidence batches instead of cloning the
full corpus into its index. Structural-only extraction also skips construction
of the independent Program evidence batch, and AST cache publication streams
one portable per-file snapshot at a time rather than cloning and retaining the
whole corpus through resolution. Medium builds now avoid parallel temporary
corpora during discovery, resolution, and graph normalization, and graph-v1
publication consumes prepared edges incrementally. Together these changes
materially reduce the peak: ripgrep falls from the earlier 732 MiB checkpoint
to a 336 MiB six-run median while adding the Rust version-2 topology. All six
repositories retain three-run byte-identical graphs at their latest correctness
checkpoints.

## Open qualification work

The parity goal remains open until the missing and ambiguous source-grounded
facts are explained or replaced by stronger evidence and the remaining peak
memory gap is materially reduced. Future changes must preserve the three-run
determinism requirement and must not weaken validation, provenance, ambiguity
handling, or bounded execution to improve a count.
