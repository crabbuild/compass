# Community detection quality qualification

This report records deterministic fixture qualification for the version-1
typed topology, native Leiden detector, quality evidence, and bounded selector.
The machine-readable authority is
[`community-detection-quality-qualification.json`](community-detection-quality-qualification.json).

## Qualified identities

```text
algorithm   seeded-leiden-modularity/v1
topology    typed-evidence-undirected/v1
quality     community-quality/v1
fixed       fixed-resolution/v1
automatic   bounded-multiresolution/v1
seed        42
limits      community-limits/v1
```

The production cutover uses fixed resolution. The automatic selector remains a
qualification capability, not an omitted-flag default.

## Deterministic fixture result

The native runner covers 15 fixture families and repeats each input with node
and edge order reversed. All checked acceptance fields pass:

| Gate | Result |
| --- | --- |
| Connected selected communities | pass |
| Repeat and permutation equality | pass |
| Required exact planted recovery | pass |
| No ARI regression greater than 0.02 | pass |
| Ring-of-cliques improvement | pass |
| Articulation improvement | pass |

The ring-of-cliques fixture improves from ARI `0.5823389021` and 5 false
merges under compatibility Louvain to ARI/AMI `1.0` with no false merges or
splits. The articulation fixture improves from ARI `0.55`, one false merge,
and one false split to exact recovery. Dense groups and required deterministic
LFR-style fixtures retain exact recovery.

Run and byte-compare the report with:

```bash
./scripts/qualify_code_graph_v1.sh --community-quality \
  --report docs/implementation/community-detection-quality-qualification.json
```

## Community hierarchy

The budgeted hierarchy has its own report:

```bash
./scripts/qualify_code_graph_v1.sh --hierarchy \
  --report docs/implementation/community-hierarchy-qualification.json
```

`compass.community-hierarchy-qualification/1` runs three shapes the real
corpus publishes — clustered directories, communities that share no
relationship at all, and groups that cite no location either — twice each and
byte-compares the reports. Acceptance requires:

| Entry | Meaning |
| --- | --- |
| `rootBudgetSatisfied` | every fixture's root count matches the budget its shape can support, and at least one fixture proves the budget is reachable |
| `completeTree` | every level's children partition the level below exactly once with matching member counts, checked independently of the builder |
| `labelsHaveProvenance` | every group label is non-empty, carries evidence, and marks itself generic exactly when it is a community id |
| `genericRootLabelsBounded` | at most a quarter of the root groups are unnamed |
| `deterministicDigest` | two builds of one fixture serialize identically with equal digests |
| `boundedLevels` | no fixture exceeds its level budget and every coarser level merges groups |

The fragmented fixture is the measured `colinhacks/zod` shape: it merges by
shared location (`locationAffinity`) because relationship evidence alone cannot
reduce it. The location-less fixture asserts the opposite: the artifact reports
`budgetSatisfied: false` instead of merging groups nothing connects.

### Hierarchy stability

```bash
./scripts/qualify_code_graph_v1.sh --hierarchy-stability \
  --report docs/implementation/community-hierarchy-stability.json
```

`compass.community-hierarchy-stability/1` replays one fixture repository
through a fixed edit sequence — add symbols to one community, move a file
between two directories, delete a community — rebuilding and reconciling after
each step, and runs the whole replay twice before byte-comparing the reports.

| Entry | Measured |
| --- | --- |
| `rootBudgetSatisfied` / `completeTree` | every generation's root fits its budget and partitions exactly |
| `ariAtLeastThreshold` / `amiAtLeastThreshold` | ARI and AMI between consecutive generations over the members they share, threshold 0.5 (measured 1.0 on this fixture) |
| `stableIdsForUntouchedGroups` | groups whose member set did not change kept their id (5/5, 6/6, 5/5 through the sequence) |
| `splitMergeEventsMatchEdits` | no event names an untouched group, and the delete is the only edit that removes one |
| `ambiguousEventsReportedNotResolved` | a forced even split reports `ambiguous` and no successor inherits the id |
| `deterministicDigest` | two replays produce identical reports and digests |

## Compact performance decision

On 2026-09-12, an aarch64 macOS debug build at candidate commit `7e216079` ran
all 15 compact fixture families 50 times per sample. Seven-process medians were:

| Profile | Median | Compatibility ratio |
| --- | ---: | ---: |
| compatibility Louvain | 1.18 s | 1.00× |
| fixed-resolution typed Leiden | 1.11 s | 0.94× |
| three-candidate typed Leiden | 1.34 s | 1.13× |

These intentionally small debug fixtures magnify topology/evidence setup cost
and are not the pinned real-repository release performance oracle. The optimized
fixed and automatic profiles clear the compact clustering overhead gate, but
automatic selection still requires the complete pinned-corpus release matrix.
The production profile therefore stays fixed-resolution. No cold-build or
peak-RSS claim is inferred from these compact numbers.

The corresponding release-mode FastAPI qualification records fixed Leiden at
`0.986×` the compatibility median on the max-inference graph and `0.944×` on
the low-inference graph, with byte-identical partition and quality output. See
[`PERFORMANCE.md`](../../PERFORMANCE.md#community-detection-performance) for
the corpus identity, graph digests, sampling method, latency, and peak-RSS
evidence.

## Corpus status and omissions

The checked-in gate is completely offline and deterministic. A cold build of
the available pinned Rust corpus (`kache` at
`1a6a4a6067ab98c2867cb2278a033e0a208ff4b9`) was attempted, but both clustered
and `--no-cluster` runs remained in pre-publication extraction work long enough
that they were stopped; no clustering-stage comparison can be inferred from
those incomplete observations. It does not claim
that paths or package names are ground truth. Release qualification must still
record reviewed expectations, exact commits, cold build time, clustering time,
peak RSS, and incremental measurements for the pinned repository corpus. A
missing ecosystem is an explicit corpus omission, not a passing measurement.
The release-mode FastAPI detector timing in `PERFORMANCE.md` closes the Python
latency and peak-RSS observation for fixed Leiden, but it does not provide the
reviewed subsystem oracle or the remaining ecosystem matrix. This fixture
report therefore cannot authorize the automatic selector as the default.

## Related pages

- [Community detection concept](../concepts/community-detection.md)
- [Technical design](community-detection-quality-technical-design.md)
- [Performance qualification](../../PERFORMANCE.md)

**Next step:** run the pinned real-repository performance matrix before a
future change enables bounded automatic selection in production.
