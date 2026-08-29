# Design: storage-neutral graph partitions

`compass-partition` owns only the partition container and deterministic encodings.
It deliberately does not depend on Prolly, Compass IR, or Compass analysis. Its
small segment encoder implements Prolly's existing escaped-zero format locally,
with golden tests pinning compatibility.

`compass-history` retains partition construction, persistence, completion evidence,
and program analysis. A thin canonical adapter converts `PartitionError` into
`HistoryError`; key functions and the container remain source-compatible re-exports.
