use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{
    BuildOptions, BuildPurpose, CoreError, ExportInputs, LoadedGraph, SemanticLayer,
    build_graph_with_layers,
};
use compass_files::BuildGuard;
use compass_languages::{Engine, Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::identity::edge_id;
use compass_model::provenance::SEMANTIC_LAYER_EXTRACTOR;
use serde_json::{Map, json};
use sha2::{Digest, Sha256};

fn semantic_edge_extraction(
    path: &Path,
    source: &[u8],
    relation: &str,
) -> Result<Extraction, Box<dyn Error>> {
    semantic_edge_extraction_with_nodes(path, source, relation, false)
}

fn semantic_owned_edge_extraction(
    path: &Path,
    source: &[u8],
    relation: &str,
) -> Result<Extraction, Box<dyn Error>> {
    semantic_edge_extraction_with_nodes(path, source, relation, true)
}

fn semantic_edge_extraction_with_nodes(
    path: &Path,
    source: &[u8],
    relation: &str,
    retain_endpoint_nodes: bool,
) -> Result<Extraction, Box<dyn Error>> {
    let mut engine = Engine::default();
    let extracted = engine.extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let mut extraction = compass_resolve::resolve(&[extracted], &sources);
    if !retain_endpoint_nodes {
        extraction.nodes.clear();
    }
    extraction.edges.retain(|edge| {
        edge.attributes
            .get("relation")
            .and_then(serde_json::Value::as_str)
            == Some(relation)
    });
    for edge in &mut extraction.edges {
        edge.attributes
            .insert("_origin".to_owned(), json!("semantic"));
        edge.attributes
            .insert("confidence".to_owned(), json!("INFERRED"));
        edge.attributes
            .insert("extractor".to_owned(), json!("test.semantic"));
    }
    extraction.raw_calls = None;
    extraction.hyperedges.clear();
    Ok(extraction)
}

fn semantic_method_alias_extraction(
    path: &Path,
    source: &[u8],
) -> Result<Extraction, Box<dyn Error>> {
    let mut engine = Engine::default();
    let extracted = engine.extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let mut extraction = compass_resolve::resolve(&[extracted], &sources);
    let method_ids = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "method")
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    extraction.edges.retain(|edge| {
        edge.attributes
            .get("relation")
            .and_then(serde_json::Value::as_str)
            == Some("contains")
            && method_ids.contains(&edge.target)
    });
    extraction.nodes.clear();
    for edge in &mut extraction.edges {
        edge.attributes
            .insert("relation".to_owned(), json!("method"));
        edge.attributes
            .insert("_origin".to_owned(), json!("semantic"));
        edge.attributes
            .insert("confidence".to_owned(), json!("INFERRED"));
        edge.attributes
            .insert("extractor".to_owned(), json!("test.semantic"));
    }
    extraction.raw_calls = None;
    extraction.hyperedges.clear();
    Ok(extraction)
}

fn semantic_layer(extraction: Extraction) -> Result<SemanticLayer, Box<dyn Error>> {
    Ok(SemanticLayer {
        fragment: serde_json::to_value(extraction)?,
        refreshed_files: Vec::new(),
        partial_files: Vec::new(),
        allow_partial: false,
    })
}

fn semantic_links_without_transport_rewrites(
    graph: &compass_model::code_graph::GraphDocument,
) -> Vec<compass_model::code_graph::EdgeRecord> {
    graph
        .links
        .iter()
        .cloned()
        .map(|mut edge| {
            edge.evidence
                .retain(|evidence| evidence.rule.as_deref() != Some("graph-ghost-endpoint-remap"));
            edge
        })
        .collect()
}

#[test]
fn export_inputs_fall_back_to_node_communities_and_tolerate_partial_sidecars()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    fs::create_dir_all(&output)?;
    let graph = output.join("graph.json");
    fs::write(
        &graph,
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A","community":0},{"id":"b","label":"B","community":"1"},{"id":"c","label":"C","community":"bad"}],"links":[]}"#,
    )?;
    fs::write(
        output.join("analysis.json"),
        r#"{"communities":{"bad":"not-an-array"},"cohesion":{"0":0.75,"bad":1,"1":"wrong"},"gods":"wrong"}"#,
    )?;
    fs::write(
        output.join("labels.json"),
        r#"{"0":"Core","1":7,"bad":"ignored"}"#,
    )?;
    fs::write(output.join("GRAPH_REPORT.md"), "# Fixture\n")?;

    let inputs = ExportInputs::load(&graph)?;
    assert_eq!(inputs.communities.get(&0), Some(&vec!["a".to_owned()]));
    assert_eq!(inputs.communities.get(&1), Some(&vec!["b".to_owned()]));
    assert_eq!(inputs.cohesion.get(&0), Some(&0.75));
    assert_eq!(inputs.labels.get(&0).map(String::as_str), Some("Core"));
    assert!(inputs.gods.is_empty());
    assert_eq!(inputs.report, "# Fixture\n");
    Ok(())
}

