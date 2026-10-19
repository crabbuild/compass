//! Prospective cross-function retrieval regression; source evidence stays owned.
use std::error::Error;
use std::fs;

use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_query::{EngineSelection, RelevanceBand, open_with_engine};

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

fn check_delegated_owner(helper: &str, own_term: &str) -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def maintain(records):\n    # Check and publish stale or broken record caches.\n    if check_piece(records):\n        return\n    publish_piece(records)\n\ndef check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n\ndef check_shadow(value):\n    return value is None\n\nclass Unrelated:\n    def check_piece(self, value):\n        return value\n",
    )?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("artifacts"));
    options.graph_storage = GraphStorage::Sqlite;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let built = build_local_graph(&options)?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let find_function = |name: &str| {
        graph.nodes.iter().find(|node| {
            node.kind == NodeKind::Function && node.qualified_name == format!("sample.{name}")
        })
    };
    let caller = find_function("maintain").ok_or("missing native caller")?;
    let target = find_function(helper).ok_or("missing native delegated helper")?;
    let disconnected = find_function("check_shadow").ok_or("missing disconnected control")?;
    let site = graph
        .links
        .iter()
        .find(|edge| {
            edge.source == caller.id && edge.target == target.id && edge.kind == EdgeKind::Calls
        })
        .ok_or("native call edge missing: this fixture cannot diagnose retrieval")?;
    assert!(site.relationship_site.is_some());
    assert!(site.evidence.iter().any(|proof| {
        proof.origin == EvidenceOrigin::Ast && proof.confidence == EvidenceConfidence::Exact
    }));
    assert!(!graph.links.iter().any(|edge| {
        edge.source == caller.id && edge.target == disconnected.id && edge.kind == EdgeKind::Calls
    }));
    let mut previous = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &path,
            None,
            &directory.path().join(format!("query-cache-{mode:?}")),
            mode,
        )?;
        assert!(
            engine
                .discover_ranked(request(
                    "Where are OrbitalPlasmid caches checked and published?"
                ))?
                .candidates
                .is_empty(),
            "a connected graph cannot replace missing subjects"
        );
        let mut direct = request("Where are stale or broken record caches checked and published?");
        direct.limits.max_depth = 1;
        let answer = engine.discover_ranked(direct.clone())?;
        let work = answer
            .discovery
            .stats
            .call_link_work
            .as_ref()
            .ok_or("call work absent")?;
        assert!(work.is_coherent());
        assert_eq!(
            work.indexed_candidate_probes,
            answer.discovery.stats.candidate_probes
        );
        let raw = engine.discover(direct)?;
        assert!(raw.has_valid_work_contract());
        assert_eq!(
            raw.schema,
            compass_model::query_contract::DISCOVERY_QUERY_SCHEMA_V2
        );
        assert_eq!(raw.stats.call_link_work.as_ref(), Some(work));
        compass_query::discovery_result_envelope(raw.clone())?.validate()?;
        let generic = compass_output::build_discovery_query_view(
            &raw,
            compass_output::AgentQueryContext::new(
                compass_output::AgentOperation::Discovery,
                "graph",
                "generation",
            ),
        )?;
        assert_eq!(generic.identity.raw_schema, raw.schema);
        assert!(
            generic
                .caveats
                .iter()
                .any(|caveat| caveat.code == "call_link_recall_work")
        );
        assert!(
            answer
                .candidates
                .iter()
                .all(|candidate| candidate.node.id != disconnected.id)
        );
        assert!(
            answer
                .candidates
                .iter()
                .all(|candidate| candidate.node.qualified_name != "sample.Unrelated.check_piece")
        );
        let candidate = answer
            .candidates
            .iter()
            .take(5)
            .find(|candidate| candidate.node.id == target.id)
            .ok_or("source-witnessed delegated helper is absent from the first shortlist")?;
        assert_eq!(candidate.relevance, RelevanceBand::Low);
        let preview = candidate
            .excerpts
            .iter()
            .find(|quote| quote.text.contains("return value"))
            .ok_or("linked helper has no inspectable retained code")?;
        assert_eq!(
            serde_json::to_value(preview)?["kind"],
            "linked_callable_preview"
        );
        assert!(preview.is_coherent());
        let original = fs::read_to_string(root.join("sample.py"))?;
        assert_eq!(
            original.get(preview.source.start_byte as usize..preview.source.end_byte as usize),
            Some(preview.text.as_str())
        );
        assert!(
            answer
                .candidates
                .iter()
                .filter(|item| item.call_link.is_none())
                .all(|item| {
                    item.excerpts.iter().all(|quote| {
                        serde_json::to_value(quote)
                            .is_ok_and(|value| value["kind"] != "linked_callable_preview")
                    })
                })
        );
        let link = candidate
            .call_link
            .as_ref()
            .ok_or("helper lost its call path")?;
        assert!(link.is_coherent_for(&candidate.node.id));
        assert_eq!(link.root.id, caller.id);
        assert_eq!(link.hops.len(), 1);
        assert_eq!(link.hops[0].edge_id, site.id);
        assert_eq!(
            link.hops[0].site,
            site.relationship_site.clone().ok_or("call site absent")?
        );
        assert!(
            answer
                .discovery
                .edges
                .iter()
                .any(|edge| edge.id.as_deref() == Some(site.id.as_str()))
        );
        assert!(
            answer.discovery.stats.candidate_probes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES
        );
        assert!(
            answer.discovery.stats.candidate_nodes
                <= compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ
        );
        assert!(answer.discovery.stats.expanded_relationships > 0);
        let view = compass_output::build_ranked_query_view(&answer, "graph", "digest")?;
        assert_eq!(view.schema, compass_output::RANKED_QUERY_LINKED_VIEW_SCHEMA);
        let text = compass_output::render_ranked_query_text(&view, 1000)?;
        assert!(text.contains("Call path (execution unverified): maintain → check_piece"));
        assert!(text.contains("Call path (execution unverified): maintain → publish_piece"));
        for cap in [1, 2, 3] {
            let mut capped =
                request("Where are stale or broken record caches checked and published?");
            capped.limits.max_candidates = cap;
            capped.limits.max_seeds = 1;
            capped.limits.max_nodes = 9;
            let capped = engine.discover_ranked(capped)?;
            assert_eq!(capped.candidates.len(), cap as usize);
            let capped_work = capped
                .discovery
                .stats
                .call_link_work
                .as_ref()
                .ok_or("presentation pruning lost call-probe work")?;
            assert!(capped_work.is_coherent());
            assert!(capped_work.candidate_probes > 0);
            assert!(capped.discovery.has_valid_work_contract());
            assert_eq!(
                capped.discovery.schema,
                compass_model::query_contract::DISCOVERY_QUERY_SCHEMA_V2
            );
            let required_edges = capped
                .candidates
                .iter()
                .filter_map(|candidate| candidate.call_link.as_ref())
                .flat_map(|link| link.hops.iter().map(|hop| hop.edge_id.as_str()))
                .collect::<std::collections::BTreeSet<_>>();
            let retained_edges = capped
                .discovery
                .edges
                .iter()
                .filter_map(|edge| edge.id.as_deref())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                retained_edges, required_edges,
                "unused call paths at cap {cap}, {mode:?}"
            );
            assert_eq!(capped.discovery.nodes.len(), capped.candidates.len());
            assert!(
                capped.discovery.stats.expanded_relationships > 0,
                "discarded presentation paths must retain actual work accounting"
            );
            let view = compass_output::build_ranked_query_view(&capped, "graph", "digest")?;
            compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?;
        }
        let mut limited = request("Where are stale or broken record caches checked and published?");
        limited.limits.max_expanded_relationships = 1;
        let limited = engine.discover_ranked(limited)?;
        assert!(limited.discovery.stats.expanded_relationships <= 1);
        assert!(
            limited
                .candidates
                .iter()
                .all(|candidate| candidate.call_link.is_none())
        );
        assert!(limited.bounded);

        assert!(candidate.matched_terms.iter().any(|term| term == own_term));
        assert!(
            candidate
                .matched_terms
                .iter()
                .all(|term| { !matches!(term.as_str(), "stale" | "broken" | "record" | "cach") }),
            "caller terms must not become the helper's own matched evidence"
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &answer.candidates, "JSON/store parity");
        } else {
            previous = Some(answer.candidates);
        }
    }
    Ok(())
}

