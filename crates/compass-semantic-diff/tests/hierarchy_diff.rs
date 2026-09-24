//! Community hierarchy diff: surviving identity, splits, ambiguity, and bounds.

use std::collections::BTreeSet;
use std::error::Error;

use compass_graph::{
    Communities, CommunityHierarchy, CommunityLimits, CommunityProfile, CommunityRequest,
    HierarchyBudget, HierarchyRequest, ReconcilePolicy, ResolutionPolicy, build_communities,
    build_community_hierarchy,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::provenance::SourceAnchor;
use compass_semantic_diff::{
    HIERARCHY_DIFF_SCHEMA, MAX_HIERARCHY_DIFF_EVENTS, compare_hierarchies,
    compare_optional_hierarchies,
};

type TestResult = Result<(), Box<dyn Error>>;

fn node(id: &str, file: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: format!("app::{id}"),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(SourceAnchor {
            file: file.to_owned(),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 1,
        }),
        details: None,
        evidence: Vec::new(),
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn edge(index: usize, source: &str, target: &str) -> EdgeRecord {
    EdgeRecord {
        id: format!("edge-{index}"),
        key: format!("edge-{index}"),
        source: source.to_owned(),
        target: target.to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: None,
        relationship_site: None,
        details: None,
        evidence: Vec::new(),
        weight: Some(1.0),
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    }
}

fn document(nodes: Vec<NodeRecord>, links: Vec<EdgeRecord>) -> GraphDocument {
    let mut document = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "hierarchy-diff-test".to_owned(),
        schema_fingerprint: "fixture-v1".to_owned(),
        source_tree_digest: "fixture".to_owned(),
        configuration_digest: "hierarchy-diff".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    });
    document.nodes = nodes;
    document.links = links;
    document
}

fn fixture(clusters: usize, per_cluster: usize) -> GraphDocument {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        for index in 0..per_cluster {
            nodes.push(node(
                &format!("c{cluster}n{index}"),
                &format!("src/group{cluster}/file{index}.rs"),
            ));
        }
        for index in 1..per_cluster {
            links.push(edge(
                links.len(),
                &format!("c{cluster}n{}", index - 1),
                &format!("c{cluster}n{index}"),
            ));
        }
        if cluster > 0 {
            links.push(edge(
                links.len(),
                &format!("c{}n0", cluster - 1),
                &format!("c{cluster}n0"),
            ));
        }
    }
    document(nodes, links)
}

fn hierarchy(
    document: &GraphDocument,
    communities: &Communities,
) -> Result<CommunityHierarchy, Box<dyn Error>> {
    let changed = BTreeSet::new();
    let result = build_communities(
        document,
        &CommunityRequest {
            profile: CommunityProfile::QualityV1,
            resolution: ResolutionPolicy::Fixed(1.0),
            exclude_hubs_percentile: None,
            previous: None,
            incremental: false,
            changed_sources: &changed,
            limits: CommunityLimits::default(),
        },
    )?;
    let draft = build_community_hierarchy(
        document,
        communities,
        &HierarchyRequest {
            identity: &result.identity,
            limits: &CommunityLimits::default(),
            resolution: result.quality.resolution,
            budget: HierarchyBudget::default(),
        },
    )?;
    Ok(CommunityHierarchy::new(
        document.graph.build.generation_id.clone(),
        format!("sha256:{}", "0".repeat(64)),
        draft,
    )?)
}

fn partition(document: &GraphDocument) -> Result<Communities, Box<dyn Error>> {
    let changed = BTreeSet::new();
    Ok(build_communities(
        document,
        &CommunityRequest {
            profile: CommunityProfile::QualityV1,
            resolution: ResolutionPolicy::Fixed(1.0),
            exclude_hubs_percentile: None,
            previous: None,
            incremental: false,
            changed_sources: &changed,
            limits: CommunityLimits::default(),
        },
    )?
    .communities)
}

#[test]
fn an_unchanged_hierarchy_diffs_as_stable() -> TestResult {
    let document = fixture(6, 3);
    let communities = partition(&document)?;
    let base = hierarchy(&document, &communities)?;
    let target = hierarchy(&document, &communities)?;
    let diff = compare_hierarchies(
        &base,
        &communities,
        &target,
        &communities,
        &ReconcilePolicy::default(),
    )?;
    assert_eq!(diff.schema, HIERARCHY_DIFF_SCHEMA);
    assert!(diff.stable > 0);
    assert_eq!(diff.split, 0);
    assert_eq!(diff.merged, 0);
    assert_eq!(diff.appeared, 0);
    assert_eq!(diff.disappeared, 0);
    assert_eq!(diff.ambiguous, 0);
    assert!(diff.is_empty());
    assert!(diff.events.is_empty(), "unchanged membership lists nothing");
    diff.validate()?;
    Ok(())
}