#[test]
fn loaded_graph_learning_overlay_marks_current_missing_and_unfingerprinted_sources()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    fs::create_dir_all(&output)?;
    let source = directory.path().join("source.rs");
    let contents = b"pub fn current() {}\n";
    fs::write(&source, contents)?;
    let fingerprint = format!("{:x}", Sha256::digest(contents));
    let graph = output.join("graph.json");
    fs::write(
        &graph,
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A"}],"links":[]}"#,
    )?;
    fs::write(
        output.join("source-root.txt"),
        directory.path().to_string_lossy().as_bytes(),
    )?;
    fs::write(
        output.join("learning.json"),
        serde_json::to_vec(&serde_json::json!({
            "nodes": {
                "current": {"source_file":"source.rs","code_fingerprint":fingerprint},
                "empty": {"source_file":"","code_fingerprint":""},
                "missing": {"source_file":"missing.rs","code_fingerprint":"abc"},
                "unfingerprinted": {"source_file":"source.rs","code_fingerprint":""},
                "wrong": {"source_file":"source.rs","code_fingerprint":"wrong"},
                "ignored": "not-an-object"
            }
        }))?,
    )?;

    let loaded = LoadedGraph::load(&graph)?;
    assert_eq!(loaded.graph.node_count(), 1);
    assert_eq!(loaded.overlay["current"]["stale"], false);
    assert_eq!(loaded.overlay["empty"]["stale"], false);
    assert_eq!(loaded.overlay["missing"]["stale"], true);
    assert_eq!(loaded.overlay["unfingerprinted"]["stale"], true);
    assert_eq!(loaded.overlay["wrong"]["stale"], true);
    assert!(!loaded.overlay.contains_key("ignored"));

    let directed = LoadedGraph::load_directed(&graph)?;
    assert_eq!(directed.graph.node_count(), 1);
    fs::write(output.join("learning.json"), "not json")?;
    assert!(LoadedGraph::load(&graph)?.overlay.is_empty());
    fs::remove_file(output.join("learning.json"))?;
    assert!(LoadedGraph::load(&graph)?.overlay.is_empty());
    Ok(())
}

#[test]
fn build_pipeline_reports_missing_and_empty_roots_and_accepts_file_only_sources()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut missing = BuildOptions::new(directory.path().join("missing"));
    missing.no_cluster = true;
    missing.no_viz = true;
    assert!(matches!(
        build_graph_with_layers(&missing, None, &[]),
        Err(CoreError::MissingRoot(_))
    ));

    let empty = directory.path().join("empty");
    fs::create_dir(&empty)?;
    let mut options = BuildOptions::new(empty.clone());
    options.no_cluster = false;
    options.no_viz = true;
    assert!(matches!(
        build_graph_with_layers(&options, None, &[]),
        Err(CoreError::EmptyGraph)
    ));

    fs::write(empty.join("comments.rs"), "// no declarations\n")?;
    let file_only = build_graph_with_layers(&options, None, &[])?;
    assert_eq!(file_only.nodes, 1);
    Ok(())
}

#[test]
fn semantic_build_normalizes_typed_origins_paths_and_incremental_updates()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("main.rs"), "pub fn main_entry() {}\n")?;
    fs::write(directory.path().join("notes.md"), "# Café 🚀\n")?;
    let mut options = BuildOptions::new(directory.path().to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let semantic = SemanticLayer {
        fragment: serde_json::json!({
            "directed":true,
            "multigraph":false,
            "hyperedges":[],
            "nodes":[
                {"id":"doc","label":"Café 🚀","file_type":"document","source_file":directory.path().join("notes.md")},
                {"id":"external","label":"External","file_type":"concept","source_file":directory.path().join("notes.md")}
            ],
            "edges":[
                {"source":"doc","target":"external","relation":"references","source_file":directory.path().join("notes.md")}
            ]
        }),
        refreshed_files: vec![directory.path().join("notes.md")],
        partial_files: Vec::new(),
        allow_partial: false,
    };
    let result = build_graph_with_layers(&options, Some(&semantic), &[])?;
    assert!(result.nodes >= 3);
    let document =
        compass_model::code_graph::GraphDocument::load(&result.output_dir.join("graph.json"))?;
    let semantic_edge = document
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
        .ok_or("missing semantic edge")?;
    assert_eq!(semantic_edge.source_file(), Some("notes.md"));
    assert_eq!(
        semantic_edge.evidence[0].origin,
        compass_model::provenance::EvidenceOrigin::Heuristic
    );

    let warm = build_graph_with_layers(&options, None, &[])?;
    assert!(!warm.outputs_changed);
    let preserved =
        compass_model::code_graph::GraphDocument::load(&warm.output_dir.join("graph.json"))?;
    assert!(preserved.nodes.iter().any(|node| node.label() == "Café 🚀"));

    fs::remove_file(directory.path().join("notes.md"))?;
    let pruned = build_graph_with_layers(&options, None, &[])?;
    assert!(pruned.outputs_changed);
    let pruned_document =
        compass_model::code_graph::GraphDocument::load(&pruned.output_dir.join("graph.json"))?;
    assert!(
        pruned_document
            .nodes
            .iter()
            .all(|node| node.source_file() != Some("notes.md"))
    );
    Ok(())
}

