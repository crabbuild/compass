//! Stable public contracts for universal evidence resolution.

use serde::{Deserialize, Serialize};

/// Internal extraction extension carrying the bounded collection-resolution outcome.
///
/// The build pipeline consumes this before publication. The corresponding public
/// contract is the graph diagnostic emitted for degraded resolution.
pub const UNIVERSAL_RESOLUTION_REPORT_EXTENSION: &str = "_compass_universal_resolution_report";

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
    UniqueModuleOrPackage,
    ExactHierarchyBase,
    DirectReceiverSuccessorDispatch,
    LinearizedReceiverDispatch,
    ClosedWorldReceiverDispatch,
    IncompleteHierarchyReceiverDispatch,
    RustAssociatedType,
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
    Unresolved,
}
