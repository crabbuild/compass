//! Budgeted community hierarchy: determinism, coverage, label provenance, and
//! honest accounting when a budget cannot be met.

use std::collections::BTreeSet;

use compass_graph::{
    Communities, CommunityError, CommunityHierarchy, CommunityLimits, CommunityProfile,
    CommunityRequest, CommunityResult, HierarchyBudget, HierarchyLabelRule, HierarchyRequest,
    LevelMerge, ReconcilePolicy, ResolutionPolicy, build_communities, build_community_hierarchy,
    reconcile_hierarchy,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::provenance::SourceAnchor;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn node(id: &str, file: Option<&str>, qualified: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: qualified.to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: file.map(|file| SourceAnchor {
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

fn edge(index: usize, source: &str, target: &str, kind: EdgeKind) -> EdgeRecord {
    EdgeRecord {
        id: format!("edge-{index}"),
        key: format!("edge-{index}"),
        source: source.to_owned(),
        target: target.to_owned(),
        kind,
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
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-test".to_owned(),
        source_commit: None,
    });
    document.nodes = nodes;
    document.links = links;
    document
}

/// `clusters` compact groups of `per_cluster` symbols. Calls inside a group are
/// strong evidence; the optional bridge is weak evidence between neighbours.
fn clustered_document(clusters: usize, per_cluster: usize, bridges: bool) -> GraphDocument {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        for index in 0..per_cluster {
            let id = format!("cluster{cluster}_symbol{index}");
            nodes.push(node(
                &id,
                Some(&format!("src/group{cluster}/file{index}.rs")),
                &format!("app::group{cluster}::symbol{index}"),
            ));
        }
        for index in 1..per_cluster {
            let previous = format!("cluster{cluster}_symbol{}", index - 1);
            let current = format!("cluster{cluster}_symbol{index}");
            links.push(edge(links.len(), &previous, &current, EdgeKind::Calls));
        }
        if bridges && cluster > 0 {
            let previous = format!("cluster{}_symbol0", cluster - 1);
            let current = format!("cluster{cluster}_symbol0");
            links.push(edge(links.len(), &previous, &current, EdgeKind::References));
        }
    }
    document(nodes, links)
}

/// The same shape without any source anchors: nothing here can cite a location,
/// so no rule may merge these groups.
fn sourceless_document(clusters: usize, per_cluster: usize) -> GraphDocument {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        for index in 0..per_cluster {
            let id = format!("cluster{cluster}_symbol{index}");
            nodes.push(node(
                &id,
                None,
                &format!("app::group{cluster}::symbol{index}"),
            ));
        }
        for index in 1..per_cluster {
            let previous = format!("cluster{cluster}_symbol{}", index - 1);
            let current = format!("cluster{cluster}_symbol{index}");
            links.push(edge(links.len(), &previous, &current, EdgeKind::Calls));
        }
    }
    document(nodes, links)
}

/// `directories` areas of `clusters` disjoint symbol chains, each chain under
/// `src/area<area>/group<cluster>`. Chains share no relationship, so the
/// location cut is the only rule that can merge them, and every named group
/// sits under one of a bounded number of real directories.
fn shared_directory_document(
    directories: usize,
    clusters: usize,
    per_cluster: usize,
) -> GraphDocument {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        let area = cluster % directories;
        for index in 0..per_cluster {
            let id = format!("area{area}_cluster{cluster}_symbol{index}");
            nodes.push(node(
                &id,
                Some(&format!("src/area{area}/group{cluster}/file{index}.rs")),
                &format!("app::area{area}::group{cluster}::symbol{index}"),
            ));
        }
        for index in 1..per_cluster {
            let previous = format!("area{area}_cluster{cluster}_symbol{}", index - 1);
            let current = format!("area{area}_cluster{cluster}_symbol{index}");
            links.push(edge(links.len(), &previous, &current, EdgeKind::Calls));
        }
    }
    document(nodes, links)
}

