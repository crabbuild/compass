use std::fs;
use std::path::Path;

use compass_languages::Extraction;
use compass_model::code_graph::GraphDocument;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_model::query_contract::{CodeQueryLimits, ImpactRequest, NodeTrailRequest};
use compass_query::open;
use serde_json::json;

fn write_sources(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root.join("src"))?;
    for (path, source) in [
        ("src/service.rs", "fn caller() {}\nfn direct() {}\n"),
        ("src/target.rs", "fn target() {}\n"),
        ("src/first.rs", "fn first() {}\n"),
        ("src/second.rs", "fn second() {}\n"),
        ("src/unique.rs", "fn unique() {}\n"),
    ] {
        fs::write(root.join(path), source)?;
    }
    Ok(())
}

fn normalize_fixture(
    root: &Path,
    extraction: &Extraction,
) -> Result<GraphDocument, Box<dyn std::error::Error>> {
    let flexible = compass_graph::build_from_extraction(extraction, true, Some(root));
    Ok(compass_graph::normalize_document_v1(
        &flexible,
        root,
        "sha256:test",
        None,
    )?)
}

fn duplicate_edge_extraction(remapped_first: bool) -> Result<Extraction, serde_json::Error> {
    let direct = json!({
        "source": "ast_caller",
        "target": "target",
        "relation": "calls",
        "confidence": "EXTRACTED",
        "source_file": "src/service.rs",
        "source_location": "L2",
        "_origin": "ast",
        "extractor": "test.direct"
    });
    let remapped = json!({
        "source": "semantic_caller",
        "target": "target",
        "relation": "calls",
        "confidence": "EXTRACTED",
        "source_file": "src/service.rs",
        "source_location": "L1",
        "_origin": "ast",
        "extractor": "test.remapped"
    });
    let edges = if remapped_first {
        vec![remapped, direct]
    } else {
        vec![direct, remapped]
    };
    serde_json::from_value(json!({
        "nodes": [
            {
                "id": "ast_caller",
                "label": "Caller",
                "qualified_name": "crate::Caller",
                "symbol_kind": "function",
                "file_type": "code",
                "language": "rust",
                "source_file": "src/service.rs",
                "source_location": "L1",
                "_origin": "ast"
            },
            {
                "id": "semantic_caller",
                "label": "Caller",
                "qualified_name": "crate::Caller",
                "symbol_kind": "function",
                "file_type": "code",
                "language": "rust",
                "source_file": "src/service.rs",
                "source_location": "L1",
                "_origin": "semantic"
            },
            {
                "id": "target",
                "label": "Target",
                "qualified_name": "crate::Target",
                "symbol_kind": "function",
                "file_type": "code",
                "language": "rust",
                "source_file": "src/target.rs",
                "source_location": "L1",
                "_origin": "ast"
            }
        ],
        "edges": edges
    }))
}

#[test]
fn exact_and_remapped_duplicate_edges_are_order_independent_and_heuristic_gated()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    write_sources(directory.path())?;
    let direct_first = normalize_fixture(directory.path(), &duplicate_edge_extraction(false)?)?;
    let remapped_first = normalize_fixture(directory.path(), &duplicate_edge_extraction(true)?)?;

    assert_eq!(
        serde_json::to_vec(&direct_first)?,
        serde_json::to_vec(&remapped_first)?
    );
    assert_eq!(direct_first.links.len(), 1);
    let edge = &direct_first.links[0];
    assert!(edge.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Ast
            && evidence.confidence == EvidenceConfidence::Exact
            && evidence.anchors.iter().any(|anchor| anchor.start_line == 2)
    }));
    assert!(edge.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence.rule.as_deref() == Some("graph-ghost-endpoint-remap")
            && evidence.score == Some(0.95)
            && evidence
                .wiring_site
                .as_ref()
                .is_some_and(|anchor| anchor.start_line == 1)
    }));

    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&direct_first)?)?;
    let engine = open(&graph_path, None, &directory.path().join("query-cache"))?;
    let impact = |include_heuristic| ImpactRequest {
        symbol: "crate::Target".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };
    assert!(
        !engine
            .impact(impact(false))?
            .nodes
            .iter()
            .any(|node| node.name == "Caller")
    );
    assert!(
        engine
            .impact(impact(true))?
            .nodes
            .iter()
            .any(|node| node.name == "Caller")
    );
    let trail = |include_heuristic| NodeTrailRequest {
        source: "crate::Caller".to_owned(),
        target: "crate::Target".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };
    assert!(engine.node_trail(trail(false))?.paths.is_empty());
    assert_eq!(engine.node_trail(trail(true))?.paths.len(), 1);
    Ok(())
}

