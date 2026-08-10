use std::collections::BTreeSet;
use std::error::Error;

use compass_graph::{
    GraphSnapshotBuilder, GraphSnapshotReader, IndexKind, SnapshotError, SnapshotReadLimits,
    active_graph_snapshot, canonical_graph_json, encode_graph_index_key,
    garbage_collect_graph_snapshots, graph_snapshot_needs_gc,
};
use compass_model::code_graph::{
    BuildMetadata, CommunityMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileNodeDetails,
    FileRecord, GraphDocument, NodeDetails, NodeKind, NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, Provenance, SourceAnchor,
};
use compass_store::SqliteStore;
use compass_store::{Key, MemoryStore, NamespaceId, PartitionKey, Store, WriteCondition};
use sha2::Digest;
use tempfile::tempdir;

fn graph() -> GraphDocument {
    let mut document = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "schema".to_owned(),
        source_tree_digest: "tree".to_owned(),
        configuration_digest: "config".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    });
    document.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: "sha256:test".to_owned(),
        byte_size: 1,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    let mut file_node = node("file");
    file_node.kind = NodeKind::File;
    file_node.name = "lib.rs".to_owned();
    file_node.qualified_name = "src/lib.rs".to_owned();
    file_node.details = Some(NodeDetails::File(FileNodeDetails {
        content_digest: "sha256:test".to_owned(),
        byte_size: 1,
        generated: false,
    }));
    document.nodes = vec![node("b"), node("a"), file_node];
    let first_id = edge_id("a", EdgeKind::Calls, "b", None, None);
    let second_id = edge_id("a", EdgeKind::Calls, "b", None, Some("second"));
    document.links = vec![
        EdgeRecord {
            id: first_id.clone(),
            key: first_id,
            source: "a".to_owned(),
            target: "b".to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: vec![evidence()],
            weight: Some(1.0),
            context: Some("a calls b".to_owned()),
            deferred: false,
            diagnostics: Vec::new(),
        },
        EdgeRecord {
            id: second_id.clone(),
            key: second_id,
            source: "a".to_owned(),
            target: "b".to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: OccurrenceRule::new("second"),
            relationship_site: None,
            details: None,
            evidence: vec![evidence()],
            weight: Some(2.0),
            context: Some("a calls b again".to_owned()),
            deferred: false,
            diagnostics: Vec::new(),
        },
    ];
    document
}

fn node(id: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: format!("crate::{id}"),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn evidence() -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    }
}

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 1,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 1,
    }
}

fn limits(max_items: usize) -> SnapshotReadLimits {
    SnapshotReadLimits {
        max_items,
        max_bytes: 256 * 1024,
        max_objects: 4_096,
        max_depth: 64,
    }
}