fn partition(document: &GraphDocument) -> Result<CommunityResult, CommunityError> {
    let changed = BTreeSet::new();
    build_communities(
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
    )
}

fn hierarchy_of(
    document: &GraphDocument,
    communities: &Communities,
    resolution: f64,
    budget: HierarchyBudget,
) -> Result<CommunityHierarchy, Box<dyn std::error::Error>> {
    let identity = partition(document)?.identity;
    let draft = build_community_hierarchy(
        document,
        communities,
        &HierarchyRequest {
            identity: &identity,
            limits: &CommunityLimits::default(),
            resolution,
            budget,
        },
    )?;
    Ok(CommunityHierarchy::new(
        "generation-test".to_owned(),
        format!("sha256:{}", "0".repeat(64)),
        draft,
    )?)
}

fn hierarchy(
    document: &GraphDocument,
    budget: HierarchyBudget,
) -> Result<CommunityHierarchy, Box<dyn std::error::Error>> {
    let result = partition(document)?;
    let resolution = result.quality.resolution;
    hierarchy_of(document, &result.communities, resolution, budget)
}

fn cover_every_child_once(
    hierarchy: &CommunityHierarchy,
) -> Result<(), Box<dyn std::error::Error>> {
    for pair in hierarchy.levels.windows(2) {
        let (Some(coarser), Some(finer)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let mut seen = vec![0usize; finer.groups.len()];
        for group in &coarser.groups {
            let mut members = 0usize;
            for child in &group.child_indices {
                let slot = seen.get_mut(*child).ok_or("child index out of range")?;
                *slot += 1;
                let child_group = finer.groups.get(*child).ok_or("missing child group")?;
                members += child_group.member_count;
            }
            assert_eq!(
                members, group.member_count,
                "level {} group {} member count disagrees with its children",
                coarser.level, group.index
            );
        }
        assert!(
            seen.iter().all(|count| *count == 1),
            "level {} children are not covered exactly once: {seen:?}",
            coarser.level
        );
    }
    Ok(())
}

#[test]
fn hierarchy_is_byte_identical_across_builds() -> TestResult {
    let document = clustered_document(12, 3, true);
    let first = hierarchy(&document, HierarchyBudget::default())?;
    let second = hierarchy(&document, HierarchyBudget::default())?;
    assert_eq!(
        serde_json::to_vec(&first)?,
        serde_json::to_vec(&second)?,
        "identical input must serialise identically"
    );
    assert_eq!(first.result_digest, second.result_digest);
    assert!(first.result_digest.starts_with("sha256:"));
    Ok(())
}

#[test]
fn group_ids_are_member_signatures_and_ignore_node_order() -> TestResult {
    let document = clustered_document(8, 3, true);
    let first = hierarchy(&document, HierarchyBudget::default())?;
    let mut shuffled = document.clone();
    shuffled.nodes.reverse();
    let second = hierarchy(&shuffled, HierarchyBudget::default())?;
    for (left, right) in first.levels.iter().zip(second.levels.iter()) {
        let ids = left
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<Vec<_>>();
        let mirrored = right
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, mirrored, "node order must not move a group id");
        for group in &left.groups {
            assert!(
                group.signature.len() == 16
                    && group
                        .signature
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert_eq!(group.id, format!("h{}-{}", left.level, group.signature));
        }
        assert_eq!(left.signature.len(), 16);
    }
    Ok(())
}

#[test]
fn an_untouched_group_keeps_its_id_when_another_group_changes() -> TestResult {
    let document = clustered_document(6, 3, true);
    let before = hierarchy(&document, HierarchyBudget::default())?;
    let mut grown = document.clone();
    grown.nodes.push(node(
        "cluster0_newcomer",
        Some("src/group0/newcomer.rs"),
        "app::group0::newcomer",
    ));
    let edge_index = grown.links.len();
    grown.links.push(edge(
        edge_index,
        "cluster0_symbol0",
        "cluster0_newcomer",
        EdgeKind::Calls,
    ));
    let after = hierarchy(&grown, HierarchyBudget::default())?;
    let finest_before = before.finest().ok_or("missing finest level")?;
    let finest_after = after.finest().ok_or("missing finest level")?;
    let unchanged = finest_before
        .groups
        .iter()
        .filter(|group| group.community != Some(0))
        .map(|group| (group.id.clone(), group.member_count))
        .collect::<Vec<_>>();
    for (id, members) in unchanged {
        assert!(
            finest_after
                .groups
                .iter()
                .any(|group| group.id == id && group.member_count == members),
            "an untouched group lost its id {id}"
        );
    }
    Ok(())
}

#[test]
fn budgeted_hierarchy_stays_inside_its_budget_and_covers_every_community() -> TestResult {
    let document = clustered_document(24, 3, true);
    let hierarchy = hierarchy(
        &document,
        HierarchyBudget {
            root_target: 6,
            level_target: 12,
            max_levels: 4,
            ..HierarchyBudget::default()
        },
    )?;
    let root = hierarchy.root().ok_or("missing root level")?;
    assert_eq!(root.level, 0, "the root level is level 0");
    assert!(
        root.groups.len() <= 6,
        "root holds {} groups, budget is 6",
        root.groups.len()
    );
    assert!(hierarchy.budget_satisfied, "the root budget was met");
    assert!(
        hierarchy.levels.len() <= 4,
        "the level budget bounds the tree"
    );
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    assert_eq!(finest.groups.len(), hierarchy.finest_community_count);
    assert!(
        hierarchy.finest_community_count >= 12,
        "the fixture should publish a partition worth coarsening, found {}",
        hierarchy.finest_community_count
    );
    assert!(
        finest
            .groups
            .iter()
            .all(|group| group.child_indices.is_empty())
    );
    assert!(hierarchy.levels.iter().all(|level| {
        match level.merge {
            LevelMerge::Relationship => level
                .resolution
                .is_some_and(|resolution| resolution.is_finite() && resolution > 0.0),
            LevelMerge::LocationAffinity => {
                level.resolution.is_none()
                    && level
                        .merge_evidence
                        .get("rule")
                        .and_then(serde_json::Value::as_str)
                        == Some("location-affinity")
            }
        }
    }));
    for pair in hierarchy.levels.windows(2) {
        let (Some(coarser), Some(finer)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        if let (Some(coarser_resolution), Some(finer_resolution)) =
            (coarser.resolution, finer.resolution)
        {
            assert!(
                coarser_resolution <= finer_resolution,
                "coarsening may not raise the resolution"
            );
        }
        assert!(
            coarser.groups.len() < finer.groups.len(),
            "every coarser level must merge groups: {} is not smaller than {}",
            coarser.groups.len(),
            finer.groups.len()
        );
    }
    let total = root
        .groups
        .iter()
        .map(|group| group.member_count)
        .sum::<usize>();
    assert_eq!(total, 24 * 3, "the root covers every assigned member");
    cover_every_child_once(&hierarchy)?;
    Ok(())
}

#[test]
fn single_community_partition_publishes_one_level() -> TestResult {
    let document = clustered_document(1, 4, false);
    let members = document
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let communities = Communities::from([(0usize, members)]);
    let hierarchy = hierarchy_of(&document, &communities, 1.0, HierarchyBudget::default())?;
    assert_eq!(hierarchy.levels.len(), 1);
    assert_eq!(hierarchy.finest_community_count, 1);
    assert!(hierarchy.budget_satisfied);
    let root = hierarchy.root().ok_or("missing root level")?;
    assert_eq!(root.groups.len(), 1);
    assert_eq!(root.groups[0].member_count, 4);
    assert!(root.groups[0].child_indices.is_empty());
    Ok(())
}

#[test]
fn labels_carry_provenance_and_fall_back_in_order() -> TestResult {
    let document = clustered_document(10, 3, true);
    let hierarchy = hierarchy(&document, HierarchyBudget::default())?;
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    let directory_labels = finest
        .groups
        .iter()
        .filter(|group| group.label.rule == HierarchyLabelRule::DominantDirectory)
        .collect::<Vec<_>>();
    assert!(
        directory_labels.len() * 5 >= finest.groups.len() * 3,
        "at least 60% of groups should be named by their directory, found {} of {}",
        directory_labels.len(),
        finest.groups.len()
    );
    for group in &directory_labels {
        let covered = group
            .label
            .evidence
            .get("coveredMembers")
            .and_then(serde_json::Value::as_u64)
            .ok_or("directory labels must record their coverage")?;
        let counted = group
            .label
            .evidence
            .get("memberSourceCount")
            .and_then(serde_json::Value::as_u64)
            .ok_or("directory labels must record the countable members")?;
        assert!(covered <= counted);
        assert!(covered as f64 >= 0.6 * counted as f64);
        assert!(
            group
                .label
                .evidence
                .get("value")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value.starts_with("src"))
        );
        assert!(!group.label.is_generic());
    }
    let generic = finest
        .groups
        .iter()
        .filter(|group| group.label.is_generic())
        .count();
    assert_eq!(generic, 0, "every group has directory or module evidence");
    Ok(())
}

#[test]
fn a_group_without_evidence_falls_back_to_a_generic_label() -> TestResult {
    let document = clustered_document(2, 2, true);
    let mut communities = partition(&document)?.communities.clone();
    // Members the document does not describe cannot name their group: no
    // source file, no qualified name, and no hub to fall back to.
    communities.insert(
        0,
        vec!["phantom_alpha".to_owned(), "phantom_beta".to_owned()],
    );
    let hierarchy = hierarchy_of(&document, &communities, 1.0, HierarchyBudget::default())?;
    let group = hierarchy
        .finest()
        .and_then(|level| level.groups.first())
        .ok_or("missing finest group")?;
    assert_eq!(group.label.rule, HierarchyLabelRule::CommunityId);
    assert!(group.label.is_generic());
    let value = group
        .label
        .evidence
        .get("community")
        .and_then(serde_json::Value::as_u64);
    assert_eq!(value, Some(0));
    Ok(())
}

#[test]
fn hierarchy_reports_an_unmet_budget_instead_of_truncating() -> TestResult {
    // These groups share neither a relationship nor a source location, so no
    // rule may merge them and the root stays larger than its target.
    let document = sourceless_document(12, 3);
    let hierarchy = hierarchy(
        &document,
        HierarchyBudget {
            root_target: 2,
            level_target: 4,
            max_levels: 3,
            ..HierarchyBudget::default()
        },
    )?;
    let root = hierarchy.root().ok_or("missing root level")?;
    assert!(!hierarchy.budget_satisfied);
    assert_eq!(
        hierarchy.levels.len(),
        1,
        "a group with no evidence keeps its own level instead of being merged"
    );
    assert!(
        root.groups.len() > 2,
        "an unmet budget keeps the achieved count instead of truncating"
    );
    assert_eq!(
        root.groups
            .iter()
            .map(|group| group.member_count)
            .sum::<usize>(),
        36,
        "no member is dropped when the budget cannot be met"
    );
    Ok(())
}

/// Twelve communities that never call each other, four to an area directory:
/// the relationship pass cannot merge them, so the level is cut by the
/// directory they cite and records that rule as its evidence.
#[test]
fn location_affinity_groups_communities_that_share_no_relationship() -> TestResult {
    let document = shared_directory_document(4, 12, 3);
    let hierarchy = hierarchy(
        &document,
        HierarchyBudget {
            root_target: 5,
            level_target: 8,
            max_levels: 4,
            ..HierarchyBudget::default()
        },
    )?;
    let root = hierarchy.root().ok_or("missing root level")?;
    assert_eq!(root.merge, LevelMerge::LocationAffinity);
    assert_eq!(
        root.merge_evidence
            .get("rule")
            .and_then(serde_json::Value::as_str),
        Some("location-affinity")
    );
    assert!(
        root.groups.len() <= 5,
        "the affinity cut must meet the root budget, found {}",
        root.groups.len()
    );
    assert_eq!(root.groups.len(), 4, "each area directory is one group");
    assert!(hierarchy.budget_satisfied);
    assert_eq!(
        root.groups
            .iter()
            .map(|group| group.member_count)
            .sum::<usize>(),
        12 * 3,
        "affinity merging keeps every member"
    );
    assert!(
        root.groups.iter().all(|group| {
            group.label.rule == HierarchyLabelRule::DominantDirectory
                && group
                    .label
                    .evidence
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value.starts_with("src/area"))
        }),
        "every affinity group is named by the directory it shares"
    );
    cover_every_child_once(&hierarchy)?;
    Ok(())
}

