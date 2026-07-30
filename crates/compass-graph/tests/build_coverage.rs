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
        document
            .nodes
            .iter()
            .any(|node| node.id == "semantic_service")
    );
    assert_eq!(document.links.len(), 2);
    let ast_edge = document
        .links
        .iter()
        .find(|edge| edge.source == "ast_service")
        .ok_or("missing AST edge")?;
    assert_eq!(ast_edge.attributes["extra"], "merged");
    assert_eq!(ast_edge.attributes["confidence_score"], 1.0);
    assert!(ast_edge.string("source_file").starts_with("src/"));
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.source == "semantic_service")
    );
    let hyperedges = document.graph["hyperedges"]
        .as_array()
        .ok_or("missing hyperedges")?;
    assert_eq!(hyperedges.len(), 1);
    assert_eq!(
        hyperedges[0]["nodes"],
        json!(["semantic_service", "target"])
    );

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

#[test]
fn ghost_coalescing_requires_matching_path_kind_qualified_name_and_details()
-> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes":[
            {
                "id":"config_dependency",
                "label":"react",
                "qualified_name":"dependencies.react",
                "type":"config_key",
                "file_type":"code",
                "source_file":"config/package.json",
                "source_location":"L2",
                "_origin":"ast",
                "config_path":"dependencies.react"
            },
            {
                "id":"package_dependency",
                "label":"react",
                "qualified_name":"react",
                "type":"package",
                "file_type":"code",
                "source_file":"config/package.json",
                "source_location":"L2",
                "_origin":"semantic",
                "package_name":"react"
            },
            {
                "id":"other_package_dependency",
                "label":"react",
                "qualified_name":"react",
                "type":"package",
                "file_type":"code",
                "source_file":"services/package.json",
                "source_location":"L2",
                "_origin":"semantic",
                "package_name":"react"
            }
        ],
        "edges":[]
    }))?;

    let document = build_from_extraction(&extraction, true, None);

    assert_eq!(document.nodes.len(), 3);
    assert!(document.nodes.iter().any(|node| {
        node.string("type") == "config_key" && node.string("source_file") == "config/package.json"
    }));
    assert!(document.nodes.iter().any(|node| {
        node.string("type") == "package" && node.string("source_file") == "config/package.json"
    }));
    assert!(document.nodes.iter().any(|node| {
        node.string("type") == "package" && node.string("source_file") == "services/package.json"
    }));
    Ok(())
}

