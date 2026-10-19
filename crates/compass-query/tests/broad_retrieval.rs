mod support;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{GraphDocument, NodeDetails, NodeKind};
use compass_model::query_contract::{
    CodeQueryLimits, DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest,
    DiscoveryTraversal, QueryDiagnosticCode, SearchRequest,
};
use compass_query::{open, open_with_store};
use compass_store::SqliteStore;

fn request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}

fn fixture(path: &std::path::Path) -> Result<GraphDocument, Box<dyn std::error::Error>> {
    support::write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    graph.nodes.clear();
    graph.links.clear();
    for at in 0..6 {
        let mut node = support::node(
            &format!("z:formatter:{at}"),
            NodeKind::Function,
            &format!("format_message_{at}"),
            &format!("protocol::format_message_{at}"),
        );
        node.details = Some(serde_json::from_value::<NodeDetails>(
            serde_json::json!({"type":"symbol","data":{"signature":"fn(errors: ProtocolDiagnostics) -> String"}}),
        )?);
        graph.nodes.push(node);
    }
    // Common-word parameter leaves must not displace implementation evidence.
    for at in 0..600 {
        graph.nodes.push(support::node(
            &format!("a:noise:{at:04}"),
            NodeKind::Parameter,
            "errors",
            "unrelated::errors",
        ));
    }
    Ok(graph)
}

#[test]
fn graph_and_alias_field_clipping_still_veto_compound_preferences()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_model::search::{search_document, search_document_with_aliases};
    for mode in ["complete", "signature_clipped", "aliases_clipped"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let template = graph
            .links
            .first()
            .cloned()
            .ok_or("missing edge template")?;
        graph.nodes.clear();
        graph.links.clear();
        let mut target = support::node(
            "target",
            NodeKind::Function,
            "read_only_fields",
            "fixture::read_only_fields",
        );
        let signature = if mode == "signature_clipped" {
            format!("fn(fields: {})", "Padding ".repeat(1200))
        } else {
            "fn(fields: Fields)".to_owned()
        };
        target.details = Some(serde_json::from_value(serde_json::json!({
            "type":"symbol", "data":{"signature":signature}
        }))?);
        let mut aliases = Vec::new();
        if mode == "aliases_clipped" {
            for index in 0..9 {
                let id = format!("alias:{index}");
                let name = format!("unrelated_alias_{index}");
                aliases.push(name.clone());
                let alias =
                    support::node(&id, NodeKind::TypeAlias, &name, &format!("fixture::{name}"));
                let mut edge = template.clone();
                edge.source = id;
                edge.target = target.id.clone();
                edge.kind = EdgeKind::Aliases;
                edge.occurrence_rule = None;
                edge.context = None;
                edge.details = None;
                edge.id = compass_model::identity::edge_id(
                    &edge.source,
                    edge.kind,
                    &edge.target,
                    edge.relationship_site.as_ref(),
                    None,
                );
                edge.key = edge.id.clone();
                graph.nodes.push(alias);
                graph.links.push(edge);
            }
        }
        let document = if mode == "aliases_clipped" {
            search_document_with_aliases(&target, aliases.iter().map(String::as_str))
        } else {
            search_document(&target)
        };
        assert_eq!(
            document.truncated,
            mode != "complete",
            "{mode}: fixture clipping"
        );
        graph.nodes.push(target);
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        let question = request("Where are read-only fields prepared?");
        let answer = json.discover_ranked(question.clone())?;
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(question)?.candidates,
            "{mode}"
        );
        let candidate = answer
            .candidates
            .iter()
            .find(|c| c.node.id == "target")
            .ok_or("missing compound owner")?;
        assert_eq!(
            candidate
                .reasons
                .iter()
                .any(|r| r.contains("Compound identifiers")),
            mode == "complete",
            "{mode}: {candidate:?}"
        );
    }
    Ok(())
}

#[test]
fn operation_hints_preserve_relationship_and_identifier_dependency_bands()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_query::RelevanceBand;
    for kind in [EdgeKind::References, EdgeKind::Contains] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let template = graph.links.first().cloned().ok_or("missing edge fixture")?;
        graph.nodes.clear();
        graph.links.clear();
        for name in ["get", "ordinary"] {
            let mut owner = support::node(
                name,
                NodeKind::Function,
                name,
                &format!("session.values.{name}"),
            );
            owner.details = Some(serde_json::from_value(serde_json::json!({
                "type":"symbol", "data":{"signature":"fn(session: Session, values: Values)"}
            }))?);
            let child_id = format!("child:{name}");
            let child = support::node(&child_id, NodeKind::Variable, "read", "fixture.read");
            let mut edge = template.clone();
            edge.source = name.to_owned();
            edge.target = child_id;
            edge.kind = kind;
            edge.relationship_site = child.source.clone();
            for proof in &mut edge.evidence {
                proof.anchors = vec![child.source.clone().ok_or("missing child source")?];
            }
            edge.id = compass_model::identity::edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key = edge.id.clone();
            graph.nodes.extend([owner, child]);
            graph.links.push(edge);
        }
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        let question = request("Where are session values read?");
        let answer = json.discover_ranked(question.clone())?;
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(question)?.candidates
        );
        let mut fields = None;
        for name in ["get", "ordinary"] {
            let candidate = answer
                .candidates
                .iter()
                .find(|row| row.node.id == name)
                .ok_or_else(|| format!("missing {kind:?} {name}: {answer:?}"))?;
            assert_eq!(
                candidate.relevance,
                RelevanceBand::Low,
                "{kind:?}: {candidate:?}"
            );
            let required = if kind == EdgeKind::Contains {
                "owned_identifier"
            } else {
                "code_reference"
            };
            assert!(
                candidate
                    .matched_fields
                    .iter()
                    .any(|field| field == required)
            );
            assert!(!candidate.matched_fields.iter().any(|field| field == "name"));
            if let Some(fields) = &fields {
                assert_eq!(&candidate.matched_fields, fields);
            } else {
                fields = Some(candidate.matched_fields.clone());
            }
        }
    }
    Ok(())
}

#[test]
fn operation_words_recall_named_owners_with_separate_literal_context()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.clear();
    graph.links.clear();
    for (id, name, signature) in [
        (
            "getter-target",
            "get",
            "fn(session: Session, values: Values)",
        ),
        (
            "setter-target",
            "set",
            "fn(session: Session, values: Values)",
        ),
        ("namespace-only", "get", "fn(widget: Widget)"),
        ("namespace-only-set", "set", "fn(widget: Widget)"),
        (
            "weak-namespace",
            "get",
            "fn(session: Session, values: Values)",
        ),
        (
            "weak-signature",
            "get",
            "fn(session: Session, values: Values, read: Reader)",
        ),
        (
            "literal-read",
            "read_session_values",
            "fn(session: Session)",
        ),
    ] {
        let qualified = if id == "weak-namespace" {
            "read::get".to_owned()
        } else if id.starts_with("namespace-only") {
            format!("session::values::{name}")
        } else {
            format!("fixture::{name}")
        };
        let mut node = support::node(id, NodeKind::Function, name, &qualified);
        node.details = Some(serde_json::from_value(serde_json::json!({
            "type":"symbol", "data":{"signature":signature}
        }))?);
        graph.nodes.push(node);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for (word, target, translation) in [
        ("read", "getter-target", "read -> get"),
        ("lookup", "getter-target", "lookup -> get"),
        ("getter", "getter-target", "getter -> get"),
        ("setter", "setter-target", "setter -> set"),
    ] {
        let question = request(&format!("Where is session values {word} implementation?"));
        let answer = json.discover_ranked(question.clone())?;
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(question)?.candidates
        );
        let target = answer
            .candidates
            .iter()
            .find(|row| row.node.id == target)
            .ok_or_else(|| format!("missing {word} target: {answer:?}"))?;
        assert_eq!(target.relevance, RelevanceBand::Low);
        assert!(
            target
                .reasons
                .iter()
                .any(|reason| reason.contains(translation)),
            "{target:?}"
        );
        assert!(
            target
                .reasons
                .iter()
                .all(|reason| !reason.contains("inspection preference"))
        );
        let negative = if word == "setter" {
            "namespace-only-set"
        } else {
            "namespace-only"
        };
        assert!(
            !answer.candidates.iter().any(|row| row.node.id == negative),
            "{word}: namespace-only hint admitted: {answer:?}"
        );
    }
    let read = json.discover_ranked(request("Where are session values read?"))?;
    assert_eq!(read.candidates[0].node.id, "literal-read");
    assert!(
        read.candidates[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("inspection preference"))
    );
    for id in ["weak-namespace", "weak-signature"] {
        let weak = read
            .candidates
            .iter()
            .find(|row| row.node.id == id)
            .ok_or("missing weak literal candidate")?;
        assert!(
            weak.reasons
                .iter()
                .all(|reason| !reason.contains("inspection preference")),
            "mixed literal/hint promotion: {weak:?}"
        );
        assert!(
            !weak.matched_fields.iter().any(|field| field == "name"),
            "hint must not masquerade as a literal name match: {weak:?}"
        );
    }

    assert_eq!(
        json.discover_ranked(request("getter-target"))?.candidates[0]
            .node
            .id,
        "getter-target"
    );
    assert!(
        json.discover_ranked(request("Where are MissingSession values read?"))?
            .candidates
            .is_empty()
    );
    for question in [
        "Where are `session` values read?",
        "Where is session values read documentation?",
    ] {
        let answer = json.discover_ranked(request(question))?;
        assert!(answer.candidates.iter().all(|row| {
            row.reasons
                .iter()
                .all(|reason| !reason.contains("read -> get"))
        }));
    }
    graph.nodes.reverse();
    let reordered = directory.path().join("reordered.json");
    std::fs::write(&reordered, serde_json::to_vec(&graph)?)?;
    let reversed = open(&reordered, None, &directory.path().join("reordered-cache"))?;
    assert_eq!(
        read.candidates,
        reversed
            .discover_ranked(request("Where are session values read?"))?
            .candidates
    );
    Ok(())
}

