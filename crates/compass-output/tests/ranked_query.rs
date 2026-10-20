use std::error::Error;

use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::SourceAnchor;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryDirectionSource, DiscoveryEdge, DiscoveryLimits,
    DiscoveryQueryResponse, DiscoveryTraversal, QueryNode,
};
use compass_output::{
    AgentCoverage, AgentEvidence, RANKED_QUERY_VIEW_SCHEMA, RankedNameMatch,
    build_ranked_query_view, build_ranked_query_view_with_context, render_ranked_query_text,
};
use compass_query::{RankedCandidate, RankedDiscoveryResponse, RelevanceBand};

fn response() -> RankedDiscoveryResponse {
    let candidates = (0..7)
        .map(|index| RankedCandidate {
            // Reverse identity order deliberately: display order is relevance order.
            node: QueryNode {
                id: format!("node:{}", 9 - index),
                kind: NodeKind::Function,
                roles: Vec::new(),
                name: format!("implementation_{index}"),
                qualified_name: format!("app::implementation_{index}"),
                language: Some("rust".to_owned()),
                framework: None,
                source: Some(SourceAnchor {
                    file: format!("src/impl_{index}.rs"),
                    start_byte: 0,
                    end_byte: 1,
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    end_column: 1,
                }),
                details: None,
                evidence: Vec::new(),
            },
            score: (10 - index) as f64,
            relevance: RelevanceBand::High,
            matched_terms: vec!["implementation".to_owned()],
            matched_fields: vec!["signature".to_owned()],
            reasons: vec!["Request subject and action match the signature.".to_owned()],
            supporting_sources: Vec::new(),
            excerpts: Vec::new(),
            call_link: None,
        })
        .collect::<Vec<_>>();
    RankedDiscoveryResponse {
        discovery: DiscoveryQueryResponse {
            schema: "compass.query.discovery/1".to_owned(),
            question: "where is the request implementation".to_owned(),
            selected_direction: DiscoveryDirection::Both,
            direction_source: DiscoveryDirectionSource::Neutral,
            relation_contexts: Vec::new(),
            scope: Vec::new(),
            traversal: DiscoveryTraversal::Bfs,
            seeds: Vec::new(),
            nodes: candidates
                .iter()
                .take(3)
                .map(|candidate| candidate.node.clone())
                .collect(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            limits: DiscoveryLimits::default(),
            stats: Default::default(),
            omissions: Default::default(),
            truncated: false,
        },
        candidates,
        eligible_count: Some(7),
        bounded: false,
    }
}

#[test]
fn candidate_bounds_do_not_claim_that_a_traversal_was_truncated() -> Result<(), Box<dyn Error>> {
    let mut input = response();
    input.bounded = true;
    input.discovery.truncated = true;
    input.eligible_count = None;
    assert!(input.discovery.edges.is_empty());
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    assert!(view.candidate_recall_bounded);
    assert!(view.response_truncated);
    let encoded = serde_json::to_value(&view)?;
    assert_eq!(encoded["responseTruncated"], true);
    assert!(encoded.get("traversalTruncated").is_none());
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("candidate recall bounded; response truncated"));
    assert!(!text.contains("traversal truncated"));
    Ok(())
}

#[test]
fn shortlist_preserves_ranks_four_and_five_independent_of_graph_seeds() -> Result<(), Box<dyn Error>>
{
    let input = response();
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    assert_eq!(view.schema, RANKED_QUERY_VIEW_SCHEMA);
    assert_eq!(view.candidates.len(), 5);
    assert_eq!(
        view.candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        ["node:9", "node:8", "node:7", "node:6", "node:5"]
    );
    assert_eq!(view.retained_count, 7);
    assert_eq!(view.eligible_count, Some(7));
    assert!(view.more_available);
    assert_eq!(view.graph_coverage, AgentCoverage::Unknown);
    // High relevance must not imply an exact name or proven structural evidence.
    assert_eq!(view.candidates[0].name_match, RankedNameMatch::Approximate);
    assert_eq!(view.candidates[0].evidence, AgentEvidence::None);
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("app::implementation_4"));
    assert!(text.contains("src/impl_4.rs:1"));
    assert!(text.len() < 800 * 4);
    assert!(text.contains("--scope 'source:src/impl_0.rs'"));
    let encoded = serde_json::to_vec(&view)?;
    assert_eq!(compass_output::RankedQueryView::from_json(&encoded)?, view);
    let mut future = view.clone();
    future.schema = "compass.query.ranked-view/99".to_owned();
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&future)?).is_err());
    let mut invalid_anchor = view.clone();
    invalid_anchor.candidates[0]
        .source
        .as_mut()
        .ok_or("source")?
        .start_line = 0;
    assert!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&invalid_anchor)?).is_err()
    );
    let mut too_small = input.clone();
    too_small.discovery.limits.max_response_bytes = 64;
    assert!(build_ranked_query_view(&too_small, "generation", "digest").is_err());
    Ok(())
}

