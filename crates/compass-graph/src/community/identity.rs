//! Stable identities for the currently shipped compatibility detector.

/// Native seeded Louvain implementation used by the compatibility profile.
pub const COMPATIBILITY_CLUSTER_ALGORITHM: &str = "seeded-louvain/v1";
/// Undirected weighted projection used by the compatibility profile.
pub const COMPATIBILITY_CLUSTER_TOPOLOGY: &str = "legacy-undirected/v1";
/// Partition evidence contract used before the quality-profile cutover.
pub const COMPATIBILITY_CLUSTER_QUALITY: &str = "density/v1";
/// Fixed-resolution selection policy used by the compatibility profile.
pub const COMPATIBILITY_CLUSTER_SELECTOR: &str = "fixed-resolution/v1";
/// Seed retained for deterministic compatibility with the historical detector.
pub const COMPATIBILITY_CLUSTER_SEED: u32 = 42;
/// Canonical profile encoding of [`COMPATIBILITY_CLUSTER_SEED`].
pub const COMPATIBILITY_CLUSTER_SEED_TEXT: &str = "42";
/// Work-limit policy for the compatibility detector.
pub const COMPATIBILITY_CLUSTER_LIMITS: &str = "community-limits/v1";

pub const QUALITY_CLUSTER_ALGORITHM: &str = "seeded-leiden-modularity/v1";
pub const QUALITY_CLUSTER_TOPOLOGY: &str = "typed-evidence-undirected/v1";
pub const QUALITY_CLUSTER_QUALITY: &str = "community-quality/v1";
pub const QUALITY_CLUSTER_SELECTOR: &str = "bounded-multiresolution/v1";
pub const QUALITY_CLUSTER_LIMITS: &str = "community-limits/v1";
