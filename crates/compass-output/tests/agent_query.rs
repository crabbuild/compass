use std::error::Error;

use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, ResolutionState, SourceAnchor,
};
use compass_model::query_contract::{
    CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, QueryDiagnostic, QueryDiagnosticCode,
    QueryEdge, QueryEvidence, QueryEvidenceLayer, QueryNode, QueryPath, SearchHit,
};
use compass_output::{
    AGENT_BRIEF_VIEW_SCHEMA, AgentEvidence, AgentExecution, AgentMatch, AgentOperation,
    AgentQueryContext, AgentResultState, AgentTextPageOptions, build_code_query_brief,
    build_code_query_view, render_agent_query_text, render_code_query_text_page,
};

fn anchor(file: &str, line: u32) -> SourceAnchor {
    SourceAnchor {
        file: file.to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 4,
    }
}

fn evidence(source: &SourceAnchor) -> QueryEvidence {
    QueryEvidence {
        layer: QueryEvidenceLayer::StructuralGraph,
        origin: EvidenceOrigin::Ast,
        extractor: "agent-query-test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        anchor: Some(source.clone()),
        rule: None,
        wiring_site: None,
        resolution: ResolutionState::Exact,
        candidates: Vec::new(),
    }
}

fn node(id: &str, label: &str, source: &SourceAnchor) -> QueryNode {
    QueryNode {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: label.to_owned(),
        qualified_name: format!("Fixture.{label}"),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(source.clone()),
        details: None,
        evidence: vec![evidence(source)],
    }
}

fn response(operation: CodeQueryOperation) -> CodeQueryResponse {
    CodeQueryResponse::empty(operation, CodeQueryLimits::default())
}

fn context(operation: AgentOperation) -> AgentQueryContext {
    AgentQueryContext::new(operation, "graph-identity", "generation-identity")
}

#[test]
fn relationship_subject_uses_the_query_engines_symbol_normalization() -> Result<(), Box<dyn Error>>
{
    let source = anchor("src/lib.rs", 1);
    for (operation, agent_operation) in [
        (CodeQueryOperation::Callers, AgentOperation::Callers),
        (CodeQueryOperation::Callees, AgentOperation::Callees),
    ] {
        let mut response = response(operation);
        response.nodes = vec![
            node("a:neighbor", "Neighbor", &source),
            node("z:subject", ".Subject()", &source),
        ];
        response.nodes[1].qualified_name = "Fixture.Subject".to_owned();
        let (from, to) = if operation == CodeQueryOperation::Callers {
            ("a:neighbor", "z:subject")
        } else {
            ("z:subject", "a:neighbor")
        };
        response.edges.push(QueryEdge {
            id: "e:call".to_owned(),
            source: from.to_owned(),
            target: to.to_owned(),
            kind: EdgeKind::Calls,
            relationship_site: Some(source.clone()),
            details: None,
            evidence: vec![evidence(&source)],
        });
        for query in [
            "Subject",
            "subject()",
            ".SUBJECT()",
            " Fixture.Subject ",
            "z:subject",
        ] {
            let view = build_code_query_view(
                &response,
                context(agent_operation)
                    .with_operand(compass_output::AgentOperandRole::Symbol, query),
            )?;
            assert!(
                view.answer.headline.ends_with("for Fixture.Subject."),
                "{query}: {}",
                view.answer.headline
            );
            assert_eq!(view.answer.basis[0].id, "z:subject");
            assert_eq!(view.primary_results[0].id, "z:subject");
        }
    }
    Ok(())
}