#[test]
fn snapshot_is_deterministic_and_reuses_immutable_objects() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let first = builder.prepare(&store, &graph())?;
    let second = builder.prepare(&store, &graph())?;
    let mut operational_variant = graph();
    operational_variant.nodes.reverse();
    operational_variant.links.reverse();
    operational_variant.graph.build.generation_id = "different-run".to_owned();
    let variant = builder.prepare(&store, &operational_variant)?;

    assert_eq!(first.manifest, second.manifest);
    assert_eq!(first.manifest.snapshot_id, variant.manifest.snapshot_id);
    assert_ne!(first.manifest.graph_digest, variant.manifest.graph_digest);
    let mut unknown = first.manifest.clone();
    unknown.schema = "compass.store.graph-index/99".to_owned();
    assert!(matches!(
        unknown.validate(),
        Err(SnapshotError::Unsupported(_))
    ));
    let mut oversized = first.manifest.clone();
    oversized.edge_count = (compass_graph::GRAPH_SNAPSHOT_MAX_ITEMS as u64) + 1;
    assert!(matches!(oversized.validate(), Err(SnapshotError::Limit(_))));
    assert_eq!(second.new_objects, 0);
    assert!(second.reused_objects > 0);
    assert_eq!(first.write_transactions, 2);
    assert_eq!(second.write_transactions, 2);
    assert!(first.bytes_written > 0);
    assert_eq!(second.bytes_written, 0);
    let selector = builder.activate(&store, &first)?;
    let reader = GraphSnapshotReader::open_selector(&store, selector)?;
    assert_eq!(
        reader.get_node("a")?.map(|node| node.id),
        Some("a".to_owned())
    );
    assert_eq!(reader.outgoing("a", limits(4))?.len(), 2);
    assert_eq!(reader.incoming("b", limits(4))?.len(), 2);
    assert!(reader.outgoing("b", limits(4))?.is_empty());
    let node_ids = ["a".to_owned(), "b".to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        reader
            .get_nodes_by_ids_bounded_work(&node_ids, limits(4))?
            .into_iter()
            .map(|node| node.id)
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let edge_ids = graph()
        .links
        .into_iter()
        .map(|edge| edge.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        reader
            .get_edges_by_ids_bounded_work(&edge_ids, limits(4))?
            .into_iter()
            .map(|edge| edge.id)
            .collect::<Vec<_>>(),
        edge_ids.into_iter().collect::<Vec<_>>()
    );
    let (named, named_truncated) = reader.nodes_by_normalized_name("A", limits(4))?;
    assert!(!named_truncated);
    assert_eq!(
        named.into_iter().map(|node| node.id).collect::<Vec<_>>(),
        ["a"]
    );
    let (term_nodes, term_truncated) =
        reader.nodes_for_terms(&["rust".to_owned()], limits(12_800))?;
    assert!(!term_truncated);
    assert_eq!(term_nodes.len(), 3);
    assert_eq!(
        reader.file_by_path("src/lib.rs")?.map(|file| file.path),
        Some("src/lib.rs".to_owned())
    );
    let (calls, calls_truncated) =
        reader.adjacency_by_kinds("a", false, &[EdgeKind::Calls], limits(4))?;
    assert!(!calls_truncated);
    assert_eq!(calls.len(), 2);
    assert_eq!(reader.export_graph()?, graph_sorted());
    assert_eq!(
        reader.export_json_bytes()?,
        serde_json::to_vec(&graph_sorted())?
    );

    let indexes = reader
        .manifest()
        .roots
        .iter()
        .map(|root| root.index)
        .collect::<Vec<_>>();
    assert_eq!(indexes, IndexKind::ALL);
    assert!(
        reader
            .manifest()
            .roots
            .iter()
            .filter(|root| {
                root.index != IndexKind::Diagnostics && root.index != IndexKind::Communities
            })
            .all(|root| root.entry_count > 0)
    );
    Ok(())
}

#[test]
fn nodes_for_terms_matches_diacritic_normalized_queries() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    let mut cafe = node("cafe");
    cafe.name = "café".to_owned();
    cafe.qualified_name = "crate::café".to_owned();
    document.nodes.push(cafe);
    let mut identifier = node("identifier");
    identifier.name = "HTTPCheckpoint_session_state".to_owned();
    identifier.qualified_name = "crate::HTTPCheckpoint_session_state".to_owned();
    document.nodes.push(identifier);

    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    assert!(reader.supports_identifier_subwords()?);

    let (nodes, truncated) = reader.nodes_for_terms(&["cafe".to_owned()], limits(128))?;
    assert!(!truncated);
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes.into_iter().map(|node| node.id).collect::<Vec<_>>(),
        ["cafe"]
    );

    for term in ["http", "checkpoint", "session", "state"] {
        let (nodes, truncated) = reader.nodes_for_terms(&[term.to_owned()], limits(128))?;
        assert!(!truncated, "{term}");
        assert_eq!(
            nodes.into_iter().map(|node| node.id).collect::<Vec<_>>(),
            ["identifier"],
            "{term}"
        );
    }

    let (nodes_with_punctuation, truncated) =
        reader.nodes_for_terms(&["café".to_owned()], limits(128))?;
    assert!(!truncated);
    assert_eq!(
        nodes_with_punctuation
            .into_iter()
            .map(|node| node.id)
            .collect::<Vec<_>>(),
        ["cafe"]
    );

    Ok(())
}