#[test]
fn every_graph_assembly_endpoint_remap_stamps_distinct_synthesized_evidence()
-> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {"id":"src_service","label":"service.rs","file_type":"code","source_file":"src/service.rs","source_location":"L1","_origin":"ast"},
            {"id":"service_helper","label":"helper","qualified_name":"helper","type":"function","file_type":"code","source_file":"src/service.rs","source_location":"L1","_origin":"semantic"},
            {"id":"guide","label":"Guide","qualified_name":"Guide","file_type":"document","source_file":"docs/guide.md","source_location":"L1","_origin":"semantic"},
            {"id":"guide_doc","label":"Guide document","qualified_name":"Guide document","file_type":"document","source_file":"docs/guide.md","source_location":"L1","_origin":"semantic"},
            {"id":"ast_worker","label":"Worker","qualified_name":"pkg::Worker","type":"function","file_type":"code","source_file":"src/worker.rs","source_location":"L1","_origin":"ast"},
            {"id":"semantic_worker","label":"Worker","qualified_name":"pkg::Worker","type":"function","file_type":"code","source_file":"src/worker.rs","source_location":"L1","_origin":"semantic"},
            {"id":"target","label":"Target","qualified_name":"pkg::Target","type":"function","file_type":"code","source_file":"src/target.rs","source_location":"L1","_origin":"ast"}
        ],
        "edges": [
            {"source":"service_helper","target":"target","relation":"calls","source_file":"src/service.rs","source_location":"L1","_origin":"ast"},
            {"source":"target","target":"guide","relation":"documents","source_file":"docs/guide.md","source_location":"L1","_origin":"ast"},
            {"source":"semantic_worker","target":"target","relation":"calls","source_file":"src/worker.rs","source_location":"L1","_origin":"ast"}
        ]
    }))?;

    let document = build_from_extraction(&extraction, true, None);
    let rewrite_entries = document
        .links
        .iter()
        .flat_map(|edge| {
            edge.attributes["_endpoint_rewrite_rules"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    let rules = rewrite_entries
        .iter()
        .filter_map(|entry| entry.get("rule").and_then(|value| value.as_str()))
        .collect::<std::collections::HashSet<_>>();
    for expected in [
        "graph-semantic-id-remap",
        "graph-document-twin-remap",
        "graph-ghost-endpoint-remap",
    ] {
        assert!(rules.contains(expected), "missing {expected}");
    }
    assert!(document.links.iter().all(|edge| {
        edge.string("_origin") == "ast"
            && edge.string("rule").is_empty()
            && !edge.string("source_file").is_empty()
            && !edge.string("source_location").is_empty()
    }));
    assert!(rewrite_entries.iter().all(|entry| {
        entry["_origin"] == "heuristic"
            && entry["confidence"] == "INFERRED"
            && entry["score"].as_f64().is_some()
            && !entry["source_file"].as_str().unwrap_or_default().is_empty()
            && !entry["source_location"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
    }));
    Ok(())
}

fn direct_and_legacy_alias_collision(reverse: bool) -> Result<Extraction, serde_json::Error> {
    let direct = json!({
        "id": "service_helper",
        "label": "Direct helper",
        "qualified_name": "direct::helper",
        "type": "function",
        "file_type": "code",
        "source_file": "src/direct.rs",
        "source_location": "L1",
        "_origin": "ast"
    });
    let legacy = json!({
        "id": "src_service_helper",
        "label": "Legacy helper",
        "qualified_name": "legacy::helper",
        "type": "function",
        "file_type": "code",
        "source_file": "src/service.rs",
        "source_location": "L1",
        "_origin": "ast"
    });
    let mut nodes = if reverse {
        vec![legacy, direct]
    } else {
        vec![direct, legacy]
    };
    nodes.push(json!({
        "id": "target",
        "label": "Target",
        "qualified_name": "crate::Target",
        "type": "function",
        "file_type": "code",
        "source_file": "src/target.rs",
        "source_location": "L1",
        "_origin": "ast"
    }));
    let members = if reverse {
        json!(["target", "service helper"])
    } else {
        json!(["service helper", "target"])
    };
    let value = json!({
        "nodes": nodes,
        "edges": [{
            "source": "service helper",
            "target": "target",
            "relation": "calls",
            "source_file": "src/direct.rs",
            "source_location": "L1",
            "_origin": "ast"
        }],
        "hyperedges": [{
            "id": "ambiguous-group",
            "nodes": members,
            "source_file": "src/direct.rs"
        }]
    });
    serde_json::from_value(value)
}

#[test]
fn direct_and_legacy_alias_candidates_union_and_omit_ambiguous_hyperedges()
-> Result<(), Box<dyn Error>> {
    let forward = build_from_extraction(&direct_and_legacy_alias_collision(false)?, true, None);
    let reverse = build_from_extraction(&direct_and_legacy_alias_collision(true)?, true, None);

    for document in [&forward, &reverse] {
        assert!(document.links.is_empty());
        assert!(document.graph.get("hyperedges").is_none());
        let diagnostics = document.graph["_compass_v1_graph_diagnostics"]
            .as_array()
            .ok_or("missing collision diagnostics")?;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic["code"] == "ambiguous_normalized_endpoint"
                        && diagnostic["relatedIds"]
                            == json!(["service_helper", "src_service_helper"])
                })
                .count(),
            2
        );
    }
    assert_eq!(
        forward.graph["_compass_v1_graph_diagnostics"],
        reverse.graph["_compass_v1_graph_diagnostics"]
    );
    Ok(())
}