/// Forty communities that share twelve directories: the exact location budget
/// can only hold one bucket for all of them, so the level escapes that bucket
/// once and shows the directories they actually share instead of a single node
/// that names nothing.
#[test]
fn location_affinity_escapes_a_single_bucket_into_shared_directories() -> TestResult {
    let document = shared_directory_document(12, 40, 3);
    let hierarchy = hierarchy(
        &document,
        HierarchyBudget {
            root_target: 5,
            level_target: 8,
            max_levels: 4,
            ..HierarchyBudget::default()
        },
    )?;
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    assert_eq!(
        finest.groups.len(),
        40,
        "the fixture publishes 40 communities"
    );
    let root = hierarchy.root().ok_or("missing root level")?;
    assert_eq!(root.level, 0);
    assert_eq!(root.merge, LevelMerge::LocationAffinity);
    assert_eq!(
        root.merge_evidence
            .get("escapedSingleBucket")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "the level records that it escaped a single-bucket cut"
    );
    assert_eq!(
        root.groups.len(),
        12,
        "the root shows the twelve shared directories, not one bucket"
    );
    assert!(
        root.groups.iter().all(|group| {
            group.label.rule == HierarchyLabelRule::DominantDirectory
                && group
                    .label
                    .evidence
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value.starts_with("src/area"))
        }),
        "every escaped group is named by the directory it shares"
    );
    assert!(
        root.groups
            .iter()
            .map(|group| group.member_count)
            .sum::<usize>()
            == 40 * 3,
        "the escape keeps every member"
    );
    cover_every_child_once(&hierarchy)?;
    Ok(())
}