#[test]
fn default_budget_compacts_explanations_before_dropping_the_fifth_candidate()
-> Result<(), Box<dyn Error>> {
    let mut input = response();
    input.bounded = true;
    input.discovery.truncated = true;
    input.eligible_count = None;
    for (at, candidate) in input.candidates.iter_mut().enumerate() {
        candidate.node.qualified_name =
            format!("src.marshmallow.schema.Schema::implementation_{at}");
        let source = candidate.node.source.as_mut().ok_or("missing source")?;
        source.file = "src/marshmallow/schema.py".to_owned();
        source.start_line = 815 + u32::try_from(at)?;
        source.end_line = source.start_line;
        candidate.relevance = RelevanceBand::Low;
        candidate.reasons = vec![
            "Negative qualification is not established by lexical evidence. These are inspection candidates, not confirmation of the requested condition".to_owned(),
            "Indexed code_reference, name, owned_document, path, qualified_name, signature evidence matches 7 of 9 query concepts".to_owned(),
            "Partial indexed evidence; missing owner-specific context: dictionary, error, callback. The requested behavior is not established by this candidate".to_owned(),
        ];
        candidate.supporting_sources = candidate.node.source.clone().into_iter().collect();
    }
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    let original = view.clone();
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("Ranked candidates: 5 shown"));
    for at in 0..5 {
        assert!(text.contains(&format!(
            "src.marshmallow.schema.Schema::implementation_{at}"
        )));
        assert!(text.contains(&format!("src/marshmallow/schema.py:{}", 815 + at)));
    }
    assert!(text.contains("relevance=low; name=approximate; evidence=none"));
    assert!(text.contains("Negative qualification is not established"));
    assert!(text.contains("Reasons abbreviated"));
    assert!(text.contains("--format json"));
    assert!(text.contains("Coverage: unknown; candidate recall bounded; response truncated"));
    assert!(text.contains("Narrow with"));
    assert!(text.len() < 800 * 4);
    assert_eq!(view, original);
    let json = serde_json::to_string(&view)?;
    assert!(json.contains("dictionary, error, callback"));
    Ok(())
}

#[test]
fn tight_budget_preserves_first_quote_before_lower_ranked_quotes() -> Result<(), Box<dyn Error>> {
    use compass_query::{RankedExcerptKind, RankedSourceExcerpt};
    let mut input = response();
    input.bounded = true;
    input.discovery.truncated = true;
    input.eligible_count = None;
    let quote = "format!(\n                \"{}: {}\\n  --> {source_name}:{line}:{column}{help}\",\n                diagnostic.code(),\n                diagnostic.message()\n            )";
    for (at, candidate) in input.candidates.iter_mut().enumerate() {
        candidate.node.qualified_name =
            format!("application::diagnostic_commands::format_diagnostic_{at}");
        candidate.relevance = RelevanceBand::Low;
        candidate.reasons = vec!["Nested lexical code context was omitted by shared inspection limits; no execution is inferred; Indexed body_identifier, name, path, qualified_name, signature evidence matches query concepts".to_owned()];
        let source = candidate.node.source.as_mut().ok_or("source")?;
        source.file = "crates/application-cli/src/diagnostic_commands.rs".to_owned();
        source.end_byte = quote.len() as u64 + 2;
        source.end_line = 5;
        source.end_column = 14;
        let mut quoted = source.clone();
        quoted.start_byte = 1;
        quoted.start_column = 1;
        quoted.end_byte -= 1;
        quoted.end_column -= 1;
        candidate.excerpts = vec![RankedSourceExcerpt {
            kind: RankedExcerptKind::NestedCallableCode,
            source: quoted.clone(),
            text: quote.to_owned(),
            clipped: false,
            nested_scope: Some(quoted),
        }];
    }
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    let original = view.clone();
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("Ranked candidates: 5 shown"), "{text}");
    let first = text.split("2. application::").next().ok_or("first row")?;
    assert!(first.contains("diagnostic.message()"), "{text}");
    assert!(!first.contains("[excerpt clipped]"), "{text}");
    assert!(
        text.contains("Lower-ranked quotes omitted for budget"),
        "{text}"
    );
    assert!(text.len() < 3200);
    assert_eq!(view, original);
    Ok(())
}

#[test]
fn budgets_keep_rows_whole_and_reserve_limits_and_refinement() -> Result<(), Box<dyn Error>> {
    let mut input = response();
    input.candidates[0].node.qualified_name = "é東京🙂".repeat(80);
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    for budget in [500, 650, 800] {
        let text = render_ranked_query_text(&view, budget)?;
        assert!(text.len() < budget * 4);
        assert!(text.contains("Coverage:"));
        assert!(text.contains("Narrow with"));
        assert!(text.contains("relevance=high; name=approximate; evidence=none"));
    }
    assert!(render_ranked_query_text(&view, 1).is_err());
    Ok(())
}

#[test]
fn empty_results_abstain_without_claiming_absence() -> Result<(), Box<dyn Error>> {
    let mut input = response();
    input.candidates.clear();
    input.eligible_count = None;
    input.bounded = true;
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("No supporting indexed evidence found"));
    assert!(text.contains("does not prove the behavior is absent"));
    assert!(view.candidate_recall_bounded);
    assert!(view.more_available);
    Ok(())
}