#[test]
fn identifier_subword_capability_is_found_in_a_multilevel_terms_tree() -> Result<(), Box<dyn Error>>
{
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    document.nodes.clear();
    document.links.clear();
    document.nodes.push(node("caller"));
    for index in 0..4_200 {
        let target_id = format!("term-{index:04}");
        let mut term_node = node(&target_id);
        term_node.name = format!("UniqueCapabilityTerm{index:04}");
        term_node.qualified_name = format!("fixture::UniqueCapabilityTerm{index:04}");
        document.nodes.push(term_node);
        let call_id = edge_id("caller", EdgeKind::Calls, &target_id, None, None);
        document.links.push(EdgeRecord {
            id: call_id.clone(),
            key: call_id,
            source: "caller".to_owned(),
            target: target_id,
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: vec![evidence()],
            weight: None,
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        });
    }

    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    let terms = reader
        .manifest()
        .roots
        .iter()
        .find(|root| root.index == IndexKind::Terms)
        .ok_or("terms root missing")?;
    assert!(terms.entry_count > 4_096);
    assert!(reader.supports_identifier_subwords()?);
    assert!(reader.supports_relationship_terms()?);
    assert!(reader.relationship_source_matches_term("caller", "uniquecapabilityterm4199")?);
    let (source_ids, truncated, work) = reader
        .source_ids_for_exact_relationship_term_bounded_work(
            "uniquecapabilityterm4199",
            limits(128),
        )?;
    assert!(!truncated);
    assert_eq!(source_ids, ["caller"]);
    assert_eq!(work.node_ids_decoded, 1);
    let (target_ids, target_truncated, target_work) = reader
        .relationship_target_ids_for_source_terms_bounded_work(
            "caller",
            &["uniquecapabilityterm4199".to_owned()]
                .into_iter()
                .collect(),
            limits(128),
        )?;
    assert!(!target_truncated);
    assert_eq!(target_ids, ["term-4199"]);
    assert_eq!(target_work.node_ids_decoded, 1);
    let common_terms = ["capability".to_owned(), "unique".to_owned()]
        .into_iter()
        .collect();
    let (bounded_targets, bounded_truncated, bounded_work) = reader
        .relationship_target_ids_for_source_terms_bounded_work(
            "caller",
            &common_terms,
            limits(17),
        )?;
    assert!(bounded_truncated);
    assert_eq!(bounded_targets.len(), 9);
    assert_eq!(
        bounded_targets.first().map(String::as_str),
        Some("term-0000")
    );
    assert_eq!(
        bounded_targets.last().map(String::as_str),
        Some("term-0008")
    );
    assert_eq!(bounded_work.node_ids_decoded, 17);
    Ok(())
}

#[test]
fn multi_term_prefix_lookup_includes_longer_symbol_terms() -> Result<(), Box<dyn Error>> {
    let mut document = graph();
    let mut list = node("n:list");
    list.name = "list".to_owned();
    list.qualified_name = "UserService.list".to_owned();
    let mut listing = node("n:listing");
    listing.name = "listing".to_owned();
    listing.qualified_name = "UserService.listing".to_owned();
    document.nodes.extend([list, listing]);
    let call_id = edge_id("a", EdgeKind::Calls, "n:list", None, None);
    document.links.push(EdgeRecord {
        id: call_id.clone(),
        key: call_id,
        source: "a".to_owned(),
        target: "n:list".to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: None,
        relationship_site: None,
        details: None,
        evidence: vec![evidence()],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    document.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let store = MemoryStore::default();
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &document)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;

    let (nodes, truncated) = reader.nodes_for_terms(
        &["userservice".to_owned(), "list".to_owned()],
        SnapshotReadLimits::default(),
    )?;

    assert!(!truncated);
    assert_eq!(
        nodes
            .into_iter()
            .map(|node| node.id)
            .filter(|id| id.starts_with("n:list"))
            .collect::<Vec<_>>(),
        ["n:list", "n:listing"]
    );
    for term in ["user", "service"] {
        let (exact_nodes, exact_truncated, _) =
            reader.nodes_for_exact_term_bounded_work(term, SnapshotReadLimits::default())?;
        assert!(!exact_truncated);
        assert_eq!(
            exact_nodes
                .into_iter()
                .map(|node| node.id)
                .filter(|id| id.starts_with("n:list"))
                .collect::<Vec<_>>(),
            ["n:list", "n:listing"]
        );
    }
    let (callers, caller_truncated, _) = reader
        .source_ids_for_exact_relationship_term_bounded_work(
            "list",
            SnapshotReadLimits::default(),
        )?;
    assert!(!caller_truncated);
    assert_eq!(callers, ["a"]);
    let (targets, target_truncated, target_work) = reader
        .relationship_target_ids_for_source_terms_bounded_work(
            "a",
            &["list".to_owned()].into_iter().collect(),
            SnapshotReadLimits::default(),
        )?;
    assert!(!target_truncated);
    assert_eq!(targets, ["n:list"]);
    assert_eq!(target_work.node_ids_decoded, 1);
    for namespace_only in ["user", "service"] {
        assert!(!reader.relationship_source_matches_term("a", namespace_only)?);
    }
    Ok(())
}

#[test]
fn relationship_target_batch_dedupes_shared_targets_under_one_low_bound()
-> Result<(), Box<dyn Error>> {
    let mut document = graph();
    let mut shared = node("t:shared");
    shared.name = "CheckpointCreate".to_owned();
    let mut create = node("t:create");
    create.name = "CreateSession".to_owned();
    document.nodes.extend([shared, create]);
    for (rule, target) in [
        (None, "t:shared"),
        (Some("parallel"), "t:shared"),
        (None, "t:create"),
    ] {
        let id = edge_id("a", EdgeKind::Calls, target, None, rule);
        document.links.push(EdgeRecord {
            id: id.clone(),
            key: id,
            source: "a".to_owned(),
            target: target.to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: rule.and_then(OccurrenceRule::new),
            relationship_site: None,
            details: None,
            evidence: vec![evidence()],
            weight: None,
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        });
    }
    document.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    document.links.sort_by(|left, right| left.id.cmp(&right.id));
    let store = MemoryStore::default();
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &document)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    let terms = ["checkpoint".to_owned(), "create".to_owned()]
        .into_iter()
        .collect();

    let (targets, truncated, work) = reader.relationship_target_ids_for_source_terms_bounded_work(
        "a",
        &terms,
        SnapshotReadLimits::default(),
    )?;
    assert!(!truncated);
    assert_eq!(targets, ["t:create", "t:shared"]);
    assert_eq!(work.node_ids_decoded, 3);

    let (bounded, bounded_truncated, bounded_work) =
        reader.relationship_target_ids_for_source_terms_bounded_work("a", &terms, limits(2))?;
    assert!(bounded_truncated);
    assert_eq!(bounded, ["t:create", "t:shared"]);
    assert_eq!(bounded_work.node_ids_decoded, 2);
    Ok(())
}

