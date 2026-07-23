use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::thread;
use std::time::Duration;

use compass_reflect::{
    Aggregate, ContestedSource, Counts, MemoryDoc, ProvenanceEvent, ReflectOptions, SourceScore,
    aggregate_lessons, build_learning_overlay, lessons_fresh, load_memory_docs, parse_memory_doc,
    reflect, render_lessons_markdown, write_learning_sidecar,
};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

fn memory(outcome: &str, date: &str, question: &str, nodes: &[&str]) -> MemoryDoc {
    MemoryDoc {
        query_type: "explain".to_owned(),
        date: date.to_owned(),
        question: question.to_owned(),
        outcome: outcome.to_owned(),
        correction: if outcome == "corrected" {
            "Use the indexed path instead".to_owned()
        } else {
            String::new()
        },
        contributor: "fixture".to_owned(),
        source_nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
        path: String::new(),
    }
}

#[test]
fn aggregation_renders_all_lesson_classes_dates_communities_and_provenance()
-> Result<(), Box<dyn Error>> {
    let now = OffsetDateTime::parse("2026-07-20T12:00:00Z", &Rfc3339)?;
    let docs = vec![
        memory("useful", "2026-07-20T10:00:00Z", "A works", &["A", "A"]),
        memory("useful", "2026-07-20", "A works again", &["A"]),
        memory("useful", "2026-07-19T12:00:00", "C might work", &["C"]),
        memory("useful", "invalid", "D worked", &["D"]),
        memory("dead_end", "2026-07-20T11:00:00Z", "D failed", &["D"]),
        memory("useful", "", "E once worked", &["E"]),
        memory(
            "corrected",
            "2026-07-20T11:30:00Z",
            "E was corrected",
            &["E"],
        ),
        memory("dead_end", "2026-07-18", "No source", &[]),
        memory("unmarked", "2026-07-20", "Not rated", &["A"]),
        memory("useful", "2026-07-20", "Unknown node", &["filtered"]),
    ];
    let communities = HashMap::from([
        ("A".to_owned(), "Core".to_owned()),
        ("D".to_owned(), "Core".to_owned()),
        ("E".to_owned(), "Adapters".to_owned()),
    ]);
    let known = ["A", "C", "D", "E"]
        .into_iter()
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let aggregate = aggregate_lessons(&docs, Some(&communities), Some(&known), now, 30.0, 2);
    assert_eq!(aggregate.total, docs.len());
    assert_eq!(
        aggregate.counts,
        Counts {
            useful: 6,
            dead_end: 2,
            corrected: 1,
            unmarked: 1
        }
    );
    assert!(aggregate.preferred.iter().any(|source| source.node == "A"));
    assert!(aggregate.tentative.iter().any(|source| source.node == "C"));
    assert!(aggregate.contested.iter().any(|source| source.node == "D"));
    assert!(aggregate.contested.iter().any(|source| source.node == "E"));
    assert!(aggregate.by_community.contains_key("Core"));
    assert!(aggregate.by_community.contains_key("Uncategorized"));

    let markdown = render_lessons_markdown(&aggregate);
    for section in [
        "Preferred sources",
        "Tentative",
        "Contested",
        "Known dead ends",
        "Corrections",
        "By topic",
    ] {
        assert!(markdown.contains(section), "missing {section}");
    }
    assert!(render_lessons_markdown(&Aggregate::default()).contains("No marked outcomes"));
    Ok(())
}

