//! Qualify the budgeted community hierarchy on the shapes real repositories
//! publish: clustered directories, communities that share no relationship at
//! all, and groups that cite no location either.
//!
//! The report is the evidence the release gate consumes: every acceptance entry
//! must be true, and two runs must produce identical bytes.

use std::collections::BTreeSet;
use std::error::Error;

use compass_graph::{
    CommunityHierarchy, CommunityLimits, CommunityProfile, CommunityRequest, HierarchyBudget,
    HierarchyRequest, LevelMerge, ResolutionPolicy, build_communities, build_community_hierarchy,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::provenance::SourceAnchor;
use serde::Serialize;

const REPORT_SCHEMA: &str = "compass.community-hierarchy-qualification/1";

/// Root groups a reporter reads before the overview stops being an overview.
const GENERIC_ROOT_SHARE_LIMIT: f64 = 0.25;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema: &'static str,
    fixture_count: usize,
    acceptance: Acceptance,
    fixtures: Vec<FixtureReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Acceptance {
    root_budget_satisfied: bool,
    complete_tree: bool,
    labels_have_provenance: bool,
    generic_root_labels_bounded: bool,
    deterministic_digest: bool,
    bounded_levels: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureReport {
    name: String,
    nodes: usize,
    edges: usize,
    communities: usize,
    levels: usize,
    root_groups: usize,
    budget_satisfied: bool,
    budget_expected: bool,
    budget_matches_expectation: bool,
    merge_rules: Vec<String>,
    complete_tree: bool,
    labels_have_provenance: bool,
    generic_root_labels: usize,
    generic_root_share: f64,
    deterministic_digest: bool,
    bounded_levels: bool,
    result_digest: String,
}

struct Fixture {
    name: &'static str,
    document: GraphDocument,
    budget: HierarchyBudget,
    expect_budget_satisfied: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut reports = Vec::new();
    for fixture in fixtures() {
        reports.push(qualify(fixture)?);
    }
    let report = Report {
        schema: REPORT_SCHEMA,
        fixture_count: reports.len(),
        acceptance: Acceptance {
            root_budget_satisfied: reports.iter().any(|fixture| fixture.budget_expected)
                && reports
                    .iter()
                    .all(|fixture| fixture.budget_matches_expectation),
            complete_tree: reports.iter().all(|fixture| fixture.complete_tree),
            labels_have_provenance: reports.iter().all(|fixture| fixture.labels_have_provenance),
            generic_root_labels_bounded: reports
                .iter()
                .all(|fixture| fixture.generic_root_share <= GENERIC_ROOT_SHARE_LIMIT),
            deterministic_digest: reports.iter().all(|fixture| fixture.deterministic_digest),
            bounded_levels: reports.iter().all(|fixture| fixture.bounded_levels),
        },
        fixtures: reports,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn qualify(fixture: Fixture) -> Result<FixtureReport, Box<dyn Error>> {
    let changed = BTreeSet::new();
    let result = build_communities(
        &fixture.document,
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
    let artifact = |budget: HierarchyBudget| -> Result<CommunityHierarchy, Box<dyn Error>> {
        let draft = build_community_hierarchy(
            &fixture.document,
            &result.communities,
            &HierarchyRequest {
                identity: &result.identity,
                limits: &CommunityLimits::default(),
                resolution: result.quality.resolution,
                budget,
            },
        )?;
        Ok(CommunityHierarchy::new(
            fixture.document.graph.build.generation_id.clone(),
            format!("sha256:{}", "0".repeat(64)),
            draft,
        )?)
    };
    let first = artifact(fixture.budget)?;
    let second = artifact(fixture.budget)?;
    let root = first.root().ok_or("missing root level")?;
    let generic_root_labels = root
        .groups
        .iter()
        .filter(|group| group.label.is_generic())
        .count();
    let root_groups = root.groups.len();
    Ok(FixtureReport {
        name: fixture.name.to_owned(),
        nodes: fixture.document.nodes.len(),
        edges: fixture.document.links.len(),
        communities: first.finest_community_count,
        levels: first.levels.len(),
        root_groups,
        budget_satisfied: first.budget_satisfied,
        budget_expected: fixture.expect_budget_satisfied,
        budget_matches_expectation: first.budget_satisfied == fixture.expect_budget_satisfied,
        merge_rules: first
            .levels
            .iter()
            .map(|level| match level.merge {
                LevelMerge::Relationship => "relationship".to_owned(),
                LevelMerge::LocationAffinity => "locationAffinity".to_owned(),
            })
            .collect(),
        complete_tree: complete_tree(&first),
        labels_have_provenance: first.levels.iter().all(|level| {
            level.groups.iter().all(|group| {
                !group.label.text.trim().is_empty()
                    && !group.label.evidence.is_empty()
                    && group.label.generic
                        == matches!(
                            group.label.rule,
                            compass_graph::HierarchyLabelRule::CommunityId
                        )
            })
        }),
        generic_root_labels,
        generic_root_share: if root_groups == 0 {
            1.0
        } else {
            generic_root_labels as f64 / root_groups as f64
        },
        deterministic_digest: first.result_digest == second.result_digest
            && serde_json::to_vec(&first)? == serde_json::to_vec(&second)?,
        bounded_levels: first.levels.len() <= fixture.budget.max_levels
            && first
                .levels
                .windows(2)
                .all(|pair| match (pair.first(), pair.get(1)) {
                    (Some(coarser), Some(finer)) => coarser.groups.len() < finer.groups.len(),
                    _ => true,
                }),
        result_digest: first.result_digest.clone(),
    })
}

/// Prove every level is exactly partitioned by the level above it, without
/// trusting the builder's own check.
fn complete_tree(hierarchy: &CommunityHierarchy) -> bool {
    for (position, level) in hierarchy.levels.iter().enumerate() {
        let Some(finer) = hierarchy.levels.get(position + 1) else {
            continue;
        };
        let mut seen = vec![0usize; finer.groups.len()];
        for group in &level.groups {
            let mut members = 0usize;
            for child in &group.child_indices {
                let Some(slot) = seen.get_mut(*child) else {
                    return false;
                };
                *slot += 1;
                let Some(child_group) = finer.groups.get(*child) else {
                    return false;
                };
                members += child_group.member_count;
            }
            if members != group.member_count {
                return false;
            }
        }
        if seen.iter().any(|count| *count != 1) {
            return false;
        }
    }
    true
}

fn fixtures() -> Vec<Fixture> {
    vec![
        clustered_fixture("directory-clusters", 12, 3, true, 4, 8, true),
        clustered_fixture("fragmented-communities", 24, 2, false, 4, 8, true),
        locationless_fixture("locationless-groups", 12),
    ]
}

/// Clusters of symbols in their own directory. Bridges are the only cross-cluster
/// evidence, so a fixture without them can only be grouped by location.
fn clustered_fixture(
    name: &'static str,
    clusters: usize,
    per_cluster: usize,
    bridges: bool,
    root_target: usize,
    level_target: usize,
    expect_budget_satisfied: bool,
) -> Fixture {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        for index in 0..per_cluster {
            let id = format!("cluster{cluster}_symbol{index}");
            nodes.push(node(
                &id,
                Some(&format!("src/group{cluster}/file{index}.rs")),
                Some(&format!("app::group{cluster}::symbol{index}")),
            ));
        }
        for index in 1..per_cluster {
            links.push(edge(
                links.len(),
                &format!("cluster{cluster}_symbol{}", index - 1),
                &format!("cluster{cluster}_symbol{index}"),
                EdgeKind::Calls,
            ));
        }
        if bridges && cluster > 0 {
            links.push(edge(
                links.len(),
                &format!("cluster{}_symbol0", cluster - 1),
                &format!("cluster{cluster}_symbol0"),
                EdgeKind::References,
            ));
        }
    }
    Fixture {
        name,
        document: document(name, nodes, links),
        budget: HierarchyBudget {
            root_target,
            level_target,
            ..HierarchyBudget::default()
        },
        expect_budget_satisfied,
    }
}

/// Groups that share neither a relationship nor a source location: no rule may
/// merge them, and the artifact has to say so.
fn locationless_fixture(name: &'static str, clusters: usize) -> Fixture {
    let mut nodes = Vec::new();
    for cluster in 0..clusters {
        for index in 0..2 {
            nodes.push(node(
                &format!("cluster{cluster}_symbol{index}"),
                None,
                Some(&format!("app::group{cluster}::symbol{index}")),
            ));
        }
    }
    Fixture {
        name,
        document: document(name, nodes, Vec::new()),
        budget: HierarchyBudget {
            root_target: 4,
            level_target: 8,
            ..HierarchyBudget::default()
        },
        expect_budget_satisfied: false,
    }
}

fn node(id: &str, file: Option<&str>, qualified: Option<&str>) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: qualified.unwrap_or(id).to_owned(),
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
        id: format!("edge-{index:04}"),
        key: format!("edge-{index:04}"),
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

fn document(name: &str, nodes: Vec<NodeRecord>, links: Vec<EdgeRecord>) -> GraphDocument {
    let mut document = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "community-hierarchy-qualification".to_owned(),
        schema_fingerprint: "fixture-v1".to_owned(),
        source_tree_digest: name.to_owned(),
        configuration_digest: "hierarchy-v1".to_owned(),
        generation_id: format!("fixture:{name}"),
        source_commit: None,
    });
    document.nodes = nodes;
    document.links = links;
    document
}
