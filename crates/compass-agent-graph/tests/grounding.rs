use std::collections::BTreeMap;

use compass_agent_graph::{
    AgentFactDraft, AgentGraphErrorCode, AgentGraphLimits, AgentNodeDraft, AssertionDraft,
    AssertionKey, AssertionSelector, BaseGenerationId, Digest, GroundingEvidence, GroundingPolicy,
    GroundingStatus, GroundingSubmission, InMemoryBaseGeneration, ground_assertion,
};
use compass_model::code_graph::{
    BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
};
use compass_model::identity::file_id;
use compass_model::provenance::SourceAnchor;

const SOURCE: &[u8] = b"pub fn grounded() {}\n";
const FILE: &str = "src/lib.rs";

fn base() -> Result<InMemoryBaseGeneration, Box<dyn std::error::Error>> {
    let file_digest = Digest::raw_bytes(SOURCE);
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-1".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id(FILE),
        path: FILE.to_owned(),
        language: Some("rust".to_owned()),
        content_digest: file_digest.as_str().to_owned(),
        byte_size: SOURCE.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: Vec::new(),
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    let identity = BaseGenerationId {
        generation_id: "generation-1".to_owned(),
        graph_digest: Digest::raw_bytes(&compass_agent_graph::canonical_bytes(&graph)?),
    };
    let sources = BTreeMap::from([(FILE.to_owned(), SOURCE.to_vec())]);
    Ok(InMemoryBaseGeneration::new(identity, graph, sources)?)
}

fn draft(evidence: Vec<GroundingEvidence>) -> Result<AssertionDraft, Box<dyn std::error::Error>> {
    Ok(AssertionDraft {
        selector: AssertionSelector::New {
            key: AssertionKey::parse("key:grounded-function")?,
        },
        fact: AgentFactDraft::Node(AgentNodeDraft {
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: "grounded".to_owned(),
            qualified_name: "crate::grounded".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            details: None,
        }),
        grounding: GroundingSubmission {
            schema: "compass.agent-graph.grounding/1".to_owned(),
            policy_id: "compass.agent-graph.topology-source-span".to_owned(),
            evidence,
        },
        summary: "This function is part of the public grounded path.".to_owned(),
    })
}

fn source_evidence() -> GroundingEvidence {
    GroundingEvidence::SourceSpan {
        file: FILE.to_owned(),
        anchor: SourceAnchor {
            file: FILE.to_owned(),
            start_byte: 0,
            end_byte: 17,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 17,
        },
        file_digest: Digest::raw_bytes(SOURCE),
        excerpt_digest: Digest::raw_bytes(&SOURCE[0..17]),
    }
}

#[test]
fn successful_verification_is_the_only_path_to_grounded() -> Result<(), Box<dyn std::error::Error>>
{
    let grounded = ground_assertion(
        &draft(vec![source_evidence()])?,
        &base()?,
        GroundingPolicy::default(),
        AgentGraphLimits::default(),
    )?;
    assert_eq!(grounded.certificate.status(), GroundingStatus::Grounded);
    let encoded = serde_json::to_value(&grounded.certificate)?;
    assert_eq!(encoded["status"], "GROUNDED");
    assert!(encoded.get("claimDigest").is_some());
    Ok(())
}

#[test]
fn file_and_excerpt_digests_are_recomputed() -> Result<(), Box<dyn std::error::Error>> {
    let mut evidence = source_evidence();
    if let GroundingEvidence::SourceSpan { excerpt_digest, .. } = &mut evidence {
        *excerpt_digest = Digest::raw_bytes(b"wrong");
    }
    let error = ground_assertion(
        &draft(vec![evidence])?,
        &base()?,
        GroundingPolicy::default(),
        AgentGraphLimits::default(),
    )
    .err()
    .ok_or("stale excerpt digest unexpectedly passed verification")?;
    assert_eq!(error.code, AgentGraphErrorCode::InvalidCitation);
    assert_eq!(error.diagnostics[0].field, "grounding.evidence[0]");
    Ok(())
}

#[test]
fn evidence_order_is_not_part_of_the_certificate_identity() -> Result<(), Box<dyn std::error::Error>>
{
    let source = source_evidence();
    let artifact = compass_agent_graph::ArtifactRecord::new(b"artifact".to_vec())?;
    let artifact_evidence = GroundingEvidence::SnapshotArtifact {
        artifact: "analysis.json".to_owned(),
        artifact_digest: Digest::raw_bytes(artifact.bytes()),
        json_pointer: None,
    };
    let base = base()?.with_artifact("analysis.json".to_owned(), artifact);
    let left = ground_assertion(
        &draft(vec![source.clone(), artifact_evidence.clone()])?,
        &base,
        GroundingPolicy::default(),
        AgentGraphLimits::default(),
    )?;
    let right = ground_assertion(
        &draft(vec![artifact_evidence, source])?,
        &base,
        GroundingPolicy::default(),
        AgentGraphLimits::default(),
    )?;
    assert_eq!(left.certificate_digest, right.certificate_digest);
    assert_eq!(left.assertion_digest, right.assertion_digest);
    Ok(())
}

#[test]
fn topology_without_source_span_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let error = ground_assertion(
        &draft(vec![GroundingEvidence::SnapshotArtifact {
            artifact: "missing".to_owned(),
            artifact_digest: Digest::raw_bytes(b"missing"),
            json_pointer: None,
        }])?,
        &base()?,
        GroundingPolicy::default(),
        AgentGraphLimits::default(),
    )
    .err()
    .ok_or("source-less topology unexpectedly passed verification")?;
    assert_eq!(error.code, AgentGraphErrorCode::InvalidCitation);
    Ok(())
}
