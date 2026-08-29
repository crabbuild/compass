# Verification

## Verification report: focused-agent-skills

| Dimension | Status |
| --- | --- |
| Completeness | 8/8 tasks; 4/4 requirements implemented |
| Correctness | 10/10 scenarios covered by build validation, unit tests, or installer contract tests |
| Coherence | Implementation follows the managed-sibling, collection-transaction, fixed-digest, and deterministic-corpus decisions |

No critical, warning, or suggestion issues remain. All checks passed; the change is ready for archive.

## Deterministic evidence

- The canonical umbrella remains byte-for-byte unchanged at SHA-256 `c6c097e081043c3f57cacb113423ff5783b0391f9881cb3262501277663a5d91`.
- `cargo test -p compass-cli --locked` passes, including all 24 installer lifecycle tests.
- `cargo clippy -p compass-cli --all-targets --all-features --locked -- -D warnings` passes.
- `cargo fmt --all -- --check`, `sh scripts/check_product_boundary.sh`, `openspec validate --all --strict`, and `git diff --check` pass.
- The final native baseline passes: `cargo clippy --workspace --lib --bins --locked -- -D warnings` and `cargo test --workspace --lib --bins --locked`.
- `SkillCollectionSnapshot` regression coverage proves restoration of all seven managed siblings after partial mutation while retaining an unrelated user skill.
- Windows drive and UNC detection rejects absolute paths without treating `https://` documentation links as drive paths.
- `cargo metadata --locked --no-deps --format-version 1` resolves both normal and build uses of the existing workspace `sha2` dependency without a lockfile update.
- `compass update .` completed after the final code edits: 711 files, 120590 nodes, 283773 edges, and 3398 communities. Compass reported a bounded partial graph with 68 omitted edges, 0 omitted nodes, and 0 identity collisions.

## Review adjudication

The first adversarial packet contained the repository-wide cumulative diff and omitted untracked focused assets, producing findings contradicted by the compiled tree. A second packet scoped the review to this change and included the untracked asset and OpenSpec files.

The second review's uninstall finding is contradicted by the current source and passing lifecycle tests. `is_managed_skill` first detects `.compass-install.json` and calls `verify_manifest(parent)`, which checks each focused directory against its own manifest and file digests. Only installations without a manifest use `legacy_skill_is_unmodified` and the umbrella digest fallback. The focused uninstall path therefore verifies the focused manifest before removing the last consumer, as covered by `every_project_platform_installs_native_content` and `uninstall_removes_one_shared_consumer_without_breaking_another`.

The second review correctly identified that the original Windows path marker matched two backslashes rather than an ordinary drive-qualified path. Validation now detects any ASCII drive prefix followed by `:` and either slash form, plus UNC prefixes, with focused unit coverage.

The final scoped review passed with 0 critical findings, 3 warnings, and 0 suggestions. Its cumulative-documentation warning describes unrelated pre-existing work in shared files rather than a C016 behavior defect. Its rollback-evidence warning is closed by `skill_collection_snapshot_restores_all_siblings_after_partial_mutation`. Its URL-scheme warning is closed by requiring a token boundary before a drive letter and covering `https://example.com/guide` as a negative case. The strict anti-sycophancy gate passed.

The installed `artifact-refiner` adapter could not run its promised PMPO pipeline because its referenced controllers, schemas, and domain files are absent from both the adapter and cached package. Deterministic OpenSpec, build-time validation, focused tests, full package tests, formatting, Clippy, and product-boundary checks provide the refinement evidence instead; no synthetic refiner state was created.