#[test]
fn code_abbreviations_recall_with_indexed_long_forms_and_literal_context()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.clear();
    graph.links.clear();
    for (id, name, signature) in [
        (
            "lookup",
            "ByName",
            "fn(params: Params, name: String) -> String",
        ),
        (
            "clock",
            "add_current",
            "fn(sec: u64, nsec: u64, duration: Duration)",
        ),
        // Long forms elsewhere in the index must not suppress abbreviated
        // implementations. They also make the pre-fix coverage veto visible.
        ("literal-route", "retrieve_route_parameter", "fn()"),
        ("literal-clock", "accumulate_nanoseconds_seconds", "fn()"),
        ("namespace-only", "inspect_params", "fn(widget: Widget)"),
    ] {
        let qualified = if id == "namespace-only" {
            "name::inspect_params".to_owned()
        } else {
            format!("fixture::{name}")
        };
        let mut node = support::node(id, NodeKind::Function, name, &qualified);
        node.details = Some(serde_json::from_value(serde_json::json!({
            "type":"symbol", "data":{"signature":signature}
        }))?);
        graph.nodes.push(node);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for (question, target, translations) in [
        (
            "Where is a route parameter retrieved by its name?",
            "lookup",
            vec!["parameter -> param"],
        ),
        (
            "Where are accumulated nanoseconds carried into duration seconds?",
            "clock",
            vec!["nanosecond -> nsec", "second -> sec"],
        ),
    ] {
        let answer = json.discover_ranked(request(question))?;
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(request(question))?.candidates
        );
        let candidate = answer
            .candidates
            .iter()
            .find(|c| c.node.id == target)
            .ok_or_else(|| format!("abbreviated implementation missing: {question}: {answer:?}"))?;
        assert_eq!(candidate.relevance, RelevanceBand::Low);
        for translation in translations {
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|reason| reason.contains(translation)),
                "{candidate:?}"
            );
        }
        assert!(
            !answer
                .candidates
                .iter()
                .any(|c| c.node.id == "namespace-only")
        );
    }
    for question in [
        "Where is a Rocket parameter retrieved by its name?",
        "Where is a `parameter` retrieved by its name?",
    ] {
        assert!(
            !json
                .discover_ranked(request(question))?
                .candidates
                .iter()
                .any(|c| c.node.id == "lookup"),
            "{question}"
        );
    }
    // Qualified questions can retain pre-existing partial literal candidates;
    // they must not acquire an abbreviated match for the named component.
    for candidate in json
        .discover_ranked(request("Where is fixture.parameter retrieved by its name?"))?
        .candidates
    {
        if candidate.node.id == "lookup" {
            assert!(
                !candidate
                    .matched_terms
                    .iter()
                    .any(|term| term == "parameter")
            );
            assert!(
                !candidate
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("parameter -> param"))
            );
        }
    }
    Ok(())
}

#[test]
fn unindexed_collection_words_require_literal_context_and_stay_tentative()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.clear();
    graph.links.clear();
    for (id, name, qualified, signature) in [
        (
            "target",
            "copy_items",
            "fixture::copy_items",
            "fn(cache: Cache) -> Map",
        ),
        (
            "namespace-only",
            "clone_items",
            "cache::clone_items",
            "fn(widget: Widget)",
        ),
    ] {
        let mut node = support::node(id, NodeKind::Function, name, qualified);
        node.details = Some(serde_json::from_value(serde_json::json!({
            "type":"symbol", "data":{"signature":signature}
        }))?);
        graph.nodes.push(node);
    }
    for index in 0..7 {
        let mut node = support::node(
            &format!("noise:{index}"),
            NodeKind::Function,
            "new_cache",
            "fixture::new_cache",
        );
        node.details = Some(serde_json::from_value(serde_json::json!({
            "type":"symbol", "data":{"signature":"fn(cache: Cache, only: Mode, containing: Flags)"}
        }))?);
        graph.nodes.push(node);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let question = request("Where is a snapshot made containing only live cache entries?");
    let answer = json.discover_ranked(question.clone())?;
    assert_eq!(
        answer.candidates,
        stored.discover_ranked(question)?.candidates
    );
    let first = answer
        .candidates
        .first()
        .ok_or("missing approximate candidate")?;
    assert_eq!(first.node.id, "target", "{answer:?}");
    assert_eq!(first.relevance, RelevanceBand::Low);
    assert!(first.matched_terms.iter().any(|term| term == "entry"));
    assert!(first.matched_terms.iter().any(|term| term == "snapshot"));
    assert!(
        first
            .reasons
            .iter()
            .any(|reason| reason.contains("Vocabulary fallback"))
    );
    assert!(
        !answer
            .candidates
            .iter()
            .any(|candidate| candidate.node.id == "namespace-only")
    );
    for unsupported in [
        "Where are rocket snapshots made?",
        "Where are `entries` copied?",
        "Where is cache.snapshot made?",
    ] {
        assert!(
            json.discover_ranked(request(unsupported))?
                .candidates
                .is_empty(),
            "{unsupported}"
        );
    }
    // A bare existing ID remains authoritative even if its spelling is an
    // otherwise unindexed vocabulary word.
    graph.nodes[0].id = "snapshot".to_owned();
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let identified = open(&path, None, &directory.path().join("identity-cache"))?;
    let exact = identified.discover_ranked(request("snapshot"))?;
    assert_eq!(exact.candidates.len(), 1);
    assert_eq!(exact.candidates[0].node.id, "snapshot");
    assert!(
        !exact.candidates[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("Vocabulary fallback"))
    );

    // A new alias-supported concept cannot evict an otherwise eligible
    // literal inspection lead by changing the coverage denominator.
    graph.nodes.push(support::node(
        "literal-inspection",
        NodeKind::Function,
        "cache",
        "fixture::cache",
    ));
    graph.nodes.push(support::node(
        "reset-widget",
        NodeKind::Function,
        "reset_widget",
        "fixture::reset_widget",
    ));
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let admission = open(&path, None, &directory.path().join("admission-cache"))?;
    let trace = admission.trace_ranked(
        request("Where are cache snapshots reset?"),
        "literal-inspection",
    )?;
    let details = trace
        .target_details
        .ok_or("missing literal inspection trace")?;
    assert!(details.supported_coverage < 0.4);
    assert_eq!(details.filter_reason, None);

    // Once the corpus actually contains the literal word, it is not replaced
    // by recall alternatives, even on a different candidate.
    graph.nodes.push(support::node(
        "literal-snapshot",
        NodeKind::Function,
        "snapshot",
        "fixture::snapshot",
    ));
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let literal = open(&path, None, &directory.path().join("literal-cache"))?;
    let trace = literal.trace_ranked(request("Where is a cache snapshot made?"), "snapshot")?;
    let details = trace.target_details.ok_or("missing literal-word trace")?;
    let concept = details
        .terms
        .iter()
        .find(|term| term.term == "snapshot")
        .ok_or("missing snapshot concept")?;
    assert!(
        !concept
            .variants
            .iter()
            .any(|term| matches!(term.as_str(), "copy" | "clone"))
    );
    Ok(())
}