#[test]
fn reflection_loads_memory_graph_context_writes_overlay_and_tracks_freshness()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let memory_dir = root.join("graphify-out/memory");
    let output_dir = root.join("graphify-out");
    fs::create_dir_all(&memory_dir)?;
    fs::write(root.join("source.rs"), "pub fn alpha() {}\n")?;
    let graph = output_dir.join("graph.json");
    fs::write(
        &graph,
        r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"node_a","label":"Alpha","source_file":"source.rs"},{"id":"node_b","label":"Duplicate"},{"id":"node_c","label":"Duplicate"}],"links":[]}"#,
    )?;
    fs::write(
        output_dir.join(".graphify_root"),
        root.to_string_lossy().as_bytes(),
    )?;
    let analysis = output_dir.join(".graphify_analysis.json");
    fs::write(
        &analysis,
        r#"{"communities":{"0":["node_a"],"1":["node_b"],"2":[]}}"#,
    )?;
    let labels = output_dir.join(".graphify_labels.json");
    fs::write(
        &labels,
        r#"{"0":"Core","1":true,"2":7,"3":{"name":"ignored"}}"#,
    )?;

    let first = memory_dir.join("01.md");
    fs::write(
        &first,
        "---\ntype: \"explain\"\ndate: \"2026-07-20\"\nquestion: \"Where is Alpha?\"\noutcome: \"useful\"\ncorrection: \"\"\ncontributor: \"fixture\"\nsource_nodes: [\"node_a\", \"Alpha\", \"missing\"]\n---\n",
    )?;
    fs::write(memory_dir.join("ignored.txt"), "ignored")?;
    fs::write(memory_dir.join("invalid.md"), "not frontmatter")?;
    assert!(parse_memory_doc("plain text").is_none());
    assert_eq!(load_memory_docs(&memory_dir).len(), 1);

    let lessons = output_dir.join("LESSONS.md");
    assert!(!lessons_fresh(
        &lessons,
        &memory_dir,
        Some(&graph),
        Some(&analysis),
        Some(&labels)
    ));
    let now = OffsetDateTime::parse("2026-07-20T12:34:56.123456Z", &Rfc3339)?
        .to_offset(UtcOffset::from_hms(-7, 0, 0)?);
    let result = reflect(&ReflectOptions {
        memory_dir: memory_dir.clone(),
        output: lessons.clone(),
        graph: Some(graph.clone()),
        analysis: Some(analysis.clone()),
        labels: Some(labels.clone()),
        now,
        half_life_days: 0.0,
        min_corroboration: 1,
    })?;
    assert_eq!(result.aggregate.total, 1);
    assert!(lessons.is_file());
    assert!(output_dir.join(".compass_learning.json").is_file());
    assert!(lessons_fresh(
        &lessons,
        &memory_dir,
        Some(&graph),
        Some(&analysis),
        Some(&labels)
    ));

    let aggregate = Aggregate {
        total: 2,
        min_corroboration: 1,
        preferred: vec![
            SourceScore {
                node: "node_a".to_owned(),
                n: 2,
                score: 1.5,
            },
            SourceScore {
                node: "Duplicate".to_owned(),
                n: 1,
                score: 0.5,
            },
            SourceScore {
                node: "missing".to_owned(),
                n: 1,
                score: 0.2,
            },
        ],
        contested: vec![ContestedSource {
            node: "Alpha".to_owned(),
            pos: 1,
            neg: 1,
            score: 0.0,
            verdict: "even".to_owned(),
            last: "2026-07-20".to_owned(),
        }],
        provenance: HashMap::from([(
            "node_a".to_owned(),
            (0..7)
                .map(|index| ProvenanceEvent {
                    date: format!("2026-07-{:02}", index + 1),
                    question: format!("question {index}"),
                    outcome: "useful".to_owned(),
                })
                .collect(),
        )]),
        ..Aggregate::default()
    };
    let overlay = build_learning_overlay(&aggregate, &graph, now);
    assert_eq!(overlay["nodes"]["node_a"]["status"], "preferred");
    assert_eq!(
        overlay["nodes"]["node_a"]["provenance"]
            .as_array()
            .map(Vec::len),
        Some(5)
    );
    assert!(overlay["nodes"].get("node_b").is_none());
    assert!(
        !overlay["nodes"]["node_a"]["code_fingerprint"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    let sidecar = write_learning_sidecar(&aggregate, &graph, now)?;
    assert!(sidecar.is_file());

    thread::sleep(Duration::from_millis(10));
    fs::write(memory_dir.join("02.md"), fs::read(&first)?)?;
    assert!(!lessons_fresh(
        &lessons,
        &memory_dir,
        Some(&graph),
        Some(&analysis),
        Some(&labels)
    ));
    Ok(())
}