#[test]
fn direct_usage_survives_the_projection_bound_ahead_of_owner_references()
-> Result<(), Box<dyn Error>> {
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response
        .nodes
        .push(node("n:target", "Target", &target_anchor));
    let caller_anchor = anchor("src/caller.rs", 10);
    response
        .nodes
        .push(node("n:caller", "Caller", &caller_anchor));
    response.edges.push(QueryEdge {
        id: "e:caller-target".to_owned(),
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(caller_anchor.clone()),
        details: None,
        evidence: vec![evidence(&caller_anchor)],
    });
    // Owner-level references outnumber the direct call and sort earlier by ID,
    // which is exactly the shape that used to hide real call sites.
    for index in 0..30 {
        let id = format!("a:reference-{index:02}");
        let anchor = anchor("src/owner.rs", 30 + index);
        response
            .nodes
            .push(node(&id, &format!("Owner{index:02}"), &anchor));
        response.edges.push(QueryEdge {
            id: format!("e:reference-{index:02}"),
            source: id,
            target: "n:target".to_owned(),
            kind: EdgeKind::References,
            relationship_site: Some(anchor.clone()),
            details: None,
            evidence: vec![evidence(&anchor)],
        });
    }
    let mut query_context = context(AgentOperation::Callers);
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "Target");

    let view = build_code_query_view(&response, query_context)?;
    assert_eq!(view.relationships[0].relation, "calls");
    assert_eq!(
        view.relationships[0].source.label,
        view.primary_results[1].label
    );
    assert!(
        view.answer.headline.contains("31"),
        "headline must count the source response: {}",
        view.answer.headline
    );
    assert!(view.omissions.relationships > 0);
    Ok(())
}

