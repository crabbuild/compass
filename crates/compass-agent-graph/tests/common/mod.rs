#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use compass_agent_graph::{
    AgentEdgeDraft, AgentFactDraft, AgentGraphLimits, AgentNodeDraft, AssertionDraft, AssertionKey,
    AssertionSelector, BaseGenerationId, BaseNodeRef, ChangeBatch, ChangeOperation, Digest,
    GroundingEvidence, GroundingSubmission, IdempotencyKey, InMemoryBaseGeneration,
    InMemoryBaseGenerationProvider, NodeRef, OperationPermission, OverlayId, PrincipalId,
    RepositoryId, WriteAuthority, WriteGrant, canonical_bytes, canonical_digest,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, ExtractionStatus, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::file_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

pub const SOURCE_PATH: &str = "src/lib.rs";
pub const SOURCE_BYTES: &[u8] = b"pub fn caller() { target(); }\npub fn target() {}\n";
pub const BASE_NODE_ID: &str = "node:target";

pub struct Fixture {
    pub identity: BaseGenerationId,
    pub generation: InMemoryBaseGeneration,
    pub provider: InMemoryBaseGenerationProvider,
    pub base_node: BaseNodeRef,
    pub anchor: SourceAnchor,
}

pub fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_for_generation("generation-1")
}

pub fn fixture_for_generation(generation_id: &str) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_for_generation_and_source(generation_id, SOURCE_BYTES)
}

pub fn fixture_for_generation_and_source(
    generation_id: &str,
    source_bytes: &[u8],
) -> Result<Fixture, Box<dyn std::error::Error>> {
    let anchor = SourceAnchor {
        file: SOURCE_PATH.to_owned(),
        start_byte: 0,
        end_byte: 29,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 29,
    };
    let provenance = Provenance::direct(
        EvidenceOrigin::Ast,
        "test.extractor",
        EvidenceConfidence::Exact,
        anchor.clone(),
    )?;
    let node = NodeRecord {
        id: BASE_NODE_ID.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: "target".to_owned(),
        qualified_name: "crate::target".to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor.clone()),
        details: None,
        evidence: vec![provenance],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    };
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: generation_id.to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id(SOURCE_PATH),
        path: SOURCE_PATH.to_owned(),
        language: Some("rust".to_owned()),
        content_digest: Digest::raw_bytes(source_bytes).as_str().to_owned(),
        byte_size: source_bytes.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: Vec::new(),
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    graph.nodes.push(node.clone());
    let identity = BaseGenerationId {
        generation_id: graph.graph.build.generation_id.clone(),
        graph_digest: Digest::raw_bytes(&canonical_bytes(&graph)?),
    };
    let base_node = BaseNodeRef {
        base_generation: identity.clone(),
        id: BASE_NODE_ID.to_owned(),
        kind: NodeKind::Function,
        record_digest: canonical_digest("compass.agent-graph.base-node-record/1", &node)?,
    };
    let generation = InMemoryBaseGeneration::new(
        identity.clone(),
        graph,
        BTreeMap::from([(SOURCE_PATH.to_owned(), source_bytes.to_vec())]),
    )?;
    Ok(Fixture {
        identity,
        generation: generation.clone(),
        provider: InMemoryBaseGenerationProvider::default().with_generation(generation),
        base_node,
        anchor,
    })
}

pub fn grounding(anchor: SourceAnchor) -> GroundingSubmission {
    let start = usize::try_from(anchor.start_byte).unwrap_or(0);
    let end = usize::try_from(anchor.end_byte).unwrap_or(0);
    GroundingSubmission {
        schema: "compass.agent-graph.grounding/1".to_owned(),
        policy_id: "compass.agent-graph.topology-source-span".to_owned(),
        evidence: vec![GroundingEvidence::SourceSpan {
            file: SOURCE_PATH.to_owned(),
            anchor,
            file_digest: Digest::raw_bytes(SOURCE_BYTES),
            excerpt_digest: Digest::raw_bytes(&SOURCE_BYTES[start..end]),
        }],
    }
}

pub fn create_batch(
    fixture: &Fixture,
    idempotency: &str,
) -> Result<ChangeBatch, Box<dyn std::error::Error>> {
    let node_key = AssertionKey::parse("key:caller")?;
    Ok(ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: OverlayId::parse("overlay:review")?,
        base_generation: fixture.identity.clone(),
        expected_revision: None,
        idempotency_key: IdempotencyKey::parse(idempotency)?,
        operations: vec![
            ChangeOperation::PutAssertion {
                assertion: AssertionDraft {
                    selector: AssertionSelector::New {
                        key: node_key.clone(),
                    },
                    fact: AgentFactDraft::Node(AgentNodeDraft {
                        kind: NodeKind::Function,
                        roles: Vec::new(),
                        name: "caller".to_owned(),
                        qualified_name: "crate::caller".to_owned(),
                        language: Some("rust".to_owned()),
                        framework: None,
                        details: None,
                    }),
                    grounding: grounding(fixture.anchor.clone()),
                    summary: "Source-backed caller function.".to_owned(),
                },
            },
            ChangeOperation::PutAssertion {
                assertion: AssertionDraft {
                    selector: AssertionSelector::New {
                        key: AssertionKey::parse("key:caller-target-edge")?,
                    },
                    fact: AgentFactDraft::Edge(AgentEdgeDraft {
                        source: NodeRef::CreatedInThisBatch { key: node_key },
                        target: NodeRef::Base {
                            node: fixture.base_node.clone(),
                        },
                        kind: EdgeKind::Calls,
                        relationship_site: Some(fixture.anchor.clone()),
                        details: None,
                        context: Some("verified call".to_owned()),
                    }),
                    grounding: grounding(fixture.anchor.clone()),
                    summary: "Caller invokes the exact target.".to_owned(),
                },
            },
        ],
    })
}

pub fn grant(
    fixture: &Fixture,
    principal: &str,
    expected_revision: Option<compass_agent_graph::OverlayRevisionId>,
) -> Result<WriteGrant, Box<dyn std::error::Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let authority = WriteAuthority::explicitly_enabled(RepositoryId::parse("repository:test")?);
    Ok(authority.mint(
        PrincipalId::parse(principal)?,
        OverlayId::parse("overlay:review")?,
        fixture.identity.clone(),
        expected_revision,
        BTreeSet::from([
            OperationPermission::PutAssertion,
            OperationPermission::RetractAssertion,
            OperationPermission::PutChallenge,
            OperationPermission::RetractChallenge,
            OperationPermission::CommitRebase,
        ]),
        true,
        now.saturating_add(3_600),
        AgentGraphLimits::default(),
    )?)
}