#[test]
fn implementation_operation_evidence_precedes_signature_mentions()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_query::RelevanceBand;
    for (operation, inflected) in [
        ("encode", "encoded"),
        ("decode", "decoded"),
        ("parse", "parsed"),
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let template = graph.links.first().cloned().ok_or("missing edge")?;
        graph.nodes.clear();
        graph.links.clear();
        for index in 0..7 {
            let mut node = support::node(
                &format!("a:context:{index}"),
                NodeKind::Function,
                "new_map",
                "fixture::new_map",
            );
            node.details = Some(serde_json::from_value(serde_json::json!({
                "type":"symbol", "data":{"signature":format!("fn(items: ItemMap, {operation}: Codec)")}
            }))?);
            graph.nodes.push(node);
        }
        for (id, signature) in [
            ("z:implementation", "fn(items: Items)"),
            ("wrong-subject", "fn(widget: Widget)"),
            ("clipped-signature", "fn(items: Items, …"),
        ] {
            let mut node = support::node(id, NodeKind::Function, "opaque", "fixture::opaque");
            node.details = Some(serde_json::from_value(serde_json::json!({
                "type":"symbol", "data":{"signature":signature}
            }))?);
            graph.nodes.push(node);
            let mut edge = template.clone();
            edge.source = id.to_owned();
            edge.target = "operation".to_owned();
            edge.kind = EdgeKind::Calls;
            edge.id = compass_model::identity::edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key = edge.id.clone();
            graph.links.push(edge);
        }
        graph.nodes.push(support::node(
            "operation",
            NodeKind::Function,
            operation,
            &format!("codec::{operation}"),
        ));
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        let question = request(&format!("Where are items {inflected} into a map?"));
        let answer = json.discover_ranked(question.clone())?;
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(question)?.candidates
        );
        assert_eq!(
            answer
                .candidates
                .first()
                .map(|candidate| candidate.node.id.as_str()),
            Some("z:implementation"),
            "{operation}: {answer:?}"
        );
        assert_eq!(answer.candidates[0].relevance, RelevanceBand::Low);
        let clipped = answer
            .candidates
            .iter()
            .find(|row| row.node.id == "clipped-signature")
            .ok_or("clipped candidate missing")?;
        assert!(
            clipped
                .reasons
                .iter()
                .all(|reason| !reason.contains("inspection preference"))
        );
        assert!(
            !answer
                .candidates
                .iter()
                .any(|candidate| candidate.node.id == "wrong-subject")
        );
        // An explicit identity remains authoritative even for a contextual mention.
        assert_eq!(
            json.discover_ranked(request("a:context:0"))?.candidates[0]
                .node
                .id,
            "a:context:0"
        );
        assert!(
            json.discover_ranked(request("Where are `MissingItems` encoded?"))?
                .candidates
                .is_empty()
        );
        let second = if operation == "encode" {
            "decoded"
        } else {
            "encoded"
        };
        let compound = json.discover_ranked(request(&format!(
            "Where are items {inflected} and {second} into a map?"
        )))?;
        assert!(compound.candidates.iter().all(|candidate| {
            candidate
                .reasons
                .iter()
                .all(|reason| !reason.contains("inspection preference"))
        }));
        for condition in ["without decoration", "before midnight"] {
            let qualified = json.discover_ranked(request(&format!(
                "Where are items {inflected} {condition}?"
            )))?;
            assert!(!qualified.candidates.is_empty());
            assert!(qualified.candidates.iter().all(|candidate| {
                candidate.relevance == RelevanceBand::Low
                    && candidate
                        .reasons
                        .iter()
                        .all(|reason| !reason.contains("inspection preference"))
            }));
        }
    }
    Ok(())
}

#[test]
fn ordinary_plural_acronyms_do_not_become_missing_symbol_names()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes.clear();
    for (id, name) in [
        ("ttl", "convert_ttl"),
        ("api", "register_api"),
        ("url", "parse_url"),
    ] {
        graph
            .nodes
            .push(support::node(id, NodeKind::Function, name, name));
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    for (plural, singular, expected) in [
        (
            "Where are TTLs converted?",
            "Where is ttl converted?",
            "ttl",
        ),
        (
            "Where are APIs registered?",
            "Where is api registered?",
            "api",
        ),
        ("Where are URLs parsed?", "Where is url parsed?", "url"),
    ] {
        let answer = engine.discover_ranked(request(plural))?;
        let singular_answer = engine.discover_ranked(request(singular))?;
        assert_eq!(answer.candidates, singular_answer.candidates, "{plural}");
        assert_eq!(
            answer.candidates.first().map(|row| row.node.id.as_str()),
            Some(expected)
        );
    }
    // Explicitly quoted names still require that exact identity, even when
    // their spelling resembles an ordinary plural acronym.
    let answer = engine.discover_ranked(request("Where are `TTLs` converted?"))?;
    assert!(answer.candidates.is_empty());
    Ok(())
}

#[test]
fn broad_method_recall_requires_witnessed_local_reference_not_target_namespace()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, SourceAnchor};
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links.first().cloned().ok_or("missing edge fixture")?;
    graph.nodes.clear();
    graph.links.clear();
    std::fs::write(directory.path().join("src/lib.rs"), b"codecode")?;
    let file = graph
        .graph
        .files
        .iter_mut()
        .find(|file| file.path == "src/lib.rs")
        .ok_or("missing source file")?;
    file.byte_size = 8;
    file.content_digest = format!("sha256:{:x}", Sha256::digest(b"codecode"));
    graph.nodes.push(support::node(
        "listeners",
        NodeKind::Property,
        "abortListeners",
        "queue.abortListeners",
    ));
    graph.nodes.push(support::node(
        "namespace-only",
        NodeKind::Property,
        "checkpoint",
        "abortListeners.checkpoint",
    ));
    for (id, kind) in [
        ("good-reference", EdgeKind::References),
        ("good-call", EdgeKind::Calls),
        ("wrong-kind", EdgeKind::Renders),
        ("no-ast-proof", EdgeKind::References),
        ("inferred-proof", EdgeKind::References),
        ("outside-owner", EdgeKind::References),
        ("wrong-direction", EdgeKind::References),
        ("namespace-borrower", EdgeKind::References),
        ("no-ast-owner-proof", EdgeKind::References),
    ] {
        let mut owner = support::node(
            id,
            NodeKind::Method,
            "process",
            &format!("queue.{id}.process"),
        );
        if id == "no-ast-owner-proof" {
            for proof in &mut owner.evidence {
                proof.origin = EvidenceOrigin::Config;
            }
        }
        graph.nodes.push(owner);
        let mut edge = template.clone();
        edge.source = id.to_owned();
        edge.target = if id == "namespace-borrower" {
            "namespace-only"
        } else {
            "listeners"
        }
        .to_owned();
        edge.kind = kind;
        match id {
            "no-ast-proof" => {
                for proof in &mut edge.evidence {
                    proof.origin = EvidenceOrigin::Config;
                }
            }
            "inferred-proof" => {
                for proof in &mut edge.evidence {
                    proof.confidence = EvidenceConfidence::Inferred;
                }
            }
            "outside-owner" => {
                let site = SourceAnchor {
                    start_byte: 4,
                    end_byte: 8,
                    start_column: 4,
                    end_column: 8,
                    ..edge.relationship_site.clone().ok_or("missing site")?
                };
                edge.relationship_site = Some(site.clone());
                for proof in &mut edge.evidence {
                    proof.anchors = vec![site.clone()];
                }
            }
            "wrong-direction" => std::mem::swap(&mut edge.source, &mut edge.target),
            _ => {}
        }
        edge.id = compass_model::identity::edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key = edge.id.clone();
        graph.links.push(edge);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let question = request("where are abort listeners removed");
    let reference = json.discover_ranked(question.clone())?;
    assert!(
        reference
            .candidates
            .first()
            .is_some_and(|candidate| matches!(
                candidate.node.id.as_str(),
                "good-reference" | "good-call"
            ))
    );
    assert_eq!(
        reference.candidates,
        stored.discover_ranked(question.clone())?.candidates
    );
    graph.nodes.reverse();
    graph.links.reverse();
    let reordered_path = directory.path().join("reordered.json");
    std::fs::write(&reordered_path, serde_json::to_vec(&graph)?)?;
    let reordered = open(
        &reordered_path,
        None,
        &directory.path().join("reordered-cache"),
    )?;
    assert_eq!(
        reference.candidates,
        reordered.discover_ranked(question.clone())?.candidates
    );
    for id in ["good-reference", "good-call"] {
        let candidate = reference
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == id)
            .ok_or_else(|| {
                format!(
                    "method {id} not recalled; candidates={:?}; trace={:?}",
                    reference.candidates,
                    json.trace_ranked(question.clone(), id)
                )
            })?;
        assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
        assert!(
            candidate
                .matched_fields
                .iter()
                .any(|field| field == "code_reference")
        );
        assert_eq!(candidate.supporting_sources.len(), 1);
        assert!(
            candidate
                .reasons
                .iter()
                .any(|reason| reason.contains("not confirmation"))
        );
    }
    for id in [
        "wrong-kind",
        "no-ast-proof",
        "inferred-proof",
        "outside-owner",
        "wrong-direction",
        "namespace-borrower",
        "no-ast-owner-proof",
    ] {
        assert!(
            !reference
                .candidates
                .iter()
                .any(|candidate| candidate.node.id == id),
            "{id}"
        );
    }
    let search = json.search(SearchRequest {
        query: question.question,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        search
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::References && edge.source == "good-reference")
    );
    assert!(
        search
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Calls && edge.source == "good-call")
    );
    Ok(())
}