#[test]
fn delegated_check_is_recalled_without_borrowing_caller_terms() -> Result<(), Box<dyn Error>> {
    check_delegated_owner("check_piece", "check")
}

#[test]
fn delegated_publish_is_recalled_without_borrowing_caller_terms() -> Result<(), Box<dyn Error>> {
    check_delegated_owner("publish_piece", "publish")
}

#[test]
fn linked_helpers_obey_request_depth_in_discovery_and_trace() -> Result<(), Box<dyn Error>> {
    for hops in [2, 3, 4] {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("source");
        fs::create_dir(&root)?;
        let mut source = String::from(
            "def maintain(records):\n    # Check and publish stale or broken record caches.\n    stage_0(records)\n    publish_piece(records)\n\n",
        );
        for index in 0..hops - 1 {
            let target = if index + 1 == hops - 1 {
                "check_piece".to_owned()
            } else {
                format!("stage_{}", index + 1)
            };
            source.push_str(&format!(
                "def stage_{index}(value):\n    return {target}(value)\n\n"
            ));
        }
        source.push_str("def check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n");
        fs::write(root.join("sample.py"), source)?;
        let mut options = BuildOptions::new(&root);
        options.output_root = Some(directory.path().join("artifacts"));
        options.graph_storage = GraphStorage::Sqlite;
        options.no_cluster = true;
        options.no_viz = true;
        options.max_workers = Some(1);
        let built = build_local_graph(&options)?;
        let path = built.output_dir.join("graph.json");
        let graph = GraphDocument::load(&path)?;
        let target = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "sample.check_piece")
            .ok_or("missing native check helper")?;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine = open_with_engine(
                &path,
                None,
                &directory.path().join(format!("cache-{mode:?}")),
                mode,
            )?;
            for depth in [1, 2, 4] {
                let mut query =
                    request("Where are stale or broken record caches checked and published?");
                query.limits.max_depth = depth;
                let answer = engine.discover_ranked(query.clone())?;
                let linked = answer
                    .candidates
                    .iter()
                    .find(|candidate| candidate.node.id == target.id)
                    .and_then(|candidate| candidate.call_link.as_ref());
                assert_eq!(
                    linked.is_some(),
                    depth >= hops,
                    "{mode:?}, depth {depth}, target hops {hops}"
                );
                if let Some(link) = linked {
                    assert_eq!(link.hops.len(), hops as usize);
                    let view = compass_output::build_ranked_query_view(&answer, "graph", "digest")?;
                    compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?;
                }
                assert!(
                    answer
                        .candidates
                        .iter()
                        .filter_map(|candidate| candidate.call_link.as_ref())
                        .all(|link| link.hops.len() <= depth as usize)
                );
                let trace = engine.trace_ranked(query, &target.id)?;
                assert_eq!(
                    trace.first_page,
                    linked.is_some(),
                    "discovery/trace disagreement at depth {depth}, {hops} hops"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn native_multi_anchor_call_keeps_selected_site_in_context() -> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{SOURCE_QUERY_FILE, SourceQueryArtifact};
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def maintain(records):\n    # Check and publish stale or broken record caches.\n    check_piece(records)\n    publish_piece(records)\n\ndef check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n",
    )?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("original"));
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let original = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    let caller = original
        .nodes
        .iter()
        .find(|node| node.qualified_name == "sample.maintain")
        .ok_or("caller")?;
    let target = original
        .nodes
        .iter()
        .find(|node| node.qualified_name == "sample.check_piece")
        .ok_or("target")?;
    let owner = caller.source.clone().ok_or("owner source")?;
    for call_site_first in [false, true] {
        let mut graph = original.clone();
        let edge = graph
            .links
            .iter_mut()
            .find(|edge| {
                edge.kind == EdgeKind::Calls && edge.source == caller.id && edge.target == target.id
            })
            .ok_or("native call")?;
        let site = edge.relationship_site.clone().ok_or("site")?;
        assert_ne!(site, owner);
        let proof = edge
            .evidence
            .iter_mut()
            .find(|proof| proof.origin == EvidenceOrigin::Ast && proof.anchors.contains(&site))
            .ok_or("native AST proof")?;
        proof.anchors = if call_site_first {
            vec![site.clone(), owner.clone()]
        } else {
            vec![owner.clone(), site.clone()]
        };
        proof.validate()?;
        let mut additional = proof.clone();
        additional.extractor = "unrelated-authored-attribution".to_owned();
        additional.origin = EvidenceOrigin::Config;
        additional.anchors = vec![owner.clone()];
        additional.validate()?;
        edge.evidence.push(additional);
        let occurrence = edge.id.clone();
        let variant = directory.path().join(format!("variant-{call_site_first}"));
        fs::create_dir(&variant)?;
        let bytes = compass_graph::canonical_graph_json(&graph)?;
        let graph_path = variant.join("graph.json");
        fs::write(&graph_path, &bytes)?;
        // This is a new authored fixture realization. The native source-body
        // owner bytes are unchanged and are explicitly bound to this new graph.
        let mut bound = artifact.clone();
        bound.graph_digest = format!("{:x}", Sha256::digest(&bytes));
        bound.validate()?;
        fs::write(variant.join(SOURCE_QUERY_FILE), serde_json::to_vec(&bound)?)?;
        let engine = open_with_engine(
            &graph_path,
            None,
            &variant.join("cache"),
            EngineSelection::Json,
        )?;
        for include_context in [false, true] {
            let answer = engine.discover_ranked_with_context(
                request("Where are stale or broken record caches checked and published?"),
                include_context,
            )?;
            let helper = answer
                .candidates
                .iter()
                .find(|candidate| candidate.node.id == target.id)
                .ok_or("linked helper missing")?;
            let link = helper.call_link.as_ref().ok_or("helper call link")?;
            assert_eq!(link.hops[0].edge_id, occurrence);
            assert_eq!(link.hops[0].site, site);
            let view = compass_output::build_ranked_query_view_with_context(
                &answer, "graph", "digest", true,
            )?;
            let context = view.context.as_ref().ok_or("context missing")?;
            let retained = context
                .edges
                .iter()
                .find(|edge| edge.id.as_deref() == Some(occurrence.as_str()))
                .ok_or("context occurrence missing")?;
            assert!(
                retained
                    .evidence
                    .iter()
                    .any(|proof| proof.anchor.as_ref() == Some(&site)
                        && proof.extractor == link.hops[0].extractor)
            );
            compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?;
        }
    }
    Ok(())
}