#[test]
fn one_ambiguous_member_omits_the_whole_hyperedge() -> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {"id":"Candidate::Shared","label":"First","type":"function","file_type":"code","source_file":"src/first.rs","_origin":"ast"},
            {"id":"candidate-shared","label":"Second","type":"function","file_type":"code","source_file":"src/second.rs","_origin":"ast"},
            {"id":"target","label":"Target","type":"function","file_type":"code","source_file":"src/target.rs","_origin":"ast"}
        ],
        "edges": [],
        "hyperedges": [{
            "id": "ambiguous-group",
            "nodes": ["candidate shared", "target"]
        }]
    }))?;

    let document = build_from_extraction(&extraction, true, None);
    assert!(document.graph.get("hyperedges").is_none());
    assert!(
        document.graph["_compass_v1_graph_diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics
                .iter()
                .any(|diagnostic| { diagnostic["code"] == "ambiguous_normalized_endpoint" }))
    );
    Ok(())
}

#[test]
fn networkx_edge_order_preserves_node_and_incident_edge_order() -> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {"id":"c","label":"C"},
            {"id":"a","label":"A"},
            {"id":"b","label":"B"}
        ],
        "edges": [
            {"source":"b","target":"c","relation":"calls"},
            {"source":"c","target":"c","relation":"calls"},
            {"source":"a","target":"c","relation":"calls"},
            {"source":"a","target":"b","relation":"calls"}
        ]
    }))?;

    let undirected = build_from_extraction(&extraction, false, None);
    assert!(!undirected.multigraph);
    assert_eq!(
        undirected
            .links
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect::<Vec<_>>(),
        [("c", "a"), ("c", "b"), ("c", "c"), ("a", "b")]
    );

    let directed = build_from_extraction(&extraction, true, None);
    assert!(!directed.multigraph);
    assert_eq!(
        directed
            .links
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect::<Vec<_>>(),
        [("c", "c"), ("a", "b"), ("a", "c"), ("b", "c")]
    );
    Ok(())
}

#[test]
fn distinct_relations_between_the_same_nodes_are_preserved() -> Result<(), Box<dyn Error>> {
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {"id":"caller","label":"caller()","file_type":"code","source_file":"main.c"},
            {"id":"target","label":"Target","file_type":"code","source_file":"main.c"}
        ],
        "edges": [
            {"source":"caller","target":"target","relation":"calls"},
            {"source":"caller","target":"target","relation":"references"}
        ]
    }))?;

    let document = build_from_extraction(&extraction, false, None);
    assert!(document.multigraph);
    let relations = document
        .links
        .iter()
        .map(|edge| edge.string("relation"))
        .collect::<Vec<_>>();
    assert_eq!(relations, ["calls", "references"]);
    Ok(())
}

#[test]
fn graph_assembly_preserves_distinct_relationship_occurrences_and_merges_identical_evidence()
-> Result<(), Box<dyn Error>> {
    let first = json!({
        "source":"caller",
        "target":"callee",
        "relation":"calls",
        "rule":"direct-call",
        "extractor":"test.first",
        "source_anchor":{
            "file":"src/lib.rs",
            "startByte":30,
            "endByte":38,
            "startLine":1,
            "startColumn":30,
            "endLine":1,
            "endColumn":38
        }
    });
    let duplicate = json!({
        "source":"caller",
        "target":"callee",
        "relation":"calls",
        "rule":"direct-call",
        "extractor":"test.second",
        "source_anchor":{
            "file":"src/lib.rs",
            "startByte":30,
            "endByte":38,
            "startLine":1,
            "startColumn":30,
            "endLine":1,
            "endColumn":38
        }
    });
    let second = json!({
        "source":"caller",
        "target":"callee",
        "relation":"calls",
        "rule":"direct-call",
        "extractor":"test.first",
        "source_anchor":{
            "file":"src/lib.rs",
            "startByte":39,
            "endByte":47,
            "startLine":1,
            "startColumn":39,
            "endLine":1,
            "endColumn":47
        }
    });
    let extraction = |edges| {
        serde_json::from_value::<Extraction>(json!({
            "nodes":[
                {"id":"caller","label":"caller()"},
                {"id":"callee","label":"callee()"}
            ],
            "edges":edges
        }))
    };

    let forward = build_from_extraction(
        &extraction(vec![first.clone(), duplicate.clone(), second.clone()])?,
        true,
        None,
    );
    let reverse = build_from_extraction(&extraction(vec![second, duplicate, first])?, true, None);

    assert_eq!(serde_json::to_vec(&forward)?, serde_json::to_vec(&reverse)?);
    assert_eq!(forward.links.len(), 2, "links={:?}", forward.links);
    assert!(forward.multigraph);
    let first_site = forward
        .links
        .iter()
        .find(|edge| edge.attributes["source_anchor"]["startByte"] == 30)
        .ok_or("missing first call site")?;
    let retained = first_site.attributes["_coalesced_edge_evidence"]
        .as_array()
        .ok_or("missing coalesced evidence")?;
    assert_eq!(retained.len(), 2);
    assert!(
        retained
            .iter()
            .any(|item| item["extractor"] == "test.first")
    );
    assert!(
        retained
            .iter()
            .any(|item| item["extractor"] == "test.second")
    );
    Ok(())
}