#[test]
fn optional_context_preserves_direction_occurrences_and_source_anchors()
-> Result<(), Box<dyn Error>> {
    let mut input = response();
    let site = input.candidates[0].node.source.clone();
    input.discovery.edges = (0..10)
        .map(|index| DiscoveryEdge {
            id: Some(format!("edge:{index}")),
            source: "node:8".to_owned(),
            target: "node:9".to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: site.clone(),
            details: None,
            evidence: Vec::new(),
            context: Some("call".to_owned()),
        })
        .collect();
    let plain = build_ranked_query_view(&input, "generation", "digest")?;
    assert!(plain.context.is_none());
    let view = build_ranked_query_view_with_context(&input, "generation", "digest", true)?;
    assert_eq!(view.candidates, plain.candidates);
    let context = view.context.as_ref().ok_or("context")?;
    assert_eq!(context.edges.len(), 8);
    assert_eq!(context.omitted_edges, 2);
    assert!(context.truncated);
    assert_eq!(context.edges[0].source, "node:8");
    assert_eq!(context.edges[0].target, "node:9");
    assert_eq!(context.edges[0].relationship_site, site);
    assert_ne!(context.edges[0].id, context.edges[1].id);
    let text = render_ranked_query_text(&view, 800)?;
    assert!(
        text.find("5.").ok_or("fifth candidate")?
            < text.find("Context (one hop)").ok_or("context")?
    );
    assert!(text.len() < 800 * 4);
    assert!(text.contains("--calls-->"));
    let encoded = serde_json::to_vec(&view)?;
    assert_eq!(compass_output::RankedQueryView::from_json(&encoded)?, view);
    Ok(())
}

#[test]
fn document_support_is_bounded_and_identity_is_explicit() -> Result<(), Box<dyn Error>> {
    let mut input = response();
    let source = input.candidates[0].node.source.clone().ok_or("source")?;
    input.candidates[0].supporting_sources = vec![source.clone(); 6];
    input.candidates[0]
        .matched_fields
        .push("owned_document".to_owned());
    input.candidates[0]
        .matched_fields
        .push("exact_name".to_owned());
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    assert_eq!(view.candidates[0].name_match, RankedNameMatch::ExactName);
    assert_eq!(view.candidates[0].supporting_sources, vec![source; 4]);
    assert_eq!(view.candidates[0].omitted_supporting_sources, 2);
    assert!(
        view.candidates[0]
            .retrieval_kinds
            .contains(&compass_output::RankedRetrievalKind::Document)
    );
    assert!(render_ranked_query_text(&view, 800)?.contains("supporting passage: src/impl_0.rs:1"));
    Ok(())
}

#[test]
fn candidate_text_distinguishes_resource_matches_from_implementation_declarations()
-> Result<(), Box<dyn Error>> {
    let mut input = response();
    input.candidates[0].node.kind = NodeKind::Resource;
    input.candidates[0].node.name = "Cookie reference".to_owned();
    input.candidates[0].node.qualified_name = "Cookie reference".to_owned();
    input.candidates[0].matched_fields = vec!["document".to_owned()];
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.starts_with("Ranked candidates: 5 shown; 7 eligible."));
    assert!(text.contains("via=document; kind=resource]"));
    assert!(text.contains("via=signature; kind=function]"));
    assert_eq!(view.candidates[0].kind, NodeKind::Resource);
    assert!(text.len() < 800 * 4);
    Ok(())
}

#[test]
fn source_quotes_are_bounded_escaped_and_do_not_displace_the_shortlist()
-> Result<(), Box<dyn Error>> {
    use compass_query::{RankedExcerptKind, RankedSourceExcerpt};
    let mut input = response();
    for candidate in &mut input.candidates {
        let text = format!("// payload \u{202e} \x1b {}", "x".repeat(470));
        let mut source = candidate.node.source.clone().ok_or("source")?;
        source.end_byte = u64::try_from(text.len())?;
        source.end_column = u32::try_from(text.len())?;
        candidate.node.source = Some(source.clone());
        candidate.excerpts = vec![RankedSourceExcerpt {
            kind: RankedExcerptKind::CallableComment,
            source,
            text,
            clipped: false,
            nested_scope: None,
        }];
    }
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("Ranked candidates: 5 shown"));
    assert!(text.len() < 3200);
    assert!(!text.contains('\u{202e}'));
    assert!(!text.contains('\x1b'));
    assert!(text.contains("Comment @") || text.contains("Quotes omitted for budget"));
    let encoded = serde_json::to_vec(&view)?;
    assert_eq!(compass_output::RankedQueryView::from_json(&encoded)?, view);
    let mut invalid = view.clone();
    invalid.candidates[0].excerpts[0].source.start_byte = 1;
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&invalid)?).is_err());
    let mut outside = view.clone();
    let quote = &mut outside.candidates[0].excerpts[0];
    quote.source.start_byte += 1;
    quote.source.end_byte += 1;
    quote.source.start_column += 1;
    quote.source.end_column += 1;
    assert!(quote.is_coherent());
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&outside)?).is_err());
    let mut foreign = view.clone();
    foreign.candidates[0].excerpts[0].source.file = "src/neighbor.rs".to_owned();
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&foreign)?).is_err());
    let mut normalized = view.clone();
    normalized.candidates[0].excerpts[0].text = "normalized".to_owned();
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&normalized)?).is_err());
    Ok(())
}

