//! Measure hierarchy stability across a deterministic edit sequence.
//!
//! The gate replays one fixture repository through add / move / delete edits,
//! reconciles each generation against the one before it, and reports what a
//! reader would have experienced: did untouched groups keep their ids, do the
//! events match the edits, and does the root grouping stay recognizable
//! (ARI/AMI) as the code moves.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use compass_graph::{
    Communities, CommunityHierarchy, CommunityLimits, CommunityProfile, CommunityRequest,
    HierarchyBudget, HierarchyRequest, ReconcilePolicy, ResolutionPolicy,
    adjusted_mutual_information, adjusted_rand_index, build_communities, build_community_hierarchy,
    reconcile_hierarchy,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::provenance::SourceAnchor;
use serde::Serialize;

const REPORT_SCHEMA: &str = "compass.community-hierarchy-stability/1";
/// Consecutive generations must stay recognizable to a reader.
const ARI_THRESHOLD: f64 = 0.5;
const AMI_THRESHOLD: f64 = 0.5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema: &'static str,
    root_target: usize,
    level_target: usize,
    ari_threshold: f64,
    ami_threshold: f64,
    acceptance: Acceptance,
    generations: Vec<GenerationReport>,
    ambiguity: AmbiguityReport,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Acceptance {
    root_budget_satisfied: bool,
    complete_tree: bool,
    ari_at_least_threshold: bool,
    ami_at_least_threshold: bool,
    stable_ids_for_untouched_groups: bool,
    split_merge_events_match_edits: bool,
    ambiguous_events_reported_not_resolved: bool,
    deterministic_digest: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationReport {
    edit: String,
    nodes: usize,
    edges: usize,
    communities: usize,
    levels: usize,
    root_groups: usize,
    digest: String,
    stable: usize,
    split: usize,
    merged: usize,
    appeared: usize,
    disappeared: usize,
    ambiguous: usize,
    /// Node ids the two generations share.
    shared_members: usize,
    ari: f64,
    ami: f64,
    untouched_groups: usize,
    untouched_groups_kept: usize,
    /// Events that named a group whose member set did not change.
    events_naming_untouched_groups: usize,
    /// Previous groups that no longer exist, measured from member sets.
    vanished_groups: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AmbiguityReport {
    /// The fixture forced a group to split evenly into two successors.
    produced: bool,
    /// Any successor inherited an id the evidence did not name.
    resolved_by_guess: bool,
    events: usize,
}

/// One generation's identity, kept between edits.
struct Generation {
    document: GraphDocument,
    communities: Communities,
    hierarchy: CommunityHierarchy,
}

fn main() -> Result<(), Box<dyn Error>> {
    let first = replay()?;
    let second = replay()?;
    let deterministic_digest = first
        .iter()
        .zip(second.iter())
        .all(|(left, right)| left.report.digest == right.report.digest);
    let ambiguity = measure_ambiguity()?;
    let acceptance = Acceptance {
        root_budget_satisfied: first
            .iter()
            .all(|generation| generation.report.root_groups <= 24 && generation.complete_tree),
        complete_tree: first.iter().all(|generation| generation.complete_tree),
        ari_at_least_threshold: first
            .iter()
            .skip(1)
            .all(|generation| generation.report.ari >= ARI_THRESHOLD),
        ami_at_least_threshold: first
            .iter()
            .skip(1)
            .all(|generation| generation.report.ami >= AMI_THRESHOLD),
        stable_ids_for_untouched_groups: first.iter().skip(1).all(|generation| {
            generation.report.untouched_groups_kept == generation.report.untouched_groups
        }),
        split_merge_events_match_edits: first.iter().skip(1).all(|generation| {
            // No event may name a group whose members did not move, and a
            // deletion is the only edit that may remove a group.
            generation.report.events_naming_untouched_groups == 0
                && generation.report.disappeared == generation.expected_disappeared
                && generation.report.disappeared <= generation.report.vanished_groups
        }),
        ambiguous_events_reported_not_resolved: ambiguity.produced && !ambiguity.resolved_by_guess,
        deterministic_digest,
    };
    let report = Report {
        schema: REPORT_SCHEMA,
        root_target: 24,
        level_target: 300,
        ari_threshold: ARI_THRESHOLD,
        ami_threshold: AMI_THRESHOLD,
        acceptance,
        generations: first
            .into_iter()
            .map(|generation| generation.report)
            .collect(),
        ambiguity,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct MeasuredGeneration {
    report: GenerationReport,
    complete_tree: bool,
    /// Groups the applied edit is allowed to remove.
    expected_disappeared: usize,
}

/// Replay the edit sequence, reconciling each generation against the last.
fn replay() -> Result<Vec<MeasuredGeneration>, Box<dyn Error>> {
    let mut generations = Vec::new();
    let mut previous: Option<Generation> = None;
    for (edit, document, expected_disappeared) in sequence() {
        let generation = build(document)?;
        let mut report = GenerationReport {
            edit: edit.to_owned(),
            nodes: generation.document.nodes.len(),
            edges: generation.document.links.len(),
            communities: generation.hierarchy.finest_community_count,
            levels: generation.hierarchy.levels.len(),
            root_groups: generation
                .hierarchy
                .root()
                .map_or(0, |level| level.groups.len()),
            digest: generation.hierarchy.result_digest.clone(),
            stable: 0,
            split: 0,
            merged: 0,
            appeared: 0,
            disappeared: 0,
            ambiguous: 0,
            shared_members: 0,
            ari: 1.0,
            ami: 1.0,
            untouched_groups: 0,
            untouched_groups_kept: 0,
            events_naming_untouched_groups: 0,
            vanished_groups: 0,
        };
        let mut next = generation;
        if let Some(previous) = previous.as_ref() {
            let previous_sets = level_sets(&previous.hierarchy, &previous.communities);
            let next_sets = level_sets(&next.hierarchy, &next.communities);
            let reconciliation = reconcile_hierarchy(
                &previous.hierarchy,
                &previous.communities,
                &mut next.hierarchy,
                &next.communities,
                &ReconcilePolicy::default(),
            )?;
            report.stable = reconciliation.stable;
            report.split = reconciliation.split;
            report.merged = reconciliation.merged;
            report.appeared = reconciliation.appeared;
            report.disappeared = reconciliation.disappeared;
            report.ambiguous = reconciliation.ambiguous;
            let (untouched, kept, vanished) = compare_members(
                &previous_sets,
                &next_sets,
                &previous.hierarchy,
                &next.hierarchy,
            );
            report.untouched_groups = untouched;
            report.untouched_groups_kept = kept;
            report.vanished_groups = vanished;
            let untouched_ids = untouched_ids(
                &previous_sets,
                &next_sets,
                &previous.hierarchy,
                &next.hierarchy,
            );
            report.events_naming_untouched_groups = reconciliation
                .events
                .iter()
                .filter(|event| {
                    event
                        .previous_ids
                        .iter()
                        .chain(event.next_ids.iter())
                        .any(|id| untouched_ids.contains(id))
                })
                .count();
            let shared = shared_members(&previous.communities, &next.communities);
            report.shared_members = shared.len();
            let left = root_partition(&previous.hierarchy, &previous.communities, &shared);
            let right = root_partition(&next.hierarchy, &next.communities, &shared);
            report.ari = adjusted_rand_index(&left, &right);
            report.ami = adjusted_mutual_information(&left, &right)?;
        }
        let complete_tree = complete_tree(&next.hierarchy);
        generations.push(MeasuredGeneration {
            report,
            complete_tree,
            expected_disappeared,
        });
        previous = Some(next);
    }
    Ok(generations)
}

/// Force the one case a source edit cannot guarantee: a group whose members
/// split evenly, so no successor is the better heir.
fn measure_ambiguity() -> Result<AmbiguityReport, Box<dyn Error>> {
    let document = cluster_document(4, 4, true);
    let generation = build(document.clone())?;
    let Some(first) = generation.communities.get(&0).cloned() else {
        return Ok(AmbiguityReport {
            produced: false,
            resolved_by_guess: false,
            events: 0,
        });
    };
    if first.len() < 4 {
        return Ok(AmbiguityReport {
            produced: false,
            resolved_by_guess: false,
            events: 0,
        });
    }
    // One published community, split evenly in two: neither successor is the
    // better heir, so the evidence names neither.
    let mut split = generation.communities.clone();
    split.insert(0, first.iter().take(first.len() / 2).cloned().collect());
    let new_id = split.keys().max().copied().unwrap_or_default() + 1;
    split.insert(
        new_id,
        first
            .iter()
            .skip(first.len() / 2)
            .cloned()
            .collect::<Vec<_>>(),
    );
    let mut carved = build_from(document, split.clone())?;
    let previous_sets = level_sets(&generation.hierarchy, &generation.communities);
    let next_sets = level_sets(&carved.hierarchy, &carved.communities);
    let _ = (previous_sets, next_sets);
    // The split group's own id is the one at stake: if either successor wears
    // it, the reconciliation guessed.
    let split_id = generation
        .hierarchy
        .levels
        .last()
        .and_then(|level| level.groups.iter().find(|group| group.community == Some(0)))
        .map(|group| group.id.clone());
    let reconciliation = reconcile_hierarchy(
        &generation.hierarchy,
        &generation.communities,
        &mut carved.hierarchy,
        &carved.communities,
        &ReconcilePolicy::default(),
    )?;
    let successors = carved
        .hierarchy
        .levels
        .last()
        .map(|level| {
            level
                .groups
                .iter()
                .filter(|group| group.community == Some(0) || group.community == Some(new_id))
                .map(|group| group.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let inherited = split_id.as_ref().is_some_and(|id| successors.contains(id));
    Ok(AmbiguityReport {
        produced: reconciliation.ambiguous >= 1,
        resolved_by_guess: inherited,
        events: reconciliation.events.len(),
    })
}

fn build(document: GraphDocument) -> Result<Generation, Box<dyn Error>> {
    let changed = BTreeSet::new();
    let result = build_communities(
        &document,
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
    build_from(document, result.communities)
}

fn build_from(
    document: GraphDocument,
    communities: Communities,
) -> Result<Generation, Box<dyn Error>> {
    let changed = BTreeSet::new();
    let result = build_communities(
        &document,
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
        &document,
        &communities,
        &HierarchyRequest {
            identity: &result.identity,
            limits: &CommunityLimits::default(),
            resolution: result.quality.resolution,
            budget: HierarchyBudget::default(),
        },
    )?;
    let hierarchy = CommunityHierarchy::new(
        document.graph.build.generation_id.clone(),
        format!("sha256:{}", "0".repeat(64)),
        draft,
    )?;
    Ok(Generation {
        document,
        communities,
        hierarchy,
    })
}

/// The fixture's deterministic edit sequence.
fn sequence() -> Vec<(&'static str, GraphDocument, usize)> {
    let base = cluster_document(6, 3, true);
    let mut added = base.clone();
    for index in 0..2 {
        let id = format!("cluster0_added{index}");
        added.nodes.push(node(&id, Some("src/group0/added.rs")));
        added.links.push(edge(
            added.links.len(),
            "cluster0_symbol0",
            &id,
            EdgeKind::Calls,
        ));
    }
    let mut moved = added.clone();
    for node in &mut moved.nodes {
        if let Some(source) = node.source.as_mut()
            && source.file.starts_with("src/group1/")
        {
            source.file = source.file.replace("src/group1/", "src/group0/");
        }
    }
    let mut deleted = moved.clone();
    deleted
        .nodes
        .retain(|node| !node.id.starts_with("cluster5"));
    deleted.links.retain(|edge| {
        !edge.source.starts_with("cluster5") && !edge.target.starts_with("cluster5")
    });
    vec![
        ("initial", base, 0),
        ("add-symbols", added, 0),
        ("move-file", moved, 0),
        ("delete-community", deleted, 1),
    ]
}

fn cluster_document(clusters: usize, per_cluster: usize, bridges: bool) -> GraphDocument {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for cluster in 0..clusters {
        for index in 0..per_cluster {
            let id = format!("cluster{cluster}_symbol{index}");
            nodes.push(node(
                &id,
                Some(&format!("src/group{cluster}/file{index}.rs")),
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
    document(nodes, links)
}

/// Member sets per level, keyed by level number.
fn level_sets(
    hierarchy: &CommunityHierarchy,
    communities: &Communities,
) -> BTreeMap<usize, Vec<BTreeSet<String>>> {
    let mut levels = BTreeMap::new();
    let mut collected = Vec::<Vec<BTreeSet<String>>>::new();
    let Some(finest) = hierarchy.levels.last() else {
        return levels;
    };
    collected.push(
        finest
            .groups
            .iter()
            .map(|group| {
                group
                    .community
                    .and_then(|community| communities.get(&community))
                    .map(|members| members.iter().cloned().collect::<BTreeSet<_>>())
                    .unwrap_or_default()
            })
            .collect(),
    );
    let mut position = hierarchy.levels.len().saturating_sub(1);
    while position > 0 {
        let Some(finer) = collected.last() else {
            break;
        };
        let Some(level) = hierarchy.levels.get(position - 1) else {
            break;
        };
        collected.push(
            level
                .groups
                .iter()
                .map(|group| {
                    let mut members = BTreeSet::new();
                    for child in &group.child_indices {
                        if let Some(child_members) = finer.get(*child) {
                            members.extend(child_members.iter().cloned());
                        }
                    }
                    members
                })
                .collect(),
        );
        position -= 1;
    }
    collected.reverse();
    for (order, sets) in collected.into_iter().enumerate() {
        if let Some(level) = hierarchy.levels.get(order) {
            levels.insert(level.level, sets);
        }
    }
    levels
}

/// Untouched groups (same member set) and how many kept their id, plus the
/// groups whose members vanished or are new.
fn compare_members(
    previous_sets: &BTreeMap<usize, Vec<BTreeSet<String>>>,
    next_sets: &BTreeMap<usize, Vec<BTreeSet<String>>>,
    previous: &CommunityHierarchy,
    next: &CommunityHierarchy,
) -> (usize, usize, usize) {
    let mut untouched = 0usize;
    let mut kept = 0usize;
    let mut vanished = 0usize;
    for (level, previous_level) in previous.levels.iter().enumerate() {
        let (Some(previous_sets), Some(next_sets)) = (
            previous_sets.get(&previous_level.level),
            next_sets.get(&previous_level.level),
        ) else {
            continue;
        };
        let Some(next_level) = next.levels.get(level) else {
            continue;
        };
        for (index, group) in previous_level.groups.iter().enumerate() {
            let Some(members) = previous_sets.get(index) else {
                continue;
            };
            match next_sets.iter().position(|candidate| candidate == members) {
                Some(match_index) => {
                    untouched += 1;
                    if next_level
                        .groups
                        .get(match_index)
                        .is_some_and(|candidate| candidate.id == group.id)
                    {
                        kept += 1;
                    }
                }
                None => vanished += 1,
            }
        }
    }
    (untouched, kept, vanished)
}

/// Ids of groups whose member set is identical in both generations.
fn untouched_ids(
    previous_sets: &BTreeMap<usize, Vec<BTreeSet<String>>>,
    next_sets: &BTreeMap<usize, Vec<BTreeSet<String>>>,
    previous: &CommunityHierarchy,
    next: &CommunityHierarchy,
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for (level, previous_level) in previous.levels.iter().enumerate() {
        let (Some(previous_sets), Some(next_sets)) = (
            previous_sets.get(&previous_level.level),
            next_sets.get(&previous_level.level),
        ) else {
            continue;
        };
        let Some(next_level) = next.levels.get(level) else {
            continue;
        };
        for (index, group) in previous_level.groups.iter().enumerate() {
            let Some(members) = previous_sets.get(index) else {
                continue;
            };
            if let Some(match_index) = next_sets.iter().position(|candidate| candidate == members)
                && let Some(candidate) = next_level.groups.get(match_index)
            {
                ids.insert(candidate.id.clone());
                ids.insert(group.id.clone());
            }
        }
    }
    ids
}

fn shared_members(previous: &Communities, next: &Communities) -> BTreeSet<String> {
    let previous_members = previous
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    next.values()
        .flatten()
        .filter(|member| previous_members.contains(*member))
        .cloned()
        .collect()
}

/// The root grouping as a partition over the members both generations share.
fn root_partition(
    hierarchy: &CommunityHierarchy,
    communities: &Communities,
    shared: &BTreeSet<String>,
) -> Communities {
    let sets = level_sets(hierarchy, communities);
    let Some(root) = hierarchy.root() else {
        return Communities::new();
    };
    let Some(root_sets) = sets.get(&root.level) else {
        return Communities::new();
    };
    let mut partition = Communities::new();
    for (index, members) in root_sets.iter().enumerate() {
        let members = members
            .iter()
            .filter(|member| shared.contains(*member))
            .cloned()
            .collect::<Vec<_>>();
        if !members.is_empty() {
            partition.insert(index, members);
        }
    }
    partition
}

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

fn node(id: &str, file: Option<&str>) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: format!("app::{id}"),
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

fn document(nodes: Vec<NodeRecord>, links: Vec<EdgeRecord>) -> GraphDocument {
    let mut document = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "community-hierarchy-stability".to_owned(),
        schema_fingerprint: "fixture-v1".to_owned(),
        source_tree_digest: "fixture".to_owned(),
        configuration_digest: "hierarchy-stability-v1".to_owned(),
        generation_id: "fixture:stability".to_owned(),
        source_commit: None,
    });
    document.nodes = nodes;
    document.links = links;
    document
}
