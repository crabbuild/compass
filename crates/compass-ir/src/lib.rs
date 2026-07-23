//! Provider-neutral, provenance-aware Program IR.

mod canonical;
mod model;
mod validation;

pub use canonical::{canonical_json_bytes, hex_sha256};
pub use model::{
    BasicBlock, Capability, Coverage, CoverageState, EvidenceId, EvidenceRecord, FunctionIr,
    ModuleIr, Operation, OperationKind, ParameterIr, ProgramBundle, ProviderDescriptor,
    ProviderKind, SourceAnchor, SymbolId, Terminator, TypeRef,
};
pub use validation::IrError;

/// Stable serialized Program IR schema identifier.
pub const PROGRAM_SCHEMA: &str = "compass.program/1";
/// Numeric Program IR schema version used by caches and history.
pub const PROGRAM_SCHEMA_VERSION: u32 = 1;