#[test]
fn dense_relationship_recall_reports_bounds_and_keeps_shared_work_ceilings()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_model::query_contract::{
        MAX_INDEXED_CANDIDATE_NODES_READ, MAX_INDEXED_CANDIDATE_PROBES,
    };
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links.first().cloned().ok_or("missing edge")?;
    graph.nodes.clear();
    graph.links.clear();
    for at in 0..40 {
        graph.nodes.push(support::node(
            &format!("target:{at}"),
            NodeKind::Property,
            "abortListeners",
            &format!("queue{at}.abortListeners"),
        ));
    }
    for at in 0..300 {
        let id = format!("owner:{at:04}");
        graph.nodes.push(support::node(
            &id,
            NodeKind::Method,
            "process",
            &format!("queue{at}.process"),
        ));
        for target in 0..40 {
            let mut edge = template.clone();
            edge.source = id.clone();
            edge.target = format!("target:{target}");
            edge.kind = EdgeKind::References;
            edge.id = compass_model::identity::edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key = edge.id.clone();
            graph.links.push(edge);
        }
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let question = request("where are abort listeners removed");
    let reference = json.discover_ranked(question.clone())?;
    for engine in [&json, &stored] {
        let answer = engine.discover_ranked(question.clone())?;
        assert!(answer.bounded && answer.eligible_count.is_none());
        assert!(answer.discovery.stats.candidate_nodes <= MAX_INDEXED_CANDIDATE_NODES_READ);
        assert!(answer.discovery.stats.candidate_probes <= MAX_INDEXED_CANDIDATE_PROBES);
        assert!(
            answer
                .discovery
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::BoundedTruncation)
        );
        assert_eq!(answer.candidates, reference.candidates);
        assert!(
            answer
                .candidates
                .iter()
                .any(|candidate| candidate.node.id.starts_with("owner:"))
        );
    }
    Ok(())
}

#[test]
fn broad_signature_recall_precedes_common_term_caps_and_preserves_rank_four_and_five()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let query = request("where are protocol errors formatted");
    let ranked = engine.discover_ranked(query.clone())?;
    assert_eq!(ranked.discovery.seeds.len(), 3);
    assert!(ranked.candidates.len() >= 6);
    for candidate in ranked.candidates.iter().take(6) {
        assert!(candidate.node.id.starts_with("z:formatter:"));
        assert!(
            candidate
                .matched_fields
                .iter()
                .any(|field| field == "signature")
        );
        assert!(
            !candidate
                .matched_fields
                .iter()
                .any(|field| field == "exact_name")
        );
    }
    let trace = engine.trace_ranked(query.clone(), "z:formatter:4")?;
    assert!(trace.graph_present && trace.admitted && trace.first_page);
    assert_eq!(trace.rank, Some(5));
    assert!(!trace.seed);
    assert_eq!(trace.schema, "compass.query.retrieval-trace/1");
    let details = trace
        .target_details
        .as_ref()
        .ok_or("missing target trace")?;
    assert_eq!(details.filter_reason, None);
    assert_eq!(details.score, Some(ranked.candidates[4].score));
    assert!(details.supported_coverage >= 0.4);
    assert!(
        details
            .terms
            .iter()
            .any(|term| { term.matched_fields.iter().any(|field| field == "signature") })
    );
    // The diagnostic observes the same ranking and does not alter later calls.
    assert_eq!(
        ranked.candidates,
        engine.discover_ranked(query.clone())?.candidates
    );
    let default = engine.discover(query.clone())?;
    assert_eq!(default.seeds[0].node_id, ranked.candidates[0].node.id);
    let search = engine.search(SearchRequest {
        query: query.question,
        limits: CodeQueryLimits::default(),
    })?;
    assert_eq!(search.results[0].node_id, ranked.candidates[0].node.id);
    Ok(())
}

#[test]
fn target_trace_names_the_supported_coverage_filter() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes = vec![
        support::node(
            "target",
            NodeKind::Function,
            "retain_payload",
            "retain_payload",
        ),
        support::node(
            "other",
            NodeKind::Function,
            "compress_encrypt_persist",
            "compress_encrypt_persist",
        ),
    ];
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let query = request("where is payload compressed encrypted persisted");
    let before = engine.discover_ranked(query.clone())?;
    let trace = engine.trace_ranked(query.clone(), "target")?;
    assert!(trace.graph_present && trace.admitted);
    assert_eq!(trace.rank, None);
    let details = trace.target_details.ok_or("missing target trace")?;
    assert_eq!(
        details.filter_reason,
        Some(compass_query::RankedFilterReason::InsufficientSupportedCoverage)
    );
    assert!(details.supported_coverage < 0.4);
    assert_eq!(details.score, None);
    assert!(
        details
            .terms
            .iter()
            .any(|term| !term.matched_fields.is_empty())
    );
    assert!(
        details
            .terms
            .iter()
            .any(|term| term.matched_fields.is_empty())
    );
    assert_eq!(before.candidates, engine.discover_ranked(query)?.candidates);
    Ok(())
}

#[test]
fn unsupported_subjects_do_not_borrow_generic_operation_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes.push(support::node(
        "run",
        NodeKind::Function,
        "run",
        "server::run",
    ));
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    for question in [
        "where does the built-in Redis cluster failover run",
        "where does the built-in redis cluster failover run",
        "where does the built-in REDIS cluster failover run",
        "where does the built-in `redis` cluster failover run",
        "where does the built-in redis_cluster failover run",
    ] {
        let answer = engine.discover_ranked(request(question))?;
        assert!(answer.candidates.is_empty(), "{question}");
        assert!(answer.discovery.seeds.is_empty(), "{question}");
        assert!(
            answer
                .discovery
                .diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::NoMatch
                    && d.message.contains("supporting indexed evidence"))
        );
    }
    Ok(())
}

#[test]
fn unfamiliar_ordinary_context_returns_tentative_source_candidates()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    std::fs::write(&path, serde_json::to_vec(&fixture(&path)?)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    for question in [
        "where are protocol errors elegantly formatted",
        "where are redis protocol errors formatted",
        "where are protocol errors elegantly decorated before presenting readable messages",
    ] {
        let answer = engine.discover_ranked(request(question))?;
        assert!(!answer.candidates.is_empty(), "{question}");
        let default = engine.discover(request(question))?;
        let search = engine.search(SearchRequest {
            query: question.to_owned(),
            limits: CodeQueryLimits::default(),
        })?;
        for diagnostics in [
            &answer.discovery.diagnostics,
            &default.diagnostics,
            &search.diagnostics,
        ] {
            assert!(diagnostics.iter().any(|diagnostic| {
                diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
                    && diagnostic.message.contains("inspection candidates")
                    && diagnostic.message.contains("not established")
            }));
        }
        for candidate in &answer.candidates {
            assert!(candidate.node.id.starts_with("z:formatter:"));
            assert_eq!(candidate.relevance, RelevanceBand::Low);
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("not established"))
            );
        }
    }
    for question in [
        "where are Redis protocol errors formatted",
        "where are `redis` protocol errors formatted",
        "where are redis_cache protocol errors formatted",
        "where are redis.client protocol errors formatted",
        "where are redis::client protocol errors formatted",
        "where are redis/client.py protocol errors formatted",
    ] {
        assert!(
            engine
                .discover_ranked(request(question))?
                .candidates
                .is_empty()
        );
    }
    Ok(())
}

#[test]
fn explicit_composite_identity_cannot_be_assembled_from_separate_name_parts()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    let mut correct = graph.nodes.first().cloned().ok_or("missing formatter")?;
    correct.id = "correct".into();
    correct.name = "format_errors".into();
    correct.qualified_name = "redis::client::format_errors".into();
    let mut wrong = correct.clone();
    wrong.id = "wrong-owner".into();
    wrong.qualified_name = "client::redis::format_errors".into();
    graph.nodes.extend([correct, wrong]);
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    for qualified in ["redis::client", "redis.client"] {
        let answer = engine.discover_ranked(request(&format!(
            "where are {qualified} protocol errors formatted"
        )))?;
        let correct = answer
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == "correct")
            .ok_or("matching owner missing")?;
        assert_eq!(correct.relevance, RelevanceBand::High);
        let wrong = answer
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == "wrong-owner")
            .ok_or("wrong-owner inspection pointer missing")?;
        assert_eq!(wrong.relevance, RelevanceBand::Low);
        assert!(wrong.reasons.iter().any(
            |reason| reason.contains("explicit identity") && reason.contains("not established")
        ));
    }
    let absent =
        engine.discover_ranked(request("where are redis_client protocol errors formatted"))?;
    assert!(!absent.candidates.is_empty());
    assert!(
        absent
            .candidates
            .iter()
            .all(|candidate| candidate.relevance == RelevanceBand::Low)
    );
    Ok(())
}

