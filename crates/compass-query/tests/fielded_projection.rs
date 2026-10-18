mod support;

use std::error::Error;
use std::path::Path;

use compass_graph::{GraphSnapshotBuilder, GraphSnapshotReader, SnapshotReadLimits};
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind, NodeRole};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_model::query_contract::{
    CodeQueryLimits, DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest,
    DiscoveryTraversal, SearchRequest,
};
use compass_query::{open, open_with_store};
use compass_store::SqliteStore;
use rusqlite::{Connection, params};

fn fixture(path: &Path, aliases: usize) -> Result<GraphDocument, Box<dyn Error>> {
    support::write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    let template = graph.links.first().cloned().ok_or("edge fixture missing")?;
    graph.nodes.clear();
    graph.links.clear();
    let mut target = support::node("owner", NodeKind::Function, "execute", "app::execute");
    target.roles = vec![NodeRole::RouteHandler];
    graph.nodes.push(target);
    for at in 0..aliases {
        let id = format!("alias:{at}");
        let name = if aliases == 1 {
            "renewal".to_owned()
        } else {
            format!("renewal_{at}")
        };
        graph.nodes.push(support::node(
            &id,
            NodeKind::TypeAlias,
            &name,
            &format!("app::{name}"),
        ));
        let mut edge = template.clone();
        edge.source = id;
        edge.target = "owner".to_owned();
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
        graph.links.push(edge);
    }
    Ok(graph)
}

fn question() -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "where are renewal route handlers".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}

#[test]
fn alias_kind_and_role_projection_matches_persisted_statistics_and_primary_postings()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut graph = fixture(&path, 1)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let connection = Connection::open(json.index_path())?;
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?;
    for (term, expected) in [
        ("renewal", 2_u64),
        ("route", 1),
        ("handler", 1),
        ("function", 1),
    ] {
        let json_df: u64 = connection.query_row(
            "SELECT frequency FROM fielded_stats WHERE term=?1",
            params![term],
            |row| row.get(0),
        )?;
        assert_eq!(json_df, expected, "{term}");
        assert_eq!(
            reader.fielded_document_frequency(term)?,
            Some(expected),
            "{term}"
        );
    }
    let reserved = "__compass_primary_v1_renewal";
    let json_primary: Vec<String> = connection
        .prepare("SELECT node_id FROM node_fts WHERE node_fts MATCH ?1 ORDER BY node_id")?
        .query_map(params![format!("\"{reserved}\"")], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    assert_eq!(json_primary, ["alias:0", "owner"]);
    let (stored_primary, truncated, _) =
        reader.nodes_for_exact_term_bounded_work(reserved, SnapshotReadLimits::default())?;
    assert!(!truncated);
    assert_eq!(
        stored_primary
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["alias:0", "owner"]
    );
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let expected = json.discover_ranked(question())?;
    assert!(expected.candidates.iter().any(|candidate| {
        candidate.node.id == "owner"
            && candidate
                .matched_fields
                .iter()
                .any(|field| field == "alias")
            && candidate
                .matched_fields
                .iter()
                .any(|field| field == "roles")
    }));
    assert_eq!(
        stored.discover_ranked(question())?.candidates,
        expected.candidates
    );
    graph.nodes.reverse();
    graph.links.reverse();
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let reordered = open(&path, None, &directory.path().join("reverse-cache"))?;
    assert_eq!(
        reordered.discover_ranked(question())?.candidates,
        expected.candidates
    );
    Ok(())
}

