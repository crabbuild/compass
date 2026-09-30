use std::error::Error;
use std::fs;

use compass_model::{Graph, GraphDocument};
use compass_query::{
    ExplanationMemberFocus, explanation_member_sources, explanation_member_sources_with_focus,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SOURCE: &str = "struct Owner {}\nfn first() { one(); }\nstruct Inner {}\nfn later() { two(); }\nfn wrong() {}\n";
const FIRST: &str = "fn first() { one(); }";
const LATER: &str = "fn later() { two(); }";

fn node(id: &str, name: &str, kind: &str, text: &str) -> Result<Value, Box<dyn Error>> {
    let start = SOURCE.find(text).ok_or("missing fixture span")?;
    let line = SOURCE[..start].bytes().filter(|b| *b == b'\n').count() + 1;
    Ok(json!({
        "id": id, "name": name, "kind": kind,
        "source": {"file": "lib.rs", "startByte": start, "endByte": start + text.len(), "startLine": line, "endLine": line, "startColumn": 0, "endColumn": text.len()},
        "details": {"type": "symbol", "data": {"sourceDigest": format!("{:x}", Sha256::digest(text.as_bytes()))}}
    }))
}
fn edge(id: &str, source: &str, target: &str, relation: &str) -> Value {
    json!({"id": id, "source": source, "target": target, "relation": relation, "confidence": "EXTRACTED", "custom": id})
}
fn document() -> Result<Value, Box<dyn Error>> {
    Ok(
        json!({"directed": true, "multigraph": true, "graph": {}, "nodes": [
            node("owner", "Owner", "struct", "struct Owner {}")?,
            node("first", "first", "method", FIRST)?,
            node("inner", "Inner", "class", "struct Inner {}")?,
            node("later", "later", "method", LATER)?,
            node("wrong", "wrong", "function", "fn wrong() {}")?
        ], "links": [
            edge("a", "owner", "first", "contains"), edge("b", "owner", "first", "contains"),
            edge("c", "owner", "inner", "contains"), edge("d", "inner", "later", "method"),
            edge("e", "owner", "wrong", "calls"), edge("f", "wrong", "owner", "contains"),
            edge("g", "inner", "owner", "contains")
        ]}),
    )
}
fn graph(value: Value) -> Result<Graph, Box<dyn Error>> {
    Ok(Graph::from_document(serde_json::from_value::<
        GraphDocument,
    >(value)?)?)
}

#[test]
fn focus_reaches_later_members_with_the_same_source_budget() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let focus = ExplanationMemberFocus::new("later later")?;
    for reversed in [false, true] {
        let mut doc = document()?;
        if reversed {
            doc["nodes"].as_array_mut().ok_or("nodes")?.reverse();
            doc["links"].as_array_mut().ok_or("links")?.reverse();
        }
        let graph = graph(doc)?;
        let baseline =
            explanation_member_sources(&graph, "owner", root.path(), LATER.len() as u64)?;
        assert_eq!(baseline.members[0].node.id, "first");
        let report = explanation_member_sources_with_focus(
            &graph,
            "owner",
            root.path(),
            LATER.len() as u64,
            Some(&focus),
        )?;
        assert_eq!(report.focus_terms, ["later"]);
        assert_eq!(report.members.len(), 1);
        assert_eq!(report.members[0].node.id, "later");
        assert_eq!(report.members[0].matched_focus_terms, ["later"]);
        assert_eq!(
            report.members[0]
                .source
                .as_ref()
                .map_err(|e| e.to_string())?
                .source,
            LATER
        );
        assert_eq!(report.source_bytes, LATER.len() as u64);
        assert_eq!(report.verification_bytes_charged, LATER.len() as u64);
        assert_eq!(report.omitted_members, 1);
        assert!(report.truncated);
        assert_eq!(report.membership.len(), baseline.membership.len());
    }
    Ok(())
}