#[test]
fn legacy_ranked_views_remain_readable_and_unknown_major_is_explicit() -> Result<(), Box<dyn Error>>
{
    let mut legacy = build_ranked_query_view(&response(), "generation", "digest")?;
    legacy.schema = "compass.query.ranked-view/1".to_owned();
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&legacy)?)?,
        legacy
    );
    let mut legacy_with_quotes = serde_json::to_value(&legacy)?;
    legacy_with_quotes["candidates"][0]["excerpts"] = serde_json::json!([]);
    assert!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&legacy_with_quotes)?)
            .is_err()
    );
    let error =
        compass_output::RankedQueryView::from_json(br#"{"schema":"compass.query.ranked-view/99"}"#)
            .err()
            .ok_or("future major accepted")?;
    assert!(
        error
            .to_string()
            .contains("unsupported ranked query view schema")
    );
    Ok(())
}

#[test]
fn code_quotes_require_view_three_and_legacy_comment_views_stay_readable()
-> Result<(), Box<dyn Error>> {
    use compass_query::{RankedExcerptKind, RankedSourceExcerpt};
    let mut input = response();
    let candidate = &mut input.candidates[0];
    let text = "normalize_payload(items)".to_owned();
    let mut source = candidate.node.source.clone().ok_or("source")?;
    source.end_byte = source.start_byte + text.len() as u64;
    source.end_line = source.start_line;
    source.end_column = source.start_column + text.len() as u32;
    candidate.node.source = Some(source.clone());
    candidate.excerpts = vec![RankedSourceExcerpt {
        kind: RankedExcerptKind::CallableCode,
        source,
        text,
        clipped: false,
        nested_scope: None,
    }];
    let mut view = build_ranked_query_view(&input, "generation", "digest")?;
    assert_eq!(view.schema, "compass.query.ranked-view/4");
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    assert!(render_ranked_query_text(&view, 1600)?.contains("Code @"));
    view.schema = "compass.query.ranked-view/2".to_owned();
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?).is_err());
    view.candidates[0].excerpts[0].kind = RankedExcerptKind::CallableComment;
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    Ok(())
}

#[test]
fn nested_code_is_separate_lexical_context_and_legacy_views_reject_it() -> Result<(), Box<dyn Error>>
{
    use compass_query::{RankedExcerptKind, RankedSourceExcerpt};
    let mut input = response();
    let candidate = &mut input.candidates[0];
    let mut parent = candidate.node.source.clone().ok_or("source")?;
    parent.end_byte = parent.start_byte + 100;
    parent.end_line = parent.start_line;
    parent.end_column = parent.start_column + 100;
    candidate.node.source = Some(parent.clone());
    let mut scope = parent.clone();
    scope.start_byte += 10;
    scope.end_byte -= 10;
    scope.start_column += 10;
    scope.end_column -= 10;
    let text = "payload_error(item)".to_owned();
    let mut source = scope.clone();
    source.start_byte += 1;
    source.start_column += 1;
    source.end_byte = source.start_byte + text.len() as u64;
    source.end_column = source.start_column + text.len() as u32;
    candidate.excerpts = vec![RankedSourceExcerpt {
        kind: RankedExcerptKind::NestedCallableCode,
        source,
        text,
        clipped: false,
        nested_scope: Some(scope),
    }];
    let view = build_ranked_query_view(&input, "generation", "digest")?;
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    let text = render_ranked_query_text(&view, 800)?;
    assert!(text.contains("Nested lexical code (execution unverified)"));
    for schema in [
        "compass.query.ranked-view/1",
        "compass.query.ranked-view/2",
        "compass.query.ranked-view/3",
    ] {
        let mut bad = view.clone();
        bad.schema = schema.to_owned();
        assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&bad)?).is_err());
    }
    let mut bad = view.clone();
    bad.candidates[0].excerpts[0].nested_scope = None;
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&bad)?).is_err());
    let mut bad = view.clone();
    bad.candidates[0].excerpts[0].nested_scope = bad.candidates[0].source.clone();
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&bad)?).is_err());
    let mut legacy = view;
    legacy.schema = "compass.query.ranked-view/3".to_owned();
    legacy.candidates[0].excerpts[0].kind = RankedExcerptKind::CallableCode;
    legacy.candidates[0].excerpts[0].nested_scope = None;
    compass_output::RankedQueryView::from_json(&serde_json::to_vec(&legacy)?)?;
    Ok(())
}

fn attach_call_work(response: &mut RankedDiscoveryResponse, probes: u64) {
    use compass_model::query_contract::{DISCOVERY_QUERY_SCHEMA_V2, MAX_INDEXED_CANDIDATE_PROBES};
    response.discovery.schema = DISCOVERY_QUERY_SCHEMA_V2.to_owned();
    response.discovery.stats.call_link_work = Some(compass_query::RankedCallWork {
        schema: compass_query::CALL_LINK_WORK_SCHEMA.to_owned(),
        candidate_probes: probes,
        candidate_probe_limit: compass_query::MAX_CALL_LINK_CANDIDATE_PROBES,
        indexed_candidate_probes: response.discovery.stats.candidate_probes,
        indexed_candidate_probe_limit: MAX_INDEXED_CANDIDATE_PROBES,
    });
}