#[test]
fn discarded_helper_metadata_has_an_early_typed_limit() -> Result<(), Box<dyn Error>> {
    use compass_model::code_graph::NodeDetails;
    use compass_model::source_query::{SOURCE_QUERY_FILE, SourceQueryArtifact};
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def maintain(records):\n    # Check and publish stale or broken record caches.\n    check_piece(records)\n    publish_piece(records)\n\ndef check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n",
    )?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("original"));
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let mut graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let target = graph
        .nodes
        .iter_mut()
        .find(|node| node.qualified_name == "sample.check_piece")
        .ok_or("helper")?;
    let Some(NodeDetails::Symbol(details)) = &mut target.details else {
        return Err("symbol details missing".into());
    };
    details.signature = Some(format!("(value) {}", "x".repeat(9 * 1024 * 1024)));
    let variant = directory.path().join("oversized");
    fs::create_dir(&variant)?;
    let bytes = compass_graph::canonical_graph_json(&graph)?;
    let graph_path = variant.join("graph.json");
    fs::write(&graph_path, &bytes)?;
    let mut artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    artifact.graph_digest = format!("{:x}", Sha256::digest(&bytes));
    artifact.validate()?;
    fs::write(
        variant.join(SOURCE_QUERY_FILE),
        serde_json::to_vec(&artifact)?,
    )?;
    let engine = open_with_engine(
        &graph_path,
        None,
        &variant.join("cache"),
        EngineSelection::Json,
    )?;
    // A helper may be excluded from a one-row presentation, but must still be
    // preflighted before the new deferred pool or a witnessed path copies it.
    let mut query = request("Where are stale or broken record caches checked and published?");
    query.limits.max_candidates = 1;
    query.limits.max_seeds = 1;
    let error = engine
        .discover_ranked(query)
        .err()
        .ok_or("oversized helper metadata was silently accepted")?;
    assert_eq!(error.code(), "linked_record_metadata_limit", "{error}");
    assert_eq!(error.kind(), compass_query::QueryErrorKind::ExpansionLimit);
    Ok(())
}