#[test]
fn absent_focus_matches_preserve_order_and_do_not_filter_members() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let graph = graph(document()?)?;
    let focus = ExplanationMemberFocus::new("nothing_matches")?;
    let baseline = explanation_member_sources(&graph, "owner", root.path(), 8000)?;
    for focus in [None, Some(&focus)] {
        let report =
            explanation_member_sources_with_focus(&graph, "owner", root.path(), 8000, focus)?;
        assert_eq!(
            report
                .members
                .iter()
                .map(|m| m.node.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "later"]
        );
        assert!(
            report
                .members
                .iter()
                .all(|m| m.matched_focus_terms.is_empty())
        );
        assert_eq!(report.source_bytes, baseline.source_bytes);
        assert_eq!(
            report.verification_bytes_charged,
            baseline.verification_bytes_charged
        );
        assert_eq!(report.truncated, baseline.truncated);
    }
    Ok(())
}

#[test]
fn focus_counts_distinct_normalized_name_terms_and_keeps_ties_stable() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    for (first_name, later_name, question, expected) in [
        (
            "check",
            "checkLoop",
            "checking loops",
            vec!["later", "first"],
        ),
        (
            "check_loop",
            "checkLoop",
            "checking loops",
            vec!["first", "later"],
        ),
        ("other", "Résumé", "résumé", vec!["later", "first"]),
    ] {
        let mut doc = document()?;
        doc["nodes"][1]["name"] = json!(first_name);
        doc["nodes"][3]["name"] = json!(later_name);
        let graph = graph(doc)?;
        let focus = ExplanationMemberFocus::new(question)?;
        let report = explanation_member_sources_with_focus(
            &graph,
            "owner",
            root.path(),
            8000,
            Some(&focus),
        )?;
        assert_eq!(
            report
                .members
                .iter()
                .map(|m| m.node.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    Ok(())
}

#[test]
fn focus_does_not_read_source_to_rank_or_suppress_stale_failures() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE.replace("two", "bad"))?;
    let graph = graph(document()?)?;
    // A body-only token must not rank a member. The matching name is stale,
    // remains first, and its failed verification still consumes work.
    let body_focus = ExplanationMemberFocus::new("two")?;
    let report = explanation_member_sources_with_focus(
        &graph,
        "owner",
        root.path(),
        8000,
        Some(&body_focus),
    )?;
    assert_eq!(report.members[0].node.id, "first");
    let focus = ExplanationMemberFocus::new("later")?;
    let report =
        explanation_member_sources_with_focus(&graph, "owner", root.path(), 8000, Some(&focus))?;
    assert_eq!(report.members[0].node.id, "later");
    assert!(report.members[0].source.is_err());
    assert!(report.members[1].source.is_ok());
    assert_eq!(report.source_bytes, FIRST.len() as u64);
    assert_eq!(
        report.verification_bytes_charged,
        (FIRST.len() + LATER.len()) as u64
    );
    Ok(())
}

#[test]
fn focus_does_not_resolve_ambiguous_roots() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let mut doc = document()?;
    doc["nodes"].as_array_mut().ok_or("nodes")?.push(node(
        "other",
        "Owner",
        "struct",
        "struct Owner {}",
    )?);
    let graph = graph(doc)?;
    let focus = ExplanationMemberFocus::new("later")?;
    assert!(
        explanation_member_sources_with_focus(&graph, "Owner", root.path(), 8000, Some(&focus))
            .is_err()
    );
    Ok(())
}

#[test]
fn focus_limits_reject_invalid_inputs_without_source_access() -> Result<(), Box<dyn Error>> {
    for text in [
        String::new(),
        "!!!".to_owned(),
        "x".repeat(4097),
        (0..33)
            .map(|i| format!("term{i}"))
            .collect::<Vec<_>>()
            .join(" "),
    ] {
        assert!(ExplanationMemberFocus::new(&text).is_err());
    }
    assert!(ExplanationMemberFocus::new(&"later ".repeat(100)).is_ok());
    Ok(())
}

