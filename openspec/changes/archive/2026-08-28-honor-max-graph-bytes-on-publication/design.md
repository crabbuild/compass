# Design: effective snapshot byte limit

`compass-store::max_graph_bytes` owns parsing and matches existing reader forms:
raw bytes, `MB`, and `GB`, with optional numeric underscores. The function
returns a finite `usize`; invalid inputs fail closed to `MAX_GRAPH_BYTES`.

`compass-graph::max_canonical_graph_bytes` exposes that effective limit at the
graph ownership boundary. C-003 preflight and every canonical snapshot
publication/validation path consume it. C-001 already moved production reads
to bounded streaming, satisfying the hard gate for raising the publication
limit without reintroducing a mandatory contiguous payload allocation.
