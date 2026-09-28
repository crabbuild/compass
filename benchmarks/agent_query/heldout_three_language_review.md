# Three-language held-out code-graph review

This is a focused source-selected comparison registered before building either
tool's graphs in `heldout_three_language_registration.json`. It covers Anyhow
(Rust), MarkItDown (Python), and Vue (TypeScript), at pinned source commits. The
suite has twelve explain, caller, callee, and path questions. It does not
measure population accuracy, community quality, god-node diagnosis, or broad
language superiority.

## Capture and correction

- The first paired capture completed Anyhow and MarkItDown. Compass's Vue
  extraction exceeded the registered 1,800-second build limit, so the capture
  has no complete twelve-question verdict. Preserve that timeout as a failure.
- A TypeScript receiver-resolution change moves a repeated receiver lookup to
  the only return-type branch that uses it. A subsequent Vue-only replay
  completed Compass extraction in 75.996 seconds; Graphify took 4.246 seconds.
  This is one post-development observation, not a representative speed result
  or a paired twelve-question rerun of one frozen Compass revision.
- Both tools failed all four registered Vue text questions in the replay. The
  `baseCompile` named function expression at `src/compiler/index.ts:10` is
  absent as a declaration in both graphs. Compass assigns the source calls at
  lines 14, 16, and 18 to the module; Graphify assigns them to the
  `createCompiler` variable. Neither is the source-proven owner.
- A separate Compass query correction makes a unique `Owner::member` suffix
  resolve in the same bounded manner as `Owner.member`. On the captured Anyhow
  graph, `callers Chain::new` now returns `answered` and includes the direct
  `ErrorImpl::chain` call at `src/error.rs:985`. This query was a `no_match` in
  the original capture. The correction was made after registration and is not
  retroactively counted as a held-out result.

## Independent source-site review

`heldout_three_language_edge_review.py` checks thirteen registered direct
call occurrences against exact source owner and target declarations, direction,
and call-site line. Its JSON output retains graph hashes and row-level evidence
outside the repository. The first two repositories use the original paired
graphs; Vue uses the post-change replay.

| Source | Reviewed calls | Compass exact | Graphify exact | Finding |
| --- | ---: | ---: | ---: | --- |
| Anyhow | 2 | 2 | 0 | Graphify links the same-named `chain` and `new` calls to wrong declarations. |
| MarkItDown | 8 | 8 | 7 | Graphify omits one repeated `convert_local` call occurrence. |
| Vue | 3 | 0 | 0 | Both omit the `baseCompile` source declaration. |
| Total | 13 | 10 | 7 | Selected sites only; no whole-graph precision estimate. |

Graphify's MarkItDown graph still contains the direct relationships for the
other seven occurrences. Its short-name CLI queries collided on several
methods, while exact-ID controls recovered some answers. Exact-ID controls
also showed that its path command did not recover the direct `convert_stream`
to `_convert` route despite the graph edge. These are distinct graph and query
findings. The runner's text-required verdict is only a recall proxy and is not
used as the final source-supported score.

## Remaining work

1. Publish named function expressions and their calls under the correct
   lexical owner, with ambiguity and shadowing regression cases; replay Vue.
2. Run a fresh full twelve-question panel with one frozen Compass revision,
   independently grade every answer, and retain complete raw artifacts.
3. Extend source-site and navigation review across more languages and assess
   community and god-node tasks before claiming overall superiority.

The earlier audit's raw external captures are not present on the currently
mounted volume. Its committed summaries remain, but those original outputs
cannot currently be replayed from retained artifacts. This new panel's raw
captures and review JSON are retained on the mounted workspace volume.