#[test]
fn term_posting_work_is_bounded_before_multi_term_intersection() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    document.nodes.clear();
    document.links.clear();
    for index in 0..128 {
        let mut candidate = node(&format!("n:{index:03}"));
        candidate.name = if index == 0 { "alpha" } else { "beta" }.to_owned();
        document.nodes.push(candidate);
    }
    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    assert_eq!(reader.nodes(limits(300))?.len(), 128);

    let (alpha, alpha_truncated, alpha_work) =
        reader.nodes_for_terms_bounded_work(&["crat".to_owned()], limits(128))?;
    assert_eq!(alpha.len(), 128);
    assert!(!alpha_truncated);
    assert_eq!(alpha_work.node_ids_decoded, 128);

    let (nodes, truncated, work) = reader
        .nodes_for_terms_bounded_work(&["crat".to_owned(), "alpha".to_owned()], limits(256))?;
    assert_eq!(nodes.len(), 1);
    assert!(!truncated);
    assert_eq!(work.chunks_decoded, 2);
    assert_eq!(work.node_ids_decoded, 129);

    let (nodes, truncated, work) =
        reader.nodes_for_terms_bounded_work(&["crat".to_owned()], limits(127))?;
    assert!(nodes.is_empty());
    assert!(truncated);
    assert_eq!(work.node_ids_decoded, 0);
    Ok(())
}

#[test]
fn selector_is_not_active_until_commit_and_reads_are_bounded() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let prepared = builder.prepare(&store, &graph())?;
    assert!(active_graph_snapshot(&store)?.is_none());
    assert!(matches!(GraphSnapshotReader::open_active(&store), Ok(None)));
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active reader missing")?;
    assert!(matches!(
        reader.nodes(limits(1)),
        Err(SnapshotError::Limit(message)) if message.contains("item limit")
    ));
    Ok(())
}