/// A coarsening step can collapse a level into one group — the whole repository
/// drawn as a single node. The builder refuses that level instead of publishing
/// it, so the root a reader opens is the achieved partition below it.
#[test]
fn a_level_that_collapses_to_one_group_is_never_published() -> TestResult {
    let document = shared_directory_document(40, 40, 3);
    let hierarchy = hierarchy(
        &document,
        HierarchyBudget {
            root_target: 5,
            level_target: 8,
            max_levels: 4,
            ..HierarchyBudget::default()
        },
    )?;
    for level in &hierarchy.levels {
        assert!(
            level.groups.len() >= 2,
            "level {} holds {} group(s): a one-group level is the repository as one node",
            level.level,
            level.groups.len()
        );
    }
    let root = hierarchy.root().ok_or("missing root level")?;
    assert_eq!(
        hierarchy.levels.len(),
        1,
        "no coarser level reduces this fixture"
    );
    assert_eq!(
        root.groups.len(),
        40,
        "the achieved partition stays the root"
    );
    assert!(
        !hierarchy.budget_satisfied,
        "the root is larger than its target and says so"
    );
    cover_every_child_once(&hierarchy)?;
    Ok(())
}

#[test]
fn the_finest_level_reports_the_published_partition_metrics() -> TestResult {
    let document = clustered_document(6, 3, true);
    let result = partition(&document)?;
    let hierarchy = hierarchy_of(
        &document,
        &result.communities,
        result.quality.resolution,
        HierarchyBudget::default(),
    )?;
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    for (community, quality) in &result.quality.communities {
        let group = finest
            .groups
            .get(*community)
            .ok_or("missing finest group")?;
        assert!(
            (group.quality.cohesion - quality.density).abs() < 1e-12,
            "group {community} cohesion {} does not match published density {}",
            group.quality.cohesion,
            quality.density
        );
        assert!(
            (group.quality.conductance - quality.conductance).abs() < 1e-12,
            "group {community} conductance {} does not match published conductance {}",
            group.quality.conductance,
            quality.conductance
        );
        assert_eq!(group.member_count, quality.member_count);
    }
    Ok(())
}