#[test]
fn plural_subjects_recall_singular_identifiers_without_changing_strict_lookup()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes.push(support::node(
        "cookie",
        NodeKind::Function,
        "store_cookie",
        "session::store_cookie",
    ));
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let answer = engine.discover_ranked(request("where are cookies stored"))?;
    assert_eq!(
        answer
            .candidates
            .first()
            .map(|candidate| candidate.node.id.as_str()),
        Some("cookie")
    );
    let exact = engine.search_exact(
        SearchRequest {
            query: "cookies".into(),
            limits: CodeQueryLimits::default(),
        },
        Default::default(),
    )?;
    assert!(exact.nodes.is_empty());
    Ok(())
}

#[test]
fn a_subject_elsewhere_cannot_establish_a_candidates_missing_context()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes.push(support::node(
        "redis-unrelated",
        NodeKind::Function,
        "redis_ping",
        "cache::redis_ping",
    ));
    let mut target = graph
        .nodes
        .first()
        .cloned()
        .ok_or("formatter fixture missing")?;
    target.id = "redis-formatter".into();
    target.qualified_name = "redis::format_errors".into();
    target.name = "format_errors".into();
    graph.nodes.push(target);
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let answer = engine.discover_ranked(request("where are Redis protocol errors formatted"))?;
    assert_eq!(
        answer
            .candidates
            .first()
            .map(|candidate| candidate.node.id.as_str()),
        Some("redis-formatter")
    );
    assert_eq!(answer.candidates[0].relevance, RelevanceBand::High);
    for generic in answer
        .candidates
        .iter()
        .filter(|candidate| candidate.node.id.starts_with("z:formatter:"))
    {
        assert_eq!(generic.relevance, RelevanceBand::Low);
        assert!(
            generic
                .reasons
                .iter()
                .any(|reason| reason.contains("redis") && reason.contains("not established"))
        );
    }
    Ok(())
}

#[test]
fn documentation_ownership_requires_ast_evidence_and_byte_enclosure()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::{EdgeKind, ResourceKind, ResourceNodeDetails};
    use compass_model::provenance::EvidenceOrigin;
    for case in [
        "owned",
        "owned_class",
        "owned_struct",
        "empty",
        "config",
        "outside_bytes",
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let mut edge = graph.links.first().cloned().ok_or("fixture has no edge")?;
        let mut owner = support::node(
            "owner",
            match case {
                "owned_class" => NodeKind::Class,
                "owned_struct" => NodeKind::Struct,
                _ => NodeKind::Function,
            },
            "opaque",
            "implementation::opaque",
        );
        if case == "outside_bytes" {
            let source = owner.source.as_mut().ok_or("missing source")?;
            source.start_byte = 1;
            source.start_column = 1;
        }
        let mut prose = support::node("prose", NodeKind::Resource, "Notes", "Notes");
        prose.details = Some(NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: ResourceKind::Rationale,
            uri: None,
            media_type: None,
            content: Some("protocol diagnostics parse".into()),
        }));
        let source = prose.source.as_mut().ok_or("missing prose source")?;
        source.end_byte = 1;
        source.end_column = 1;
        edge.source = prose.id.clone();
        edge.target = owner.id.clone();
        edge.kind = EdgeKind::Documents;
        edge.id = compass_model::identity::edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            None,
        );
        edge.key = edge.id.clone();
        if case == "empty" {
            edge.evidence.clear();
        }
        if case == "config" {
            for evidence in &mut edge.evidence {
                evidence.origin = EvidenceOrigin::Config;
            }
        }
        graph.nodes = vec![owner, prose];
        graph.links = vec![edge];
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        if case == "empty" {
            assert!(
                open(&path, None, &directory.path().join("cache")).is_err(),
                "provenance-free relationships must fail graph validation"
            );
            continue;
        }
        let engine = open(&path, None, &directory.path().join("cache"))?;
        let answer = engine.discover_ranked(request("where are protocol diagnostics parsed"))?;
        let owner = answer
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == "owner");
        assert_eq!(
            owner.is_some(),
            case.starts_with("owned"),
            "{case}: {answer:?}"
        );
        if let Some(owner) = owner {
            assert!(
                owner.excerpts.is_empty(),
                "enclosure alone does not establish verbatim raw retention"
            );
            assert!(
                owner
                    .matched_fields
                    .iter()
                    .any(|field| field == "owned_document")
            );
            assert_eq!(owner.supporting_sources.len(), 1);
            assert!(
                !answer
                    .candidates
                    .iter()
                    .any(|candidate| candidate.node.id == "prose"),
                "owned prose should be supporting evidence, not a duplicate candidate"
            );
        }
    }
    Ok(())
}

#[test]
fn negation_is_not_silently_rewritten_into_positive_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    for question in [
        "where are protocol errors not formatted",
        "WHERE are protocol errors NOT formatted",
        "where are protocol errors formatted without decoration",
    ] {
        let answer = engine.discover_ranked(request(question))?;
        assert!(!answer.candidates.is_empty());
        assert!(answer.discovery.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
                && diagnostic.message.contains("Negative qualification")
        }));
        for candidate in answer.candidates {
            assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
            assert!(candidate.reasons.iter().any(|reason| {
                reason.contains("Negative qualification") && reason.contains("not established")
            }));
        }
    }
    Ok(())
}

#[test]
fn temporal_qualifications_remain_visible_without_becoming_lexical_subjects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    graph.nodes = vec![
        support::node(
            "parser",
            NodeKind::Function,
            "parse_payload",
            "parse_payload",
        ),
        support::node(
            "noise",
            NodeKind::Function,
            "before_after_during_until",
            "before_after_during_until",
        ),
    ];
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for condition in ["before", "after", "during", "until", "BEFORE"] {
        let question = format!("where are payloads parsed {condition}");
        let answer = json.discover_ranked(request(&question))?;
        assert_eq!(answer.discovery.question, question);
        assert_eq!(
            answer.candidates,
            stored.discover_ranked(request(&question))?.candidates
        );
        let target = answer.candidates.first().ok_or("missing payload parser")?;
        assert_eq!(target.node.id, "parser");
        assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
        assert!(target.reasons.iter().any(|reason| {
            reason.contains("Temporal qualification") && reason.contains("not established")
        }));
        assert!(answer.discovery.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
                && diagnostic.message.contains("Temporal qualification")
                && diagnostic.message.contains(&condition.to_ascii_lowercase())
        }));
        let trace = json.trace_ranked(request(&question), "parser")?;
        let details = trace.target_details.ok_or("missing trace details")?;
        assert!(
            !details
                .terms
                .iter()
                .any(|term| { ["before", "after", "dur", "until"].contains(&term.term.as_str()) })
        );
    }
    for question in [
        "where are `before` helpers",
        "where are \"during\" helpers",
        "where are before_after helpers",
        "where are src/before.ts helpers",
    ] {
        let answer = json.discover_ranked(request(question))?;
        assert!(
            !answer
                .discovery
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.message.contains("Temporal qualification") }),
            "{question}"
        );
        if question.contains('`') || question.contains('"') {
            let trace = json.trace_ranked(request(question), "noise")?;
            let details = trace
                .target_details
                .ok_or("missing quoted identity trace")?;
            let expected = if question.contains("during") {
                "dur"
            } else {
                "before"
            };
            assert!(
                details.terms.iter().any(|term| term.term == expected),
                "{question}"
            );
        }
    }
    Ok(())
}

#[test]
fn an_identifier_containing_without_is_not_a_negative_condition()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    let node = graph.nodes.first_mut().ok_or("missing formatter")?;
    node.name = "format_protocol_errors_without_cache".into();
    node.qualified_name = "protocol::format_protocol_errors_without_cache".into();
    let target = node.id.clone();
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let answer =
        engine.discover_ranked(request("where are protocol errors without_cache formatted"))?;
    assert_eq!(
        answer
            .candidates
            .first()
            .map(|candidate| &candidate.node.id),
        Some(&target)
    );
    assert!(
        !answer
            .discovery
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("Negative qualification") })
    );
    Ok(())
}