#[test]
fn file_node_delta_reuses_unaffected_index_trees() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let previous = graph();
    let first = builder.prepare(&store, &previous)?;
    builder.activate(&store, &first)?;

    let mut current = previous.clone();
    current.graph.build.generation_id = "next-generation".to_owned();
    current.graph.files[0].content_digest = "sha256:changed".to_owned();
    current.graph.files[0].byte_size = 2;
    let file_node = current
        .nodes
        .iter_mut()
        .find(|node| node.kind == NodeKind::File)
        .ok_or("file node missing")?;
    file_node.details = Some(NodeDetails::File(FileNodeDetails {
        content_digest: "sha256:changed".to_owned(),
        byte_size: 2,
        generated: false,
    }));

    let content = builder.prepare_file_node_delta(&store, &previous, &current)?;
    let graph_bytes = canonical_graph_json(&current)?;
    let graph_digest = format!("{:x}", sha2::Sha256::digest(&graph_bytes));
    let delta = builder.finish_content(&store, content, graph_digest, graph_bytes.len() as u64)?;
    let first_roots = first
        .manifest
        .roots
        .iter()
        .map(|root| (root.index, root.digest.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let delta_roots = delta
        .manifest
        .roots
        .iter()
        .map(|root| (root.index, root.digest.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for index in [
        IndexKind::Edges,
        IndexKind::Outgoing,
        IndexKind::Incoming,
        IndexKind::Files,
        IndexKind::Names,
        IndexKind::Terms,
        IndexKind::Communities,
        IndexKind::Diagnostics,
    ] {
        assert_eq!(
            first_roots.get(&index),
            delta_roots.get(&index),
            "{index:?}"
        );
    }
    assert_ne!(
        first_roots.get(&IndexKind::Nodes),
        delta_roots.get(&IndexKind::Nodes)
    );
    assert_ne!(
        first_roots.get(&IndexKind::Metadata),
        delta_roots.get(&IndexKind::Metadata)
    );

    builder.activate(&store, &delta)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    assert_eq!(
        reader.get_node("file")?.and_then(|node| {
            node.details.and_then(|details| match details {
                NodeDetails::File(file) => Some(file.content_digest),
                _ => None,
            })
        }),
        Some("sha256:changed".to_owned())
    );
    assert_eq!(reader.metadata()?.graph.files[0].byte_size, 2);
    assert_eq!(reader.export_graph()?, graph_sorted_with(&current));
    Ok(())
}

#[test]
fn graph_delta_rebuilds_relationship_indexes_without_rewriting_nodes() -> Result<(), Box<dyn Error>>
{
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let previous = graph();
    let first = builder.prepare(&store, &previous)?;
    builder.activate(&store, &first)?;

    let mut current = previous.clone();
    current.graph.build.generation_id = "next-generation".to_owned();
    let mut replacement = current.links.pop().ok_or("relationship missing")?;
    replacement.source = "b".to_owned();
    replacement.target = "a".to_owned();
    replacement.occurrence_rule = OccurrenceRule::new("third");
    replacement.id = edge_id(
        &replacement.source,
        replacement.kind,
        &replacement.target,
        replacement.relationship_site.as_ref(),
        replacement
            .occurrence_rule
            .as_ref()
            .map(OccurrenceRule::as_str),
    );
    replacement.key.clone_from(&replacement.id);
    current.links.push(replacement.clone());

    let content = builder.prepare_graph_delta(&store, &previous, &current)?;
    let graph_bytes = canonical_graph_json(&current)?;
    let graph_digest = format!("{:x}", sha2::Sha256::digest(&graph_bytes));
    let delta = builder.finish_content(&store, content, graph_digest, graph_bytes.len() as u64)?;
    let first_roots = first
        .manifest
        .roots
        .iter()
        .map(|root| (root.index, root.digest.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let delta_roots = delta
        .manifest
        .roots
        .iter()
        .map(|root| (root.index, root.digest.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for index in [
        IndexKind::Nodes,
        IndexKind::Names,
        IndexKind::Files,
        IndexKind::Communities,
        IndexKind::Diagnostics,
    ] {
        assert_eq!(
            first_roots.get(&index),
            delta_roots.get(&index),
            "{index:?}"
        );
    }
    for index in [
        IndexKind::Metadata,
        IndexKind::Edges,
        IndexKind::Outgoing,
        IndexKind::Incoming,
    ] {
        assert_ne!(
            first_roots.get(&index),
            delta_roots.get(&index),
            "{index:?}"
        );
    }

    builder.activate(&store, &delta)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    assert_eq!(reader.get_edge(&replacement.id)?, Some(replacement));
    assert_eq!(reader.outgoing("b", limits(4))?.len(), 1);
    assert_eq!(reader.export_graph()?, graph_sorted_with(&current));
    Ok(())
}

#[test]
fn graph_delta_rebuilds_discovery_scope_postings() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut previous = graph();
    for path in ["src/old_a.rs", "src/new.rs"] {
        let mut file = previous.graph.files[0].clone();
        file.id = file_id(path);
        file.path = path.to_owned();
        previous.graph.files.push(file);
    }
    previous
        .graph
        .files
        .sort_by(|left, right| left.id.cmp(&right.id));
    let previous_node = previous
        .nodes
        .iter_mut()
        .find(|node| node.id == "a")
        .ok_or("node a missing")?;
    previous_node.source = Some(SourceAnchor {
        file: "src/old_a.rs".to_owned(),
        ..anchor()
    });
    previous_node.community = Some(CommunityMetadata {
        id: 7,
        label: Some("old-community".to_owned()),
        score: None,
        color: None,
    });
    let first = builder.prepare(&store, &previous)?;
    builder.activate(&store, &first)?;

    let mut current = previous.clone();
    let current_node = current
        .nodes
        .iter_mut()
        .find(|node| node.id == "a")
        .ok_or("node a missing")?;
    current_node.qualified_name = "new_package::a".to_owned();
    current_node.source = Some(SourceAnchor {
        file: "src/new.rs".to_owned(),
        ..anchor()
    });
    current_node.community = Some(CommunityMetadata {
        id: 8,
        label: Some("new-community".to_owned()),
        score: None,
        color: None,
    });

    let content = builder.prepare_graph_delta(&store, &previous, &current)?;
    let graph_bytes = canonical_graph_json(&current)?;
    let graph_digest = format!("{:x}", sha2::Sha256::digest(&graph_bytes));
    let delta = builder.finish_content(&store, content, graph_digest, graph_bytes.len() as u64)?;
    builder.activate(&store, &delta)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;

    for (kind, value) in [
        ("node-qname", "crate::a"),
        ("source", "src/old_a.rs"),
        ("community-label", "old-community"),
    ] {
        assert!(
            reader
                .resolve_scope_values(kind, value, limits(16))?
                .0
                .is_empty()
        );
    }
    assert_eq!(
        reader
            .resolve_scope_values("node-qname", "new_package::a", limits(16))?
            .0,
        ["a"]
    );
    assert_eq!(
        reader
            .resolve_scope_values("source", "src/new.rs", limits(16))?
            .0,
        ["src/new.rs"]
    );
    assert_eq!(
        reader
            .resolve_scope_values("community-label", "new-community", limits(16))?
            .0,
        ["8"]
    );
    Ok(())
}

#[test]
fn missing_or_tampered_objects_fail_closed() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let prepared = builder.prepare(&store, &graph())?;
    let selector = builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_selector(&store, selector)?;
    let root = reader
        .manifest()
        .roots
        .iter()
        .find(|root| root.index == IndexKind::Nodes)
        .ok_or("nodes root missing")?;
    let namespace = NamespaceId::graph();
    let partition = PartitionKey::new("graph-snapshot/objects")?;
    let key = Key::new(format!("object/{}", root.digest))?;
    store.put(
        &namespace,
        &partition,
        &key,
        b"corrupt",
        WriteCondition::Any,
    )?;
    assert!(matches!(
        reader.get_node("a"),
        Err(SnapshotError::Corrupt(_))
    ));
    Ok(())
}

#[test]
fn sqlite_adapter_round_trips_the_same_snapshot_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("snapshot.sqlite3");
    let store = SqliteStore::open(&path)?;
    let builder = GraphSnapshotBuilder::new();
    let prepared = builder.prepare(&store, &graph())?;
    builder.activate(&store, &prepared)?;
    drop(store);

    let reopened = SqliteStore::open_read_only(&path)?;
    let reader = GraphSnapshotReader::open_active(&reopened)?.ok_or("active snapshot missing")?;
    assert_eq!(
        reader.get_node("a")?.map(|node| node.id),
        Some("a".to_owned())
    );
    assert_eq!(reader.outgoing("a", limits(4))?.len(), 2);
    assert_eq!(reader.manifest().snapshot_id, prepared.manifest.snapshot_id);
    assert_eq!(reader.export_json_bytes()?, canonical_graph_json(&graph())?);
    let reference = reopened.snapshot_reference()?;
    assert_eq!(reference.snapshot_id, prepared.manifest.snapshot_id);
    assert_eq!(reference.manifest_digest, prepared.manifest_digest);
    assert_eq!(reference.graph_digest, prepared.manifest.graph_digest);
    Ok(())
}

#[test]
fn exact_leaf_fanout_does_not_publish_an_empty_trailing_child() -> Result<(), Box<dyn Error>> {
    const EXACT_LEAF_FANOUT: usize = 128;
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    for index in document.nodes.len()..EXACT_LEAF_FANOUT {
        document.nodes.push(node(&format!("node-{index:04}")));
    }

    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;

    assert_eq!(
        reader.nodes(limits(document.nodes.len()))?.len(),
        document.nodes.len()
    );
    assert_eq!(
        reader.export_json_bytes()?,
        canonical_graph_json(&document)?
    );
    Ok(())
}

#[test]
fn graph_snapshot_gc_retains_only_selected_immutable_trees() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let first = builder.prepare(&store, &graph())?;
    let first_selector = builder.activate(&store, &first)?;
    let mut changed = graph();
    changed.nodes.push(node("c"));
    let second = builder.prepare(&store, &changed)?;
    let second_selector = builder.activate(&store, &second)?;
    changed.nodes.push(node("d"));
    let _third = builder.prepare(&store, &changed)?;
    assert!(graph_snapshot_needs_gc(&store, 2)?);

    let stats =
        garbage_collect_graph_snapshots(&store, std::slice::from_ref(&second_selector), 10_000)?;
    assert_eq!(stats.retained_manifests, 1);
    assert!(stats.retained_objects > 1);
    assert!(stats.deleted_entries > 0);
    assert!(stats.delete_transactions > 0);
    assert!(!graph_snapshot_needs_gc(&store, 2)?);
    assert!(GraphSnapshotReader::open_selector(&store, first_selector).is_err());
    assert_eq!(
        GraphSnapshotReader::open_selector(&store, second_selector)?
            .get_node("c")?
            .map(|node| node.id),
        Some("c".to_owned())
    );
    Ok(())
}

#[test]
fn graph_key_vectors_are_namespace_safe_and_orderable() -> Result<(), Box<dyn Error>> {
    let left = encode_graph_index_key(IndexKind::Nodes, &[b"a"])?;
    let right = encode_graph_index_key(IndexKind::Nodes, &[b"b"])?;
    assert_eq!(
        left,
        vec![
            1, 2, 0, 0, 0, 5, b'n', b'o', b'd', b'e', b's', 0, 0, 0, 1, b'a'
        ]
    );
    assert!(left < right);
    assert_ne!(
        encode_graph_index_key(IndexKind::Nodes, &[b"a", b"b"])?,
        encode_graph_index_key(IndexKind::Nodes, &[b"a|b"])?
    );
    Ok(())
}

#[test]
fn graph_valid_long_qualified_names_round_trip_through_the_store() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    document.nodes[0].qualified_name = "call_chain.".repeat(120);

    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;

    assert_eq!(
        reader.export_json_bytes()?,
        canonical_graph_json(&document)?
    );
    assert_eq!(
        reader.get_node("b")?.map(|node| node.qualified_name),
        Some("call_chain.".repeat(120))
    );
    Ok(())
}

#[test]
fn graph_valid_punctuation_names_use_an_explicit_empty_name_bucket() -> Result<(), Box<dyn Error>> {
    let store = MemoryStore::default();
    let builder = GraphSnapshotBuilder::new();
    let mut document = graph();
    let mut punctuation = node("punctuation");
    punctuation.name = "...".to_owned();
    punctuation.qualified_name = ".".to_owned();
    document.nodes.push(punctuation);

    let prepared = builder.prepare(&store, &document)?;
    builder.activate(&store, &prepared)?;
    let reader = GraphSnapshotReader::open_active(&store)?.ok_or("active snapshot missing")?;
    let (matches, truncated) = reader.nodes_by_normalized_name(".", limits(8))?;
    assert!(!truncated);
    assert_eq!(
        matches.into_iter().map(|node| node.id).collect::<Vec<_>>(),
        ["punctuation"]
    );
    assert_eq!(
        reader.export_json_bytes()?,
        canonical_graph_json(&document)?
    );
    Ok(())
}

fn graph_sorted() -> GraphDocument {
    graph_sorted_with(&graph())
}

fn graph_sorted_with(source: &GraphDocument) -> GraphDocument {
    let mut document = source.clone();
    document.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    document.links.sort_by(|left, right| left.id.cmp(&right.id));
    document
}