#[test]
fn alias_overflow_is_visible_and_omits_primary_owner_alias_terms_on_both_backends()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let graph = fixture(&path, 9)?;
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = open(&path, None, &directory.path().join("json-cache"))?;
    let connection = Connection::open(json.index_path())?;
    let json_df: u64 = connection.query_row(
        "SELECT frequency FROM fielded_stats WHERE term='renewal'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(json_df, 9);
    let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?;
    assert_eq!(reader.fielded_document_frequency("renewal")?, Some(9));
    let reserved = "__compass_primary_v1_renewal";
    let json_primary: Vec<String> = connection
        .prepare("SELECT node_id FROM node_fts WHERE node_fts MATCH ?1 ORDER BY node_id")?
        .query_map(params![format!("\"{reserved}\"")], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let aliases: Vec<String> = (0..9).map(|at| format!("alias:{at}")).collect();
    assert_eq!(json_primary, aliases);
    let (stored_primary, truncated, _) =
        reader.nodes_for_exact_term_bounded_work(reserved, SnapshotReadLimits::default())?;
    assert!(!truncated);
    assert_eq!(
        stored_primary
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>(),
        aliases
    );
    let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
    let expected = json.discover_ranked(question())?;
    assert!(expected.bounded);
    assert_eq!(expected.eligible_count, None);
    assert_eq!(
        stored.discover_ranked(question())?.candidates,
        expected.candidates
    );
    assert!(stored.discover_ranked(question())?.bounded);
    Ok(())
}

#[test]
fn fielded_aliases_filter_provenance_with_raw_bounds_and_preserve_legacy_lookup()
-> Result<(), Box<dyn Error>> {
    for (aliases, exact_aliases) in [(3, 0), (8, 1), (9, 1)] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        let mut graph = fixture(&path, aliases)?;
        for (at, edge) in graph
            .links
            .iter_mut()
            .enumerate()
            .take(aliases - exact_aliases)
        {
            let mut weaker = edge.evidence.first().cloned().ok_or("alias evidence")?;
            if at % 3 == 0 {
                weaker.origin = EvidenceOrigin::Heuristic;
                weaker.rule = Some("fixture-alias".to_owned());
                weaker.wiring_site = edge.relationship_site.clone();
                // Even exact confidence cannot promote heuristic provenance.
                edge.evidence = vec![weaker];
            } else {
                weaker.confidence = EvidenceConfidence::Inferred;
                if at % 3 == 1 {
                    edge.evidence = vec![weaker];
                } else {
                    // One exact entry cannot hide another inferred entry.
                    edge.evidence.push(weaker);
                }
            }
        }
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let connection = Connection::open(json.index_path())?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let reader = GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?;
        let owner_has_alias = exact_aliases > 0 && aliases <= 8;
        let frequency = aliases as u64 + u64::from(owner_has_alias);
        let json_df: u64 = connection.query_row(
            "SELECT frequency FROM fielded_stats WHERE term='renewal'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(json_df, frequency, "{aliases} raw aliases");
        assert_eq!(
            reader.fielded_document_frequency("renewal")?,
            Some(frequency)
        );
        let reserved = "__compass_primary_v1_renewal";
        let json_primary: Vec<String> = connection
            .prepare("SELECT node_id FROM node_fts WHERE node_fts MATCH ?1 ORDER BY node_id")?
            .query_map(params![format!("\"{reserved}\"")], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let (stored_primary, truncated, _) =
            reader.nodes_for_exact_term_bounded_work(reserved, SnapshotReadLimits::default())?;
        assert!(!truncated);
        assert_eq!(
            stored_primary
                .iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>(),
            json_primary
        );
        assert_eq!(json_primary.iter().any(|id| id == "owner"), owner_has_alias);

        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        for engine in [&json, &stored] {
            for at in 0..aliases {
                let strict = engine.search(SearchRequest {
                    query: format!("renewal_{at}"),
                    limits: CodeQueryLimits::default(),
                })?;
                assert!(strict.results.iter().any(|hit| hit.node_id == "owner"));
            }
        }
        let mut request = question();
        request.question = "where is renewal".to_owned();
        let expected = json.discover_ranked(request.clone())?;
        let owner = expected
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == "owner");
        assert_eq!(owner.is_some(), owner_has_alias, "{aliases} raw aliases");
        if let Some(owner) = owner {
            assert!(owner.matched_fields.iter().any(|field| field == "alias"));
        }
        assert_eq!(
            stored.discover_ranked(request.clone())?.candidates,
            expected.candidates
        );
        if aliases > 8 {
            assert!(expected.bounded);
            assert_eq!(expected.eligible_count, None);
        }
        request.include_heuristic = true;
        assert_eq!(
            json.discover_ranked(request.clone())?.candidates,
            expected.candidates
        );
        assert_eq!(
            stored.discover_ranked(request.clone())?.candidates,
            expected.candidates
        );
        graph.nodes.reverse();
        graph.links.reverse();
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let reordered = open(&path, None, &directory.path().join("reverse-cache"))?;
        assert_eq!(
            reordered.discover_ranked(request)?.candidates,
            expected.candidates
        );
    }
    Ok(())
}
