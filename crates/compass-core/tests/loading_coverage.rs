use std::error::Error;
use std::fs;

use compass_core::{
    BuildOptions, CoreError, ExportInputs, LoadedGraph, SemanticLayer, build_graph_with_layers,
};
use compass_files::BuildGuard;
use compass_languages::{Engine, Extraction, RawEdgeRecord, RawNodeRecord};
use serde_json::{Map, json};
use sha2::{Digest, Sha256};

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
        output.join(".compass_analysis.json"),
        r#"{"communities":{"bad":"not-an-array"},"cohesion":{"0":0.75,"bad":1,"1":"wrong"},"gods":"wrong"}"#,
    )?;
    fs::write(
        output.join(".compass_labels.json"),
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
        output.join(".compass_root"),
        directory.path().to_string_lossy().as_bytes(),
    )?;
    fs::write(
        output.join(".compass_learning.json"),
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
    fs::write(output.join(".compass_learning.json"), "not json")?;
    assert!(LoadedGraph::load(&graph)?.overlay.is_empty());
    fs::remove_file(output.join(".compass_learning.json"))?;
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
fn incremental_ast_endpoint_remap_retains_exact_typed_rewrite_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("main.rs");
    let notes = root.join("notes.md");
    fs::write(&source, "pub fn target() {}\n")?;
    fs::write(&notes, "# Semantic caller\n")?;
    let ast = Engine::default().extract(&source)?;
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

    let initial = build_graph_with_layers(&options, None, &[serde_json::to_value(supplemental)?])?;
    let initial_graph =
        compass_model::code_graph::GraphDocument::load(&initial.output_dir.join("graph.json"))?;
    assert!(
        initial_graph
            .links
            .iter()
            .any(|edge| edge.kind == compass_model::code_graph::EdgeKind::References),
        "initial links={:?}",
        initial_graph.links
    );
    fs::write(&source, "pub fn target() { let _changed = true; }\n")?;
    build_graph_with_layers(&options, None, &[])?;
    let graph_path = BuildGuard::resolve_artifact(&root.join("compass-out"), "graph.json")?;
    let first = compass_model::code_graph::GraphDocument::load(&graph_path)?;
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

    let first_bytes = fs::read(&graph_path)?;
    build_graph_with_layers(&options, None, &[])?;
    let second_bytes = fs::read(&graph_path)?;
    let second = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    assert_eq!(second.links, first.links);
    assert_eq!(second_bytes, first_bytes);
    Ok(())
}
