mod common;

use std::collections::BTreeMap;

use compass_agent_graph::{
    AgentEdgeDraft, AgentFactDraft, AgentGraphOverlay, AgentNodeDraft, AssertionDraft,
    AssertionKey, AssertionSelector, BaseGenerationId, BaseGenerationView, ChangeBatch,
    ChangeOperation, Digest, IdempotencyKey, InMemoryBaseGeneration, IngestionPreparationRequest,
    NodeRef, OverlayId, OverlayRepository, ReadRequest, ReadResult, RepositoryId,
    SourceSpanRequest, canonical_bytes,
};
use compass_model::code_graph::{EdgeKind, EdgeRecord, NodeKind};
use compass_store::MemoryStore;

#[test]
fn preparation_produces_apply_ready_base_refs_and_grounding()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let overlay = OverlayId::parse("overlay:review")?;
    let request = IngestionPreparationRequest {
        base_node_ids: vec![common::BASE_NODE_ID.to_owned()],
        base_edge_ids: Vec::new(),
        source_spans: vec![SourceSpanRequest {
            file: common::SOURCE_PATH.to_owned(),
            start_byte: 0,
            end_byte: 29,
        }],
    };
    let ReadResult::IngestionPreparation(prepared) =
        repository.read(ReadRequest::PrepareIngestion {
            overlay: overlay.clone(),
            base_generation: fixture.identity.clone(),
            request,
        })?
    else {
        return Err("expected ingestion preparation".into());
    };
    assert_eq!(
        prepared.schema,
        "compass.agent-graph.ingestion-preparation/1"
    );
    assert_eq!(prepared.expected_revision, None);
    assert_eq!(prepared.base_nodes, vec![fixture.base_node.clone()]);
    assert_eq!(prepared.grounding.evidence.len(), 2);
    let encoded = serde_json::to_string(&prepared)?;
    assert!(!encoded.contains("GROUNDED"));

    let node_key = AssertionKey::parse("key:prepared-caller")?;
    let batch = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: overlay.clone(),
        base_generation: prepared.base_generation.clone(),
        expected_revision: prepared.expected_revision.clone(),
        idempotency_key: IdempotencyKey::parse("idempotency:prepared-ingestion")?,
        operations: vec![
            ChangeOperation::PutAssertion {
                assertion: AssertionDraft {
                    selector: AssertionSelector::New {
                        key: node_key.clone(),
                    },
                    fact: AgentFactDraft::Node(AgentNodeDraft {
                        kind: NodeKind::Function,
                        roles: Vec::new(),
                        name: "prepared_caller".to_owned(),
                        qualified_name: "crate::prepared_caller".to_owned(),
                        language: Some("rust".to_owned()),
                        framework: None,
                        details: None,
                    }),
                    grounding: prepared.grounding.clone(),
                    summary: "Prepared source-backed caller.".to_owned(),
                },
            },
            ChangeOperation::PutAssertion {
                assertion: AssertionDraft {
                    selector: AssertionSelector::New {
                        key: AssertionKey::parse("key:prepared-edge")?,
                    },
                    fact: AgentFactDraft::Edge(AgentEdgeDraft {
                        source: NodeRef::CreatedInThisBatch { key: node_key },
                        target: NodeRef::Base {
                            node: prepared.base_nodes[0].clone(),
                        },
                        kind: EdgeKind::Calls,
                        relationship_site: None,
                        details: None,
                        context: Some("prepared exact endpoint".to_owned()),
                    }),
                    grounding: prepared.grounding,
                    summary: "Prepared caller reaches the exact Base node.".to_owned(),
                },
            },
        ],
    };
    let receipt = repository.apply(&common::grant(&fixture, "principal:owner", None)?, batch)?;
    let ReadResult::EffectiveGraph(effective) = repository.read(ReadRequest::EffectiveGraph {
        overlay: overlay.clone(),
        revision: receipt.revision.clone(),
        profile: compass_agent_graph::CompositionProfile::Augment,
    })?
    else {
        return Err("expected Effective Graph".into());
    };
    assert_eq!(effective.graph.links.len(), 1);
    assert_eq!(effective.graph.links[0].target, common::BASE_NODE_ID);

    let ReadResult::IngestionPreparation(next) =
        repository.read(ReadRequest::PrepareIngestion {
            overlay,
            base_generation: fixture.identity,
            request: IngestionPreparationRequest {
                base_node_ids: Vec::new(),
                base_edge_ids: Vec::new(),
                source_spans: vec![SourceSpanRequest {
                    file: common::SOURCE_PATH.to_owned(),
                    start_byte: 0,
                    end_byte: 29,
                }],
            },
        })?
    else {
        return Err("expected subsequent ingestion preparation".into());
    };
    assert_eq!(next.expected_revision, Some(receipt.revision));
    Ok(())
}

