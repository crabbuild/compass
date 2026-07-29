use std::fs;
use std::path::Path;

use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
    NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, Provenance, SourceAnchor,
};
use sha2::{Digest, Sha256};

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    }
}

fn evidence() -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    }
}

fn node(id: &str, kind: NodeKind, name: &str, qualified_name: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind,
        roles: Vec::new(),
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

pub fn write_graph(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = b"code";
    let source_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("src/lib.rs");
    fs::create_dir_all(source_path.parent().unwrap_or_else(|| Path::new(".")))?;
    fs::write(&source_path, source)?;
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "sha256:schema".to_owned(),
        source_tree_digest: "sha256:tree".to_owned(),
        configuration_digest: "sha256:config".to_owned(),
        generation_id: "sha256:generation".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(source)),
        byte_size: 4,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    graph.nodes = vec![
        node("n:list", NodeKind::Function, "list", "UserService.list"),
        node(
            "n:listing",
            NodeKind::Function,
            "listing",
            "UserService.listing",
        ),
        node("n:other", NodeKind::Function, "list", "Other.list"),
        node("n:unicode", NodeKind::Function, "café", "Menu.café"),
        node("n:alias", NodeKind::TypeAlias, "fetchUsers", "fetchUsers"),
        node("n:caller", NodeKind::Function, "caller", "Api.caller"),
        node("n:callee", NodeKind::Function, "callee", "Store.callee"),
        node(
            "n:route",
            NodeKind::Route,
            "GET /users",
            "express::GET::/users",
        ),
        node("n:dependent", NodeKind::Module, "dependent", "dependent"),
        node(
            "n:heuristic",
            NodeKind::Function,
            "dynamicCaller",
            "Api.dynamicCaller",
        ),
    ];
    let alias_id = edge_id(
        "n:alias",
        EdgeKind::Aliases,
        "n:list",
        Some(&anchor()),
        None,
    );
    graph.links.push(EdgeRecord {
        id: alias_id.clone(),
        key: alias_id,
        source: "n:alias".to_owned(),
        target: "n:list".to_owned(),
        kind: EdgeKind::Aliases,
        occurrence_rule: None,
        relationship_site: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    for (source, kind, target) in [
        ("n:caller", EdgeKind::Calls, "n:list"),
        ("n:list", EdgeKind::Calls, "n:callee"),
        ("n:route", EdgeKind::RoutesTo, "n:list"),
        ("n:dependent", EdgeKind::Imports, "n:caller"),
    ] {
        let id = edge_id(source, kind, target, Some(&anchor()), None);
        graph.links.push(EdgeRecord {
            id: id.clone(),
            key: id,
            source: source.to_owned(),
            target: target.to_owned(),
            kind,
            occurrence_rule: None,
            relationship_site: Some(anchor()),
            details: None,
            evidence: vec![evidence()],
            weight: None,
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        });
    }
    let heuristic_id = edge_id(
        "n:heuristic",
        EdgeKind::Calls,
        "n:list",
        Some(&anchor()),
        Some("dynamic-dispatch"),
    );
    graph.links.push(EdgeRecord {
        id: heuristic_id.clone(),
        key: heuristic_id,
        source: "n:heuristic".to_owned(),
        target: "n:list".to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: OccurrenceRule::new("dynamic-dispatch"),
        relationship_site: Some(anchor()),
        details: None,
        evidence: vec![Provenance {
            origin: EvidenceOrigin::Heuristic,
            extractor: "test.dynamic".to_owned(),
            confidence: EvidenceConfidence::Inferred,
            rule: Some("dynamic-dispatch".to_owned()),
            anchors: Vec::new(),
            wiring_site: Some(anchor()),
            score: None,
            candidates: Vec::new(),
        }],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    fs::write(path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(())
}

#[allow(dead_code)]
pub fn write_deferred_external_inheritance_graph(
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    let child = node("n:child", NodeKind::Class, "Child", "App\\Child");
    let mut external = node(
        "n:external",
        NodeKind::Class,
        "Model",
        "Illuminate\\Database\\Eloquent\\Model",
    );
    external.source = None;
    external.evidence = vec![Provenance {
        origin: EvidenceOrigin::Heuristic,
        extractor: "compass.graph.external-placeholder".to_owned(),
        confidence: EvidenceConfidence::Inferred,
        rule: Some("external-symbol-placeholder".to_owned()),
        anchors: Vec::new(),
        wiring_site: Some(anchor()),
        score: None,
        candidates: Vec::new(),
    }];
    graph.nodes.extend([child, external]);
    let id = edge_id(
        "n:child",
        EdgeKind::Extends,
        "n:external",
        Some(&anchor()),
        None,
    );
    graph.links.push(EdgeRecord {
        id: id.clone(),
        key: id,
        source: "n:child".to_owned(),
        target: "n:external".to_owned(),
        kind: EdgeKind::Extends,
        occurrence_rule: None,
        relationship_site: Some(anchor()),
        details: None,
        evidence: vec![
            evidence(),
            Provenance {
                origin: EvidenceOrigin::Heuristic,
                extractor: "compass.graph.external-placeholder".to_owned(),
                confidence: EvidenceConfidence::Inferred,
                rule: Some("external-symbol-placeholder".to_owned()),
                anchors: Vec::new(),
                wiring_site: Some(anchor()),
                score: None,
                candidates: Vec::new(),
            },
        ],
        weight: None,
        context: None,
        deferred: true,
        diagnostics: Vec::new(),
    });
    fs::write(path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(())
}

#[allow(dead_code)]
pub fn write_endpoint_remap_graph(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/service.rs"), "fn caller() {}\n")?;
    fs::write(root.join("src/target.rs"), "fn target() {}\n")?;
    let extraction: compass_languages::Extraction = serde_json::from_value(serde_json::json!({
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
        "edges": [{
            "source": "semantic_caller",
            "target": "target",
            "relation": "calls",
            "confidence": "EXTRACTED",
            "source_file": "src/service.rs",
            "source_location": "L1",
            "_origin": "ast",
            "extractor": "test.rust"
        }]
    }))?;
    let flexible = compass_graph::build_from_extraction(&extraction, true, Some(root));
    let graph = compass_graph::normalize_document_v1(&flexible, root, "sha256:test", None)?;
    fs::write(path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(())
}