#[test]
fn self_referential_symbols_publish_a_valid_hierarchy() -> TestResult {
    // A package that calls into itself keeps a self-loop in the projected
    // topology. The group holding it must still publish a bounded cohesion that
    // matches the partition's density, and the artifact must validate.
    let mut document = clustered_document(2, 3, true);
    let loop_index = document.links.len();
    document.links.push(edge(
        loop_index,
        "cluster0_symbol1",
        "cluster0_symbol1",
        EdgeKind::Calls,
    ));
    let result = partition(&document)?;
    let hierarchy = hierarchy_of(
        &document,
        &result.communities,
        result.quality.resolution,
        HierarchyBudget::default(),
    )?;
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    for (community, quality) in &result.quality.communities {
        let group = finest
            .groups
            .get(*community)
            .ok_or("missing finest group")?;
        assert!(
            (0.0..=1.0).contains(&quality.density),
            "community {community} density {} is not a share of member pairs",
            quality.density
        );
        assert!(
            (group.quality.cohesion - quality.density).abs() < 1e-12,
            "group {community} cohesion {} does not match published density {}",
            group.quality.cohesion,
            quality.density
        );
    }
    Ok(())
}

#[test]
fn boundary_kinds_are_counted_and_the_exact_set_is_recorded() -> TestResult {
    let mut document = clustered_document(2, 4, true);
    if let Some(route) = document
        .nodes
        .iter_mut()
        .find(|node| node.id == "cluster0_symbol0")
    {
        route.kind = NodeKind::Route;
    }
    if let Some(table) = document
        .nodes
        .iter_mut()
        .find(|node| node.id == "cluster1_symbol0")
    {
        table.kind = NodeKind::DatabaseTable;
    }
    let hierarchy = hierarchy(&document, HierarchyBudget::default())?;
    assert!(hierarchy.boundary_kinds.iter().any(|kind| kind == "route"));
    assert!(
        hierarchy
            .boundary_kinds
            .iter()
            .any(|kind| kind == "database_table")
    );
    let counted = hierarchy
        .finest()
        .map(|level| {
            level
                .groups
                .iter()
                .flat_map(|group| group.quality.boundary_kinds.iter())
                .map(|(_, count)| *count)
                .sum::<usize>()
        })
        .unwrap_or_default();
    assert_eq!(counted, 2, "both boundary members are accounted for");
    let root = hierarchy.root().ok_or("missing root level")?;
    let root_boundary = root
        .groups
        .iter()
        .flat_map(|group| group.quality.boundary_kinds.values())
        .sum::<usize>();
    assert_eq!(
        root_boundary, 2,
        "coarser levels inherit boundary accounting"
    );
    Ok(())
}