#[test]
fn brief_projection_keeps_answer_semantics_and_drops_audit_detail() -> Result<(), Box<dyn Error>> {
    let caller_anchor = anchor("src/caller.rs", 10);
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response.nodes = vec![
        node("n:caller", "Caller", &caller_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.edges.push(QueryEdge {
        id: "e:caller-target".to_owned(),
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(caller_anchor.clone()),
        details: None,
        evidence: vec![evidence(&caller_anchor)],
    });
    let mut query_context = context(AgentOperation::Callers);
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "Target");

    let brief = build_code_query_brief(&response, query_context.clone())?;
    assert_eq!(brief.schema, AGENT_BRIEF_VIEW_SCHEMA);
    assert_eq!(brief.status.result_state, "answered");
    assert_eq!(brief.relationships.len(), 1);
    assert!(brief.relationships[0].source.contains("Caller"));
    assert_eq!(brief.relationships[0].relation, "calls");
    assert!(brief.relationships[0].target.contains("Target"));
    assert!(
        brief.relationships[0]
            .site
            .as_deref()
            .is_some_and(|site| site.starts_with("src/caller.rs"))
    );

    let full = build_code_query_view(&response, query_context)?;
    assert_eq!(
        brief.relationships[0].source,
        full.relationships[0].source.label
    );
    let brief_bytes = serde_json::to_vec(&brief)?.len();
    let full_bytes = serde_json::to_vec(&full)?.len();
    assert!(
        brief_bytes < full_bytes,
        "brief {brief_bytes} bytes must be smaller than {full_bytes}"
    );
    let brief_json = serde_json::to_string(&brief)?;
    assert!(!brief_json.contains("\"identity\""));
    assert!(!brief_json.contains("viewDigest"));
    // The status, headline, and caveats stay readable.
    assert!(brief_json.contains("\"caveats\""));
    assert_eq!(brief.answer, full.answer.headline);
    Ok(())
}

#[test]
fn paged_text_covers_records_beyond_the_compact_view_and_continues() -> Result<(), Box<dyn Error>> {
    let caller_anchor = anchor("src/caller.rs", 10);
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response
        .nodes
        .push(node("n:target", "Target", &target_anchor));
    for index in 0..40 {
        let id = format!("n:caller-{index:02}");
        response
            .nodes
            .push(node(&id, &format!("Caller{index:02}"), &caller_anchor));
        response.edges.push(QueryEdge {
            id: format!("e:caller-{index:02}"),
            source: id,
            target: "n:target".to_owned(),
            kind: EdgeKind::Calls,
            relationship_site: Some(caller_anchor.clone()),
            details: None,
            evidence: vec![evidence(&caller_anchor)],
        });
    }
    let mut query_context = context(AgentOperation::Callers);
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "Target");

    let first = render_code_query_text_page(
        &response,
        query_context.clone(),
        AgentTextPageOptions {
            token_budget: 300,
            cursor: None,
        },
    )?;
    assert!(first.entry_total > 24, "ledger must exceed the view bound");
    assert!(first.entry_end > first.entry_start);
    assert!(first.text.contains("Pagination:"));
    let cursor = first.next_cursor.clone().ok_or("expected a continuation")?;

    let second = render_code_query_text_page(
        &response,
        query_context.clone(),
        AgentTextPageOptions {
            token_budget: 300,
            cursor: Some(&cursor),
        },
    )?;
    assert_eq!(second.entry_start, first.entry_end);
    assert!(second.entry_end > second.entry_start);
    assert!(second.page > first.page);

    let tampered = format!("{}x", &cursor[..cursor.len() - 1]);
    assert!(
        render_code_query_text_page(
            &response,
            query_context,
            AgentTextPageOptions {
                token_budget: 300,
                cursor: Some(&tampered),
            },
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn paged_map_renders_digest_verified_source_context() -> Result<(), Box<dyn Error>> {
    let first_anchor = anchor("src/first.rs", 1);
    let second_anchor = anchor("src/second.rs", 1);
    let mut response = response(CodeQueryOperation::Explore);
    response.nodes = vec![
        node("n:first", "First", &first_anchor),
        node("n:second", "Second", &second_anchor),
    ];
    response.paths.push(QueryPath {
        id: "p:first-second".to_owned(),
        node_ids: vec!["n:first".to_owned(), "n:second".to_owned()],
        edge_ids: Vec::new(),
        weakest_confidence: EvidenceConfidence::Exact,
        weakest_resolution: ResolutionState::Exact,
    });
    response.files = vec![
        compass_model::query_contract::QueryFile {
            path: "src/first.rs".to_owned(),
            content_digest: "sha256:first".to_owned(),
            source: Some("fn first() {\n    body();\n}\n".to_owned()),
            truncated: false,
        },
        compass_model::query_contract::QueryFile {
            path: "src/second.rs".to_owned(),
            content_digest: "sha256:second".to_owned(),
            source: None,
            truncated: false,
        },
    ];
    let mut query_context = context(AgentOperation::Explore);
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "First");
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "Second");

    let page = render_code_query_text_page(
        &response,
        query_context,
        AgentTextPageOptions {
            token_budget: 2_000,
            cursor: None,
        },
    )?;
    assert!(page.text.contains("SOURCE"), "{}", page.text);
    assert!(page.text.contains("src/first.rs L1-L1 (verified)"));
    assert!(page.text.contains("1: fn first() {"), "{}", page.text);
    assert!(
        !page.text.contains("src/second.rs L1-L1"),
        "stale or missing source must not be rendered as verified context: {}",
        page.text
    );
    Ok(())
}

#[test]
fn paged_text_reaches_the_end_without_a_continuation() -> Result<(), Box<dyn Error>> {
    let caller_anchor = anchor("src/caller.rs", 10);
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response.nodes = vec![
        node("n:caller", "Caller", &caller_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.edges.push(QueryEdge {
        id: "e:caller-target".to_owned(),
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(caller_anchor.clone()),
        details: None,
        evidence: vec![evidence(&caller_anchor)],
    });
    let mut query_context = context(AgentOperation::Callers);
    query_context = query_context.with_operand(compass_output::AgentOperandRole::Symbol, "Target");
    let mut cursor: Option<String> = None;
    let mut pages = 0_u32;
    loop {
        let page = render_code_query_text_page(
            &response,
            query_context.clone(),
            AgentTextPageOptions {
                token_budget: 2_000,
                cursor: cursor.as_deref(),
            },
        )?;
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => {
                assert_eq!(page.entry_end, page.entry_total);
                break;
            }
        }
        assert!(pages < 10, "pagination must terminate");
    }
    Ok(())
}

#[test]
fn exact_relationship_view_is_answer_first_and_round_trips() -> Result<(), Box<dyn Error>> {
    let caller_anchor = anchor("src/caller.rs", 10);
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response.nodes = vec![
        node("n:caller", "Caller", &caller_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.edges.push(QueryEdge {
        id: "e:caller-target".to_owned(),
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(caller_anchor),
        details: None,
        evidence: vec![evidence(&target_anchor)],
    });
    let view = build_code_query_view(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
    )?;
    assert_eq!(view.status.result_state, AgentResultState::Answered);
    assert_eq!(view.status.match_state, AgentMatch::Exact);
    assert_eq!(view.status.evidence_state, AgentEvidence::Exact);
    assert_eq!(view.status.source_execution, AgentExecution::Complete);
    assert_eq!(view.relationships[0].source.label, "Fixture.Caller");
    assert_eq!(view.relationships[0].target.label, "Fixture.Target");
    let text = render_agent_query_text(&view)?;
    // The resolved states are the default and stay out of the envelope; the
    // coverage qualifier is the one a caller has to weigh.
    assert!(
        text.starts_with("RESULT answered · coverage="),
        "a resolved page states only its qualifiers: {}",
        text.lines().next().unwrap_or_default()
    );
    assert!(
        text.find("ANSWER").ok_or("missing answer")?
            < text
                .find("PRIMARY RESULTS")
                .ok_or("missing primary results")?
    );
    assert!(text.contains("Fixture.Caller --calls--> Fixture.Target"));
    let json = serde_json::to_vec(&view)?;
    assert_eq!(compass_output::AgentQueryView::from_json(&json)?, view);
    Ok(())
}

#[test]
fn no_match_and_ambiguous_results_are_not_positive_answers() -> Result<(), Box<dyn Error>> {
    let mut response = response(CodeQueryOperation::Search);
    response.diagnostics.push(QueryDiagnostic {
        code: QueryDiagnosticCode::NoMatch,
        message: "NO EXACT MATCH for Targat".to_owned(),
        node_id: None,
        path: None,
    });
    let view = build_code_query_view(
        &response,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "Targat"),
    )?;
    assert_eq!(view.status.result_state, AgentResultState::NoMatch);
    assert_eq!(view.status.match_state, AgentMatch::None);
    assert!(view.answer.headline.starts_with("No exact match"));
    assert!(view.caveats.iter().any(|caveat| caveat.code == "no_match"));
    let text = render_agent_query_text(&view)?;
    assert!(
        text.find("CAVEATS").ok_or("missing caveats")?
            < text
                .find("PRIMARY RESULTS")
                .ok_or("missing primary results")?
    );
    Ok(())
}

#[test]
fn unresolved_ambiguity_without_retained_candidates_stays_explicit() -> Result<(), Box<dyn Error>> {
    let mut response = response(CodeQueryOperation::Callers);
    response.diagnostics.push(QueryDiagnostic {
        code: QueryDiagnosticCode::AmbiguousMatch,
        message: "Symbol Target matched multiple nodes".to_owned(),
        node_id: None,
        path: None,
    });
    let view = build_code_query_view(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
    )?;
    assert_eq!(view.status.result_state, AgentResultState::NeedsResolution);
    assert!(view.primary_results.is_empty());
    assert!(
        view.answer
            .basis
            .iter()
            .any(|basis| basis.kind == "operation")
    );
    Ok(())
}

#[test]
fn reverse_path_keeps_published_direction_visible() -> Result<(), Box<dyn Error>> {
    let source_anchor = anchor("src/source.rs", 1);
    let target_anchor = anchor("src/target.rs", 2);
    let mut response = response(CodeQueryOperation::NodeTrail);
    response.nodes = vec![
        node("n:source", "Source", &source_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.edges.push(QueryEdge {
        id: "e:source-target".to_owned(),
        source: "n:source".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(source_anchor.clone()),
        details: None,
        evidence: vec![evidence(&source_anchor)],
    });
    response.paths.push(QueryPath {
        id: "p:reverse".to_owned(),
        node_ids: vec!["n:target".to_owned(), "n:source".to_owned()],
        edge_ids: vec!["e:source-target".to_owned()],
        weakest_resolution: ResolutionState::Exact,
        weakest_confidence: EvidenceConfidence::Exact,
    });
    let view = build_code_query_view(
        &response,
        context(AgentOperation::NodeTrail)
            .with_operand(compass_output::AgentOperandRole::Source, "Source")
            .with_operand(compass_output::AgentOperandRole::Target, "Target"),
    )?;
    assert_eq!(view.status.result_state, AgentResultState::Answered);
    assert_eq!(
        view.paths[0].steps[0].direction,
        compass_output::AgentPathDirection::Reverse
    );
    Ok(())
}

#[test]
fn direction_mismatch_is_a_no_path_blocker() -> Result<(), Box<dyn Error>> {
    let source_anchor = anchor("src/source.rs", 1);
    let target_anchor = anchor("src/target.rs", 2);
    let mut response = response(CodeQueryOperation::NodeTrail);
    response.nodes = vec![
        node("n:source", "Source", &source_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.diagnostics.push(QueryDiagnostic {
        code: QueryDiagnosticCode::DirectionMismatch,
        message: "A reverse-only connection was found".to_owned(),
        node_id: Some("n:source".to_owned()),
        path: None,
    });
    let view = build_code_query_view(
        &response,
        context(AgentOperation::NodeTrail)
            .with_operand(compass_output::AgentOperandRole::Source, "Source")
            .with_operand(compass_output::AgentOperandRole::Target, "Target"),
    )?;
    assert_eq!(view.status.result_state, AgentResultState::NoPath);
    assert!(
        view.caveats
            .iter()
            .any(|caveat| caveat.code == "direction_mismatch")
    );
    assert!(view.next_actions.iter().any(|action| {
        action.kind == "inspect_undirected_path"
            && action
                .cli
                .as_ref()
                .is_some_and(|cli| cli.argv == ["compass", "path", "Source", "Target"])
    }));
    Ok(())
}

#[test]
fn equivalent_collection_order_has_one_view_digest() -> Result<(), Box<dyn Error>> {
    let first_anchor = anchor("src/first.rs", 1);
    let second_anchor = anchor("src/second.rs", 2);
    let mut left = response(CodeQueryOperation::Search);
    left.nodes = vec![
        node("n:first", "First", &first_anchor),
        node("n:second", "Second", &second_anchor),
    ];
    left.results.push(SearchHit {
        node_id: "n:first".to_owned(),
        score: 1.0,
        matched_fields: vec!["name".to_owned()],
    });
    let mut right = left.clone();
    right.nodes.reverse();
    let left_view = build_code_query_view(
        &left,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "First"),
    )?;
    let right_view = build_code_query_view(
        &right,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "First"),
    )?;
    assert_eq!(
        left_view.identity.source_result_digest,
        right_view.identity.source_result_digest
    );
    assert_eq!(
        left_view.identity.view_digest,
        right_view.identity.view_digest
    );
    Ok(())
}

/// A callers response with `count` call sites of one target.
fn callers_fixture(count: usize) -> CodeQueryResponse {
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response
        .nodes
        .push(node("n:target", "Target", &target_anchor));
    for index in 0..count {
        let file = format!("src/caller_{index}.rs");
        let caller_anchor = anchor(&file, index as u32 + 1);
        let id = format!("n:caller-{index}");
        response
            .nodes
            .push(node(&id, &format!("Caller{index}"), &caller_anchor));
        response.edges.push(QueryEdge {
            id: format!("e:caller-{index}"),
            source: id,
            target: "n:target".to_owned(),
            kind: EdgeKind::Calls,
            relationship_site: Some(caller_anchor.clone()),
            details: None,
            evidence: vec![evidence(&caller_anchor)],
        });
    }
    response
}

#[test]
fn text_page_keeps_the_agent_view_profile_and_pages_the_rest() -> Result<(), Box<dyn Error>> {
    let response = callers_fixture(24);
    let page = render_code_query_text_page(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
        AgentTextPageOptions {
            // A budget large enough that only the profile decides the page.
            token_budget: 32_000,
            cursor: None,
        },
    )?;
    let entities = page
        .text
        .lines()
        .filter(|line| line.starts_with("- Fixture.Caller") || line.starts_with("- Fixture.Target"))
        .count();
    assert_eq!(
        entities, 12,
        "one page renders the Agent View profile: {}",
        page.text
    );
    assert!(
        page.text.contains("range=1-12 of "),
        "the page reports the ledger's true total: {}",
        page.text.lines().last().unwrap_or_default()
    );
    assert!(
        page.next_cursor.is_some(),
        "the rest of the ledger continues"
    );
    assert!(
        page.entry_total >= 25,
        "the ledger keeps the target, the callers and their edges: {}",
        page.entry_total
    );
    assert!(
        page.text.contains(&format!("of {}", page.entry_total)),
        "the page reports the ledger's true total"
    );
    // The machine views keep every record the page defers.
    assert_eq!(response.nodes.len(), 25);
    Ok(())
}

#[test]
fn text_page_prints_identifiers_only_where_it_resolves_a_name() -> Result<(), Box<dyn Error>> {
    let response = callers_fixture(3);
    let resolved = render_code_query_text_page(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
        AgentTextPageOptions {
            token_budget: 2_000,
            cursor: None,
        },
    )?;
    assert!(
        !resolved.text.contains("  id: "),
        "a resolved caller row is addressed by name and anchor: {}",
        resolved.text
    );
    // A pick-list row is addressed by the qualified name and source anchor it
    // prints, so a candidate page pays for identifiers only where the printed
    // name cannot pick the row out of the page.
    let mut candidates = response.clone();
    candidates.operation = CodeQueryOperation::Search;
    candidates.results.push(SearchHit {
        node_id: "n:target".to_owned(),
        score: 1.0,
        matched_fields: vec!["name".to_owned()],
    });
    let exact_pick_list = render_code_query_text_page(
        &candidates,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "Target"),
        AgentTextPageOptions {
            token_budget: 2_000,
            cursor: None,
        },
    )?;
    assert!(
        !exact_pick_list.text.contains("  id: "),
        "an exact-name pick list is addressed by name and anchor: {}",
        exact_pick_list.text
    );
    let fuzzy_pick_list = render_code_query_text_page(
        &candidates,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "Targat"),
        AgentTextPageOptions {
            token_budget: 2_000,
            cursor: None,
        },
    )?;
    assert!(
        !fuzzy_pick_list.text.contains("  id: "),
        "a candidate page of distinct names is addressed by those names: {}",
        fuzzy_pick_list.text
    );
    // Two retained rows under one label cannot be told apart by that label, so
    // both carry the identifier that separates them.
    let mut colliding = candidates.clone();
    colliding
        .nodes
        .push(node("n:twin", "Target", &anchor("src/twin.rs", 40)));
    colliding.results.push(SearchHit {
        node_id: "n:twin".to_owned(),
        score: 0.9,
        matched_fields: vec!["name".to_owned()],
    });
    let colliding_pick_list = render_code_query_text_page(
        &colliding,
        context(AgentOperation::Search)
            .with_operand(compass_output::AgentOperandRole::Query, "Targat"),
        AgentTextPageOptions {
            token_budget: 2_000,
            cursor: None,
        },
    )?;
    assert!(
        colliding_pick_list.text.matches("  id: ").count() >= 2,
        "rows that share a label keep the identifiers that separate them: {}",
        colliding_pick_list.text
    );
    // The JSON projection carries identifiers for both.
    let view = build_code_query_view(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
    )?;
    assert!(
        view.primary_results
            .iter()
            .all(|entity| !entity.id.is_empty())
    );
    Ok(())
}

#[test]
fn text_page_envelope_stays_compact_and_cursors_stay_short() -> Result<(), Box<dyn Error>> {
    let response = callers_fixture(24);
    let page = render_code_query_text_page(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
        AgentTextPageOptions {
            token_budget: 32_000,
            cursor: None,
        },
    )?;
    assert!(
        page.text.starts_with("RESULT "),
        "the page opens with one RESULT line: {}",
        page.text
    );
    assert!(
        !page.text.contains("\nState:"),
        "the multi-line state block is gone: {}",
        page.text
    );
    let pagination = page
        .text
        .lines()
        .find(|line| line.starts_with("Pagination:"))
        .ok_or("missing pagination line")?;
    assert!(!pagination.contains("version="), "{pagination}");
    assert!(!pagination.contains("budget_tokens"), "{pagination}");
    let cursor = page.next_cursor.ok_or("expected a continuation")?;
    assert!(
        cursor.len() <= 120,
        "a cursor is re-printed on every page, so it stays compact: {} chars",
        cursor.len()
    );
    Ok(())
}

#[test]
fn legacy_page_cursor_encoding_is_rejected_with_a_version_error() -> Result<(), Box<dyn Error>> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    // Two earlier encodings exist: the release that wrote long keys with a
    // string version and full digests, and the base64url-wrapped compact JSON
    // that followed it. Neither may be reinterpreted as a field in the current
    // form, so both must fail explicitly instead of carrying a caller into
    // another query's page.
    let legacy_verbose = serde_json::json!({
        "version": "compass.query.agent-text-page/1",
        "operation": "callers",
        "graphIdentity": "a".repeat(64),
        "page": 2,
        "prefixCount": 1,
        "prefixDigest": "b".repeat(64),
    });
    let legacy_compact = serde_json::json!({
        "v": 1,
        "o": "callers",
        "g": "a".repeat(16),
        "p": 2,
        "c": 1,
        "d": "b".repeat(16),
    });
    for legacy in [legacy_verbose, legacy_compact] {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&legacy)?);
        let checksum = format!("{:x}", Sha256::digest(payload.as_bytes()));
        let cursor = format!("{payload}.{checksum}");
        let error = compass_output::decode_agent_text_page_cursor(&cursor)
            .err()
            .ok_or("a legacy cursor must not be reinterpreted")?;
        let message = error.to_string();
        assert!(
            message.contains("cursor"),
            "the failure names the cursor: {message}"
        );
    }
    Ok(())
}