#[test]
fn call_link_projection_is_explicit_bounded_and_versioned() -> Result<(), Box<dyn Error>> {
    use compass_query::{RankedCallEndpoint, RankedCallHint, RankedCallHop, RankedCallLink};
    let mut response = response();
    let root = &response.candidates[0].node;
    let target = &response.candidates[1].node;
    let endpoint = |node: &QueryNode| -> Result<RankedCallEndpoint, Box<dyn Error>> {
        Ok(RankedCallEndpoint {
            id: node.id.clone(),
            name: node.qualified_name.clone(),
            source: node.source.clone().ok_or("missing source")?,
        })
    };
    let link = RankedCallLink {
        schema: "compass.query.call-link/1".to_owned(),
        root: endpoint(root)?,
        hops: vec![RankedCallHop {
            edge_id: "edge:call".to_owned(),
            source_id: root.id.clone(),
            target: endpoint(target)?,
            site: root.source.clone().ok_or("missing root source")?,
            occurrence_rule: Some("fixture-call".to_owned()),
            extractor: "native-fixture".to_owned(),
            rule: Some("direct-call".to_owned()),
        }],
        hint: RankedCallHint {
            requested: "detect".to_owned(),
            spelling: "validate".to_owned(),
        },
    };
    attach_call_work(&mut response, 1);
    response.candidates[1].call_link = Some(link.clone());
    response.candidates[1].relevance = RelevanceBand::Low;
    let view = build_ranked_query_view(&response, "graph", "digest")?;
    assert_eq!(view.schema, compass_output::RANKED_QUERY_LINKED_VIEW_SCHEMA);
    assert_eq!(view.candidates[1].call_link.as_ref(), Some(&link));
    let serialized = serde_json::to_vec(&view)?;
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serialized)?,
        view
    );
    let mut old = serde_json::to_value(&view)?;
    for schema in [
        "compass.query.ranked-view/4",
        "compass.query.ranked-view/5",
        "compass.query.ranked-view/6",
    ] {
        old["schema"] = serde_json::json!(schema);
        assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&old)?).is_err());
    }
    for failure in [
        "too_many_hops",
        "broken_chain",
        "wrong_owner",
        "strong_relevance",
        "unknown_path_schema",
    ] {
        let mut changed = response.clone();
        if failure == "strong_relevance" {
            changed.candidates[1].relevance = RelevanceBand::High;
        }
        if let Some(link) = &mut changed.candidates[1].call_link {
            match failure {
                "too_many_hops" => link.hops = vec![link.hops[0].clone(); 5],
                "broken_chain" => link.hops[0].source_id = "not-the-root".to_owned(),
                "wrong_owner" => link.hops[0].target.source.file = "unrelated.py".to_owned(),
                "unknown_path_schema" => link.schema = "compass.query.call-link/99".to_owned(),
                _ => {}
            }
        }
        assert!(
            build_ranked_query_view(&changed, "graph", "digest").is_err(),
            "{failure}"
        );
    }
    // Even a discarded sixth candidate cannot bypass compact-path input bounds.
    response.candidates[5].call_link = Some(link);
    if let Some(link) = &mut response.candidates[5].call_link {
        link.hops = vec![link.hops[0].clone(); 5];
    }
    assert!(build_ranked_query_view(&response, "graph", "digest").is_err());
    Ok(())
}

fn shared_call_paths() -> Result<RankedDiscoveryResponse, Box<dyn Error>> {
    use compass_query::{RankedCallEndpoint, RankedCallHint, RankedCallHop, RankedCallLink};
    let mut answer = response();
    attach_call_work(&mut answer, 2);
    let endpoint = |node: &QueryNode| -> Result<RankedCallEndpoint, Box<dyn Error>> {
        Ok(RankedCallEndpoint {
            id: node.id.clone(),
            name: node.qualified_name.clone(),
            source: node.source.clone().ok_or("missing source")?,
        })
    };
    let root = endpoint(&answer.candidates[0].node)?;
    let mut intermediate = root.clone();
    intermediate.id = "intermediate".to_owned();
    intermediate.name = "app::bridge".to_owned();
    let prefix = RankedCallHop {
        edge_id: "shared-prefix".to_owned(),
        source_id: root.id.clone(),
        target: intermediate.clone(),
        site: root.source.clone(),
        occurrence_rule: Some("call".to_owned()),
        extractor: "native-fixture".to_owned(),
        rule: None,
    };
    for index in [1, 2] {
        answer.candidates[index].relevance = RelevanceBand::Low;
        answer.candidates[index].call_link = Some(RankedCallLink {
            schema: "compass.query.call-link/1".to_owned(),
            root: root.clone(),
            hops: vec![
                prefix.clone(),
                RankedCallHop {
                    edge_id: format!("terminal-{index}"),
                    source_id: intermediate.id.clone(),
                    target: endpoint(&answer.candidates[index].node)?,
                    site: intermediate.source.clone(),
                    occurrence_rule: Some("call".to_owned()),
                    extractor: "native-fixture".to_owned(),
                    rule: None,
                },
            ],
            hint: RankedCallHint {
                requested: "detect".to_owned(),
                spelling: "validate".to_owned(),
            },
        });
    }
    Ok(answer)
}