#[test]
fn preparation_rejects_unknown_refs_duplicate_spans_and_out_of_range_source()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let request = IngestionPreparationRequest {
        base_node_ids: vec!["node:missing".to_owned()],
        base_edge_ids: Vec::new(),
        source_spans: vec![SourceSpanRequest {
            file: common::SOURCE_PATH.to_owned(),
            start_byte: 0,
            end_byte: 1,
        }],
    };
    let error = compass_agent_graph::prepare_ingestion(
        &fixture.generation,
        OverlayId::parse("overlay:review")?,
        None,
        &request,
        compass_agent_graph::AgentGraphLimits::default(),
    )
    .err()
    .ok_or("unknown Base node unexpectedly prepared")?;
    assert_eq!(
        error.code,
        compass_agent_graph::AgentGraphErrorCode::MissingEndpoint
    );

    let duplicate = IngestionPreparationRequest {
        base_node_ids: Vec::new(),
        base_edge_ids: Vec::new(),
        source_spans: vec![
            SourceSpanRequest {
                file: common::SOURCE_PATH.to_owned(),
                start_byte: 0,
                end_byte: 1,
            },
            SourceSpanRequest {
                file: common::SOURCE_PATH.to_owned(),
                start_byte: 0,
                end_byte: 1,
            },
        ],
    };
    assert!(
        duplicate
            .validate(compass_agent_graph::AgentGraphLimits::default())
            .is_err()
    );

    let outside = IngestionPreparationRequest {
        base_node_ids: Vec::new(),
        base_edge_ids: Vec::new(),
        source_spans: vec![SourceSpanRequest {
            file: common::SOURCE_PATH.to_owned(),
            start_byte: 0,
            end_byte: 10_000,
        }],
    };
    let error = compass_agent_graph::prepare_ingestion(
        &fixture.generation,
        OverlayId::parse("overlay:review")?,
        None,
        &outside,
        compass_agent_graph::AgentGraphLimits::default(),
    )
    .err()
    .ok_or("out-of-range source span unexpectedly prepared")?;
    assert_eq!(
        error.code,
        compass_agent_graph::AgentGraphErrorCode::InvalidCitation
    );
    Ok(())
}

#[test]
fn preparation_returns_canonical_directed_base_edge_refs() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = common::fixture()?;
    let mut graph = fixture.generation.graph().clone();
    let edge_id = compass_model::identity::edge_id(
        common::BASE_NODE_ID,
        EdgeKind::Calls,
        common::BASE_NODE_ID,
        Some(&fixture.anchor),
        None,
    );
    graph.links.push(EdgeRecord {
        id: edge_id.clone(),
        key: edge_id.clone(),
        source: common::BASE_NODE_ID.to_owned(),
        target: common::BASE_NODE_ID.to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: None,
        relationship_site: Some(fixture.anchor.clone()),
        details: None,
        evidence: graph.nodes[0].evidence.clone(),
        weight: None,
        context: Some("base self call".to_owned()),
        deferred: false,
        diagnostics: Vec::new(),
    });
    let identity = BaseGenerationId {
        generation_id: graph.graph.build.generation_id.clone(),
        graph_digest: Digest::raw_bytes(&canonical_bytes(&graph)?),
    };
    let generation = InMemoryBaseGeneration::new(
        identity,
        graph,
        BTreeMap::from([(
            common::SOURCE_PATH.to_owned(),
            common::SOURCE_BYTES.to_vec(),
        )]),
    )?;
    let prepared = compass_agent_graph::prepare_ingestion(
        &generation,
        OverlayId::parse("overlay:review")?,
        None,
        &IngestionPreparationRequest {
            base_node_ids: Vec::new(),
            base_edge_ids: vec![edge_id],
            source_spans: vec![SourceSpanRequest {
                file: common::SOURCE_PATH.to_owned(),
                start_byte: 0,
                end_byte: 29,
            }],
        },
        compass_agent_graph::AgentGraphLimits::default(),
    )?;
    assert_eq!(prepared.base_edges.len(), 1);
    assert_eq!(prepared.base_edges[0].source, common::BASE_NODE_ID);
    assert_eq!(prepared.base_edges[0].target, common::BASE_NODE_ID);
    assert_eq!(prepared.base_edges[0].kind, EdgeKind::Calls);
    assert_eq!(prepared.grounding.evidence.len(), 2);
    Ok(())
}

#[test]
fn preparation_requires_rebase_when_active_overlay_uses_another_base()
-> Result<(), Box<dyn std::error::Error>> {
    let original = common::fixture_for_generation("generation-original")?;
    let rebuilt = common::fixture_for_generation("generation-rebuilt")?;
    let provider = compass_agent_graph::InMemoryBaseGenerationProvider::default()
        .with_generation(original.generation.clone())
        .with_generation(rebuilt.generation.clone());
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        provider,
        RepositoryId::parse("repository:test")?,
    );
    repository.apply(
        &common::grant(&original, "principal:owner", None)?,
        common::create_batch(&original, "idempotency:before-rebuild")?,
    )?;

    let error = repository
        .read(ReadRequest::PrepareIngestion {
            overlay: OverlayId::parse("overlay:review")?,
            base_generation: rebuilt.identity,
            request: IngestionPreparationRequest {
                base_node_ids: Vec::new(),
                base_edge_ids: Vec::new(),
                source_spans: vec![SourceSpanRequest {
                    file: common::SOURCE_PATH.to_owned(),
                    start_byte: 0,
                    end_byte: 29,
                }],
            },
        })
        .err()
        .ok_or("preparation unexpectedly crossed Base Generations")?;
    assert_eq!(
        error.code,
        compass_agent_graph::AgentGraphErrorCode::RebaseRequired
    );
    Ok(())
}