#[test]
fn reconciliation_keeps_ids_and_never_changes_membership() -> TestResult {
    let document = clustered_document(6, 3, true);
    let result = partition(&document)?;
    let reconciliation = |budget: HierarchyBudget| -> Result<_, Box<dyn std::error::Error>> {
        let previous = hierarchy_of(
            &document,
            &result.communities,
            result.quality.resolution,
            budget,
        )?;
        let mut next = hierarchy_of(
            &document,
            &result.communities,
            result.quality.resolution,
            budget,
        )?;
        let before = next.levels.clone();
        let report = reconcile_hierarchy(
            &previous,
            &result.communities,
            &mut next,
            &result.communities,
            &ReconcilePolicy::default(),
        )?;
        Ok((report, before, next))
    };

    // An unchanged rebuild keeps every id and reports nothing else.
    let (report, _, next) = reconciliation(HierarchyBudget::default())?;
    assert_eq!(report.split, 0);
    assert_eq!(report.merged, 0);
    assert_eq!(report.appeared, 0);
    assert_eq!(report.disappeared, 0);
    assert_eq!(report.ambiguous, 0);
    assert!(report.matched > 0);
    assert_eq!(
        report.stable,
        next.levels
            .iter()
            .map(|level| level.groups.len())
            .sum::<usize>()
    );

    // Identity is the only thing reconciliation may touch.
    let (report, before, next) = reconciliation(HierarchyBudget::default())?;
    // Nothing changed, so nothing is reported: a group that survives 1:1 is
    // counted in `stable` and never listed as an event.
    assert!(
        report.events.is_empty(),
        "an unchanged rebuild reports no event"
    );
    assert_eq!(report.omitted_events, 0);
    for (previous_level, next_level) in before.iter().zip(next.levels.iter()) {
        assert_eq!(previous_level.groups.len(), next_level.groups.len());
        for (previous_group, next_group) in previous_level.groups.iter().zip(&next_level.groups) {
            assert_eq!(previous_group.member_count, next_group.member_count);
            assert_eq!(previous_group.child_indices, next_group.child_indices);
            assert_eq!(previous_group.community, next_group.community);
            assert_eq!(previous_group.signature, next_group.signature);
        }
    }
    Ok(())
}

