use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_resolve::resolve;

fn extract(file: &str, source: &str) -> Result<Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(file), source.as_bytes())?)
}

#[test]
fn cross_file_fields_publish_exact_caller_direction_occurrences_and_stable_order()
-> Result<(), Box<dyn Error>> {
    let model = "package model; public class Cell { public int value; }\n";
    let caller = "package app;\nimport model.Cell;\nclass Box {\n void run(Cell cell) {\n  cell.value = cell.value + 1;\n }\n}\n";
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("model"))?;
    std::fs::create_dir_all(directory.path().join("app"))?;
    std::fs::write(directory.path().join("model/Cell.java"), model)?;
    std::fs::write(directory.path().join("app/Box.java"), caller)?;
    let sources = HashMap::from([
        ("model/Cell.java".into(), model.into()),
        ("app/Box.java".into(), caller.into()),
    ]);
    let inputs = vec![
        extract("model/Cell.java", model)?,
        extract("app/Box.java", caller)?,
    ];
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let build =
        BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:java-field-access")?;
    let graph = normalize_v1(resolved, build)?;
    let run = graph
        .nodes
        .iter()
        .find(|n| n.qualified_name == "app.Box::run")
        .ok_or("missing run")?;
    let field = graph
        .nodes
        .iter()
        .find(|n| n.name == "value" && n.kind == NodeKind::Field)
        .ok_or("missing field")?;
    let edges = graph
        .links
        .iter()
        .filter(|e| e.source == run.id && e.target == field.id)
        .collect::<Vec<_>>();
    assert_eq!(edges.len(), 2, "{:#?}", graph.links);
    let mut starts = BTreeSet::new();
    for edge in edges {
        assert_eq!(edge.kind, EdgeKind::References);
        assert!(
            edge.occurrence_rule
                .as_ref()
                .is_some_and(|rule| rule.as_str().starts_with("universal-member-access-"))
        );
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing occurrence")?;
        assert_eq!(site.file, "app/Box.java");
        assert_eq!(site.start_line, 5);
        let start = usize::try_from(site.start_byte)?;
        let end = usize::try_from(site.end_byte)?;
        assert_eq!(caller.get(start..end), Some("value"));
        assert!(starts.insert(start));
        assert!(edge.evidence.iter().any(|e| e.anchors.contains(site)));
    }
    let mut reversed = inputs;
    reversed.reverse();
    let second = resolve(&reversed, &sources);
    let build =
        BuildEvidence::from_extraction(directory.path(), &second, "sha256:java-field-access")?;
    let second = normalize_v1(second, build)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&second.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&second.links)?
    );
    Ok(())
}

#[test]
fn ambiguous_unknown_inherited_and_unregistered_owners_never_publish_false_field_edges()
-> Result<(), Box<dyn Error>> {
    for source in [
        "package p; class Cell { int value; } class Box { void run() { class Local { int value = new Cell().value; } } }",
        "package p; class Cell { int value; int value; } class Box { int run(Cell item) { return item.value; } }",
        "package p; class Cell { int value; } class Box { int run(Unknown item) { return item.value; } }",
        "package p; class Cell { int value; } class Box { void run() { class Local { int get(Cell item) { return item.value; } } } }",
        "package p; class Box { int value; class Inner extends Unknown { int get() { return value; } } }",
        "package p; import a.*; class Cell { int value; } class Box { int run(Unknown item) { return item.value; } }",
        "package p; class Cell { int value; } class Box { Cell item; int run(Object obj) { if (!(obj instanceof Cell item)) { log(); return 0; } return item.value; } }",
    ] {
        let resolved = resolve(
            &[extract("Box.java", source)?],
            &HashMap::from([("Box.java".into(), source.into())]),
        );
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        assert!(
            !resolved
                .edges
                .iter()
                .any(|edge| edge.string("relation") == "accesses"),
            "{source}: {:#?}",
            resolved.edges
        );
    }
    Ok(())
}

#[test]
fn state_access_is_separate_from_method_selector_and_read_write_classification()
-> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; int get() { return value; } } class Box { Cell item; void run() { item.get(); } }";
    let resolved = resolve(
        &[extract("Box.java", source)?],
        &HashMap::from([("Box.java".into(), source.into())]),
    );
    let fields = resolved
        .nodes
        .iter()
        .filter(|n| n.string("symbol_kind") == "field")
        .map(|n| n.id.as_str())
        .collect::<BTreeSet<_>>();
    let accesses = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "accesses")
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 2);
    assert!(
        accesses
            .iter()
            .all(|edge| fields.contains(edge.target.as_str()))
    );
    assert!(
        resolved
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "calls")
    );
    Ok(())
}