#[test]
fn fielded_rank_and_reasons_are_identical_across_json_store_and_reordered_input()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let mut reversed = graph;
    reversed.nodes.reverse();
    std::fs::write(&path, serde_json::to_vec(&reversed)?)?;
    let reordered = open(&path, None, &directory.path().join("reverse-cache"))?;
    for question in [
        "where are protocol errors formatted",
        "where are protocol errors elegantly formatted",
        "where are protocol errors formatted without decoration",
        "where are `redis` protocol errors formatted",
    ] {
        let query = request(question);
        let expected = json.discover_ranked(query.clone())?;
        let actual = stored.discover_ranked(query.clone())?;
        assert_eq!(actual.candidates, expected.candidates, "{question}");
        assert_eq!(
            actual.discovery.seeds, expected.discovery.seeds,
            "{question}"
        );
        assert_eq!(actual.bounded, expected.bounded, "{question}");
        assert_eq!(
            actual.discovery.diagnostics, expected.discovery.diagnostics,
            "{question}"
        );
        assert_eq!(
            reordered.discover_ranked(query)?.candidates,
            expected.candidates,
            "{question}"
        );
    }
    Ok(())
}

#[test]
fn cancellation_flag_false_preserves_broad_routing_and_true_is_typed()
-> Result<(), Box<dyn std::error::Error>> {
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let question = request("where are protocol errors formatted");
    let ordinary = engine.discover_with_cancellation(question.clone(), None)?;
    let unset = AtomicBool::new(false);
    let cancelable = engine.discover_with_cancellation(question.clone(), Some(&unset))?;
    assert_eq!(ordinary.seeds, cancelable.seeds);
    assert_eq!(ordinary.nodes, cancelable.nodes);
    assert_eq!(ordinary.edges, cancelable.edges);
    assert_eq!(ordinary.diagnostics, cancelable.diagnostics);
    let set = AtomicBool::new(true);
    let result = engine.discover_with_cancellation(question, Some(&set));
    assert!(
        matches!(result, Err(error) if error.kind() == compass_query::QueryErrorKind::Cancelled)
    );
    Ok(())
}

#[test]
fn ranked_public_paths_share_an_already_expired_engine_deadline()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?
        .with_deadline(std::time::Instant::now());
    let question = request("where are protocol errors formatted");
    for result in [
        engine.discover_ranked(question.clone()),
        engine.discover_ranked_with_context(question.clone(), true),
        engine.discover_ranked_with_context(question.clone(), false),
    ] {
        assert!(
            matches!(result, Err(error) if error.kind() == compass_query::QueryErrorKind::Timeout)
        );
    }
    assert!(
        matches!(engine.trace_ranked(question.clone(), "z:formatter:4"), Err(error) if error.kind() == compass_query::QueryErrorKind::Timeout)
    );
    let unset = std::sync::atomic::AtomicBool::new(false);
    assert!(
        matches!(engine.discover_with_cancellation(question, Some(&unset)), Err(error) if error.kind() == compass_query::QueryErrorKind::Timeout)
    );
    Ok(())
}

#[test]
fn ordinary_broad_discovery_uses_its_own_byte_budget_and_trims_coherently()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut edge = GraphDocument::load(&path)?
        .links
        .into_iter()
        .next()
        .ok_or("edge fixture missing")?;
    let mut graph = fixture(&path)?;
    edge.source = "z:formatter:0".to_owned();
    edge.target = "z:formatter:5".to_owned();
    edge.kind = compass_model::code_graph::EdgeKind::Calls;
    edge.id = compass_model::identity::edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        edge.relationship_site.as_ref(),
        edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
    );
    edge.key = edge.id.clone();
    graph.links.push(edge);
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let question = request("where are protocol errors formatted");
    let ordinary = engine.discover(question.clone())?;
    assert!(!ordinary.edges.is_empty());
    let raw_bytes = serde_json::to_vec(&ordinary)?.len();
    let ranked = engine.discover_ranked_with_context(question.clone(), true)?;
    let fits_raw = raw_bytes as u64 + 128;
    assert!(serde_json::to_vec(&ranked)?.len() as u64 > fits_raw);

    let mut limited = question.clone();
    limited.limits.max_response_bytes = fits_raw;
    let fits = engine.discover(limited.clone())?;
    assert_eq!(fits.nodes, ordinary.nodes);
    assert_eq!(fits.edges, ordinary.edges);
    assert!(serde_json::to_vec(&fits)?.len() as u64 <= fits_raw);
    for include_context in [false, true] {
        assert!(
            matches!(engine.discover_ranked_with_context(limited.clone(), include_context), Err(error) if error.code() == "ranked_response_limit")
        );
    }

    let mut without_edges = ordinary.clone();
    without_edges.edges.clear();
    limited.limits.max_response_bytes = serde_json::to_vec(&without_edges)?
        .len()
        .saturating_sub(256) as u64;
    let trimmed = engine.discover(limited)?;
    assert!(trimmed.truncated);
    assert!(trimmed.nodes.len() < ordinary.nodes.len());
    assert_eq!(trimmed.seeds, ordinary.seeds);
    assert!(serde_json::to_vec(&trimmed)?.len() as u64 <= trimmed.limits.max_response_bytes);
    let retained = trimmed
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        trimmed
            .seeds
            .iter()
            .all(|seed| retained.contains(seed.node_id.as_str()))
    );
    assert!(trimmed.edges.iter().all(|edge| {
        retained.contains(edge.source.as_str()) && retained.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn ranked_exact_id_does_not_admit_identifier_prefix_matches()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path)?;
    let exact_id = format!("sha256:{}", "0123456789abcdef".repeat(4));
    graph.nodes = vec![
        support::node(
            &exact_id,
            NodeKind::Function,
            "render_message",
            "output::render_message",
        ),
        support::node(
            "digest-helper",
            NodeKind::Function,
            "sha256_hex",
            "crypto::sha256_hex",
        ),
        support::node(
            "digest-wrapper",
            NodeKind::Function,
            "sha256_digest",
            "crypto::sha256_digest",
        ),
    ];
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for engine in [&json, &stored] {
        let ordinary = engine.discover(request(&exact_id))?;
        let ranked = engine.discover_ranked(request(&exact_id))?;
        assert_eq!(ranked.discovery.seeds, ordinary.seeds);
        assert_eq!(ranked.discovery.nodes, ordinary.nodes);
        assert_eq!(ranked.discovery.edges, ordinary.edges);
        assert_eq!(
            ranked
                .candidates
                .iter()
                .map(|candidate| candidate.node.id.as_str())
                .collect::<Vec<_>>(),
            vec![exact_id.as_str()]
        );
        let broad = engine.discover_ranked(request("where is sha256 computed"))?;
        assert!(broad.candidates.iter().any(|candidate| {
            matches!(
                candidate.node.id.as_str(),
                "digest-helper" | "digest-wrapper"
            )
        }));
    }
    Ok(())
}

#[test]
fn ranked_structural_fallback_bounds_its_wrapper_independently_of_raw_discovery()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let question = request("who calls callee");
    let ordinary = engine.discover(question.clone())?;
    let ranked = engine.discover_ranked(question.clone())?;
    assert!(!ranked.candidates.is_empty());
    assert!(ranked.candidates.iter().all(|candidate| {
        candidate
            .reasons
            .iter()
            .any(|reason| reason.starts_with("Structural query seed"))
    }));
    let mut limited = question;
    limited.limits.max_response_bytes = serde_json::to_vec(&ordinary)?.len() as u64 + 64;
    assert!(serde_json::to_vec(&ranked)?.len() as u64 > limited.limits.max_response_bytes);
    let raw = engine.discover(limited.clone())?;
    assert_eq!(raw.nodes, ordinary.nodes);
    assert_eq!(raw.edges, ordinary.edges);
    assert!(serde_json::to_vec(&raw)?.len() as u64 <= raw.limits.max_response_bytes);
    for result in [
        engine.discover_ranked(limited.clone()),
        engine.discover_ranked_with_context(limited, false),
    ] {
        assert!(matches!(result, Err(error) if error.code() == "ranked_response_limit"));
    }
    Ok(())
}

#[test]
fn broad_search_marks_witness_edges_omitted_by_the_edge_cap()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::{EdgeKind, ResourceKind, ResourceNodeDetails};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links.first().cloned().ok_or("edge fixture missing")?;
    graph.nodes.clear();
    graph.links.clear();
    let mut paragraph = support::node("paragraph", NodeKind::Resource, "Notes", "Notes");
    paragraph.details = Some(NodeDetails::Resource(ResourceNodeDetails {
        resource_kind: ResourceKind::Document,
        uri: None,
        media_type: Some("text/markdown".to_owned()),
        content: Some("OrchardOne and OrchardTwo implement orchard harvesting".to_owned()),
    }));
    graph.nodes.push(paragraph);
    for name in ["OrchardOne", "OrchardTwo"] {
        graph
            .nodes
            .push(support::node(name, NodeKind::Class, name, name));
        let mut edge = template.clone();
        edge.source = "paragraph".to_owned();
        edge.target = name.to_owned();
        edge.kind = EdgeKind::References;
        edge.id = compass_model::identity::edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            None,
        );
        edge.key = edge.id.clone();
        graph.links.push(edge);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for engine in [&json, &stored] {
        let search = SearchRequest {
            query: "where is orchard harvesting".to_owned(),
            limits: CodeQueryLimits::default(),
        };
        let complete = engine.search(search.clone())?;
        assert!(!complete.truncated, "{complete:?}");
        assert_eq!(complete.edges.len(), 2);
        let mut limited = search;
        limited.limits.max_edges = 1;
        let bounded = engine.search(limited)?;
        assert!(bounded.truncated);
        assert_eq!(bounded.results, complete.results);
        assert_eq!(bounded.edges.len(), 1);
        assert!(complete.edges.contains(&bounded.edges[0]));
        assert_eq!(bounded.edges[0].source, "paragraph");
        assert!(
            bounded
                .nodes
                .iter()
                .any(|node| node.id == bounded.edges[0].target)
        );
    }
    Ok(())
}

