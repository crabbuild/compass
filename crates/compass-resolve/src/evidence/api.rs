//! Stable public contracts for universal evidence resolution.

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
