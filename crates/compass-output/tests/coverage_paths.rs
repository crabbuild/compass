use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

use compass_graph::{
    Communities, GodNode, SuggestedQuestion, SurpriseConnection, blind_spot_report,
};
use compass_model::GraphDocument;
use compass_output::{
    DetectionSummary, ORIENTATION_JSON_MAX_BYTES, REPORT_MARKDOWN_MAX_CHARS, ReportOptions,
    TokenCost, agent_orientation_with_blind_spots, generate_report,
    generate_report_with_blind_spots, graphml_document, render_agent_report_markdown,
    render_orientation_json, write_graphml,
};
use serde_json::json;

fn document(value: serde_json::Value) -> Result<GraphDocument, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn graphml_serializes_every_scope_type_escape_and_empty_edge() -> Result<(), Box<dyn Error>> {
    let graph = document(json!({
        "directed": false,
        "multigraph": false,
        "graph": {
            "id":"ignored",
            "node_default":{},
            "edge_default":{},
            "enabled":true,
            "count":7,
            "huge":18446744073709551615_u64,
            "ratio":2.0,
            "nothing":null,
            "nested":{"z":"雪","a":[1,"😀"]},
            "title":"A&B <graph>"
        },
        "nodes":[
            {"id":"a&\"'","label":"Alpha <one>","enabled":false,"count":-2,"ratio":1.25,"_private":"skip"},
            {"id":"b","label":"Beta","list":[true,null,{"x":"é"}]}
        ],
        "links":[
            {"source":"a&\"'","target":"b","relation":"calls & uses","weight":3.0,"_private":"skip"},
            {"source":"b","target":"a&\"'"}
        ]
    }))?;
    let communities = Communities::from([(4, vec!["a&\"'".to_owned()])]);
    let xml = graphml_document(&graph, &communities);
    assert!(xml.contains("edgedefault=\"undirected\""));
    assert!(xml.contains("attr.type=\"boolean\""));
    assert!(xml.contains("attr.type=\"long\""));
    assert!(xml.contains("attr.type=\"double\""));
    assert!(xml.contains("attr.type=\"string\""));
    assert!(xml.contains("A&amp;B &lt;graph&gt;"));
    assert!(xml.contains("id=\"a&amp;&quot;&apos;\""));
    assert!(xml.contains("\\u96ea"));
    assert!(xml.contains("\\ud83d\\ude00"));
    assert!(xml.contains(">True<"));
    assert!(xml.contains(">False<"));
    assert!(xml.contains(">2.0<"));
    assert!(xml.contains("<edge source=\"b\" target=\"a&amp;&quot;&apos;\" />"));
    assert!(!xml.contains("_private"));

    let directory = tempfile::tempdir()?;
    let output = directory.path().join("nested/graph.graphml");
    write_graphml(&graph, &communities, &output)?;
    assert_eq!(fs::read_to_string(output)?, xml);
    fs::write(directory.path().join("file-parent"), "not a directory")?;
    assert!(
        write_graphml(
            &graph,
            &communities,
            directory.path().join("file-parent/graph.graphml")
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn reports_cover_navigation_quality_learning_hyperedges_and_questions() -> Result<(), Box<dyn Error>>
{
    let mut nodes = vec![
        json!({"id":"file_a","label":"a.rs","source_file":"src/a.rs","file_type":"code"}),
        json!({"id":"file_b","label":"b.rs","source_file":"src/b.rs","file_type":"code"}),
        json!({"id":"concept","label":"Concept","source_file":"","file_type":"document"}),
        json!({"id":"rationale","label":"Why","source_file":"why.md","file_type":"rationale"}),
    ];
    for index in 0..10 {
        nodes.push(json!({
            "id":format!("n{index}"),
            "label":format!("Node {index}"),
            "source_file":format!("src/n{index}.rs"),
            "file_type":"code"
        }));
    }
    let graph = document(json!({
        "directed":true,
        "graph":{"hyperedges":[
            {"label":"Pipeline","nodes":["n0","n1"],"confidence":"INFERRED","confidence_score":0.75},
            {"id":"Fallback","nodes":["n2"],"confidence":"EXTRACTED"},
            {"nodes":"invalid"}
        ]},
        "nodes":nodes,
        "links":[
            {"source":"file_a","target":"file_b","relation":"imports_from","confidence":"EXTRACTED","source_file":"src/a.rs"},
            {"source":"file_b","target":"file_a","relation":"imports_from","confidence":"INFERRED","confidence_score":0.9,"source_file":"src/b.rs"},
            {"source":"n0","target":"n1","relation":"calls","confidence":"INFERRED","source_file":"src/n0.rs"},
            {"source":"n1","target":"n2","relation":"uses","confidence":"AMBIGUOUS","source_file":"src/n1.rs"},
            {"source":"n2","target":"n3","relation":"uses","confidence":"AMBIGUOUS"},
            {"source":"n3","target":"n4","relation":"calls"}
        ]
    }))?;
    let communities = Communities::from([
        (
            0,
            (0..10).map(|index| format!("n{index}")).collect::<Vec<_>>(),
        ),
        (1, vec!["file_a".to_owned(), "file_b".to_owned()]),
        (2, vec!["missing".to_owned()]),
    ]);
    let cohesion = BTreeMap::from([(0, 0.875)]);
    let labels = BTreeMap::from([(0, "Runtime/Flow.md".to_owned()), (2, "[]:#^".to_owned())]);
    let gods = vec![GodNode {
        id: "n0".to_owned(),
        label: "Node 0".to_owned(),
        degree: 9,
    }];
    let surprises = vec![
        SurpriseConnection {
            source: "Node 0".to_owned(),
            target: "Concept".to_owned(),
            source_files: ["src/n0.rs".to_owned(), "docs/concept.md".to_owned()],
            confidence: "INFERRED".to_owned(),
            relation: "semantically_similar_to".to_owned(),
            why: None,
            note: Some("shared contract".to_owned()),
        },
        SurpriseConnection {
            source: "A".to_owned(),
            target: "B".to_owned(),
            source_files: ["a.rs".to_owned(), "b.rs".to_owned()],
            confidence: "EXTRACTED".to_owned(),
            relation: "calls".to_owned(),
            why: Some("explicit".to_owned()),
            note: None,
        },
    ];
    let detection = DetectionSummary {
        total_files: 12_345,
        total_words: 9_876_543,
        warning: None,
    };
    let questions = vec![
        SuggestedQuestion {
            kind: "community".to_owned(),
            question: Some("How does runtime flow?".to_owned()),
            why: "spans hubs".to_owned(),
        },
        SuggestedQuestion {
            kind: "community".to_owned(),
            question: None,
            why: "omitted text".to_owned(),
        },
        SuggestedQuestion {
            kind: "community_gap".to_owned(),
            question: Some("What evidence would directly connect Alpha and Beta?".to_owned()),
            why: "Structural gap score 0.5000: 2 shared two-hop intermediaries and 0 direct topical edges; wiring-only relations are excluded.".to_owned(),
        },
    ];
    let learning = json!({
        "overlay":{
            "n0":{"status":"preferred","label":"Node 0","uses":4,"score":0.9,"stale":true},
            "n1":{"status":"preferred","uses":4,"score":0.8},
            "ignored":{"status":"neutral","uses":99}
        },
        "dead_ends":[
            {"question":"Where is X?","nodes":["n8","n9",7]},
            {"question":"Empty compass","nodes":[]}
        ]
    });
    let options = ReportOptions {
        root: "fixture",
        min_community_size: 3,
        built_at_commit: Some("αβγδεζηθ-extra"),
        obsidian: true,
        today: Some("2026-07-20"),
        health: compass_output::OrientationHealth {
            publication: Some(compass_output::PublicationStatus::Complete),
            omitted_nodes: Some(0),
            omitted_edges: Some(0),
            identity_collisions: Some(0),
            diagnostic_examples_omitted: Some(0),
            corpus_measurements_available: true,
            ..compass_output::OrientationHealth::default()
        },
    };
    let report = generate_report(
        &graph,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &detection,
        TokenCost {
            input: 12_345,
            output: 6_789,
        },
        Some(&questions),
        Some(&learning),
        &options,
    );
    for expected in [
        "# Agent Orientation",
        "Publication: complete",
        "commit=αβγδεζηθ-extra",
        "files: 12345 · words: 9876543",
        "Evidence label: Runtime/Flow.md",
        "Metric: incident edge count with separate incoming and outgoing evidence",
        "incoming: 0 · outgoing: 1",
        "relations: calls=1",
        "Surprising Connections",
        "semantically＿similar＿to",
        "shared contract",
        "Import Cycles",
        "Hyperedges",
        "Pipeline",
        "Ambiguous Edge Evidence",
        "Work-Memory Observations",
        "code changed; re-verify",
        "known＿dead＿end",
        "Suggested Compass Queries",
        "How does runtime flow?",
        "Structural gap score",
    ] {
        assert!(report.contains(expected), "missing {expected:?}\n{report}");
    }

    let blind_spots = blind_spot_report(&graph, &communities, &labels);
    let typed_report = generate_report_with_blind_spots(
        &graph,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &detection,
        TokenCost::default(),
        Some(&questions),
        Some(&blind_spots),
        Some(&learning),
        &options,
    );
    assert!(typed_report.contains("Structural Blind Spots"));
    assert!(typed_report.contains("compass.graph-insights/1"));

    let warning = DetectionSummary {
        warning: Some("Corpus warning".to_owned()),
        ..DetectionSummary::default()
    };
    let no_signal = [SuggestedQuestion {
        kind: "no_signal".to_owned(),
        question: None,
        why: "No unique signal".to_owned(),
    }];
    let minimal = generate_report(
        &document(json!({"nodes":[],"links":[]}))?,
        &Communities::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &[],
        &warning,
        TokenCost::default(),
        Some(&no_signal),
        Some(&json!({"overlay":{},"dead_ends":[]})),
        &ReportOptions::new("empty"),
    );
    assert!(minimal.contains("Corpus warning"));
    assert!(minimal.contains("Publication: unknown"));
    assert!(minimal.contains("files: unknown · words: unknown"));
    assert!(minimal.contains("Work-Memory Observations"));
    Ok(())
}

#[test]
fn typed_blind_spots_remain_within_orientation_and_report_budgets() -> Result<(), Box<dyn Error>> {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let long_label = "x".repeat(160);
    for component in 0..33 {
        for member in 0..100 {
            let id = format!("component-{component}-member-{member}");
            nodes.push(json!({
                "id": id,
                "label": format!("{long_label}-{component}-{member}"),
                "source_file": format!("src/{component}/member-{member}.rs"),
                "file_type": "code"
            }));
            if member > 0 {
                links.push(json!({
                    "source": format!("component-{component}-member-{}", member - 1),
                    "target": format!("component-{component}-member-{member}"),
                    "relation": "calls",
                    "confidence": "EXTRACTED"
                }));
            }
        }
    }
    let graph = document(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": nodes,
        "links": links
    }))?;
    let communities = Communities::new();
    let labels = BTreeMap::new();
    let blind_spots = blind_spot_report(&graph, &communities, &labels);
    assert_eq!(blind_spots.disconnected_component_count, 33);
    assert_eq!(blind_spots.disconnected_components.len(), 32);

    let options = ReportOptions::new("bounded");
    let model = agent_orientation_with_blind_spots(
        &graph,
        &communities,
        &BTreeMap::new(),
        &labels,
        &[],
        &[],
        &DetectionSummary::default(),
        TokenCost::default(),
        None,
        Some(&blind_spots),
        None,
        &options,
    );
    let orientation = render_orientation_json(&model)?;
    assert!(orientation.len() <= ORIENTATION_JSON_MAX_BYTES);
    let report = render_agent_report_markdown(&model, false)?;
    assert!(report.chars().count() <= REPORT_MARKDOWN_MAX_CHARS);
    Ok(())
}
