# Refinement log — C-015

## Iteration 1 — implementation closure

- Added the versioned engine-neutral structural query contract and current JSON
  implementation, plus generation-pinned closed Surreal statements for callers,
  callees, impact, directed path, subgraph, and stable relation pagination.
- Added public integration coverage for bounds, ambiguity, negative results,
  provenance, confidence, parallel/reverse/self-loop relations, cursor binding,
  page concatenation, and dual-engine semantic equivalence.
- Added deterministic medium/large samples and an explicit full-medium evidence
  runner.

## Iteration 2 — scale defect correction

- The first exact medium activation exposed 87.6 GB peak physical footprint.
  Sampling located repeated Surreal Mem B-tree cloning at transaction savepoints.
- Replaced the graph-sized transaction with idempotent 512-record staging
  commits, an immutable incomplete manifest claim, exact staged identity
  validation, and pointer-last publication.
- Re-ran the entire Mem integration surface and the exact medium measurement.
  It completed in 253.18 seconds with 3,646,308,352 bytes maximum RSS and zero
  semantic mismatches.

## Iteration 3 — pre-ratified decision

- Retained all raw samples, identities, limits, digests, commands, ratios, and
  decisions in `surreal-dual-engine-decision-v1.json`; the qualification
  manifest closes over the evidence.
- The candidate failed the <=1.10x query-regression threshold and both native-
  value conjuncts by several orders of magnitude. The threshold was not changed.
- Strict OpenSpec and every functional integration gate pass, but the product
  experiment is rejected under its own falsifier and is not eligible for archive
  as a completed accepted change.

The installed artifact-refiner adapter lacks its referenced canonical
controllers, schemas, and validator. The repository's established deterministic
fallback format was used. Overall result: **REJECT / BLOCKED BY RATIFIED GATES**.
