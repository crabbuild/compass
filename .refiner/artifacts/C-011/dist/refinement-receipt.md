# C-011 refinement receipt

Result: PASS

The user's explicit `ACCEPT` decision is recorded with exact provenance and the
closed semantics defined before approval: every named SurrealDB 3.2.4 artifact
profile is approved under its stated release conditions. The decision remains
limited to the pinned version and license digest, preserves all BSL notice,
redistribution, and Database Service conditions, and fails closed on upgrades.

C-012 and C-013 have retained completion evidence, and C-011's corrected
pre-archive fresh-context review passed with zero critical findings. C-011 is
published in the dated OpenSpec archive, satisfying the Wave 5 entry gate and
unblocking C-014/C-015. C-020 remains conditional on a separately recorded independent
user problem. This decision record adds no dependency or runtime behavior.

The C-013 plan precommitment remains byte-identical to its pinned SHA-256, and a
bounded re-fetch of the official raw v3.2.4 license matched the retained fixture
byte-for-byte at the pinned digest.

The final archived-state external review passed with zero critical findings and
three warnings; its strict anti-sycophancy gate passed. All findings remain in
the durable C-011 KBD review record.