#[test]
fn reconciliation_reports_splits_ambiguity_and_bounded_events() -> TestResult {
    let document = clustered_document(1, 4, false);
    let mut sorted = document
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    sorted.sort();
    let previous_communities = Communities::from([(0usize, sorted.clone())]);
    let mut split = Communities::new();
    split.insert(0, sorted.iter().take(2).cloned().collect::<Vec<_>>());
    split.insert(1, sorted.iter().skip(2).cloned().collect::<Vec<_>>());
    let previous = hierarchy_of(
        &document,
        &previous_communities,
        1.0,
        HierarchyBudget::default(),
    )?;
    let mut next = hierarchy_of(&document, &split, 1.0, HierarchyBudget::default())?;
    let structure = next
        .levels
        .iter()
        .map(|level| {
            level
                .groups
                .iter()
                .map(|group| {
                    (
                        group.member_count,
                        group.child_indices.clone(),
                        group.community,
                        group.signature.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let report = reconcile_hierarchy(
        &previous,
        &previous_communities,
        &mut next,
        &split,
        &ReconcilePolicy::default(),
    )?;
    let after = next
        .levels
        .iter()
        .map(|level| {
            level
                .groups
                .iter()
                .map(|group| {
                    (
                        group.member_count,
                        group.child_indices.clone(),
                        group.community,
                        group.signature.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(structure, after, "membership is never reconciled");
    assert!(
        report.ambiguous + report.split >= 1,
        "one previous group over two successors is reported, not resolved: {report:?}"
    );
    assert_eq!(
        report.policy.max_events,
        ReconcilePolicy::default().max_events
    );

    // A tiny bound truncates the list but keeps the accounting exact.
    let mut bounded_next = hierarchy_of(&document, &split, 1.0, HierarchyBudget::default())?;
    let bounded = reconcile_hierarchy(
        &previous,
        &previous_communities,
        &mut bounded_next,
        &split,
        &ReconcilePolicy {
            max_events: 1,
            ..ReconcilePolicy::default()
        },
    )?;
    let total = bounded.stable
        + bounded.split
        + bounded.merged
        + bounded.appeared
        + bounded.disappeared
        + bounded.ambiguous;
    assert_eq!(bounded.events.len(), 1);
    assert_eq!(bounded.omitted_events, total.saturating_sub(1));
    Ok(())
}

#[test]
fn an_edit_sequence_reports_appeared_and_disappeared_groups() -> TestResult {
    let document = clustered_document(5, 3, true);
    let before = hierarchy(&document, HierarchyBudget::default())?;
    let mut shrunk = document.clone();
    shrunk.nodes.retain(|node| !node.id.starts_with("cluster0"));
    shrunk.links.retain(|edge| {
        !edge.source.starts_with("cluster0") && !edge.target.starts_with("cluster0")
    });
    let mut after = hierarchy(&shrunk, HierarchyBudget::default())?;
    let report = reconcile_hierarchy(
        &before,
        &partition(&document)?.communities,
        &mut after,
        &partition(&shrunk)?.communities,
        &ReconcilePolicy::default(),
    )?;
    assert!(
        report.disappeared >= 1,
        "a removed group is reported: {report:?}"
    );
    let disappeared = report
        .events
        .iter()
        .filter(|event| event.kind == compass_graph::HierarchyEventKind::Disappeared)
        .collect::<Vec<_>>();
    assert!(!disappeared.is_empty());
    assert!(
        disappeared
            .iter()
            .all(|event| !event.previous_ids.is_empty() && event.next_ids.is_empty()),
        "a disappearance names the previous group and no successor"
    );
    Ok(())
}

#[test]
fn limits_fail_with_a_typed_error_instead_of_truncating() -> TestResult {
    let document = clustered_document(4, 3, true);
    let result = partition(&document)?;
    let identity = result.identity.clone();
    let zero_levels = build_community_hierarchy(
        &document,
        &result.communities,
        &HierarchyRequest {
            identity: &identity,
            limits: &CommunityLimits::default(),
            resolution: result.quality.resolution,
            budget: HierarchyBudget {
                max_levels: 0,
                ..HierarchyBudget::default()
            },
        },
    );
    assert!(
        matches!(
            zero_levels,
            Err(CommunityError::InvalidHierarchyBudget { .. })
        ),
        "a zero level budget must fail instead of publishing a truncated tree"
    );
    let tight = CommunityLimits {
        max_nodes: 2,
        ..CommunityLimits::default()
    };
    let exceeded = build_community_hierarchy(
        &document,
        &result.communities,
        &HierarchyRequest {
            identity: &identity,
            limits: &tight,
            resolution: result.quality.resolution,
            budget: HierarchyBudget::default(),
        },
    );
    assert!(
        matches!(exceeded, Err(CommunityError::Topology(_))),
        "a node limit must surface the topology error"
    );
    Ok(())
}

#[test]
fn sparse_community_ids_survive_with_their_published_identity() -> TestResult {
    let document = clustered_document(2, 2, true);
    let result = partition(&document)?;
    let mut communities = Communities::new();
    for (community, members) in &result.communities {
        communities.insert(community + 10, members.clone());
    }
    let identity = result.identity.clone();
    let draft = build_community_hierarchy(
        &document,
        &communities,
        &HierarchyRequest {
            identity: &identity,
            limits: &CommunityLimits::default(),
            resolution: result.quality.resolution,
            budget: HierarchyBudget::default(),
        },
    )?;
    let hierarchy = CommunityHierarchy::new(
        "generation-test".to_owned(),
        format!("sha256:{}", "0".repeat(64)),
        draft,
    )?;
    let finest = hierarchy.finest().ok_or("missing finest level")?;
    let published = finest
        .groups
        .iter()
        .filter_map(|group| group.community)
        .collect::<Vec<_>>();
    assert_eq!(
        published,
        communities.keys().copied().collect::<Vec<_>>(),
        "the finest level keeps the published community ids"
    );
    assert!(hierarchy.budget_satisfied);
    Ok(())
}
