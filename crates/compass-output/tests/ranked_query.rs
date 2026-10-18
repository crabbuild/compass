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