#[test]
fn members_outside_the_owner_span_keep_nested_direction_and_parallel_evidence()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let mut summaries = Vec::new();
    for reversed in [false, true] {
        let mut doc = document()?;
        if reversed {
            doc["nodes"].as_array_mut().ok_or("nodes")?.reverse();
            doc["links"].as_array_mut().ok_or("links")?.reverse();
        }
        let graph = graph(doc)?;
        let report = explanation_member_sources(&graph, "owner", root.path(), 8000)?;
        assert!(!report.truncated);
        assert_eq!(report.omitted_members, 0);
        assert_eq!(
            report
                .members
                .iter()
                .map(|m| m.node.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "later"]
        );
        assert_eq!(report.source_bytes, (FIRST.len() + LATER.len()) as u64);
        assert_eq!(report.verification_bytes_charged, report.source_bytes);
        let excerpts = report
            .members
            .iter()
            .map(|m| {
                m.source
                    .as_ref()
                    .map(|s| (s.source.clone(), s.digest_verified))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        assert_eq!(
            excerpts,
            [(FIRST.to_owned(), true), (LATER.to_owned(), true)]
        );
        assert_eq!(report.membership.len(), 5);
        assert_eq!(
            report
                .membership
                .iter()
                .filter(|e| e.source == "owner" && e.target == "first")
                .count(),
            2
        );
        summaries.push(
            report
                .membership
                .iter()
                .map(|e| serde_json::to_string(e))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    assert_eq!(summaries[0], summaries[1]);
    Ok(())
}

#[test]
fn source_budget_is_shared_and_omissions_are_explicit() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let graph = graph(document()?)?;
    let exact = explanation_member_sources(&graph, "owner", root.path(), FIRST.len() as u64)?;
    assert_eq!(exact.members.len(), 1);
    assert_eq!(exact.omitted_members, 1);
    assert!(exact.truncated);
    let partial = explanation_member_sources(&graph, "owner", root.path(), FIRST.len() as u64 + 3)?;
    assert_eq!(partial.members.len(), 2);
    assert_eq!(partial.omitted_members, 0);
    assert!(partial.truncated);
    assert_eq!(partial.source_bytes, FIRST.len() as u64 + 3);
    assert_eq!(
        partial.members[1]
            .source
            .as_ref()
            .map_err(|e| e.to_string())?
            .source,
        "fn "
    );
    assert!(
        partial.members[1]
            .source
            .as_ref()
            .map_err(|e| e.to_string())?
            .digest_verified
    );
    assert_eq!(
        partial.verification_bytes_charged,
        (FIRST.len() + LATER.len()) as u64
    );
    Ok(())
}

#[test]
fn stale_and_unverified_members_keep_their_individual_status() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE.replace("one", "uno"))?;
    let mut doc = document()?;
    doc["nodes"][3]["details"]["data"]
        .as_object_mut()
        .ok_or("details")?
        .remove("sourceDigest");
    let graph = graph(doc)?;
    let report = explanation_member_sources(&graph, "owner", root.path(), 8000)?;
    assert!(report.members[0].source.is_err());
    let second = report.members[1]
        .source
        .as_ref()
        .map_err(|e| e.to_string())?;
    assert_eq!(second.source, LATER);
    assert!(!second.digest_verified);
    assert_eq!(report.source_bytes, LATER.len() as u64);
    assert_eq!(
        report.verification_bytes_charged,
        (FIRST.len() + LATER.len()) as u64
    );
    Ok(())
}

#[test]
fn ambiguous_roots_and_undirected_graphs_are_not_guessed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let mut doc = document()?;
    doc["nodes"].as_array_mut().ok_or("nodes")?.push(node(
        "other",
        "Owner",
        "struct",
        "struct Owner {}",
    )?);
    let ambiguous = graph(doc)?;
    assert!(explanation_member_sources(&ambiguous, "Owner", root.path(), 8000).is_err());
    let mut doc = document()?;
    doc["directed"] = json!(false);
    assert!(explanation_member_sources(&graph(doc)?, "owner", root.path(), 8000).is_err());
    Ok(())
}