#[test]
fn call_link_depth_and_identity_contracts_reject_forged_views() -> Result<(), Box<dyn Error>> {
    let answer = shared_call_paths()?;
    let view = build_ranked_query_view(&answer, "graph", "digest")?;
    // Sharing the exact same prefix is legitimate.
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    let mut accepted = Vec::new();
    for failure in [
        "request_depth",
        "duplicate_edge",
        "intermediate_conflict",
        "occurrence_conflict",
        "displayed_intermediate_conflict",
        "root_after_helper",
        "oversized_site",
    ] {
        let mut input = answer.clone();
        match failure {
            "request_depth" => input.discovery.limits.max_depth = 1,
            "duplicate_edge" => {
                let link = input.candidates[1]
                    .call_link
                    .as_mut()
                    .ok_or("missing link")?;
                link.hops[1].edge_id = link.hops[0].edge_id.clone();
            }
            "intermediate_conflict" => {
                input.candidates[2]
                    .call_link
                    .as_mut()
                    .ok_or("missing link")?
                    .hops[0]
                    .target
                    .name = "app::different_bridge".to_owned();
            }
            "occurrence_conflict" => {
                input.candidates[2]
                    .call_link
                    .as_mut()
                    .ok_or("missing link")?
                    .hops[0]
                    .extractor = "another-extractor".to_owned();
            }
            "displayed_intermediate_conflict" => {
                input.candidates[3].node.id = "intermediate".to_owned();
                input.candidates[3].node.qualified_name = "app::different_bridge".to_owned();
            }
            "root_after_helper" => input.candidates.swap(0, 1),
            "oversized_site" => {
                input.candidates[1]
                    .call_link
                    .as_mut()
                    .ok_or("missing link")?
                    .hops[0]
                    .site
                    .file = "x".repeat(4097);
            }
            _ => return Err("unknown test case".into()),
        }
        // Exercise the decoder separately from producer admission.
        let mut forged = view.clone();
        forged.limits = input.discovery.limits.clone();
        for (index, candidate) in input.candidates.iter().take(5).enumerate() {
            forged.candidates[index].id = candidate.node.id.clone();
            forged.candidates[index].name = candidate.node.qualified_name.clone();
            forged.candidates[index].source = candidate.node.source.clone();
            forged.candidates[index].call_link = candidate.call_link.clone();
        }
        if failure == "root_after_helper" {
            forged = view.clone();
            forged.candidates.swap(0, 1);
            for (index, candidate) in forged.candidates.iter_mut().enumerate() {
                candidate.rank = index as u32 + 1;
            }
        }
        if compass_output::RankedQueryView::from_json(&serde_json::to_vec(&forged)?).is_ok() {
            accepted.push(format!("reader: {failure}"));
        }
        if build_ranked_query_view(&input, "graph", "digest").is_ok() {
            accepted.push(format!("producer: {failure}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "forged paths were accepted: {accepted:?}"
    );
    Ok(())
}

#[test]
fn compact_and_context_occurrences_share_one_identity() -> Result<(), Box<dyn Error>> {
    use compass_model::provenance::{
        EvidenceConfidence, EvidenceOrigin, OccurrenceRule, ResolutionCandidate, ResolutionState,
    };
    use compass_model::query_contract::{QueryEvidence, QueryEvidenceLayer};
    let mut answer = shared_call_paths()?;
    let prefix = answer.candidates[1]
        .call_link
        .as_ref()
        .ok_or("missing link")?
        .hops[0]
        .clone();
    let mut intermediate = answer.candidates[0].node.clone();
    intermediate.id = prefix.target.id.clone();
    intermediate.name = prefix.target.name.clone();
    intermediate.qualified_name = prefix.target.name.clone();
    intermediate.source = Some(prefix.target.source.clone());
    answer.discovery.nodes = answer
        .candidates
        .iter()
        .map(|candidate| candidate.node.clone())
        .chain([intermediate])
        .collect();
    answer.discovery.edges = vec![DiscoveryEdge {
        id: Some(prefix.edge_id.clone()),
        source: prefix.source_id.clone(),
        target: prefix.target.id.clone(),
        kind: EdgeKind::Calls,
        occurrence_rule: prefix
            .occurrence_rule
            .as_ref()
            .and_then(OccurrenceRule::new),
        relationship_site: Some(prefix.site.clone()),
        details: None,
        context: None,
        evidence: vec![QueryEvidence {
            layer: QueryEvidenceLayer::StructuralGraph,
            origin: EvidenceOrigin::Ast,
            extractor: prefix.extractor.clone(),
            confidence: EvidenceConfidence::Exact,
            anchor: Some(prefix.site.clone()),
            rule: prefix.rule.clone(),
            wiring_site: None,
            resolution: ResolutionState::Exact,
            candidates: Vec::new(),
        }],
    }];
    let view = build_ranked_query_view_with_context(&answer, "graph", "digest", true)?;
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    // A compact path needs no optional context copy.
    build_ranked_query_view(&answer, "graph", "digest")?;
    // Nonmatching additional proofs do not erase the selected exact attribution.
    let mut additional = answer.clone();
    let mut unrelated = additional.discovery.edges[0].evidence[0].clone();
    unrelated.origin = EvidenceOrigin::Config;
    unrelated.confidence = EvidenceConfidence::Inferred;
    unrelated.resolution = ResolutionState::Unresolved;
    unrelated.extractor = "unrelated-proof".to_owned();
    additional.discovery.edges[0].evidence.insert(0, unrelated);
    let enriched = build_ranked_query_view_with_context(&additional, "graph", "digest", true)?;
    compass_output::RankedQueryView::from_json(&serde_json::to_vec(&enriched)?)?;
    let exact_candidate = ResolutionCandidate {
        node_id: prefix.target.id.clone(),
        reason: "source-backed target".to_owned(),
        confidence: EvidenceConfidence::Exact,
        score: None,
        anchor: Some(prefix.target.source.clone()),
    };
    let mut single = answer.clone();
    single.discovery.edges[0].evidence[0].candidates = vec![exact_candidate.clone()];
    let single = build_ranked_query_view_with_context(&single, "graph", "digest", true)?;
    compass_output::RankedQueryView::from_json(&serde_json::to_vec(&single)?)?;
    let mut accepted = Vec::new();
    for failure in [
        "source",
        "target",
        "kind",
        "site",
        "rule",
        "extractor",
        "proof_rule",
        "proof_anchor",
        "proof_origin",
        "proof_confidence",
        "ambiguous",
        "unresolved",
        "multiple_candidates",
        "foreign_candidate",
        "inferred_candidate",
        "empty_reason",
        "invalid_candidate_anchor",
        "duplicate_context",
        "context_node",
    ] {
        let mut input = answer.clone();
        let edge = &mut input.discovery.edges[0];
        match failure {
            "source" => edge.source = input.candidates[1].node.id.clone(),
            "target" => edge.target = input.candidates[1].node.id.clone(),
            "kind" => edge.kind = EdgeKind::References,
            "site" => edge.relationship_site.as_mut().ok_or("site")?.file = "another.rs".to_owned(),
            "rule" => edge.occurrence_rule = None,
            "extractor" => edge.evidence[0].extractor = "another".to_owned(),
            "proof_rule" => edge.evidence[0].rule = Some("another".to_owned()),
            "proof_anchor" => edge.evidence[0].anchor = None,
            "proof_origin" => edge.evidence[0].origin = EvidenceOrigin::Config,
            "proof_confidence" => edge.evidence[0].confidence = EvidenceConfidence::Inferred,
            "ambiguous" => edge.evidence[0].resolution = ResolutionState::Ambiguous,
            "unresolved" => edge.evidence[0].resolution = ResolutionState::Unresolved,
            "multiple_candidates" => edge.evidence[0].candidates = vec![exact_candidate.clone(); 2],
            "foreign_candidate"
            | "inferred_candidate"
            | "empty_reason"
            | "invalid_candidate_anchor" => {
                let mut candidate = exact_candidate.clone();
                match failure {
                    "foreign_candidate" => candidate.node_id = "other".to_owned(),
                    "inferred_candidate" => candidate.confidence = EvidenceConfidence::Inferred,
                    "empty_reason" => candidate.reason = " ".to_owned(),
                    "invalid_candidate_anchor" => {
                        candidate
                            .anchor
                            .as_mut()
                            .ok_or("candidate anchor")?
                            .start_byte = 2
                    }
                    _ => return Err("unknown candidate case".into()),
                }
                edge.evidence[0].candidates = vec![candidate];
            }
            "duplicate_context" => {
                let mut duplicate = edge.clone();
                duplicate.context = Some("contradictory retained occurrence".to_owned());
                input.discovery.edges.push(duplicate);
            }
            "context_node" => {
                let node = input
                    .discovery
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == prefix.target.id)
                    .ok_or("intermediate")?;
                node.qualified_name = "different".to_owned();
            }
            _ => return Err("unknown test case".into()),
        }
        let mut forged = view.clone();
        let context = forged.context.as_mut().ok_or("context")?;
        context.edges = input.discovery.edges.clone();
        context.nodes = input.discovery.nodes.clone();
        if compass_output::RankedQueryView::from_json(&serde_json::to_vec(&forged)?).is_ok() {
            accepted.push(format!("reader: {failure}"));
        }
        if build_ranked_query_view_with_context(&input, "graph", "digest", true).is_ok() {
            accepted.push(format!("producer: {failure}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "contradictory occurrences were accepted: {accepted:?}"
    );
    Ok(())
}

#[test]
fn call_link_text_budgets_preserve_root_before_helper() -> Result<(), Box<dyn Error>> {
    let view = build_ranked_query_view(&shared_call_paths()?, "graph", "digest")?;
    let mut saw_helper = false;
    let mut saw_root_only = false;
    for budget in [150, 200, 250, 300, 400, 500, 650, 800, 1000, 1500] {
        let Ok(text) = render_ranked_query_text(&view, budget) else {
            continue;
        };
        let root = text
            .find("1. app::implementation_0")
            .ok_or("displayed prefix lost root")?;
        if let Some(helper) = text.find("Call path (execution unverified)") {
            assert!(root < helper);
            saw_helper = true;
        } else {
            saw_root_only = true;
        }
        assert!(text.len() < budget * 4);
    }
    assert!(
        saw_helper && saw_root_only,
        "both budget projections must be exercised"
    );
    Ok(())
}

#[test]
fn call_work_is_reported_without_a_selected_path_and_validated_before_projection()
-> Result<(), Box<dyn Error>> {
    let mut input = response();
    attach_call_work(&mut input, 40);
    let view = build_ranked_query_view(&input, "graph", "digest")?;
    assert_eq!(view.schema, compass_output::RANKED_QUERY_LINKED_VIEW_SCHEMA);
    assert!(
        view.candidates
            .iter()
            .all(|candidate| candidate.call_link.is_none())
    );
    assert!(render_ranked_query_text(&view, 1000)?.contains("Call-linked lookups: 40/40"));
    let value = serde_json::to_value(&view)?;
    assert_eq!(value["callLinkWork"]["candidateProbes"], 40);
    for failure in [
        "absent",
        "unknown",
        "overcount",
        "changed_limit",
        "old_major",
    ] {
        let mut forged = value.clone();
        match failure {
            "absent" => {
                forged
                    .as_object_mut()
                    .ok_or("view object")?
                    .remove("callLinkWork");
            }
            "unknown" => {
                forged["callLinkWork"]["schema"] =
                    serde_json::json!("compass.query.call-link-work/2")
            }
            "overcount" => forged["callLinkWork"]["candidateProbes"] = serde_json::json!(41),
            "changed_limit" => {
                forged["callLinkWork"]["candidateProbeLimit"] = serde_json::json!(41)
            }
            _ => forged["schema"] = serde_json::json!("compass.query.ranked-view/4"),
        }
        assert!(
            compass_output::RankedQueryView::from_json(&serde_json::to_vec(&forged)?).is_err(),
            "{failure}"
        );
    }
    input.discovery.stats.candidate_probes += 1;
    assert!(build_ranked_query_view(&input, "graph", "digest").is_err());
    let linked = shared_call_paths()?;
    let mut view = build_ranked_query_view(&linked, "graph", "digest")?;
    view.call_link_work
        .as_mut()
        .ok_or("call work")?
        .candidate_probes = 1;
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?).is_err());
    Ok(())
}

#[test]
fn helper_previews_require_the_linked_profile_and_an_actual_helper_path()
-> Result<(), Box<dyn Error>> {
    use compass_query::{RankedExcerptKind, RankedSourceExcerpt};
    let mut answer = shared_call_paths()?;
    answer.candidates[1].excerpts = vec![RankedSourceExcerpt {
        kind: RankedExcerptKind::LinkedCallablePreview,
        source: answer.candidates[1].node.source.clone().ok_or("source")?,
        text: "x".to_owned(),
        clipped: false,
        nested_scope: None,
    }];
    let view = build_ranked_query_view(&answer, "graph", "digest")?;
    assert_eq!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?,
        view
    );
    assert!(
        render_ranked_query_text(&view, 4000)?
            .contains("Helper code preview (behavior unverified)")
    );
    for schema in [
        "compass.query.ranked-view/1",
        "compass.query.ranked-view/2",
        "compass.query.ranked-view/3",
        "compass.query.ranked-view/4",
    ] {
        let mut forged = view.clone();
        forged.schema = schema.to_owned();
        forged.call_link_work = None;
        for candidate in &mut forged.candidates {
            candidate.call_link = None;
        }
        assert!(
            compass_output::RankedQueryView::from_json(&serde_json::to_vec(&forged)?).is_err(),
            "{schema}"
        );
    }
    let mut root_preview = view.clone();
    root_preview.candidates[0].excerpts = vec![RankedSourceExcerpt {
        source: root_preview.candidates[0].source.clone().ok_or("source")?,
        ..root_preview.candidates[1].excerpts[0].clone()
    }];
    assert!(
        compass_output::RankedQueryView::from_json(&serde_json::to_vec(&root_preview)?).is_err()
    );
    let mut strong = view.clone();
    strong.candidates[1].relevance = RelevanceBand::High;
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&strong)?).is_err());
    let mut outside = view.clone();
    outside.candidates[1].excerpts[0].source.start_byte += 1;
    outside.candidates[1].excerpts[0].source.end_byte += 1;
    outside.candidates[1].excerpts[0].source.start_column += 1;
    outside.candidates[1].excerpts[0].source.end_column += 1;
    assert!(compass_output::RankedQueryView::from_json(&serde_json::to_vec(&outside)?).is_err());
    let mut accepted = Vec::new();
    for kind in [NodeKind::File, NodeKind::Class] {
        let mut non_callable = answer.clone();
        non_callable.candidates[1].node.kind = kind;
        if build_ranked_query_view(&non_callable, "graph", "digest").is_ok() {
            accepted.push(format!("projection: {kind:?}"));
        }
        let mut forged = view.clone();
        forged.candidates[1].kind = kind;
        if compass_output::RankedQueryView::from_json(&serde_json::to_vec(&forged)?).is_ok() {
            accepted.push(format!("reader: {kind:?}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "non-callable preview accepted: {accepted:?}"
    );
    Ok(())
}
