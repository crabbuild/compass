# Compass patch provenance

This directory contains the crates.io source for `gemm-common` 0.19.0,
published from upstream commit `86102c5b712737978371ac9ef7a11982f686d7bc`.
The unmodified source is MIT licensed; the upstream license is retained as
`LICENSE`.

Compass carries one platform-correctness patch in `src/simd.rs`: the four
AArch64 vector FP16 helpers are annotated with
`#[target_feature(enable = "neon,fp16")]`. Their callers already select the
FP16 path through runtime `feature_detected!("fp16")` dispatch. The annotations
therefore let LLVM assemble the guarded functions without making FP16 a
process-wide build requirement or allowing an unsupported CPU to execute
those instructions.

Remove this patch when a released `gemm-common` version contains the same
annotations and Candle can consume it.