fn remapped_rule_extraction(
    edges: Vec<serde_json::Value>,
) -> Result<Extraction, serde_json::Error> {
    serde_json::from_value(json!({
        "nodes":[
            {
                "id":"ast_caller",
                "label":"Caller",
                "qualified_name":"crate::Caller",
                "symbol_kind":"function",
                "source_file":"src/lib.rs",
                "_origin":"ast"
            },
            {
                "id":"semantic_caller",
                "label":"Caller",
                "qualified_name":"crate::Caller",
                "symbol_kind":"function",
                "source_file":"src/lib.rs",
                "_origin":"semantic"
            },
            {"id":"callee","label":"callee()"}
        ],
        "edges":edges
    }))
}

fn ruled_call(source: &str, rule: &str, extractor: &str) -> serde_json::Value {
    json!({
        "source":source,
        "target":"callee",
        "relation":"calls",
        "rule":rule,
        "extractor":extractor,
        "source_anchor":{
            "file":"src/lib.rs",
            "startByte":30,
            "endByte":38,
            "startLine":1,
            "startColumn":30,
            "endLine":1,
            "endColumn":38
        }
    })
}

#[test]
fn direct_and_remapped_same_rule_occurrences_merge_order_independently()
-> Result<(), Box<dyn Error>> {
    let direct = ruled_call("ast_caller", "rust-call-expression", "test.direct");
    let remapped = ruled_call("semantic_caller", "rust-call-expression", "test.remapped");
    let forward = build_from_extraction(
        &remapped_rule_extraction(vec![direct.clone(), remapped.clone()])?,
        true,
        None,
    );
    let reverse = build_from_extraction(
        &remapped_rule_extraction(vec![remapped, direct])?,
        true,
        None,
    );

    assert_eq!(serde_json::to_vec(&forward)?, serde_json::to_vec(&reverse)?);
    assert_eq!(forward.links.len(), 1, "links={:?}", forward.links);
    Ok(())
}

#[test]
fn remapped_same_site_occurrences_with_distinct_producer_rules_remain_distinct()
-> Result<(), Box<dyn Error>> {
    let extraction = remapped_rule_extraction(vec![
        ruled_call("semantic_caller", "rust-call-expression", "test.rust"),
        ruled_call("semantic_caller", "scip-call-reference", "test.scip"),
    ])?;

    let document = build_from_extraction(&extraction, true, None);

    assert_eq!(document.links.len(), 2, "links={:?}", document.links);
    Ok(())
}

#[test]
fn endpoint_synthesis_rule_is_evidence_not_occurrence_identity() -> Result<(), Box<dyn Error>> {
    let mut direct = ruled_call("ast_caller", "", "test.direct");
    direct
        .as_object_mut()
        .ok_or("direct edge must be an object")?
        .remove("rule");
    let mut synthesized = ruled_call(
        "semantic_caller",
        "unique-stub-endpoint-resolution",
        "test.synthesized",
    );
    let synthesized_attributes = synthesized
        .as_object_mut()
        .ok_or("synthesized edge must be an object")?;
    synthesized_attributes.remove("rule");
    synthesized_attributes.insert(
        "_endpoint_rewrite_rules".to_owned(),
        json!([{
            "_origin":"heuristic",
            "confidence":"INFERRED",
            "extractor":"test.synthesized",
            "rule":"unique-stub-endpoint-resolution",
            "score":0.8
        }]),
    );

    let document = build_from_extraction(
        &remapped_rule_extraction(vec![direct, synthesized])?,
        true,
        None,
    );

    assert_eq!(document.links.len(), 1, "links={:?}", document.links);
    Ok(())
}

