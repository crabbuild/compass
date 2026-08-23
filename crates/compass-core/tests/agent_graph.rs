use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use compass_agent_graph::{
    AgentFactDraft, AgentGraphLimits, AgentNodeDraft, AssertionDraft, AssertionKey,
    AssertionSelector, ChangeBatch, ChangeOperation, CompositionProfile, Digest, GroundingEvidence,
    GroundingSubmission, IdempotencyKey, OperationPermission, OverlayId, PrincipalId, ReadRequest,
    ReadResult, WriteAuthority,
};
use compass_core::{
    AgentGraphContext, HistoricalAgentGraphContext, TaskContextIntent, TaskContextLimits,
    TaskContextRequest, attach_agent_knowledge, build_task_context,
};
use compass_history::{
    BuildProfile, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryStore,
    PublishRequest, Repository,
};
use compass_model::code_graph::{
    BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
};
use compass_model::identity::file_id;
use compass_model::provenance::SourceAnchor;

#[test]
fn non_git_context_requires_and_uses_explicit_confined_state()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let graph_path = project.join("graph.json");
    let graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-current".to_owned(),
        source_commit: None,
    });
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    let state_root = project.join("agent-state");

    let context = AgentGraphContext::open_current(&project, &graph_path, Some(&state_root))?;
    assert_eq!(
        context.base_generation().generation_id,
        "generation-current"
    );
    assert!(context.paths().database().is_file());
    assert!(context.repository_id().as_str().starts_with("repository:"));
    Ok(())
}

#[test]
fn exact_historical_overlay_reopens_with_the_same_effective_identity_without_mutating_history()
-> Result<(), Box<dyn std::error::Error>> {
    const SOURCE_PATH: &str = "src/lib.rs";
    const SOURCE: &[u8] = b"pub fn grounded() {}\n";

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    git(&root, &["init", "--quiet"])?;
    git(&root, &["config", "user.name", "Compass Test"])?;
    git(&root, &["config", "user.email", "compass@example.invalid"])?;
    std::fs::create_dir(root.join("src"))?;
    std::fs::write(root.join(SOURCE_PATH), SOURCE)?;
    git(&root, &["add", SOURCE_PATH])?;
    git(&root, &["commit", "--quiet", "-m", "historical source"])?;

    let git_repository = Repository::discover(&root)?;
    let commit = git_repository.resolve("HEAD")?;
    let history = HistoryStore::create(&git_repository)?;
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-historical".to_owned(),
        source_commit: Some(commit.to_string()),
    });
    graph.graph.files.push(FileRecord {
        id: file_id(SOURCE_PATH),
        path: SOURCE_PATH.to_owned(),
        language: Some("rust".to_owned()),
        content_digest: Digest::raw_bytes(SOURCE).as_str().to_owned(),
        byte_size: SOURCE.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["history-test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    let mut profile = BuildProfile::default();
    profile.insert("graph_schema", compass_history::HISTORY_GRAPH_SCHEMA)?;
    let published = history.publish(PublishRequest {
        commit,
        parents: Vec::new(),
        profile,
        fingerprint: "a".repeat(64).parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts::from_trusted(graph.clone(), None, None, None)?,
        completion: CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 0,
            semantic_files_completed: 0,
            failed_chunks: 0,
        },
        make_preferred: true,
    })?;
    let preferred_before = history
        .preferred(&published.version.git_commit.parse()?)?
        .ok_or("missing preferred realization")?;

    let context = HistoricalAgentGraphContext::open_exact(&root, &history, &published.id, None)?;
    let overlay = OverlayId::parse("overlay:historical")?;
    let anchor = SourceAnchor {
        file: SOURCE_PATH.to_owned(),
        start_byte: 0,
        end_byte: SOURCE.len() as u64,
        start_line: 1,
        start_column: 0,
        end_line: 2,
        end_column: 0,
    };
    let batch = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: overlay.clone(),
        base_generation: context.base_generation().clone(),
        expected_revision: None,
        idempotency_key: IdempotencyKey::parse("idempotency:historical-create")?,
        operations: vec![ChangeOperation::PutAssertion {
            assertion: AssertionDraft {
                selector: AssertionSelector::New {
                    key: AssertionKey::parse("key:historical-grounded")?,
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
                    evidence: vec![GroundingEvidence::SourceSpan {
                        file: SOURCE_PATH.to_owned(),
                        anchor,
                        file_digest: Digest::raw_bytes(SOURCE),
                        excerpt_digest: Digest::raw_bytes(SOURCE),
                    }],
                },
                summary: "Exact historical source defines grounded.".to_owned(),
            },
        }],
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let grant = WriteAuthority::explicitly_enabled(context.repository_id().clone()).mint(
        PrincipalId::parse("principal:history-test")?,
        overlay.clone(),
        context.base_generation().clone(),
        None,
        BTreeSet::from([OperationPermission::PutAssertion]),
        false,
        now.saturating_add(300),
        AgentGraphLimits::default(),
    )?;
    let receipt = context.apply(&grant, batch)?;
    let ReadResult::EffectiveGraph(first) = context.read(ReadRequest::EffectiveGraph {
        overlay: overlay.clone(),
        revision: receipt.revision.clone(),
        profile: CompositionProfile::Augment,
    })?
    else {
        return Err("expected exact historical Effective Graph".into());
    };
    let ReadResult::Overlay { state, .. } = context.read(ReadRequest::Overlay {
        overlay: overlay.clone(),
        revision: Some(receipt.revision.clone()),
    })?
    else {
        return Err("expected exact historical Overlay Revision".into());
    };
    let engine = compass_query::open_with_verified_document(
        first.graph.clone(),
        first.effective_identity.as_str().to_owned(),
        &root.join("historical-effective.json"),
        None,
        &root.join("query-cache"),
    )?;
    let request = TaskContextRequest {
        intent: TaskContextIntent::Explain,
        target: "crate::grounded".to_owned(),
        repository_root: root.to_string_lossy().into_owned(),
        limits: TaskContextLimits::default(),
    };
    let mut task_context = build_task_context(&engine, &request, &[])?;
    attach_agent_knowledge(
        &mut task_context,
        &first,
        &receipt.revision,
        &state,
        20,
        request.limits.max_response_bytes,
    )?;
    let agent_knowledge = task_context
        .agent_knowledge
        .as_ref()
        .ok_or("missing Agent knowledge")?;
    assert_eq!(agent_knowledge.assertions.len(), 1);
    assert_eq!(agent_knowledge.assertions[0].grounding_status, "GROUNDED");
    assert_eq!(
        agent_knowledge.assertions[0].structural_confidence,
        "inferred"
    );
    assert_eq!(agent_knowledge.effective_identity, first.effective_identity);
    drop(context);

    let reopened = HistoricalAgentGraphContext::open_exact(&root, &history, &published.id, None)?;
    let ReadResult::EffectiveGraph(second) = reopened.read(ReadRequest::EffectiveGraph {
        overlay,
        revision: receipt.revision,
        profile: CompositionProfile::Augment,
    })?
    else {
        return Err("expected reopened historical Effective Graph".into());
    };
    assert_eq!(first.effective_identity, second.effective_identity);
    assert_eq!(first.graph, second.graph);
    assert_eq!(history.reader(&published.id)?.graph_document()?, graph);
    assert_eq!(
        history
            .preferred(&published.version.git_commit.parse()?)?
            .ok_or("missing preferred realization after overlay write")?,
        preferred_before
    );
    Ok(())
}

fn git(directory: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}