#[test]
fn member_type_precedes_imported_type_across_files() -> Result<(), Box<dyn Error>> {
    let sources = HashMap::from([
        ("remote/Cell.java".into(), "package remote; public class Cell { public int value; }".into()),
        ("p/Box.java".into(), "package p; import remote.Cell; class Box { class Cell { int value; } int run(Cell item) { return item.value; } }".into()),
    ]);
    let inputs = sources
        .iter()
        .map(|(file, source): (&String, &String)| extract(file, source))
        .collect::<Result<Vec<_>, _>>()?;
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let targets = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "accesses")
        .map(|edge| {
            resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map(|node| node.string("qualified_name"))
        })
        .collect::<Vec<_>>();
    assert_eq!(targets, vec![Some("p.Box::Cell::value".to_owned())]);
    Ok(())
}

#[test]
fn ambiguous_receiver_types_cannot_be_selected_by_field_availability() -> Result<(), Box<dyn Error>>
{
    let sources = HashMap::from([
        (
            "one/Cell.java".into(),
            "package p; class Cell { int value; }".into(),
        ),
        (
            "two/Cell.java".into(),
            "package p; class Cell { int other; }".into(),
        ),
        (
            "p/Box.java".into(),
            "package p; class Box { int run(Cell item) { return item.value; } }".into(),
        ),
    ]);
    let inputs = sources
        .iter()
        .map(|(file, source): (&String, &String)| extract(file, source))
        .collect::<Result<Vec<_>, _>>()?;
    for inputs in [inputs.clone(), inputs.into_iter().rev().collect()] {
        let resolved = resolve(&inputs, &sources);
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        assert!(
            !resolved
                .edges
                .iter()
                .any(|edge| edge.string("relation") == "accesses"),
            "{:#?}",
            resolved.edges
        );
    }
    Ok(())
}

#[test]
fn field_receiver_decisions_preserve_ambiguity_limits_and_exact_evidence()
-> Result<(), Box<dyn Error>> {
    use compass_languages::{
        CandidateRelation, EvidenceBuilder, EvidenceLimits, EvidenceRange, ResolutionConstraint,
        SemanticRole, UniversalEvidenceRegistry,
    };
    use compass_resolve::evidence::{
        ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
    };
    let pipeline = UniversalEvidenceRegistry::pipeline("java").ok_or("Java pipeline")?;
    for (receivers, budget, exact) in [
        (0, 256, false),
        (1, 256, false),
        (2, 256, false),
        (2, 1, false),
        (2, 256, true),
    ] {
        let range = |start: u32| EvidenceRange {
            source_file: "fixture.java".into(),
            start_byte: u64::from(start),
            end_byte: u64::from(start + 1),
            start_line: 1,
            end_line: 1,
            start_column: start,
            end_column: start + 1,
        };
        let mut builder = EvidenceBuilder::new(
            pipeline,
            "field-receiver-test",
            "fixture.java",
            EvidenceLimits::default(),
        );
        let caller = builder.declare(
            "method",
            "caller",
            "run",
            "p.Box::run",
            Some("p"),
            None,
            range(0),
        )?;
        let field = builder.declare(
            "field",
            "field",
            "value",
            "p.Cell::value",
            Some("p"),
            None,
            range(2),
        )?;
        for i in 0..receivers {
            builder.declare(
                "class",
                &format!("class-{i}"),
                "Cell",
                "p.Cell",
                Some("p"),
                None,
                range(4 + i),
            )?;
        }
        let occurrence = builder.occur(
            SemanticRole::MemberAccess,
            &caller,
            "value",
            Some("item"),
            None,
            range(10),
        )?;
        let candidate = builder.relate(
            CandidateRelation::AccessesMember,
            &caller,
            Some(&occurrence),
            None,
            "value",
            ResolutionConstraint {
                qualified_name: Some("p.Cell::value".into()),
                allowed_target_kinds: vec!["field".into()],
                exact_target_declaration_id: exact.then(|| field.clone()),
                ..ResolutionConstraint::default()
            },
        )?;
        let index = UniversalResolutionIndex::new(
            &[builder.finish()?],
            UniversalResolutionLimits {
                candidates_per_lookup: budget,
                ..UniversalResolutionLimits::default()
            },
        )?;
        let decision = index.resolve(&candidate);
        if exact || receivers == 1 {
            assert!(
                matches!(decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == field)
            );
        } else if receivers == 0 {
            assert_eq!(decision, ResolutionDecision::Unresolved);
        } else {
            assert_eq!(
                decision,
                ResolutionDecision::Ambiguous { candidate_count: 2 }
            );
        }
    }
    Ok(())
}
