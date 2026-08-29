# Verification: honor graph-byte override on publication

PASS. The explicit override is parsed compatibly, defaults to 2 GiB on absent
or invalid values, and controls the complete canonical snapshot path. C-001's
bounded streaming read remains the production path.

Evidence: focused store/graph/CLI tests; affected all-target/all-feature
Clippy; workspace fmt, diff, Clippy, and lib/bin tests; strict OpenSpec;
`compass_product`; product boundary; code-graph fixture qualification; and a
successful Compass graph refresh with zero identity collisions.

Both independent K3 review calls returned no response, so no review verdict is
claimed.