#[test]
fn incremental_update_preserves_then_replaces_owned_semantic_facts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    let notes = directory.path().join("notes.md");
    fs::write(&source, "pub fn first() {}\n")?;
    fs::write(&notes, "# Notes\n")?;
    let mut options = BuildOptions::new(directory.path().to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;

    let first = SemanticLayer {
        fragment: serde_json::json!({
            "nodes":[
                {"id":"concept-a","label":"A","file_type":"concept","source_file":notes},
                {"id":"concept-b","label":"B","file_type":"concept","source_file":notes}
            ],
            "edges":[{"source":"concept-a","target":"concept-b","relation":"references","source_file":notes}],
            "hyperedges":[]
        }),
        refreshed_files: vec![notes.clone()],
        partial_files: Vec::new(),
        allow_partial: false,
    };
    build_graph_with_layers(&options, Some(&first), &[])?;

    fs::write(&source, "pub fn second() {}\n")?;
    build_graph_with_layers(&options, None, &[])?;
    let preserved_path =
        BuildGuard::resolve_artifact(&directory.path().join("compass-out"), "graph.json")?;
    let preserved = compass_model::code_graph::GraphDocument::load(&preserved_path)?;
    assert!(
        preserved.nodes.iter().any(|node| node.label() == "A")
            && preserved.nodes.iter().any(|node| node.label() == "B")
            && preserved
                .links
                .iter()
                .filter(|edge| { edge.kind == compass_model::code_graph::EdgeKind::References })
                .count()
                == 1
    );

    let replacement = SemanticLayer {
        fragment: serde_json::json!({
            "nodes":[
                {"id":"concept-a","label":"A2","file_type":"concept","source_file":notes},
                {"id":"concept-b","label":"B2","file_type":"concept","source_file":notes}
            ],
            "edges":[],
            "hyperedges":[]
        }),
        refreshed_files: vec![notes],
        partial_files: Vec::new(),
        allow_partial: false,
    };
    build_graph_with_layers(&options, Some(&replacement), &[])?;
    let replaced_path =
        BuildGuard::resolve_artifact(&directory.path().join("compass-out"), "graph.json")?;
    let replaced = compass_model::code_graph::GraphDocument::load(&replaced_path)?;
    assert!(replaced.nodes.iter().any(|node| node.label() == "A2"));
    assert!(replaced.nodes.iter().any(|node| node.label() == "B2"));
    assert!(
        replaced
            .links
            .iter()
            .all(|edge| edge.kind != compass_model::code_graph::EdgeKind::References)
    );
    Ok(())
}

#[test]
fn incremental_mixed_origin_nodes_use_fresh_ast_typed_data() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    fs::write(&source, "pub fn target() {}\n")?;

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial = build_graph_with_layers(&options, None, &[])?;
    let mut initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    let initial_target = initial_graph
        .nodes
        .iter_mut()
        .find(|node| node.name == "target" || node.label() == "target()")
        .ok_or("missing initial target")?;
    let mut semantic_evidence = initial_target
        .evidence
        .first()
        .cloned()
        .ok_or("missing initial AST evidence")?;
    semantic_evidence.origin = compass_model::provenance::EvidenceOrigin::Heuristic;
    semantic_evidence.extractor = SEMANTIC_LAYER_EXTRACTOR.to_owned();
    semantic_evidence.confidence = compass_model::provenance::EvidenceConfidence::Inferred;
    semantic_evidence.rule = Some("semantic-extraction".to_owned());
    semantic_evidence.wiring_site = initial_target.source.clone();
    semantic_evidence.anchors.clear();
    initial_target.evidence.push(semantic_evidence);
    fs::write(
        initial.output_dir.join("graph.json"),
        serde_json::to_vec_pretty(&initial_graph)?,
    )?;
    fs::write(initial.output_dir.join("semantic-marker.json"), b"{}")?;

    fs::write(
        &source,
        "// moved\npub fn target(value: u32) { let _ = value; }\n",
    )?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_target = incremental_graph
        .nodes
        .iter()
        .find(|node| node.name == "target" || node.label() == "target()")
        .ok_or_else(|| format!("missing incremental target: {:?}", incremental_graph.nodes))?;
    let incremental_signature =
        incremental_target
            .details
            .as_ref()
            .and_then(|details| match details {
                compass_model::code_graph::NodeDetails::Symbol(symbol) => {
                    symbol.signature.as_deref()
                }
                _ => None,
            });
    assert!(
        incremental_signature.is_some_and(|signature| signature.contains("u32")),
        "target={incremental_target:?}"
    );

    options.force = true;
    let clean = build_graph_with_layers(&options, None, &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    let clean_target = clean_graph
        .nodes
        .iter()
        .find(|node| node.name == "target" || node.label() == "target()")
        .ok_or("missing clean target")?;
    assert_eq!(incremental_target.details, clean_target.details);
    assert_eq!(incremental_target.source, clean_target.source);
    assert!(
        incremental_target
            .source
            .as_ref()
            .is_some_and(|anchor| anchor.start_byte > 0),
        "target={incremental_target:?}"
    );
    assert!(
        incremental_target.evidence.iter().any(|evidence| {
            evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
                && evidence.wiring_site.as_ref() == incremental_target.source.as_ref()
        }),
        "semantic evidence was not preserved at the fresh source site: {incremental_target:?}"
    );
    Ok(())
}

#[test]
fn incremental_mixed_origin_edges_use_fresh_ast_relationship_sites() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    fs::write(
        &source,
        "pub fn caller() { target(); }\npub fn target() {}\n",
    )?;

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial = build_graph_with_layers(&options, None, &[])?;
    let mut initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    let initial_call = initial_graph
        .links
        .iter_mut()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .ok_or("missing initial call")?;
    let mut semantic_evidence = initial_call
        .evidence
        .first()
        .cloned()
        .ok_or("missing initial AST call evidence")?;
    semantic_evidence.origin = compass_model::provenance::EvidenceOrigin::Heuristic;
    semantic_evidence.extractor = SEMANTIC_LAYER_EXTRACTOR.to_owned();
    semantic_evidence.confidence = compass_model::provenance::EvidenceConfidence::Inferred;
    semantic_evidence.rule = Some("semantic-call".to_owned());
    semantic_evidence.wiring_site = initial_call.relationship_site.clone();
    semantic_evidence.anchors.clear();
    initial_call.evidence.push(semantic_evidence);
    fs::write(
        initial.output_dir.join("graph.json"),
        serde_json::to_vec_pretty(&initial_graph)?,
    )?;
    fs::write(initial.output_dir.join("semantic-marker.json"), b"{}")?;

    fs::write(
        &source,
        "// moved relationship site\npub fn caller() { target(); }\npub fn target() {}\n",
    )?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_calls = incremental_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(
        incremental_calls.len(),
        1,
        "stale mixed-origin call survived beside fresh AST call: {incremental_calls:?}"
    );
    assert!(incremental_calls[0].evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
            && evidence.rule.as_deref() == Some("semantic-call")
    }));

    let unchanged = build_graph_with_layers(&options, None, &[])?;
    let unchanged_graph =
        compass_model::code_graph::GraphDocument::load(&unchanged.output_dir.join("graph.json"))?;
    assert_eq!(incremental_graph.links, unchanged_graph.links);
    Ok(())
}

