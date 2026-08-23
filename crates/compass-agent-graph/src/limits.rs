use serde::{Deserialize, Serialize};

use crate::{AgentGraphError, AgentGraphErrorCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentGraphLimits {
    pub max_batch_bytes: usize,
    pub max_operations: usize,
    pub max_citations_per_assertion: usize,
    pub max_evidence_bytes: usize,
    pub max_text_bytes: usize,
    pub max_dependency_depth: usize,
    pub max_candidates: usize,
    pub max_agent_nodes: usize,
    pub max_agent_edges: usize,
    pub max_diagnostics: usize,
    pub max_audit_bytes: usize,
}

impl Default for AgentGraphLimits {
    fn default() -> Self {
        Self {
            max_batch_bytes: 1024 * 1024,
            max_operations: 100,
            max_citations_per_assertion: 16,
            max_evidence_bytes: 1024 * 1024,
            max_text_bytes: 2 * 1024,
            max_dependency_depth: 16,
            max_candidates: 10,
            max_agent_nodes: 10_000,
            max_agent_edges: 100_000,
            max_diagnostics: 20,
            max_audit_bytes: 4 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardLimits;

impl HardLimits {
    pub const BATCH_BYTES: usize = 16 * 1024 * 1024;
    pub const OPERATIONS: usize = 1_000;
    pub const CITATIONS_PER_ASSERTION: usize = 64;
    pub const EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
    pub const TEXT_BYTES: usize = 8 * 1024;
    pub const DEPENDENCY_DEPTH: usize = 32;
    pub const CANDIDATES: usize = 20;
    pub const AGENT_NODES: usize = 100_000;
    pub const AGENT_EDGES: usize = 500_000;
    pub const DIAGNOSTICS: usize = 100;
    pub const AUDIT_BYTES: usize = 16 * 1024;
}

impl AgentGraphLimits {
    pub fn validate(self) -> Result<Self, AgentGraphError> {
        let checks = [
            (
                "maxBatchBytes",
                self.max_batch_bytes,
                HardLimits::BATCH_BYTES,
            ),
            ("maxOperations", self.max_operations, HardLimits::OPERATIONS),
            (
                "maxCitationsPerAssertion",
                self.max_citations_per_assertion,
                HardLimits::CITATIONS_PER_ASSERTION,
            ),
            (
                "maxEvidenceBytes",
                self.max_evidence_bytes,
                HardLimits::EVIDENCE_BYTES,
            ),
            ("maxTextBytes", self.max_text_bytes, HardLimits::TEXT_BYTES),
            (
                "maxDependencyDepth",
                self.max_dependency_depth,
                HardLimits::DEPENDENCY_DEPTH,
            ),
            ("maxCandidates", self.max_candidates, HardLimits::CANDIDATES),
            (
                "maxAgentNodes",
                self.max_agent_nodes,
                HardLimits::AGENT_NODES,
            ),
            (
                "maxAgentEdges",
                self.max_agent_edges,
                HardLimits::AGENT_EDGES,
            ),
            (
                "maxDiagnostics",
                self.max_diagnostics,
                HardLimits::DIAGNOSTICS,
            ),
            (
                "maxAuditBytes",
                self.max_audit_bytes,
                HardLimits::AUDIT_BYTES,
            ),
        ];
        for (field, actual, maximum) in checks {
            if actual == 0 || actual > maximum {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::LimitExceeded,
                    format!("{field} must be between 1 and {maximum}; got {actual}"),
                ));
            }
        }
        Ok(self)
    }
}
