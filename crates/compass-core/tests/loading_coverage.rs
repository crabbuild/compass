use std::error::Error;
use std::fs;

use compass_core::{
    BuildOptions, CoreError, ExportInputs, LoadedGraph, SemanticLayer, build_graph_with_layers,
};
use compass_files::BuildGuard;
use compass_languages::{Engine, Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::identity::edge_id;
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
    options.force = true;
    let second_result = build_graph_with_layers(&options, None, &[])?;
    options.force = false;
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