#[test]
fn incremental_deleted_mixed_occurrence_keeps_only_revalidated_semantic_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let initial_source = b"pub fn caller() { target(); }\npub fn target() {}\n";
    fs::write(&source, initial_source)?;
    let initial_semantic =
        semantic_owned_edge_extraction(Path::new("main.rs"), initial_source, "calls")?;

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial_layer = semantic_layer(initial_semantic)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    let mut final_semantic_call = initial_graph
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .cloned()
        .ok_or("missing initial mixed call")?;
    assert!(
        final_semantic_call
            .evidence
            .iter()
            .any(|evidence| { evidence.origin == compass_model::provenance::EvidenceOrigin::Ast })
    );
    assert!(final_semantic_call.evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));
    final_semantic_call.evidence.retain(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    });
    final_semantic_call.relationship_site = None;

    let final_source = b"pub fn caller() { /*none!*/ }\npub fn target() {}\n";
    fs::write(&source, final_source)?;
    final_semantic_call.id = edge_id(
        &final_semantic_call.source,
        final_semantic_call.kind,
        &final_semantic_call.target,
        None,
        final_semantic_call
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    final_semantic_call.key.clone_from(&final_semantic_call.id);
    let mut final_semantic_document = initial_graph.clone();
    final_semantic_document.links = vec![final_semantic_call];
    let final_semantic = compass_graph::extraction_from_v1(&final_semantic_document);

    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_calls = incremental_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(
        incremental_calls.len(),
        1,
        "the final semantic fragment independently revalidates one semantic-only call, so the deleted mixed occurrence must be converted instead of surviving with stale AST, transient-remap, or exact-site evidence: {incremental_calls:?}"
    );
    assert_eq!(incremental_calls[0].relationship_site, None);
    assert!(
        incremental_calls[0]
            .evidence
            .iter()
            .all(
                |evidence| evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
                    && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
            ),
        "semantic-only relationship retained stale occurrence evidence: {incremental_calls:?}"
    );
    assert!(incremental_calls[0].evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));
    let mut clean_options = BuildOptions::new(root.to_path_buf());
    clean_options.output_root = Some(root.join("clean-deletion-out"));
    clean_options.no_cluster = true;
    clean_options.no_viz = true;
    clean_options.force = true;
    clean_options.purpose = BuildPurpose::Extract;
    let final_layer = semantic_layer(final_semantic)?;
    let clean = build_graph_with_layers(&clean_options, Some(&final_layer), &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    assert_eq!(
        semantic_links_without_transport_rewrites(&incremental_graph),
        semantic_links_without_transport_rewrites(&clean_graph)
    );
    Ok(())
}

#[test]
fn incremental_deleted_remapped_mixed_occurrence_rebinds_trusted_semantic_residue()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let initial_source = b"pub fn caller() { target(); }\npub fn target() {}\n";
    fs::write(&source, initial_source)?;
    let initial_semantic =
        semantic_owned_edge_extraction(Path::new("main.rs"), initial_source, "calls")?;

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial_layer = semantic_layer(initial_semantic)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let graph_path = initial.output_dir.join("graph.json");
    let initial_graph = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    let canonical_target = initial_graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == compass_model::code_graph::NodeKind::Function
                && (node.name == "target" || node.label() == "target()")
        })
        .map(|node| node.id.clone())
        .ok_or("missing target function")?;

    let mut final_semantic_call = initial_graph
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .cloned()
        .ok_or("missing initial mixed call")?;
    final_semantic_call.evidence.retain(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    });
    final_semantic_call.relationship_site = None;
    final_semantic_call.id = edge_id(
        &final_semantic_call.source,
        final_semantic_call.kind,
        &final_semantic_call.target,
        None,
        final_semantic_call
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    final_semantic_call.key.clone_from(&final_semantic_call.id);
    let mut final_semantic_document = initial_graph.clone();
    final_semantic_document.links = vec![final_semantic_call];
    let final_semantic = compass_graph::extraction_from_v1(&final_semantic_document);

    let stale_target = format!("stale:{canonical_target}");
    let mut stale_graph = initial_graph;
    stale_graph
        .nodes
        .iter_mut()
        .find(|node| node.id == canonical_target)
        .ok_or("missing target node")?
        .id
        .clone_from(&stale_target);
    for edge in &mut stale_graph.links {
        if edge.source == canonical_target {
            edge.source.clone_from(&stale_target);
        }
        if edge.target == canonical_target {
            edge.target.clone_from(&stale_target);
        }
        edge.id = edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key.clone_from(&edge.id);
    }
    fs::write(&graph_path, serde_json::to_vec_pretty(&stale_graph)?)?;
    compass_model::code_graph::GraphDocument::load(&graph_path)?;

    let final_source = b"pub fn caller() { /*none!*/ }\npub fn target() {}\n";
    fs::write(&source, final_source)?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_calls = incremental_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(incremental_calls.len(), 1);
    let call = incremental_calls[0];
    assert_eq!(call.target, canonical_target);
    assert_eq!(call.relationship_site, None);
    assert!(call.evidence.iter().all(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    }));
    let projected = compass_graph::extraction_from_v1(&incremental_graph);
    let projected_call = projected
        .edges
        .iter()
        .find(|edge| {
            edge.attributes
                .get("relation")
                .and_then(serde_json::Value::as_str)
                == Some("calls")
        })
        .ok_or("missing projected call")?;
    let trusted: compass_model::code_graph::EdgeRecord = serde_json::from_value(
        projected_call
            .attributes
            .get(compass_model::provenance::TRUSTED_EDGE_RECORD_ATTRIBUTE)
            .cloned()
            .ok_or("missing trusted edge record")?,
    )?;
    assert_eq!(trusted.source, projected_call.source);
    assert_eq!(trusted.target, projected_call.target);
    assert_eq!(trusted.target, canonical_target);

    let mut clean_options = BuildOptions::new(root.to_path_buf());
    clean_options.output_root = Some(root.join("clean-deletion-remap-out"));
    clean_options.no_cluster = true;
    clean_options.no_viz = true;
    clean_options.force = true;
    clean_options.purpose = BuildPurpose::Extract;
    let final_layer = semantic_layer(final_semantic)?;
    let clean = build_graph_with_layers(&clean_options, Some(&final_layer), &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    let incremental_semantics = semantic_links_without_transport_rewrites(&incremental_graph);
    let clean_semantics = semantic_links_without_transport_rewrites(&clean_graph);
    assert_eq!(incremental_semantics, clean_semantics);
    assert_eq!(
        serde_json::to_vec(&incremental_semantics)?,
        serde_json::to_vec(&clean_semantics)?
    );
    Ok(())
}