#[test]
fn unresolved_relationship_answers_never_speak_for_another_symbol() -> Result<(), Box<dyn Error>> {
    let caller_anchor = anchor("src/caller.rs", 10);
    let target_anchor = anchor("src/target.rs", 20);
    let mut response = response(CodeQueryOperation::Callers);
    response.nodes = vec![
        node("n:caller", "Caller", &caller_anchor),
        node("n:target", "Target", &target_anchor),
    ];
    response.edges.push(QueryEdge {
        id: "e:caller-target".to_owned(),
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(caller_anchor.clone()),
        details: None,
        evidence: vec![evidence(&caller_anchor)],
    });

    // An exact match states the count for the resolved subject.
    let resolved = build_code_query_view(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Target"),
    )?;
    assert_eq!(resolved.status.result_state, AgentResultState::Answered);
    assert!(
        resolved
            .answer
            .headline
            .starts_with("Found 1 incoming usage relationship(s) for "),
        "{}",
        resolved.answer.headline
    );

    // A query with no exact match reports the missing subject; it never
    // presents another symbol's relationships as the answer to it.
    response.diagnostics.push(QueryDiagnostic {
        code: QueryDiagnosticCode::NoMatch,
        message: "NO EXACT MATCH for Missing".to_owned(),
        node_id: None,
        path: None,
    });
    let unresolved = build_code_query_view(
        &response,
        context(AgentOperation::Callers)
            .with_operand(compass_output::AgentOperandRole::Symbol, "Missing"),
    )?;
    assert_eq!(unresolved.status.result_state, AgentResultState::NoMatch);
    assert!(
        unresolved
            .answer
            .headline
            .starts_with("No exact match for \"Missing\""),
        "{}",
        unresolved.answer.headline
    );
    assert!(
        !unresolved
            .answer
            .headline
            .starts_with("Found 1 incoming usage relationship(s) for "),
        "a fallback's evidence must not be attributed to the request: {}",
        unresolved.answer.headline
    );
    Ok(())
}
