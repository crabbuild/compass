use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;

use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
    MAX_INDEXED_CANDIDATE_NODES_READ, MAX_INDEXED_CANDIDATE_PROBES,
};
use compass_query::{EngineSelection, open_with_engine};

fn request() -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "Where are damaged local indexes checked and restored?".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}

/// Context paths retain published calls; they do not turn related callees into
/// lexically supported answers or establish execution of a requested behavior.
#[test]
fn broad_context_follows_exact_calls_within_requested_depth_without_promoting_callees()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("repo");
    fs::create_dir(&root)?;
    // Explicit project policy isolates this retrieval regression from default
    // source-root identity and its separate publication tests.
    for (file, text) in [
        ("pyproject.toml", "[project]\nname = 'example'\n"),
        (
            "factory.py",
            "from validation import intact\n\ndef open_snapshot(path):\n    if not intact(path):\n        return create_snapshot(path)\n    return path.read_text()\n\ndef create_snapshot(path):\n    path.write_text('{}')\n    return '{}'\n",
        ),
        (
            "validation.py",
            "import json\n\ndef intact(path):\n    try:\n        json.loads(path.read_text())\n    except (OSError, ValueError):\n        return False\n    return True\n",
        ),
        ("disconnected.py", "def intact(path): return False\n"),
        (
            "test_recovery.py",
            "from factory import open_snapshot\n\ndef test_damaged_local_indexes_are_checked_and_restored(tmp_path):\n    path = tmp_path / 'state.json'\n    path.write_text('{')\n    assert open_snapshot(path) == '{}'\n    assert open_snapshot(path) == '{}'\n",
        ),
    ] {
        fs::write(root.join(file), text)?;
    }
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(directory.path().join("out"));
    options.graph_storage = GraphStorage::Sqlite;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let ids: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|n| (n.qualified_name.as_str(), n.id.as_str()))
        .collect();
    let seed = ids
        .get("test_recovery.test_damaged_local_indexes_are_checked_and_restored")
        .ok_or("missing test")?;
    let opening = ids.get("factory.open_snapshot").ok_or("missing opening")?;
    let checked = ids.get("validation.intact").ok_or("missing validation")?;
    let restored = ids
        .get("factory.create_snapshot")
        .ok_or("missing rebuild")?;
    let disconnected = ids.get("disconnected.intact").ok_or("missing negative")?;
    let original_calls: BTreeMap<_, _> = graph
        .links
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| (e.id.as_str(), e))
        .collect();
    assert_eq!(
        original_calls.len(),
        4,
        "graph precondition must hold before retrieval is assessed"
    );
    let mut previous = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &path,
            None,
            &directory.path().join(format!("cache-{mode:?}")),
            mode,
        )?;
        let query = request();
        let ranked = engine.discover_ranked(query.clone())?;
        assert_eq!(ranked.candidates.len(), 1);
        assert_eq!(ranked.candidates[0].node.id, *seed);
        let mut shallow = query.clone();
        shallow.limits.max_depth = 1;
        let one = engine.discover(shallow)?;
        let names: BTreeSet<_> = one.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(names.contains(opening));
        assert!(!names.contains(checked) && !names.contains(restored));
        let contextual = engine.discover_ranked_with_context(query.clone(), true)?;
        assert_eq!(
            contextual.candidates, ranked.candidates,
            "context must not manufacture lexical admission or relevance"
        );
        let two = engine.discover(query.clone())?;
        let names: BTreeSet<_> = two.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(
            names.contains(checked),
            "validation call at depth2 was not retained"
        );
        assert!(
            names.contains(restored),
            "rebuild call at depth2 was not retained"
        );
        assert!(!names.contains(disconnected));
        assert_eq!(
            two.edges
                .iter()
                .filter(|e| e.kind == EdgeKind::Calls)
                .count(),
            4
        );
        for edge in two.edges.iter().filter(|e| e.kind == EdgeKind::Calls) {
            let original = original_calls
                .get(edge.id.as_deref().ok_or("call lost identity")?)
                .ok_or("invented call")?;
            assert_eq!(edge.source, original.source);
            assert_eq!(edge.target, original.target);
            assert_eq!(edge.relationship_site, original.relationship_site);
            assert_eq!(edge.occurrence_rule, original.occurrence_rule);
            assert_eq!(edge.context, original.context);
            assert_eq!(edge.details, original.details);
            assert!(!edge.evidence.is_empty());
            assert_eq!(edge.evidence.len(), original.evidence.len());
            for (retained, published) in edge.evidence.iter().zip(&original.evidence) {
                assert_eq!(retained.origin, published.origin);
                assert_eq!(retained.extractor, published.extractor);
                assert_eq!(retained.confidence, published.confidence);
                assert_eq!(retained.anchor.as_ref(), published.anchors.first());
                assert_eq!(retained.rule, published.rule);
                assert_eq!(retained.wiring_site, published.wiring_site);
                assert_eq!(retained.candidates, published.candidates);
            }
        }
        assert!(two.stats.candidate_probes <= MAX_INDEXED_CANDIDATE_PROBES);
        assert!(two.stats.candidate_nodes <= MAX_INDEXED_CANDIDATE_NODES_READ);
        assert!(two.stats.expanded_relationships <= query.limits.max_expanded_relationships);
        let semantic = (
            two.seeds,
            two.nodes,
            two.edges,
            two.diagnostics,
            two.truncated,
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &semantic, "JSON/store semantic context parity");
        } else {
            previous = Some(semantic);
        }
        let mut tiny = query;
        tiny.limits.max_expanded_relationships = 1;
        let bounded = engine.discover(tiny)?;
        assert!(bounded.truncated);
        assert!(bounded.stats.expanded_relationships <= 1);
    }
    Ok(())
}