#[test]
fn leading_document_proof_retrieves_opaque_owner_with_json_store_parity()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::{
        DiagnosticSeverity, EdgeKind, GraphDiagnostic, ResourceKind, ResourceNodeDetails,
    };
    use compass_model::provenance::{LEADING_DOCUMENTATION_RULE, OccurrenceRule, SourceAnchor};
    for proven in [true, false] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let content = "/// protocol diagnostics parsed\n";
        let excerpt = SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 0,
            end_byte: content.len() as u64,
            start_line: 1,
            start_column: 0,
            end_line: 2,
            end_column: 0,
        };
        let definition = SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: excerpt.end_byte,
            end_byte: excerpt.end_byte + 14,
            start_line: 2,
            start_column: 0,
            end_line: 2,
            end_column: 14,
        };
        let actual_source = format!("{content}fn opaque() {{}}");
        std::fs::write(directory.path().join("src/lib.rs"), &actual_source)?;
        use sha2::Digest;
        let source_file = graph
            .graph
            .files
            .iter_mut()
            .find(|file| file.path == "src/lib.rs")
            .ok_or("missing source file")?;
        source_file.byte_size = actual_source.len() as u64;
        source_file.content_digest = format!(
            "sha256:{:x}",
            sha2::Sha256::digest(actual_source.as_bytes())
        );
        let mut owner = support::node(
            "owner",
            NodeKind::Function,
            "opaque",
            "implementation::opaque",
        );
        owner.source = Some(definition.clone());
        owner.evidence[0].anchors = vec![definition.clone()];
        let mut prose = support::node("prose", NodeKind::Resource, "Notes", "Notes");
        prose.source = Some(excerpt.clone());
        prose.evidence[0].anchors = vec![excerpt.clone()];
        prose.details = Some(NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: ResourceKind::Rationale,
            uri: None,
            media_type: None,
            content: Some(content.to_owned()),
        }));
        let mut edge = graph.links.first().cloned().ok_or("missing fixture edge")?;
        edge.kind = EdgeKind::Documents;
        edge.source = prose.id.clone();
        edge.target = owner.id.clone();
        edge.relationship_site = Some(excerpt.clone());
        edge.occurrence_rule = if proven {
            OccurrenceRule::new(LEADING_DOCUMENTATION_RULE)
        } else {
            None
        };
        edge.evidence[0].anchors = vec![excerpt];
        edge.evidence[0].rule = None;
        if proven {
            edge.evidence[0].anchors.push(definition);
            edge.evidence[0].rule = Some(LEADING_DOCUMENTATION_RULE.to_owned());
        }
        edge.id = compass_model::identity::edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key = edge.id.clone();
        graph.nodes = vec![owner, prose];
        graph.links = vec![edge];
        graph.graph.diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "leading_documentation_omission_summary".to_owned(),
            message: "An additional oversized documentation group was omitted.".to_owned(),
            anchor: None,
            related_ids: Vec::new(),
        });
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        let question = request("where are protocol diagnostics parsed");
        let expected = json.discover_ranked(question.clone())?;
        let actual = stored.discover_ranked(question)?;
        assert_eq!(actual.candidates, expected.candidates);
        assert_eq!(actual.discovery.diagnostics, expected.discovery.diagnostics);
        assert!(
            actual
                .discovery
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("oversized documentation group"))
        );
        let owner = actual
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == "owner");
        assert_eq!(owner.is_some(), proven, "{actual:?}");
        if let Some(owner) = owner {
            assert!(
                owner
                    .matched_fields
                    .iter()
                    .any(|field| field == "owned_document")
            );
            assert_eq!(owner.supporting_sources.len(), 1);
            assert_eq!(owner.supporting_sources[0].start_line, 1);
            let quote = owner
                .excerpts
                .first()
                .ok_or("missing proven raw documentation quote")?;
            assert_eq!(
                quote.kind,
                compass_query::RankedExcerptKind::AstDocumentation
            );
            assert_eq!(quote.text, content);
            assert_eq!(quote.source, owner.supporting_sources[0]);
            assert_eq!(quote.source.start_byte, 0);
            assert_eq!(quote.source.end_byte, content.len() as u64);
            assert!(!quote.clipped);
            assert!(
                !actual
                    .candidates
                    .iter()
                    .any(|candidate| candidate.node.id == "prose")
            );
        }
    }
    Ok(())
}

#[test]
fn broad_local_identifier_recall_requires_unique_exact_callable_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
    use compass_model::query_contract::{DiscoveryScope, DiscoveryScopeKind};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links.first().cloned().ok_or("missing edge")?;
    graph.nodes.clear();
    graph.links.clear();
    for case in [
        "good",
        "ambiguous",
        "no-ast",
        "inferred",
        "outside",
        "namespace",
        "nested",
        "truncated",
        "no-owner-ast",
    ] {
        let owner_id = format!("owner:{case}");
        let child_id = format!("child:{case}");
        let mut owner = support::node(
            &owner_id,
            NodeKind::Function,
            "opaque",
            &format!("sample.{case}.opaque"),
        );
        if case == "no-owner-ast" {
            for proof in &mut owner.evidence {
                proof.origin = EvidenceOrigin::Config;
            }
        }
        let mut child = support::node(
            &child_id,
            if case == "nested" {
                NodeKind::Function
            } else {
                NodeKind::Variable
            },
            if case == "namespace" {
                "checkpoint"
            } else {
                "annotations"
            },
            &format!("sample.annotations.{case}"),
        );
        if case == "outside" {
            // Both ranges remain individually valid, but the child is in a
            // different source file and cannot be owned by this declaration.
            let anchor = child.source.as_mut().ok_or("missing child source")?;
            anchor.file = "src/payments/gateway.rs".to_owned();
            for proof in &mut child.evidence {
                proof.anchors = vec![anchor.clone()];
            }
        }
        graph.nodes.push(owner);
        graph.nodes.push(child.clone());
        let parent_count = match case {
            "ambiguous" => 2,
            "truncated" => 5,
            _ => 1,
        };
        for parent in 0..parent_count {
            let parent_id = if parent == 0 {
                owner_id.clone()
            } else {
                format!("{owner_id}:{parent}")
            };
            if parent > 0 {
                graph.nodes.push(support::node(
                    &parent_id,
                    NodeKind::Function,
                    "opaque",
                    &format!("sample.{case}.other{parent}"),
                ));
            }
            let mut edge = template.clone();
            edge.source = parent_id;
            edge.target = child_id.clone();
            edge.kind = EdgeKind::Contains;
            edge.relationship_site = child.source.clone();
            for proof in &mut edge.evidence {
                proof.anchors = vec![child.source.clone().ok_or("missing source")?];
                if case == "no-ast" {
                    proof.origin = EvidenceOrigin::Config;
                }
                if case == "inferred" {
                    proof.confidence = EvidenceConfidence::Inferred;
                }
            }
            edge.id = compass_model::identity::edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key = edge.id.clone();
            graph.links.push(edge);
        }
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let question = request("Which function owns annotations?");
    let reference = json.discover_ranked(question.clone())?;
    let owner = reference
        .candidates
        .iter()
        .find(|candidate| candidate.node.id == "owner:good")
        .ok_or("unique owner not recalled")?;
    assert!(
        owner
            .matched_fields
            .iter()
            .any(|field| field == "owned_identifier")
    );
    assert_eq!(owner.relevance, compass_query::RelevanceBand::Low);
    assert_eq!(owner.supporting_sources.len(), 1);
    assert!(
        owner
            .reasons
            .iter()
            .any(|reason| reason.contains("not confirmation"))
    );
    assert!(
        reference
            .candidates
            .iter()
            .filter(|candidate| candidate
                .matched_fields
                .iter()
                .any(|field| field == "owned_identifier"))
            .all(|candidate| candidate.node.id == "owner:good")
    );
    assert!(
        reference.bounded,
        "hidden ownership parents must be disclosed"
    );
    for engine in [&json, &stored] {
        assert_eq!(
            reference.candidates,
            engine.discover_ranked(question.clone())?.candidates
        );
        let mut scoped = question.clone();
        scoped.scope.push(DiscoveryScope {
            kind: DiscoveryScopeKind::Node,
            value: "owner:good".to_owned(),
        });
        let answer = engine.discover_ranked(scoped)?;
        assert_eq!(answer.candidates.len(), 1);
        assert_eq!(answer.candidates[0].node.id, "owner:good");
        assert!(
            answer.candidates[0]
                .matched_fields
                .iter()
                .any(|field| field == "owned_identifier")
        );
        for case in [
            "ambiguous",
            "no-ast",
            "inferred",
            "outside",
            "namespace",
            "nested",
            "truncated",
            "no-owner-ast",
        ] {
            let mut negative = question.clone();
            negative.scope.push(DiscoveryScope {
                kind: DiscoveryScopeKind::Node,
                value: format!("owner:{case}"),
            });
            let answer = engine.discover_ranked(negative)?;
            assert!(
                answer.candidates.iter().all(|candidate| !candidate
                    .matched_fields
                    .iter()
                    .any(|field| field == "owned_identifier")),
                "{case}"
            );
        }
        assert!(
            reference.discovery.stats.candidate_nodes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ
        );
        assert!(
            reference.discovery.stats.candidate_probes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES
        );
        let search = engine.search(SearchRequest {
            query: question.question.clone(),
            limits: CodeQueryLimits::default(),
        })?;
        assert!(
            search
                .edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::Contains
                    && edge.source == "owner:good"
                    && edge.target == "child:good")
        );
        assert!(search.nodes.iter().any(|node| node.id == "child:good"));
    }
    graph.nodes.reverse();
    graph.links.reverse();
    let reordered_path = directory.path().join("reordered.json");
    std::fs::write(&reordered_path, serde_json::to_vec(&graph)?)?;
    let reordered = open(
        &reordered_path,
        None,
        &directory.path().join("reordered-cache"),
    )?;
    assert_eq!(
        reference.candidates,
        reordered.discover_ranked(question)?.candidates
    );
    Ok(())
}