#[test]
fn structured_and_scalar_exact_anchors_merge_commutatively() -> Result<(), Box<dyn Error>> {
    let structured = json!({
        "source":"caller",
        "target":"callee",
        "relation":"calls",
        "rule":"direct-call",
        "extractor":"test.structured",
        "source_anchor":{
            "file":"src/lib.rs",
            "startByte":30,
            "endByte":38,
            "startLine":2,
            "startColumn":4,
            "endLine":2,
            "endColumn":12
        }
    });
    let scalar = json!({
        "source":"caller",
        "target":"callee",
        "relation":"calls",
        "rule":"direct-call",
        "extractor":"test.scalar",
        "source_file":"src/lib.rs",
        "source_location":"legacy-location-that-must-not-split-an-exact-range",
        "start_byte":30,
        "end_byte":38,
        "line_start":2,
        "column_start":4,
        "line_end":2,
        "column_end":12
    });
    let extraction = |edges| {
        serde_json::from_value::<Extraction>(json!({
            "nodes":[
                {"id":"caller","label":"caller()"},
                {"id":"callee","label":"callee()"}
            ],
            "edges":edges
        }))
    };

    let forward = build_from_extraction(
        &extraction(vec![structured.clone(), scalar.clone()])?,
        true,
        None,
    );
    let reverse = build_from_extraction(&extraction(vec![scalar, structured])?, true, None);

    assert_eq!(serde_json::to_vec(&forward)?, serde_json::to_vec(&reverse)?);
    assert_eq!(forward.links.len(), 1, "links={:?}", forward.links);
    let evidence = forward.links[0].attributes["_coalesced_edge_evidence"]
        .as_array()
        .ok_or("missing coalesced evidence")?;
    assert_eq!(evidence.len(), 2);
    Ok(())
}

#[test]
fn opposite_direction_relations_are_preserved_in_undirected_documents() -> Result<(), Box<dyn Error>>
{
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {"id":"a","label":"A","file_type":"code","source_file":"main.go"},
            {"id":"b","label":"B","file_type":"code","source_file":"main.go"}
        ],
        "edges": [
            {"source":"a","target":"b","relation":"calls"},
            {"source":"b","target":"a","relation":"calls"}
        ]
    }))?;

    let document = build_from_extraction(&extraction, false, None);
    assert!(document.multigraph);
    let true_directions = document
        .links
        .iter()
        .map(|edge| {
            (
                edge.attributes.get("_src").and_then(|value| value.as_str()),
                edge.attributes.get("_tgt").and_then(|value| value.as_str()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        true_directions,
        [(Some("a"), Some("b")), (Some("b"), Some("a"))]
    );
    Ok(())
}

#[test]
fn directed_reciprocals_and_self_loops_promote_only_for_parallel_endpoints()
-> Result<(), Box<dyn Error>> {
    let reciprocal: Extraction = serde_json::from_value(json!({
        "nodes": [{"id":"a","label":"A"}, {"id":"b","label":"B"}],
        "edges": [
            {"source":"a","target":"b","relation":"calls"},
            {"source":"b","target":"a","relation":"calls"},
            {"source":"a","target":"a","relation":"references"}
        ]
    }))?;
    let directed = build_from_extraction(&reciprocal, true, None);
    assert!(!directed.multigraph);

    let parallel_self_loops: Extraction = serde_json::from_value(json!({
        "nodes": [{"id":"a","label":"A"}],
        "edges": [
            {"source":"a","target":"a","relation":"calls"},
            {"source":"a","target":"a","relation":"references"}
        ]
    }))?;
    let directed = build_from_extraction(&parallel_self_loops, true, None);
    assert!(directed.multigraph);
    Ok(())
}