#[test]
fn ignored_relations_still_consume_the_discovery_work_budget() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let mut doc = document()?;
    doc["links"] = Value::Array(
        (0..10001)
            .map(|i| edge(&format!("call-{i}"), "owner", "wrong", "calls"))
            .collect(),
    );
    let graph = graph(doc)?;
    let error = explanation_member_sources(&graph, "owner", root.path(), 8000)
        .err()
        .ok_or("work limit not enforced")?;
    assert!(error.to_string().contains("10000-adjacency"));
    Ok(())
}

#[test]
fn heuristic_and_deferred_members_are_not_promoted_to_recorded_source_members()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let mut doc = document()?;
    let mut inferred = edge("inferred", "owner", "wrong", "contains");
    inferred["confidence"] = json!("INFERRED");
    let mut deferred = edge("deferred", "owner", "wrong", "contains");
    deferred["deferred"] = json!(true);
    doc["links"]
        .as_array_mut()
        .ok_or("links")?
        .extend([inferred, deferred]);
    let graph = graph(doc)?;
    let report = explanation_member_sources(&graph, "owner", root.path(), 8000)?;
    assert_eq!(report.members.len(), 2);
    assert!(report.members.iter().all(|m| m.node.id != "wrong"));
    Ok(())
}

#[test]
fn metadata_and_parameter_limits_fail_before_source_reads() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let base = graph(document()?)?;
    for budget in [0, 1_048_577] {
        assert!(explanation_member_sources(&base, "owner", root.path(), budget).is_err());
    }
    let mut doc = document()?;
    doc["nodes"][0]["large"] = json!("x".repeat(1_048_577));
    assert!(explanation_member_sources(&graph(doc)?, "owner", root.path(), 8000).is_err());
    Ok(())
}

#[test]
fn complete_span_work_is_bounded_even_for_failed_source_reads() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    let mut doc = document()?;
    doc["nodes"][1]["source"]["endByte"] = json!(10_000_000);
    doc["nodes"][3]["source"]["endByte"] = json!(10_000_000);
    let graph = graph(doc)?;
    let report = explanation_member_sources(&graph, "owner", root.path(), 8000)?;
    assert_eq!(report.members.len(), 1);
    assert!(report.members[0].source.is_err());
    assert_eq!(report.omitted_members, 1);
    assert!(report.truncated);
    assert_eq!(report.source_bytes, 0);
    assert!(report.verification_bytes_charged <= 16_777_216);
    Ok(())
}

#[test]
fn candidate_and_nesting_caps_are_explicit_errors() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let mut wide = document()?;
    wide["links"] = json!([]);
    for i in 0..129 {
        let id = format!("member-{i}");
        wide["nodes"]
            .as_array_mut()
            .ok_or("nodes")?
            .push(node(&id, &id, "method", FIRST)?);
        wide["links"]
            .as_array_mut()
            .ok_or("links")?
            .push(edge(&id, "owner", &id, "contains"));
    }
    let wide = graph(wide)?;
    assert!(
        explanation_member_sources(&wide, "owner", root.path(), 8000)
            .err()
            .ok_or("missing cap")?
            .to_string()
            .contains("128-callable")
    );
    let mut deep = document()?;
    deep["links"] = json!([]);
    let mut parent = "owner".to_owned();
    for i in 0..5 {
        let id = format!("nested-{i}");
        deep["nodes"].as_array_mut().ok_or("nodes")?.push(node(
            &id,
            &id,
            "class",
            "struct Inner {}",
        )?);
        deep["links"]
            .as_array_mut()
            .ok_or("links")?
            .push(edge(&id, &parent, &id, "contains"));
        parent = id;
    }
    let deep = graph(deep)?;
    assert!(
        explanation_member_sources(&deep, "owner", root.path(), 8000)
            .err()
            .ok_or("missing cap")?
            .to_string()
            .contains("depth limit")
    );
    Ok(())
}
