//! Stable public contracts for universal evidence resolution.

use serde::{Deserialize, Serialize};

/// Internal extraction extension carrying the bounded collection-resolution outcome.
///
/// The build pipeline consumes this before publication. The corresponding public
/// contract is the graph diagnostic emitted for degraded resolution.
pub const UNIVERSAL_RESOLUTION_REPORT_EXTENSION: &str = "_compass_universal_resolution_report";

/// Optional owning `imports_from` edge attribute binding a source-load
/// resolution to its exact observed kind, spelling and occurrence.
pub const TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE: &str = "typescript_source_load";

/// Version 1 source-load proof. This records an owning project's resolution,
/// not a guessed mapping derived from a same-spelled symbol import. The target
/// source is retained on the edge's existing inventory/target_file interface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptSourceLoadProof {
    pub version: u32,
    pub dependency: compass_languages::TypeScriptMergeDependency,
}

/// Aggregate fact cardinalities used to select a bounded resolution strategy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UniversalResolutionCounts {
    pub declarations: usize,
    pub bindings: usize,
    pub occurrences: usize,
    pub candidates: usize,
    pub scopes: usize,
}

impl UniversalResolutionCounts {
    #[must_use]
    pub const fn fits(self, limits: UniversalResolutionLimits) -> bool {
        self.declarations <= limits.declarations
            && self.bindings <= limits.bindings
            && self.occurrences <= limits.occurrences
            && self.candidates <= limits.candidates
            && self.scopes <= limits.candidates
    }
}

/// Outcome of project-wide universal resolution.
///
/// `degraded` means some relationship candidates could not safely be resolved
/// under the bounded partition strategy. Declarations that survive the selected
/// inference profile are still projected.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UniversalResolutionReport {
    pub partitioned: bool,
    pub degraded: bool,
    pub partitions: usize,
    pub failed_partitions: usize,
    pub compacted_declarations: usize,
    pub omitted_candidates: usize,
    pub input: UniversalResolutionCounts,
    pub retained: UniversalResolutionCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Outcome of projecting an already constructed universal resolution index.
///
/// Ambiguous or unsupported source semantics remain ordinary unresolved
/// decisions. `omitted_candidates` instead counts explicit resource-limit
/// decisions: those relationships were not projected and the resulting graph
/// must be treated as degraded. A zero count does not certify compiler-grade
/// completeness or the absence of unresolved source semantics.
#[must_use = "inspect omitted_candidates before treating materialized relationships as complete"]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UniversalMaterializationReport {
    pub omitted_candidates: usize,
}

/// Aggregate and per-lookup limits for the universal evidence resolver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalResolutionLimits {
    pub declarations: usize,
    pub bindings: usize,
    pub occurrences: usize,
    pub candidates: usize,
    pub candidates_per_lookup: usize,
}

impl Default for UniversalResolutionLimits {
    fn default() -> Self {
        Self {
            declarations: 1_000_000,
            bindings: 1_000_000,
            occurrences: 5_000_000,
            candidates: 5_000_000,
            candidates_per_lookup: 256,
        }
    }
}

/// The strongest rule that justified one resolution decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionRule {
    ExactSourceDeclaration,
    ExactLexicalDeclaration,
    ExplicitBinding,
    ProjectModuleBinding,
    MemberBinding,
    DeferredReceiver,
    WildcardBinding,
    PhpGlobalFunctionFallback,
    KotlinNamedDefaultArguments,
    UniqueModuleOrPackage,
    ExactHierarchyBase,
    DirectReceiverSuccessorDispatch,
    LinearizedReceiverDispatch,
    ClosedWorldReceiverDispatch,
    IncompleteHierarchyReceiverDispatch,
    RustAssociatedType,
    RustGenericBoundDispatch,
    ExactSourceInventory,
    QualifiedExternal,
}

/// Provenance retained for one successful resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionEvidence {
    pub rule: ResolutionRule,
    pub candidate_count: usize,
}

/// Conservative result of resolving one relationship candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionDecision {
    Resolved {
        declaration_id: String,
        evidence: ResolutionEvidence,
    },
    ResolvedInventory {
        graph_node_id: String,
        evidence: ResolutionEvidence,
    },
    QualifiedExternal {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    DeferredReceiver {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    Ambiguous {
        candidate_count: usize,
    },
    /// A bounded lookup stopped without selecting a partial target set.
    ResourceLimit {
        resource: &'static str,
        limit: usize,
    },
    Unresolved,
}