#[test]
fn linked_optional_context_shares_the_early_metadata_limit() -> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{SOURCE_QUERY_FILE, SourceQueryArtifact};
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def maintain(records):\n    # Check and publish stale or broken record caches.\n    check_piece(records)\n    publish_piece(records)\n\ndef check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n",
    )?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("original"));
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let mut graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let root_id = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "sample.maintain")
        .ok_or("caller")?
        .id
        .clone();
    // A native non-call occurrence is outside linked candidate enumeration but
    // is inspected by the optional incident context, even if later rejected by
    // direction filtering. No file or historical realization is rewritten.
    let edge = graph
        .links
        .iter_mut()
        .find(|edge| {
            edge.kind != EdgeKind::Calls && (edge.source == root_id || edge.target == root_id)
        })
        .ok_or("native non-call occurrence")?;
    edge.context = Some("x".repeat(9 * 1024 * 1024));
    let variant = directory.path().join("oversized-context");
    fs::create_dir(&variant)?;
    let bytes = compass_graph::canonical_graph_json(&graph)?;
    let graph_path = variant.join("graph.json");
    fs::write(&graph_path, &bytes)?;
    let mut artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    artifact.graph_digest = format!("{:x}", Sha256::digest(&bytes));
    artifact.validate()?;
    fs::write(
        variant.join(SOURCE_QUERY_FILE),
        serde_json::to_vec(&artifact)?,
    )?;
    let engine = open_with_engine(
        &graph_path,
        None,
        &variant.join("cache"),
        EngineSelection::Json,
    )?;
    let query = request("Where are stale or broken record caches checked and published?");
    let ordinary = engine.discover_ranked_with_context(query.clone(), false)?;
    assert!(
        ordinary
            .candidates
            .iter()
            .any(|candidate| candidate.call_link.is_some())
    );
    let error = engine
        .discover_ranked_with_context(query, true)
        .err()
        .ok_or("oversized incident metadata was accepted")?;
    assert_eq!(error.code(), "linked_record_metadata_limit", "{error}");
    assert_eq!(error.kind(), compass_query::QueryErrorKind::ExpansionLimit);
    Ok(())
}

