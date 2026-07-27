# Versioned History Performance Qualification

Date: 2026-07-26  
Machine: Apple Silicon Mac Studio, Darwin 25.5.0, local SSD  
Compass: 0.1.7 release build from `44ec831` plus the cache portability and
deterministic-rendering fix committed with this report

## Result

The hard-cutover implementation makes already-materialized history interactive:

- semantic diff is 0.13 seconds warm on CocoIndex and 0.23 seconds warm on
  Podman;
- historical viewer export is 0.06 seconds on CocoIndex and 0.09 seconds on
  Podman;
- no-op history build is 0.08 seconds on both repositories;
- first and repeated semantic/viewer outputs are byte-identical;
- both source qualification checkouts remain clean.

The remaining gap is first-time graph construction. It is substantially faster
than the old CocoIndex baseline, but it does not yet meet the design's
current-extractor-relative time or memory targets.

## Repositories and revisions

| Repository | Old | New |
|---|---|---|
| CocoIndex | `90571539fa291fc6e6b248095bd2c8a2ff68bab4` | `71f9cc9dc693080310181a2d011fb737420f7907` |
| Podman | `d8380c9c80d9c4acf5afd59b65c4c779aaacbbf5` | `7ac3e837075460ecdea5ce59e607cdaa6b6709fc` |

Qualification ran against clean shared clones under `/tmp`; it did not modify
the developers' existing CocoIndex or Podman checkouts. The Podman run shared
the host with another Compass debug build, so its wall-clock build measurements
are conservative.

## Release measurements

Peak memory is the maximum resident set reported by the qualification harness.

| Operation | CocoIndex time | CocoIndex peak RSS | Podman time | Podman peak RSS |
|---|---:|---:|---:|---:|
| Current cold extraction | 2.101 s | 1,176,080 KiB | 16.579 s | 2,606,512 KiB |
| Current incremental extraction | 2.332 s | 1,056,640 KiB | 18.071 s | 2,326,528 KiB |
| First unseen history build | 14.054 s | 2,154,448 KiB | 57.823 s | 5,716,208 KiB |
| Adjacent history build | 14.367 s | 2,359,712 KiB | 55.378 s | 5,035,920 KiB |
| No-op history build | 0.079 s | 14,432 KiB | 0.083 s | 12,464 KiB |
| First semantic diff | 0.389 s | 212,432 KiB | 1.021 s | 665,056 KiB |
| Repeated semantic diff | 0.126 s | 41,072 KiB | 0.228 s | 173,248 KiB |
| Historical viewer export | 0.061 s | 18,240 KiB | 0.087 s | 62,720 KiB |
| Repeated viewer export | 0.060 s | 18,288 KiB | 0.089 s | 62,704 KiB |

The viewer projection is warmed when a realization is published, so both
measured viewer operations exercise the normal cache-hit path.

## CocoIndex before and after

| Operation | Before | After | Improvement |
|---|---:|---:|---:|
| First semantic diff | 5.19 s | 0.389 s | 13.3x |
| Repeated semantic diff | 5.19 s | 0.126 s | 41.3x |
| Historical viewer export | 152.40 s | 0.061 s | 2,498x |
| Adjacent history build | 154.65 s | 14.367 s | 10.8x |

## Determinism

| Artifact | First SHA-256 | Repeated SHA-256 |
|---|---|---|
| CocoIndex semantic diff | `0d3ddc41989cde4f78f490d2b46605b049075b6b6368fd5e93112dab3780f4b2` | `0d3ddc41989cde4f78f490d2b46605b049075b6b6368fd5e93112dab3780f4b2` |
| CocoIndex viewer | `ce0c07a4859162b101451177e5dfbbb4f4c0701d59180f946113489c96f68427` | `ce0c07a4859162b101451177e5dfbbb4f4c0701d59180f946113489c96f68427` |
| Podman semantic diff | `75f79869f9c40bdc995d1668d978b1d43a250907f1dd6819e92a9bce1ef95b79` | `75f79869f9c40bdc995d1668d978b1d43a250907f1dd6819e92a9bce1ef95b79` |
| Podman viewer | `0c02ab7edbe4f2fbd7f830288421863cad3cf2701a51aa6fa43bdac2a6ad7c1a` | `0c02ab7edbe4f2fbd7f830288421863cad3cf2701a51aa6fa43bdac2a6ad7c1a` |

Each qualification performed one derived-cache miss followed by one hit for
semantic diff. Viewer publication warmed one cache entry and both exports hit
it. No reconstruction or full validation occurs on sealed no-op, diff, or
viewer paths; the contract tests cover those call boundaries.

## SLO assessment

| Contract | CocoIndex | Podman |
|---|---|---|
| First diff of materialized graphs | Pass: 0.389 s ≤ 1 s | Pass: 1.021 s ≤ 2 s |
| Repeated semantic diff | Pass: 0.126 s ≤ 0.25 s | Pass: 0.228 s ≤ 0.5 s |
| Existing viewer overview | Pass: 0.061 s ≤ 0.25 s | Pass: 0.087 s ≤ 0.5 s |
| No-op history build | Pass: 0.079 s ≤ 0.25 s | Pass: 0.083 s ≤ 0.5 s |
| Adjacent build ≤ 2x current incremental | Gap: 14.367 s > 4.665 s | Gap: 55.378 s > 36.143 s |
| First unseen build ≤ 1.25x current cold | Gap: 14.054 s > 2.626 s | Gap: 57.823 s > 20.724 s |
| Diff/view peak RSS below 512 MiB | Pass | Gap: first diff is 649 MiB; repeat is 169 MiB |
| History build RSS within 25% of current extraction | Gap | Gap |

The implementation therefore satisfies the interactive graph/diff/viewing
objective and makes first construction usable relative to the previous
baseline. A follow-up should profile immutable publication and exact-tree
validation, which now dominate build time and peak memory after extraction was
accelerated.