fn normalized_alias_collision(reverse_nodes: bool) -> Result<Extraction, serde_json::Error> {
    let first = json!({
        "id": "Candidate::Shared",
        "label": "First",
        "qualified_name": "crate::First",
        "symbol_kind": "function",
        "file_type": "code",
        "language": "rust",
        "source_file": "src/first.rs",
        "source_location": "L1",
        "_origin": "ast"
    });
    let second = json!({
        "id": "candidate-shared",
        "label": "Second",
        "qualified_name": "crate::Second",
        "symbol_kind": "function",
        "file_type": "code",
        "language": "rust",
        "source_file": "src/second.rs",
        "source_location": "L1",
        "_origin": "ast"
    });
    let candidates = if reverse_nodes {
        vec![second, first]
    } else {
        vec![first, second]
    };
    let mut nodes = candidates;
    nodes.push(json!({
        "id": "Unique::Caller",
        "label": "Unique",
        "qualified_name": "crate::Unique",
        "symbol_kind": "function",
        "file_type": "code",
        "language": "rust",
        "source_file": "src/unique.rs",
        "source_location": "L1",
        "_origin": "ast"
    }));
    nodes.push(json!({
        "id": "target",
        "label": "Target",
        "qualified_name": "crate::Target",
        "symbol_kind": "function",
        "file_type": "code",
        "language": "rust",
        "source_file": "src/target.rs",
        "source_location": "L1",
        "_origin": "ast"
    }));
    serde_json::from_value(json!({
        "nodes": nodes,
        "edges": [
            {
                "source": "candidate shared",
                "target": "target",
                "relation": "calls",
                "confidence": "EXTRACTED",
                "source_file": "src/first.rs",
                "source_location": "L1",
                "_origin": "ast",
                "extractor": "test.alias"
            },
            {
                "source": "unique caller",
                "target": "target",
                "relation": "calls",
                "confidence": "EXTRACTED",
                "source_file": "src/unique.rs",
                "source_location": "L1",
                "_origin": "ast",
                "extractor": "test.unique-alias"
            }
        ]
    }))
}

#[test]
fn lossy_normalized_alias_collisions_are_diagnostic_and_never_choose_a_node()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    write_sources(directory.path())?;
    let forward = normalize_fixture(directory.path(), &normalized_alias_collision(false)?)?;
    let reverse = normalize_fixture(directory.path(), &normalized_alias_collision(true)?)?;

    assert_eq!(serde_json::to_vec(&forward)?, serde_json::to_vec(&reverse)?);
    assert_eq!(forward.links.len(), 1);
    assert!(forward.links[0].evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.rule.as_deref() == Some("graph-normalized-id-remap")
            && evidence.score == Some(0.8)
            && evidence.wiring_site.is_some()
    }));
    assert!(forward.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "ambiguous_normalized_endpoint"
            && diagnostic.related_ids == ["Candidate::Shared", "candidate-shared"]
    }));
    let mut candidates = forward
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.qualified_name.as_str(),
                "crate::First" | "crate::Second"
            )
        })
        .map(|node| (node.qualified_name.clone(), node.id.clone()))
        .collect::<Vec<_>>();
    candidates.sort();
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].1, candidates[1].1);
    assert!(candidates.iter().all(|(_, id)| !id.is_empty()));

    let graph_path = directory.path().join("collision.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&forward)?)?;
    let engine = open(&graph_path, None, &directory.path().join("collision-cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "crate::Target".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(!impact.nodes.iter().any(|node| {
        matches!(
            node.qualified_name.as_str(),
            "crate::First" | "crate::Second" | "crate::Unique"
        )
    }));
    assert!(
        engine
            .node_trail(NodeTrailRequest {
                source: "crate::First".to_owned(),
                target: "crate::Target".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })?
            .paths
            .is_empty()
    );
    Ok(())
}