#[test]
fn branching_navigation_reaches_deeper_helpers_within_requested_depth() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    fs::create_dir(&root)?;
    let mut source = String::from(
        "def maintain(records):\n    # Check and publish stale or broken record caches.\n",
    );
    for index in 0..5 {
        source.push_str(&format!("    spare_{index}(records)\n"));
    }
    source.push_str("    stage_0(records)\n    publish_piece(records)\n\ndef stage_0(value):\n    return stage_1(value)\n\ndef stage_1(value):\n    return check_piece(value)\n\ndef check_piece(value):\n    return value is not None\n\ndef publish_piece(value):\n    return value\n");
    for index in 0..5 {
        source.push_str(&format!("\ndef spare_{index}(value):\n    return value\n"));
    }
    fs::write(root.join("sample.py"), source)?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("artifacts"));
    options.graph_storage = GraphStorage::Sqlite;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "sample.check_piece")
        .ok_or("native deep helper missing")?;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &path,
            None,
            &directory.path().join(format!("branch-cache-{mode:?}")),
            mode,
        )?;
        for depth in [2, 3, 4] {
            let mut query =
                request("Where are stale or broken record caches checked and published?");
            query.limits.max_depth = depth;
            let answer = engine.discover_ranked(query.clone())?;
            let linked = answer
                .candidates
                .iter()
                .find(|candidate| candidate.node.id == target.id)
                .and_then(|candidate| candidate.call_link.as_ref());
            assert_eq!(
                linked.is_some(),
                depth >= 3,
                "branching native calls, {mode:?}, requested depth {depth}"
            );
            if let Some(link) = linked {
                assert_eq!(link.hops.len(), 3);
                let work = answer
                    .discovery
                    .stats
                    .call_link_work
                    .as_ref()
                    .ok_or("branch work missing")?;
                assert!(work.is_coherent());
                assert!(work.candidate_probes >= 9 && work.candidate_probes <= 40);
                assert!(link.is_coherent_for(&target.id));
                let helper = answer
                    .candidates
                    .iter()
                    .find(|candidate| candidate.node.id == target.id)
                    .ok_or("displayed helper missing")?;
                assert_eq!(helper.relevance, RelevanceBand::Low);
                assert!(
                    helper.matched_terms.iter().all(|term| !matches!(
                        term.as_str(),
                        "stale" | "broken" | "record" | "cach"
                    ))
                );
            }
            let view = compass_output::build_ranked_query_view(&answer, "graph", "digest")?;
            compass_output::RankedQueryView::from_json(&serde_json::to_vec(&view)?)?;
            let trace = engine.trace_ranked(query, &target.id)?;
            assert_eq!(trace.first_page, linked.is_some());
            assert_eq!(trace.call_link_work, answer.discovery.stats.call_link_work);
        }
    }
    Ok(())
}