#[test]
fn broad_local_identifier_recall_bounds_children_before_parent_reads()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::EdgeKind;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links.first().cloned().ok_or("missing edge")?;
    graph.nodes = vec![support::node(
        "owner",
        NodeKind::Function,
        "opaque",
        "sample.opaque",
    )];
    graph.links.clear();
    for at in 0..40 {
        let id = format!("child:{at:02}");
        graph.nodes.push(support::node(
            &id,
            NodeKind::Variable,
            "annotations",
            &format!("sample.local{at}.annotations"),
        ));
        let mut edge = template.clone();
        edge.source = "owner".to_owned();
        edge.target = id;
        edge.kind = EdgeKind::Contains;
        edge.id = compass_model::identity::edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            None,
        );
        edge.key = edge.id.clone();
        graph.links.push(edge);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let question = request("Which function owns annotations?");
    let expected = json.discover_ranked(question.clone())?;
    for engine in [&json, &stored] {
        let answer = engine.discover_ranked(question.clone())?;
        assert_eq!(answer.candidates, expected.candidates);
        assert!(answer.bounded && answer.eligible_count.is_none());
        assert!(answer.candidates.iter().any(|candidate| {
            candidate.node.id == "owner"
                && candidate
                    .matched_fields
                    .iter()
                    .any(|field| field == "owned_identifier")
        }));
        assert!(
            answer.discovery.stats.candidate_nodes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ
        );
        assert!(
            answer.discovery.stats.candidate_probes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES
        );
    }
    Ok(())
}

#[test]
fn generic_facets_cannot_replace_subject_evidence_but_facet_listings_remain_valid()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.clear();
    graph.links.clear();
    let mut python_file = graph
        .graph
        .files
        .first()
        .cloned()
        .ok_or("missing source file")?;
    python_file.path = "src/sample.py".to_owned();
    python_file.id = compass_model::identity::file_id(&python_file.path);
    python_file.language = Some("python".to_owned());
    std::fs::write(directory.path().join(&python_file.path), b"code")?;
    graph.graph.files.push(python_file);
    for (id, name, language) in [
        ("payload", "process_payload", "rust"),
        ("generic-action", "handle", "rust"),
        ("generic-kind", "spin", "rust"),
        ("python", "opaque", "python"),
    ] {
        let mut node = support::node(id, NodeKind::Function, name, &format!("sample.{name}"));
        node.language = Some(language.to_owned());
        if language == "python" {
            node.source.as_mut().ok_or("missing source")?.file = "src/sample.py".to_owned();
            for proof in &mut node.evidence {
                for anchor in &mut proof.anchors {
                    anchor.file = "src/sample.py".to_owned();
                }
            }
        }
        graph.nodes.push(node);
    }
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    for engine in [&json, &stored] {
        for question in [
            "Which function owns payload?",
            "Which function handles payload?",
            "Which function mysteriously handles payload?",
        ] {
            let answer = engine.discover_ranked(request(question))?;
            assert_eq!(
                answer.candidates.len(),
                1,
                "{question}: {:?}",
                answer.candidates
            );
            assert_eq!(answer.candidates[0].node.id, "payload");
            assert!(
                answer.candidates[0]
                    .matched_fields
                    .iter()
                    .any(|field| !matches!(field.as_str(), "kind" | "language"))
            );
            if question.contains("mysteriously") {
                assert_eq!(
                    answer.candidates[0].relevance,
                    compass_query::RelevanceBand::Low
                );
            }
        }
        assert!(
            engine
                .discover_ranked(request("Which function handles ethereal flux?"))?
                .candidates
                .is_empty()
        );
        let all = engine.discover_ranked(request("Which functions are defined?"))?;
        assert_eq!(all.candidates.len(), 4);
        let python = engine.discover_ranked(request("Which Python functions are defined?"))?;
        assert_eq!(python.candidates.len(), 1);
        assert_eq!(python.candidates[0].node.id, "python");
        let exact = engine.discover_ranked(request("opaque"))?;
        assert_eq!(
            exact
                .candidates
                .first()
                .map(|candidate| candidate.node.id.as_str()),
            Some("python")
        );
    }
    for question in [
        "Which function owns payload?",
        "Which functions are defined?",
        "Which Python functions are defined?",
    ] {
        assert_eq!(
            json.discover_ranked(request(question))?.candidates,
            stored.discover_ranked(request(question))?.candidates
        );
    }
    graph.nodes.reverse();
    let reordered_path = directory.path().join("reordered.json");
    std::fs::write(&reordered_path, serde_json::to_vec(&graph)?)?;
    let reordered = open(
        &reordered_path,
        None,
        &directory.path().join("reordered-cache"),
    )?;
    assert_eq!(
        json.discover_ranked(request("Which function owns payload?"))?
            .candidates,
        reordered
            .discover_ranked(request("Which function owns payload?"))?
            .candidates
    );
    Ok(())
}

#[test]
fn optional_semantics_keeps_spelling_hints_low_without_a_literal_topic()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::RelevanceBand;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes = vec![
        support::node(
            "target",
            NodeKind::Function,
            "formatFastest",
            "fixture.formatFastest",
        ),
        support::node(
            "bridge",
            NodeKind::Class,
            "formatRequests",
            "fixture.formatRequests",
        ),
        support::node(
            "other",
            NodeKind::Function,
            "invoicePayment",
            "fixture.invoicePayment",
        ),
    ];
    graph.links.clear();
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let query = request("Where are fast requests formatted?");
    let mut previous = None;
    for mut engine in [json, stored] {
        let literal = engine.discover_ranked(query.clone())?;
        assert!(literal.candidates.iter().all(|c| c.node.id != "target"));
        engine.set_semantic_search(true);
        let semantic = engine.discover_ranked(query.clone())?;
        let candidate = semantic
            .candidates
            .iter()
            .find(|c| c.node.id == "target")
            .ok_or_else(|| {
                format!(
                    "optional semantic source lead missing: {semantic:?}; trace={:?}",
                    engine.trace_ranked(query.clone(), "target")
                )
            })?;
        assert!(candidate.matched_fields.iter().any(|f| f == "semantic_lsa"));
        assert!(!candidate.matched_terms.iter().any(|t| t == "request"));
        assert_eq!(candidate.relevance, RelevanceBand::Low);
        assert!(
            candidate
                .reasons
                .iter()
                .any(|r| r.contains("fast -> fastest"))
        );
        assert!(
            candidate
                .reasons
                .iter()
                .all(|r| !r.contains("inspection preference"))
        );
        assert_eq!(
            semantic.candidates,
            engine.discover_ranked(query.clone())?.candidates
        );
        engine.set_semantic_search(false);
        assert_eq!(
            literal.candidates,
            engine.discover_ranked(query.clone())?.candidates
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &semantic.candidates);
        } else {
            previous = Some(semantic.candidates);
        }
    }
    Ok(())
}