#[test]
fn incremental_mixed_occurrence_cardinality_matches_exact_sites_and_preserves_residue()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let initial_source =
        b"pub fn caller() {\n    target();\n    target();\n    target();\n}\npub fn target() {}\n";
    fs::write(&source, initial_source)?;
    let initial_semantic =
        semantic_owned_edge_extraction(Path::new("main.rs"), initial_source, "calls")?;
    assert_eq!(initial_semantic.edges.len(), 3);

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial_layer = semantic_layer(initial_semantic)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    let mut initial_calls = initial_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .cloned()
        .collect::<Vec<_>>();
    initial_calls.sort_by_key(|edge| {
        edge.relationship_site
            .as_ref()
            .map(|site| (site.file.clone(), site.start_byte, site.end_byte))
    });
    assert_eq!(initial_calls.len(), 3);
    assert!(initial_calls.iter().all(|edge| {
        edge.evidence
            .iter()
            .any(|evidence| evidence.origin == compass_model::provenance::EvidenceOrigin::Ast)
            && edge.evidence.iter().any(|evidence| {
                evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
            })
    }));
    let exact_surviving_site = initial_calls[0]
        .relationship_site
        .clone()
        .ok_or("missing exact surviving site")?;
    let deleted_sites = initial_calls[1..]
        .iter()
        .filter_map(|edge| edge.relationship_site.clone())
        .collect::<Vec<_>>();

    let mut exact_semantic = initial_calls[0].clone();
    exact_semantic.evidence.retain(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    });
    exact_semantic.id = edge_id(
        &exact_semantic.source,
        exact_semantic.kind,
        &exact_semantic.target,
        exact_semantic.relationship_site.as_ref(),
        exact_semantic
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    exact_semantic.key.clone_from(&exact_semantic.id);
    let mut semantic_only_residue = initial_calls[1].clone();
    semantic_only_residue.evidence.retain(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    });
    semantic_only_residue.relationship_site = None;
    semantic_only_residue.id = edge_id(
        &semantic_only_residue.source,
        semantic_only_residue.kind,
        &semantic_only_residue.target,
        None,
        semantic_only_residue
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    semantic_only_residue
        .key
        .clone_from(&semantic_only_residue.id);
    let mut second_semantic_only_residue = initial_calls[2].clone();
    second_semantic_only_residue.evidence.retain(|evidence| {
        evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
            && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
    });
    second_semantic_only_residue.relationship_site = None;
    second_semantic_only_residue.id = edge_id(
        &second_semantic_only_residue.source,
        second_semantic_only_residue.kind,
        &second_semantic_only_residue.target,
        None,
        second_semantic_only_residue
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    second_semantic_only_residue
        .key
        .clone_from(&second_semantic_only_residue.id);
    let mut final_semantic_document = initial_graph.clone();
    final_semantic_document.links = vec![
        exact_semantic,
        semantic_only_residue,
        second_semantic_only_residue,
    ];
    let final_semantic = compass_graph::extraction_from_v1(&final_semantic_document);

    let final_source =
        b"pub fn caller() {\n    target();\n  target();  \n    /*none*/ \n}\npub fn target() {}\n";
    assert_eq!(initial_source.len(), final_source.len());
    fs::write(&source, final_source)?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_calls = incremental_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(
        incremental_calls.len(),
        3,
        "expected one matched mixed occurrence, one unmatched AST occurrence, and one independently revalidated semantic-only residue: {incremental_calls:?}"
    );
    let matched = incremental_calls
        .iter()
        .find(|edge| edge.relationship_site.as_ref() == Some(&exact_surviving_site))
        .ok_or("missing exact-site matched occurrence")?;
    assert!(
        matched
            .evidence
            .iter()
            .any(|evidence| { evidence.origin == compass_model::provenance::EvidenceOrigin::Ast })
    );
    assert!(matched.evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
            && evidence.rule.as_deref() == Some("universal-call-exact-lexical-declaration")
    }));
    let semantic_only = incremental_calls
        .iter()
        .find(|edge| edge.relationship_site.is_none())
        .ok_or("missing semantic-only unmatched prior residue")?;
    assert!(semantic_only.evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));
    assert!(
        semantic_only.evidence.iter().all(|evidence| {
            evidence.origin != compass_model::provenance::EvidenceOrigin::Ast
                && evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")
        }),
        "semantic-only residue retained stale occurrence evidence: {semantic_only:?}"
    );
    let unmatched_current = incremental_calls
        .iter()
        .find(|edge| {
            edge.relationship_site.is_some()
                && edge.relationship_site.as_ref() != Some(&exact_surviving_site)
        })
        .ok_or("missing unmatched current AST occurrence")?;
    assert!(
        unmatched_current
            .evidence
            .iter()
            .all(|evidence| { evidence.origin == compass_model::provenance::EvidenceOrigin::Ast })
    );
    assert!(deleted_sites.iter().all(|deleted| {
        incremental_calls
            .iter()
            .all(|edge| edge.relationship_site.as_ref() != Some(deleted))
    }));
    assert!(
        incremental_calls.iter().all(|edge| {
            edge.evidence
                .iter()
                .all(|evidence| evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap"))
        }),
        "transient remap provenance survived: {incremental_calls:?}"
    );

    let mut clean_options = BuildOptions::new(root.to_path_buf());
    clean_options.output_root = Some(root.join("clean-cardinality-out"));
    clean_options.no_cluster = true;
    clean_options.no_viz = true;
    clean_options.force = true;
    clean_options.purpose = BuildPurpose::Extract;
    let final_layer = semantic_layer(final_semantic)?;
    let clean = build_graph_with_layers(&clean_options, Some(&final_layer), &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    assert_eq!(
        semantic_links_without_transport_rewrites(&incremental_graph),
        semantic_links_without_transport_rewrites(&clean_graph)
    );
    Ok(())
}

#[test]
fn incremental_mixed_origin_alias_edges_use_canonical_fresh_relationship_sites()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let initial_source = b"pub struct Widget;\nimpl Widget {\n    pub fn run(&self) {}\n}\n";
    fs::write(&source, initial_source)?;
    let initial_semantic = semantic_method_alias_extraction(Path::new("main.rs"), initial_source)?;
    assert_eq!(initial_semantic.edges.len(), 1);

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial_layer = semantic_layer(initial_semantic)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    assert!(initial_graph.links.iter().any(|edge| {
        edge.kind == compass_model::code_graph::EdgeKind::Contains
            && edge.evidence.iter().any(|evidence| {
                evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
            })
    }));

    let final_source =
        b"// moved method site\npub struct Widget;\nimpl Widget {\n    pub fn run(&self) {}\n}\n";
    fs::write(&source, final_source)?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let method_ids = incremental_graph
        .nodes
        .iter()
        .filter(|node| node.kind == compass_model::code_graph::NodeKind::Method)
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    let incremental_methods = incremental_graph
        .links
        .iter()
        .filter(|edge| {
            edge.kind == compass_model::code_graph::EdgeKind::Contains
                && method_ids.contains(&edge.target.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        incremental_methods.len(),
        1,
        "stale alias edge survived beside fresh canonical edge: methods={method_ids:?} edges={:?}",
        incremental_graph.links
    );
    assert!(incremental_methods[0].evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));

    let final_semantic = semantic_method_alias_extraction(Path::new("main.rs"), final_source)?;
    assert_eq!(final_semantic.edges.len(), 1);
    let mut clean_options = BuildOptions::new(root.to_path_buf());
    clean_options.output_root = Some(root.join("clean-out"));
    clean_options.no_cluster = true;
    clean_options.no_viz = true;
    clean_options.force = true;
    clean_options.purpose = BuildPurpose::Extract;
    let final_layer = semantic_layer(final_semantic)?;
    let clean = build_graph_with_layers(&clean_options, Some(&final_layer), &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    assert_eq!(incremental_graph.links, clean_graph.links);
    Ok(())
}

#[test]
fn refreshed_mixed_edge_drops_stale_incremental_endpoint_remap_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let initial_source = b"pub fn caller() { target(); }\npub fn target() {}\n";
    fs::write(&source, initial_source)?;
    let initial_semantic = semantic_edge_extraction(Path::new("main.rs"), initial_source, "calls")?;
    assert_eq!(initial_semantic.edges.len(), 1);

    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let initial_layer = semantic_layer(initial_semantic)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let graph_path = initial.output_dir.join("graph.json");
    let mut initial_graph = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    let target_id = initial_graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == compass_model::code_graph::NodeKind::Function
                && (node.name == "target" || node.label() == "target()")
        })
        .map(|node| node.id.clone())
        .ok_or("missing target function")?;
    let stale_target_id = format!("stale:{target_id}");
    initial_graph
        .nodes
        .iter_mut()
        .find(|node| node.id == target_id)
        .ok_or("missing target node")?
        .id
        .clone_from(&stale_target_id);
    for edge in &mut initial_graph.links {
        if edge.source == target_id {
            edge.source.clone_from(&stale_target_id);
        }
        if edge.target == target_id {
            edge.target.clone_from(&stale_target_id);
        }
        edge.id = edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key.clone_from(&edge.id);
    }
    fs::write(&graph_path, serde_json::to_vec_pretty(&initial_graph)?)?;
    compass_model::code_graph::GraphDocument::load(&graph_path)?;

    let final_source =
        b"// moved relationship site\npub fn caller() { target(); }\npub fn target() {}\n";
    fs::write(&source, final_source)?;
    let incremental = build_graph_with_layers(&options, None, &[])?;
    let incremental_graph =
        compass_model::code_graph::GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
    let incremental_calls = incremental_graph
        .links
        .iter()
        .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(incremental_calls.len(), 1);
    assert!(incremental_calls[0].evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));
    assert!(
        incremental_calls[0]
            .evidence
            .iter()
            .all(|evidence| evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap")),
        "stale endpoint remap evidence survived: {:?}",
        incremental_calls[0].evidence
    );
    assert!(!serde_json::to_string(&incremental_graph)?.contains("incremental-ast-endpoint-remap"));

    let final_semantic = semantic_edge_extraction(Path::new("main.rs"), final_source, "calls")?;
    let mut clean_options = BuildOptions::new(root.to_path_buf());
    clean_options.output_root = Some(root.join("clean-remap-out"));
    clean_options.no_cluster = true;
    clean_options.no_viz = true;
    clean_options.force = true;
    clean_options.purpose = BuildPurpose::Extract;
    let final_layer = semantic_layer(final_semantic)?;
    let clean = build_graph_with_layers(&clean_options, Some(&final_layer), &[])?;
    let clean_graph =
        compass_model::code_graph::GraphDocument::load(&clean.output_dir.join("graph.json"))?;
    assert_eq!(incremental_graph.links, clean_graph.links);
    Ok(())
}