#[test]
fn a_deleted_cluster_diffs_as_disappeared() -> TestResult {
    let document = fixture(6, 3);
    let communities = partition(&document)?;
    let base = hierarchy(&document, &communities)?;
    let mut shrunk = document.clone();
    shrunk.nodes.retain(|node| !node.id.starts_with("c5"));
    shrunk
        .links
        .retain(|edge| !edge.source.starts_with("c5") && !edge.target.starts_with("c5"));
    let target_communities = partition(&shrunk)?;
    let target = hierarchy(&shrunk, &target_communities)?;
    let diff = compare_hierarchies(
        &base,
        &communities,
        &target,
        &target_communities,
        &ReconcilePolicy::default(),
    )?;
    assert!(
        diff.disappeared >= 1,
        "a removed group is reported: {diff:?}"
    );
    assert!(
        diff.events
            .iter()
            .filter(|event| event.kind == compass_graph::HierarchyEventKind::Disappeared)
            .all(|event| !event.base_ids.is_empty() && event.target_ids.is_empty())
    );
    diff.validate()?;
    Ok(())
}

#[test]
fn an_even_split_is_ambiguous_and_never_resolved() -> TestResult {
    let document = fixture(1, 8);
    let ids = document
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    // Two hand-made communities, so the split is exactly even: one base group
    // becomes two successors that overlap it identically.
    let mut published = Communities::new();
    published.insert(0, ids.iter().take(4).cloned().collect::<Vec<_>>());
    published.insert(1, ids.iter().skip(4).cloned().collect::<Vec<_>>());
    let base = hierarchy(&document, &published)?;
    let mut carved = Communities::new();
    carved.insert(0, ids.iter().take(2).cloned().collect::<Vec<_>>());
    carved.insert(1, ids.iter().skip(2).take(2).cloned().collect::<Vec<_>>());
    carved.insert(2, ids.iter().skip(4).cloned().collect::<Vec<_>>());
    let target = hierarchy(&document, &carved)?;
    let diff = compare_hierarchies(
        &base,
        &published,
        &target,
        &carved,
        &ReconcilePolicy::default(),
    )?;
    assert_eq!(
        diff.ambiguous, 1,
        "an even split is reported as ambiguous: {diff:?}"
    );
    let ambiguous = diff
        .events
        .iter()
        .find(|event| event.kind == compass_graph::HierarchyEventKind::Ambiguous)
        .ok_or("missing ambiguous event")?;
    assert_eq!(ambiguous.base_ids.len(), 1);
    assert_eq!(ambiguous.target_ids.len(), 2, "both candidates are named");
    assert!((ambiguous.overlap - 0.5).abs() < 1e-9);
    diff.validate()?;
    Ok(())
}

#[test]
fn an_absent_hierarchy_is_unavailable_not_empty() -> TestResult {
    let document = fixture(4, 3);
    let communities = partition(&document)?;
    let base = hierarchy(&document, &communities)?;
    assert!(
        compare_optional_hierarchies(
            Some((&base, &communities)),
            None,
            &ReconcilePolicy::default()
        )?
        .is_none(),
        "a target without a hierarchy has nothing to compare"
    );
    assert!(
        compare_optional_hierarchies(None, None, &ReconcilePolicy::default())?.is_none(),
        "two absent hierarchies are not an empty diff"
    );
    assert!(
        compare_optional_hierarchies(
            Some((&base, &communities)),
            Some((&base, &communities)),
            &ReconcilePolicy::default()
        )?
        .is_some()
    );
    Ok(())
}

#[test]
fn the_event_list_is_bounded_and_digest_is_deterministic() -> TestResult {
    let document = fixture(8, 3);
    let communities = partition(&document)?;
    let base = hierarchy(&document, &communities)?;
    let mut changed = document.clone();
    for cluster in 0..8 {
        changed.nodes.push(node(
            &format!("c{cluster}extra"),
            &format!("src/group{cluster}/extra.rs"),
        ));
    }
    for cluster in 0..8 {
        changed.links.push(edge(
            changed.links.len(),
            &format!("c{cluster}n0"),
            &format!("c{cluster}extra"),
        ));
    }
    let target_communities = partition(&changed)?;
    let target = hierarchy(&changed, &target_communities)?;
    let policy = ReconcilePolicy {
        max_events: 1,
        ..ReconcilePolicy::default()
    };
    let first = compare_hierarchies(&base, &communities, &target, &target_communities, &policy)?;
    let second = compare_hierarchies(&base, &communities, &target, &target_communities, &policy)?;
    assert_eq!(first.events.len(), 1);
    let total = first.stable
        + first.split
        + first.merged
        + first.appeared
        + first.disappeared
        + first.ambiguous;
    assert!(total >= first.events.len());
    assert_eq!(
        first.omitted_events + first.events.len(),
        total
            .min(MAX_HIERARCHY_DIFF_EVENTS.max(1))
            .max(first.events.len())
    );
    assert_eq!(first.result_digest, second.result_digest);
    assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
    first.validate()?;
    Ok(())
}
