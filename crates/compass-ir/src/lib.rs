//! Provider-neutral, provenance-aware Program IR.

mod canonical;
mod model;
mod validation;

pub use canonical::{canonical_json_bytes, hex_sha256};
pub use model::{
    BasicBlock, Capability, Coverage, CoverageState, EvidenceId, EvidenceRecord, ExceptionEffect,
    ExceptionKind, ExecutionMode, FunctionIr, ModuleIr, Operation, OperationKind, ParameterIr,
    ParameterKind, ProgramBundle, ProviderDescriptor, ProviderKind, SourceAnchor, SymbolId,
    Terminator, TypeRef, Visibility,
};
pub use validation::IrError;

/// Stable serialized Program IR schema identifier.
pub const PROGRAM_SCHEMA: &str = "http://crab.build/compass/v1";
/// Numeric Program IR schema version used by caches and history.
pub const PROGRAM_SCHEMA_VERSION: u32 = 3;