#[test]
fn incremental_ast_endpoint_remap_retains_exact_typed_rewrite_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let notes = root.join("notes.md");
    let source_text = "pub fn target() {}\n";
    fs::write(&source, source_text)?;
    fs::write(&notes, "# Semantic caller\n")?;
    let extracted = Engine::default().extract_source(&source, source_text.as_bytes())?;
    let sources = HashMap::from([(
        source.to_string_lossy().into_owned(),
        source_text.to_owned(),
    )]);
    let ast = compass_resolve::resolve_with_root(&[extracted], &sources, root);
    let mut semantic_target = ast
        .nodes
        .iter()
        .find(|node| node.label() == "target()")
        .cloned()
        .ok_or("missing raw target")?;
    semantic_target.id = "semantic-target-alias".to_owned();
    semantic_target
        .attributes
        .insert("_origin".to_owned(), json!("semantic"));
    semantic_target
        .attributes
        .insert("extractor".to_owned(), json!("test.semantic"));
    semantic_target
        .attributes
        .insert("source_file".to_owned(), json!("main.rs"));
    if let Some(source_anchor) = semantic_target
        .attributes
        .get_mut("source_anchor")
        .and_then(serde_json::Value::as_object_mut)
    {
        source_anchor.insert("file".to_owned(), json!("main.rs"));
    }
    let anchor = json!({
        "file":"notes.md",
        "startByte":0,
        "endByte":10,
        "startLine":1,
        "startColumn":0,
        "endLine":1,
        "endColumn":10
    });
    let supplemental = Extraction {
        nodes: vec![
            RawNodeRecord {
                id: "semantic-caller".to_owned(),
                attributes: Map::from_iter([
                    ("label".to_owned(), json!("Semantic caller")),
                    ("qualified_name".to_owned(), json!("semantic::caller")),
                    ("file_type".to_owned(), json!("concept")),
                    ("source_file".to_owned(), json!("notes.md")),
                    ("source_anchor".to_owned(), anchor.clone()),
                    ("_origin".to_owned(), json!("semantic")),
                    ("extractor".to_owned(), json!("test.semantic")),
                ]),
            },
            semantic_target,
        ],
        edges: vec![RawEdgeRecord {
            source: "semantic-caller".to_owned(),
            target: "semantic-target-alias".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("references")),
                ("rule".to_owned(), json!("semantic-reference")),
                ("source_file".to_owned(), json!("notes.md")),
                ("source_anchor".to_owned(), anchor),
                ("_origin".to_owned(), json!("semantic")),
                ("confidence".to_owned(), json!("INFERRED")),
                ("extractor".to_owned(), json!("test.semantic")),
            ]),
        }],
        ..Extraction::default()
    };
    let mut options = BuildOptions::new(root.to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;

    let initial_layer = semantic_layer(supplemental)?;
    let initial = build_graph_with_layers(&options, Some(&initial_layer), &[])?;
    let initial_path = initial.output_dir.join("graph.json");
    let mut initial_graph = compass_model::code_graph::GraphDocument::load(&initial_path)?;
    assert!(
        initial_graph
            .links
            .iter()
            .any(|edge| edge.kind == compass_model::code_graph::EdgeKind::References),
        "initial links={:?}",
        initial_graph.links
    );
    let site_less_template = {
        let reference = initial_graph
            .links
            .iter_mut()
            .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
            .ok_or("missing initial reference")?;
        reference.relationship_site = None;
        reference.id = edge_id(
            &reference.source,
            reference.kind,
            &reference.target,
            None,
            reference.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        reference.key.clone_from(&reference.id);
        let rewrite_evidence = reference
            .evidence
            .iter()
            .find(|evidence| evidence.rule.as_deref() == Some("graph-ghost-endpoint-remap"))
            .cloned()
            .ok_or("missing initial graph rewrite evidence")?;
        let mut template = reference.clone();
        template.evidence = vec![rewrite_evidence];
        template
    };
    for index in 0..105 {
        let mut site_less = site_less_template.clone();
        let occurrence =
            compass_model::provenance::OccurrenceRule::new(format!("site-less-reference-{index}"))
                .ok_or("invalid site-less occurrence")?;
        site_less.occurrence_rule = Some(occurrence);
        site_less.id = edge_id(
            &site_less.source,
            site_less.kind,
            &site_less.target,
            None,
            site_less.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        site_less.key.clone_from(&site_less.id);
        initial_graph.links.push(site_less);
    }
    initial_graph.multigraph = true;
    fs::write(&initial_path, serde_json::to_vec_pretty(&initial_graph)?)?;
    compass_model::code_graph::GraphDocument::load(&initial_path)?;

    fs::write(&source, "pub fn target() { let _changed = true; }\n")?;
    let first_result = build_graph_with_layers(&options, None, &[])?;
    let graph_path = first_result.output_dir.join("graph.json");
    let first = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    assert_eq!(
        first
            .links
            .iter()
            .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
            .count(),
        1,
        "unsafe site-less remaps were published: {:?}",
        first.links
    );
    assert_eq!(
        first
            .graph
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "dropped_incremental_remap_without_wiring_site"
            })
            .count(),
        100
    );
    assert_eq!(
        first
            .graph
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "incremental_remap_without_wiring_site_truncated"
            })
            .count(),
        1
    );
    let edge = first
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
        .ok_or("missing preserved semantic edge")?;

    assert_eq!(
        edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        Some("semantic-reference")
    );
    assert!(edge.evidence.iter().any(|evidence| {
        evidence.rule.as_deref() == Some("semantic-reference")
            && evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
    }));
    assert!(
        edge.evidence.iter().any(|evidence| {
            evidence.rule.as_deref() == Some("incremental-ast-endpoint-remap")
                && evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
                && evidence
                    .wiring_site
                    .as_ref()
                    .is_some_and(|site| site.file == "notes.md" && site.start_byte == 0)
        }),
        "evidence={:?}",
        edge.evidence
    );

    let mut accumulated = first.clone();
    accumulated
        .graph
        .diagnostics
        .push(compass_model::code_graph::GraphDiagnostic {
            severity: compass_model::code_graph::DiagnosticSeverity::Info,
            code: "unrelated_fixture_diagnostic".to_owned(),
            message: "must survive remap-family compaction".to_owned(),
            anchor: None,
            related_ids: vec!["fixture".to_owned()],
        });
    let prior_target = accumulated
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
        .map(|edge| edge.target.clone())
        .ok_or("missing first-build reference target")?;
    let stale_target = format!("second-build-stale:{prior_target}");
    accumulated
        .nodes
        .iter_mut()
        .find(|node| node.id == prior_target)
        .ok_or("missing first-build target node")?
        .id
        .clone_from(&stale_target);
    for edge in &mut accumulated.links {
        if edge.source == prior_target {
            edge.source.clone_from(&stale_target);
        }
        if edge.target == prior_target {
            edge.target.clone_from(&stale_target);
        }
        edge.id = edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key.clone_from(&edge.id);
    }
    let mut second_template = accumulated
        .links
        .iter()
        .find(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
        .cloned()
        .ok_or("missing first-build reference")?;
    let rewrite_evidence = second_template
        .evidence
        .iter()
        .find(|evidence| evidence.rule.as_deref() == Some("incremental-ast-endpoint-remap"))
        .cloned()
        .ok_or("missing first-build incremental rewrite evidence")?;
    second_template.evidence = vec![rewrite_evidence];
    second_template.relationship_site = None;
    second_template.id = edge_id(
        &second_template.source,
        second_template.kind,
        &second_template.target,
        None,
        second_template
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
    );
    second_template.key.clone_from(&second_template.id);
    for index in 0..105 {
        let mut site_less = second_template.clone();
        let occurrence = compass_model::provenance::OccurrenceRule::new(format!(
            "second-site-less-reference-{index}"
        ))
        .ok_or("invalid second site-less occurrence")?;
        site_less.occurrence_rule = Some(occurrence);
        site_less.id = edge_id(
            &site_less.source,
            site_less.kind,
            &site_less.target,
            None,
            site_less.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        site_less.key.clone_from(&site_less.id);
        accumulated.links.push(site_less);
    }
    fs::write(&graph_path, serde_json::to_vec_pretty(&accumulated)?)?;
    compass_model::code_graph::GraphDocument::load(&graph_path)?;

    fs::write(
        &source,
        "// shifted before target\npub fn target() { let _changed_again = \"second\"; }\n",
    )?;
    let second_result = build_graph_with_layers(&options, None, &[])?;
    let second_graph_path = second_result.output_dir.join("graph.json");
    let second = compass_model::code_graph::GraphDocument::load(&second_graph_path)?;
    assert_eq!(
        second
            .links
            .iter()
            .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::References)
            .count(),
        1,
        "second incremental build did not process the injected site-less edges"
    );
    assert_eq!(
        second
            .graph
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "dropped_incremental_remap_without_wiring_site"
            })
            .count(),
        100,
        "remap diagnostics grew across incremental builds"
    );
    let summaries = second
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "incremental_remap_without_wiring_site_truncated")
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].message,
        "omitted 110 additional incremental remap diagnostics"
    );
    assert!(
        second
            .graph
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unrelated_fixture_diagnostic")
    );
    let diagnostic_order = second
        .graph
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.code.as_str(), diagnostic.message.as_str()))
        .collect::<Vec<_>>();
    assert!(
        diagnostic_order.windows(2).all(|pair| pair[0] <= pair[1]),
        "diagnostics are not in deterministic order"
    );

    let second_bytes = fs::read(&second_graph_path)?;
    let third_result = build_graph_with_layers(&options, None, &[])?;
    let third_graph_path = third_result.output_dir.join("graph.json");
    let third_bytes = fs::read(&third_graph_path)?;
    let third = compass_model::code_graph::GraphDocument::load(&third_graph_path)?;
    assert_eq!(third.links, second.links);
    assert_eq!(third_bytes, second_bytes);
    Ok(())
}
