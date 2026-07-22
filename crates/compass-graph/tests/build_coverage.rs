use std::error::Error;
use std::fs;

use compass_graph::{build, build_from_extraction};
use compass_languages::Extraction;
use serde_json::json;

#[test]
fn build_wrapper_remaps_document_twins_ghosts_edges_paths_and_hyperedges()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("docs"))?;
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes":[
            {"id":"guide","label":"Guide","file_type":"document","source_file":directory.path().join("docs/guide.md"),"_origin":"semantic"},
            {"id":"guide_doc","label":"Guide document","file_type":"document","source_file":directory.path().join("docs/guide.md"),"_origin":"semantic"},
            {"id":"ast_service","label":"Service","file_type":"code","source_file":"src/service.rs","source_location":"L1","_origin":"ast"},
            {"id":"semantic_service","label":"Service","file_type":"concept","source_file":"other/service.rs","source_location":"L2","_origin":"semantic"},
            {"id":"target","label":"Target","file_type":"code","source_file":"src/target.rs","source_location":"L3","_origin":"ast"}
        ],
        "edges":[
            {"source":"guide","target":"guide_doc","relation":"documents","weight":"bad"},
            {"source":"semantic_service","target":"target","relation":"calls","source_file":directory.path().join("src/service.rs")},
            {"source":"ast_service","target":"target","relation":"calls","confidence_score":"bad","extra":"merged"},
            {"source":"missing","target":"target","relation":"ignored"},
            {"source":"target","target":"missing","relation":"ignored"}
        ],
        "hyperedges":[
            4,
            {"id":"group","nodes":["semantic_service","target","missing"],"source_file":directory.path().join("src/service.rs")},
            {"id":"empty","members":["missing"]}
        ]
    }))?;
    let document = build(
        std::slice::from_ref(&extraction),
        false,
        false,
        Some(directory.path()),
    )?;
    assert!(!document.directed);
    assert_eq!(
        document
            .nodes
            .iter()
            .filter(|node| node.string("file_type") == "document")
            .count(),
        1
    );
    assert!(
        document
            .nodes
            .iter()
            .any(|node| node.id.ends_with("guide_doc"))
    );
    assert!(document.nodes.iter().any(|node| node.id == "ast_service"));
    assert!(
        !document
            .nodes
            .iter()
            .any(|node| node.id == "semantic_service")
    );
    assert_eq!(document.links.len(), 1);
    assert_eq!(document.links[0].attributes["extra"], "merged");
    assert_eq!(document.links[0].attributes["confidence_score"], 1.0);
    assert!(document.links[0].string("source_file").starts_with("src/"));
    let hyperedges = document.graph["hyperedges"]
        .as_array()
        .ok_or("missing hyperedges")?;
    assert_eq!(hyperedges.len(), 1);
    assert_eq!(hyperedges[0]["nodes"], json!(["ast_service", "target"]));

    let deduplicated = build(
        std::slice::from_ref(&extraction),
        true,
        true,
        Some(directory.path()),
    )?;
    assert!(deduplicated.directed);
    assert!(!deduplicated.nodes.is_empty());
    Ok(())
}

#[test]
fn cross_language_phantoms_are_dropped_while_supported_families_survive()
-> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes":[
            {"id":"java","label":"Java","source_file":"src/A.java","file_type":"code"},
            {"id":"kotlin","label":"Kotlin","source_file":"src/B.kt","file_type":"code"},
            {"id":"cpp","label":"Cpp","source_file":"src/c.cpp","file_type":"code"},
            {"id":"objc","label":"Objc","source_file":"src/d.mm","file_type":"code"},
            {"id":"ruby","label":"Ruby","source_file":"src/e.rb","file_type":"code"},
            {"id":"php","label":"Php","source_file":"src/f.php","file_type":"code"},
            {"id":"swift","label":"Swift","source_file":"src/g.swift","file_type":"code"},
            {"id":"lua","label":"Lua","source_file":"src/h.lua","file_type":"code"}
        ],
        "edges":[
            {"source":"java","target":"kotlin","relation":"calls","confidence":"INFERRED"},
            {"source":"cpp","target":"objc","relation":"calls","confidence":"INFERRED"},
            {"source":"ruby","target":"php","relation":"calls","confidence":"INFERRED"},
            {"source":"swift","target":"lua","relation":"calls","confidence":"INFERRED"}
        ]
    }))?;
    let document = build_from_extraction(&extraction, true, None);
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.source == "java" && edge.target == "kotlin")
    );
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.source == "cpp" && edge.target == "objc")
    );
    assert!(
        !document
            .links
            .iter()
            .any(|edge| edge.source == "ruby" && edge.target == "php")
    );
    assert!(
        !document
            .links
            .iter()
            .any(|edge| edge.source == "swift" && edge.target == "lua")
    );
    Ok(())
}
