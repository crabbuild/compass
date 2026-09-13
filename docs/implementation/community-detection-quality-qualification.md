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

## Compact performance decision

On 2026-09-12, an aarch64 macOS debug build at candidate commit `46181d25` ran
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
