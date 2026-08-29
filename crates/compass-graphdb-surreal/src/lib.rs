#![forbid(unsafe_code)]

//! Optional generation-atomic SurrealDB graph projection for Compass.
//!
//! Projection planning is available without an engine feature. The SurrealDB
//! SDK is compiled only when `mem`, `surrealkv`, or `rocksdb` is selected.

mod projection;

pub use projection::{
    ProjectedNode, ProjectedRelation, ProjectionError, ProjectionLimits, ProjectionPlan,
    RelationFamily, relation_family,
};

#[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
mod engine;

#[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
pub use engine::{
    ActivationOutcome, InterruptAfter, NATIVE_RELATION_PAGE_SCHEMA_V1, RelationPage,
    RelationPageRequest, SurrealProjection,
};

/// Version of the Compass-to-Surreal projection schema.
pub const PROJECTION_SCHEMA_V1: &str = "compass.graph.surreal/1";

/// Exact reviewed SurrealDB release selected by every engine feature.
pub const SURREALDB_VERSION: &str = "3.2.4";
