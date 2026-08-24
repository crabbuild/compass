use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contract::validate_bounded_text;
use crate::grounding::{prepare_base_edge_ref, prepare_base_node_ref, prepare_source_span};
use crate::{
    AgentGraphError, AgentGraphErrorCode, AgentGraphLimits, BaseEdgeRef, BaseFactRef,
    BaseGenerationId, BaseGenerationView, BaseNodeRef, GroundingEvidence, GroundingSubmission,
    OverlayId, OverlayRevisionId,
};

pub const AGENT_GRAPH_INGESTION_PREPARATION_SCHEMA_V1: &str =
    "compass.agent-graph.ingestion-preparation/1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSpanRequest {
    pub file: String,
    pub start_byte: u64,
    pub end_byte: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestionPreparationRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_node_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_edge_ids: Vec<String>,
    pub source_spans: Vec<SourceSpanRequest>,
}

impl IngestionPreparationRequest {
    pub fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        let limits = limits.validate()?;
        if self.source_spans.is_empty() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::GroundingFailed,
                "ingestion preparation requires at least one source span",
            ));
        }
        let base_facts = self
            .base_node_ids
            .len()
            .saturating_add(self.base_edge_ids.len());
        if base_facts > limits.max_candidates {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "ingestion preparation names {base_facts} Base facts; maximum is {}",
                    limits.max_candidates
                ),
            ));
        }
        let citations = base_facts.saturating_add(self.source_spans.len());
        if citations > limits.max_citations_per_assertion {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "ingestion preparation would create {citations} citations; maximum is {}",
                    limits.max_citations_per_assertion
                ),
            ));
        }
        validate_unique_ids("baseNodeIds", &self.base_node_ids)?;
        validate_unique_ids("baseEdgeIds", &self.base_edge_ids)?;
        let mut spans = BTreeSet::new();
        for span in &self.source_spans {
            validate_bounded_text("sourceSpans.file", &span.file, 4_096, false)?;
            if span.start_byte >= span.end_byte {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::InvalidCitation,
                    "source span startByte must be less than endByte",
                ));
            }
            if !spans.insert((&span.file, span.start_byte, span.end_byte)) {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::InvalidCitation,
                    "ingestion preparation contains a duplicate source span",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestionPreparation {
    pub schema: String,
    pub overlay: OverlayId,
    pub base_generation: BaseGenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<OverlayRevisionId>,
    pub base_nodes: Vec<BaseNodeRef>,
    pub base_edges: Vec<BaseEdgeRef>,
    pub grounding: GroundingSubmission,
}

pub fn prepare_ingestion(
    base: &dyn BaseGenerationView,
    overlay: OverlayId,
    expected_revision: Option<OverlayRevisionId>,
    request: &IngestionPreparationRequest,
    limits: AgentGraphLimits,
) -> Result<IngestionPreparation, AgentGraphError> {
    request.validate(limits)?;

    let mut node_ids = request.base_node_ids.clone();
    node_ids.sort();
    let mut edge_ids = request.base_edge_ids.clone();
    edge_ids.sort();
    let mut spans = request.source_spans.clone();
    spans.sort_by(|left, right| {
        (&left.file, left.start_byte, left.end_byte).cmp(&(
            &right.file,
            right.start_byte,
            right.end_byte,
        ))
    });

    let base_nodes = node_ids
        .iter()
        .map(|id| prepare_base_node_ref(base, id))
        .collect::<Result<Vec<_>, _>>()?;
    let base_edges = edge_ids
        .iter()
        .map(|id| prepare_base_edge_ref(base, id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut evidence = spans
        .iter()
        .map(|span| prepare_source_span(base, span))
        .collect::<Result<Vec<_>, _>>()?;
    evidence.extend(base_nodes.iter().cloned().map(|node| {
        let record_digest = node.record_digest.clone();
        GroundingEvidence::BaseFact {
            fact: BaseFactRef::Node(node),
            record_digest,
        }
    }));
    evidence.extend(base_edges.iter().cloned().map(|edge| {
        let record_digest = edge.record_digest.clone();
        GroundingEvidence::BaseFact {
            fact: BaseFactRef::Edge(edge),
            record_digest,
        }
    }));
    let grounding = GroundingSubmission {
        schema: crate::grounding::GROUNDING_SCHEMA_V1.to_owned(),
        policy_id: crate::grounding::DEFAULT_GROUNDING_POLICY_ID.to_owned(),
        evidence,
    };
    grounding.validate(limits)?;
    Ok(IngestionPreparation {
        schema: AGENT_GRAPH_INGESTION_PREPARATION_SCHEMA_V1.to_owned(),
        overlay,
        base_generation: base.identity().clone(),
        expected_revision,
        base_nodes,
        base_edges,
        grounding,
    })
}

fn validate_unique_ids(field: &str, ids: &[String]) -> Result<(), AgentGraphError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        validate_bounded_text(field, id, 4_096, false)?;
        if !seen.insert(id) {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidIdentifier,
                format!("{field} contains duplicate ID {id:?}"),
            ));
        }
    }
    Ok(())
}
