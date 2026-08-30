mod model;
mod names;
mod relation;
mod scope;

use std::collections::{BTreeMap, BTreeSet};

use compass_graph::{Communities, score_communities};
use compass_model::{GraphDocument, NodeRecord};
use thiserror::Error;

pub use model::*;

use names::{
    community_name, disambiguate_names, membership_signature, owner_display_name, owner_name,
    stable_fragment,
};
use relation::classify_relation;
use scope::{classify_source, generated_paths, normalized_path, path_matches_prefix};

pub struct ArchitectureProjectionInput<'a> {
    pub document: &'a GraphDocument,
    pub communities: &'a Communities,
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub overlay: Option<&'a ArchitectureOverlay>,
    pub project_name: &'a str,
    pub built_at_commit: Option<&'a str>,
    pub generated_at: Option<&'a str>,
}

#[derive(Debug, Error)]
pub enum ArchitectureProjectionError {
    #[error("architecture projection requires at least one scope")]
    MissingScope,
    #[error("architecture projection limit {name} exceeded: required {required}, limit {limit}")]
    LimitExceeded {
        name: &'static str,
        required: usize,
        limit: usize,
    },
    #[error("invalid architecture overlay: {0}")]
    InvalidOverlay(String),
    #[error("node {node} belongs to communities {first} and {second}")]
    DuplicateCommunity {
        node: String,
        first: usize,
        second: usize,
    },
    #[error("architecture projection invariant failed: {0}")]
    Invariant(String),
}

#[derive(Clone, Debug)]
struct OwnerAssignment {
    key: String,
    display_name: String,
    provenance: ArchitectureNameProvenance,
    evidence: Vec<String>,
    pinned: bool,
}

#[derive(Clone, Debug)]
struct GroupBuild {
    id: String,
    parent_id: Option<String>,
    kind: ArchitectureGroupKind,
    name: ArchitectureGroupName,
    owner_key: String,
    community_ids: Vec<usize>,
    node_ids: Vec<String>,
    relationship_count: usize,
    neighbors: BTreeSet<String>,
    cohesion: f64,
    source_scopes: ArchitectureSourceCounts,
    pinned: bool,
    rank: usize,
}

#[derive(Clone, Debug, Default)]
struct RouteBuild {
    relationship_count: usize,
    relation_classes: ArchitectureClassCounts,
    evidence: ArchitectureEvidenceCounts,
}

#[derive(Clone, Debug)]
struct ScopeBuild {
    groups: Vec<GroupBuild>,
    memberships: Vec<MembershipBuild>,
    node_to_leaf: BTreeMap<String, String>,
    node_to_overview: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct MembershipBuild {
    node_id: String,
    group_id: String,
}

pub fn project_architecture(
    input: ArchitectureProjectionInput<'_>,
    options: &ArchitectureProjectionOptions,
) -> Result<ArchitectureViewModel, ArchitectureProjectionError> {
    validate_options(options)?;
    validate_overlay(input.overlay)?;
    check_limit(
        "max_nodes",
        input.document.nodes.len(),
        options.limits.max_nodes,
    )?;
    check_limit(
        "max_relationships",
        input.document.links.len(),
        options.limits.max_relationships,
    )?;
    let node_communities = node_communities(input.document, input.communities)?;
    let generated = generated_paths(input.document);
    let source_nodes = input
        .document
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = input
        .document
        .nodes
        .iter()
        .map(|node| {
            let source_file = node.source_file().map(str::to_owned).or_else(|| {
                let value = node.string("source_file");
                (!value.is_empty()).then_some(value)
            });
            let scope = classify_source(source_file.as_deref(), &generated, input.overlay);
            ArchitectureNode {
                id: node.id.clone(),
                label: node.display_label(),
                kind: node.kind_name().to_owned(),
                source_file,
                source_scope: scope.scope,
                scope_reason: scope.reason.to_owned(),
                community: node_communities.get(&node.id).copied(),
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let node_index = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    if node_index.len() != nodes.len() {
        return Err(ArchitectureProjectionError::Invariant(
            "node IDs must be unique".to_owned(),
        ));
    }
    let relationships = architecture_relationships(input.document);
    if let Some(relationship) = relationships.iter().find(|relationship| {
        !node_index.contains_key(relationship.source.as_str())
            || !node_index.contains_key(relationship.target.as_str())
    }) {
        return Err(ArchitectureProjectionError::Invariant(format!(
            "relationship {} has an endpoint outside the projected node set",
            relationship.id
        )));
    }
    let community_scores = score_communities(input.document, input.communities);
    let mut projections = Vec::new();
    for scope in &options.scopes {
        projections.push(build_scope_projection(
            *scope,
            options.default_lens,
            &nodes,
            &node_index,
            &relationships,
            &source_nodes,
            input.community_labels,
            input.overlay,
            &community_scores,
            options.limits,
        )?);
    }
    projections.sort_by_key(|projection| projection.scope);
    let confidence_count = |expected: &str| {
        relationships
            .iter()
            .filter(|relationship| relationship.confidence == expected)
            .count()
    };
    Ok(ArchitectureViewModel {
        schema: ARCHITECTURE_VIEWER_SCHEMA,
        title: format!("{} — Architecture", input.project_name),
        statistics: ArchitectureStatistics {
            nodes: nodes.len(),
            relationships: relationships.len(),
            communities: input.communities.len(),
            extracted: confidence_count("extracted"),
            inferred: confidence_count("inferred"),
            ambiguous: confidence_count("ambiguous"),
        },
        nodes,
        relationships,
        projections,
        provenance: ArchitectureProvenance {
            project_name: input.project_name.to_owned(),
            built_at_commit: input.built_at_commit.map(str::to_owned),
            generated_at: input.generated_at.map(str::to_owned),
        },
        limits: options.limits,
    })
}

fn validate_options(
    options: &ArchitectureProjectionOptions,
) -> Result<(), ArchitectureProjectionError> {
    if options.scopes.is_empty() {
        return Err(ArchitectureProjectionError::MissingScope);
    }
    for (name, value) in [
        ("max_nodes", options.limits.max_nodes),
        ("max_relationships", options.limits.max_relationships),
        ("max_groups", options.limits.max_groups),
        ("max_routes", options.limits.max_routes),
        ("max_overview_groups", options.limits.max_overview_groups),
        ("max_overview_routes", options.limits.max_overview_routes),
        ("max_name_candidates", options.limits.max_name_candidates),
        ("max_name_evidence", options.limits.max_name_evidence),
        ("max_diagnostics", options.limits.max_diagnostics),
        (
            "max_omission_witnesses",
            options.limits.max_omission_witnesses,
        ),
    ] {
        if value == 0 {
            return Err(ArchitectureProjectionError::LimitExceeded {
                name,
                required: 1,
                limit: 0,
            });
        }
    }
    Ok(())
}

fn validate_overlay(
    overlay: Option<&ArchitectureOverlay>,
) -> Result<(), ArchitectureProjectionError> {
    let Some(overlay) = overlay else {
        return Ok(());
    };
    if overlay.schema != ARCHITECTURE_OVERLAY_SCHEMA {
        return Err(ArchitectureProjectionError::InvalidOverlay(format!(
            "unsupported schema {}",
            overlay.schema
        )));
    }
    let mut source_prefixes = Vec::new();
    for rule in &overlay.source_rules {
        validate_overlay_prefix(&rule.path_prefix)?;
        let prefix = normalized_path(&rule.path_prefix);
        if prefix.is_empty() {
            return Err(ArchitectureProjectionError::InvalidOverlay(
                "source path prefixes must not be empty".to_owned(),
            ));
        }
        source_prefixes.push(prefix);
    }
    reject_overlapping_prefixes(&source_prefixes, "source rules")?;

    let mut ids = BTreeSet::new();
    let mut communities = BTreeSet::new();
    let mut group_prefixes = Vec::new();
    for group in &overlay.groups {
        let id = group.id.trim();
        if id.is_empty() || group.name.trim().is_empty() {
            return Err(ArchitectureProjectionError::InvalidOverlay(
                "group IDs and names must not be empty".to_owned(),
            ));
        }
        if !ids.insert(id.to_owned()) {
            return Err(ArchitectureProjectionError::InvalidOverlay(format!(
                "duplicate group ID {id}"
            )));
        }
        if group.path_prefixes.is_empty() && group.communities.is_empty() {
            return Err(ArchitectureProjectionError::InvalidOverlay(format!(
                "group {id} has no selectors"
            )));
        }
        for community in &group.communities {
            if !communities.insert(*community) {
                return Err(ArchitectureProjectionError::InvalidOverlay(format!(
                    "community {community} is selected by more than one group"
                )));
            }
        }
        for prefix in &group.path_prefixes {
            validate_overlay_prefix(prefix)?;
            let prefix = normalized_path(prefix);
            if prefix.is_empty() {
                return Err(ArchitectureProjectionError::InvalidOverlay(format!(
                    "group {id} has an empty path prefix"
                )));
            }
            group_prefixes.push(prefix);
        }
    }
    reject_overlapping_prefixes(&group_prefixes, "group selectors")
}

fn validate_overlay_prefix(prefix: &str) -> Result<(), ArchitectureProjectionError> {
    let portable = prefix.trim().replace('\\', "/");
    if portable.is_empty()
        || portable.starts_with('/')
        || portable.contains('\0')
        || portable
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
        || portable
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Err(ArchitectureProjectionError::InvalidOverlay(format!(
            "path prefix must be a contained relative path: {prefix}"
        )));
    }
    Ok(())
}

fn reject_overlapping_prefixes(
    prefixes: &[String],
    label: &str,
) -> Result<(), ArchitectureProjectionError> {
    for (index, left) in prefixes.iter().enumerate() {
        for right in prefixes.iter().skip(index + 1) {
            if path_matches_prefix(left, right) || path_matches_prefix(right, left) {
                return Err(ArchitectureProjectionError::InvalidOverlay(format!(
                    "overlapping {label}: {left} and {right}"
                )));
            }
        }
    }
    Ok(())
}

fn node_communities(
    document: &GraphDocument,
    communities: &Communities,
) -> Result<BTreeMap<String, usize>, ArchitectureProjectionError> {
    let mut result = BTreeMap::new();
    for (community, members) in communities {
        for member in members {
            if let Some(previous) = result.insert(member.clone(), *community)
                && previous != *community
            {
                return Err(ArchitectureProjectionError::DuplicateCommunity {
                    node: member.clone(),
                    first: previous,
                    second: *community,
                });
            }
        }
    }
    for node in &document.nodes {
        if !result.contains_key(&node.id)
            && let Some(community) = node
                .unsigned("community")
                .and_then(|value| usize::try_from(value).ok())
        {
            result.insert(node.id.clone(), community);
        }
    }
    Ok(result)
}

fn architecture_relationships(document: &GraphDocument) -> Vec<ArchitectureRelationship> {
    let mut keyed = document
        .links
        .iter()
        .map(|edge| {
            let relation = edge.string("relation").to_ascii_lowercase();
            let confidence = confidence(&edge.string("confidence"));
            let canonical_attributes = edge
                .attributes
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            let attributes = serde_json::to_string(&canonical_attributes).unwrap_or_default();
            let key = format!(
                "{}\0{}\0{}\0{}\0{}",
                edge.source, edge.target, relation, confidence, attributes
            );
            (
                key,
                edge.source.clone(),
                edge.target.clone(),
                relation,
                confidence,
            )
        })
        .collect::<Vec<_>>();
    keyed.sort();
    let mut duplicate_ordinals = BTreeMap::<String, usize>::new();
    keyed
        .into_iter()
        .map(|(key, source, target, relation, confidence)| {
            let ordinal = duplicate_ordinals.entry(key.clone()).or_default();
            let id = format!("relationship:{}:{ordinal}", stable_fragment(&key));
            *ordinal += 1;
            ArchitectureRelationship {
                id,
                source,
                target,
                relation_class: classify_relation(&relation),
                relation,
                confidence,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_scope_projection(
    scope: ArchitectureScope,
    default_lens: ArchitectureLens,
    nodes: &[ArchitectureNode],
    node_index: &BTreeMap<&str, usize>,
    relationships: &[ArchitectureRelationship],
    source_nodes: &BTreeMap<String, &NodeRecord>,
    community_labels: Option<&BTreeMap<usize, String>>,
    overlay: Option<&ArchitectureOverlay>,
    community_scores: &BTreeMap<usize, f64>,
    limits: ArchitectureProjectionLimits,
) -> Result<ArchitectureScopeProjection, ArchitectureProjectionError> {
    let mut owner_communities = BTreeMap::<String, BTreeMap<Option<usize>, Vec<String>>>::new();
    let mut owner_assignments = BTreeMap::<String, OwnerAssignment>::new();
    let mut scoped_node_ids = BTreeSet::new();
    let mut source_counts = ArchitectureSourceCounts::default();
    for node in nodes
        .iter()
        .filter(|node| scope_includes(scope, node.source_scope))
    {
        scoped_node_ids.insert(node.id.clone());
        source_counts.increment(node.source_scope);
        let assignment = owner_assignment(node, overlay);
        owner_communities
            .entry(assignment.key.clone())
            .or_default()
            .entry(node.community)
            .or_default()
            .push(node.id.clone());
        owner_assignments
            .entry(assignment.key.clone())
            .or_insert(assignment);
    }
    let mut groups = Vec::<GroupBuild>::new();
    let mut memberships = Vec::new();
    let mut node_to_leaf = BTreeMap::new();
    let mut node_to_overview = BTreeMap::new();
    for (owner_key, community_buckets) in owner_communities {
        let Some(assignment) = owner_assignments.get(&owner_key) else {
            return Err(ArchitectureProjectionError::Invariant(format!(
                "owner {owner_key} has no assignment"
            )));
        };
        let owner_id = format!("owner:{}", stable_fragment(&owner_key));
        let mut owner_nodes = community_buckets
            .values()
            .flat_map(|ids| ids.iter().cloned())
            .collect::<Vec<_>>();
        owner_nodes.sort();
        let owner_source_counts = source_counts_for_ids(&owner_nodes, nodes, node_index);
        let mut community_ids = community_buckets
            .keys()
            .filter_map(|value| *value)
            .collect::<Vec<_>>();
        community_ids.sort_unstable();
        let owner_cohesion = average_community_score(&community_ids, community_scores);
        if community_buckets.len() <= 1 {
            let community = community_buckets.keys().next().copied().flatten();
            let name = owner_name(
                assignment.display_name.clone(),
                &owner_nodes,
                assignment.provenance,
                assignment.evidence.clone(),
            );
            for node_id in &owner_nodes {
                memberships.push(MembershipBuild {
                    node_id: node_id.clone(),
                    group_id: owner_id.clone(),
                });
                node_to_leaf.insert(node_id.clone(), owner_id.clone());
                node_to_overview.insert(node_id.clone(), owner_id.clone());
            }
            groups.push(GroupBuild {
                id: owner_id,
                parent_id: None,
                kind: ArchitectureGroupKind::Subsystem,
                name,
                owner_key,
                community_ids: community.into_iter().collect(),
                node_ids: owner_nodes,
                relationship_count: 0,
                neighbors: BTreeSet::new(),
                cohesion: owner_cohesion,
                source_scopes: owner_source_counts,
                pinned: assignment.pinned,
                rank: 0,
            });
            continue;
        }
        groups.push(GroupBuild {
            id: owner_id.clone(),
            parent_id: None,
            kind: ArchitectureGroupKind::Owner,
            name: owner_name(
                assignment.display_name.clone(),
                &owner_nodes,
                assignment.provenance,
                assignment.evidence.clone(),
            ),
            owner_key: owner_key.clone(),
            community_ids: community_ids.clone(),
            node_ids: owner_nodes,
            relationship_count: 0,
            neighbors: BTreeSet::new(),
            cohesion: owner_cohesion,
            source_scopes: owner_source_counts,
            pinned: assignment.pinned,
            rank: 0,
        });
        for (community, mut member_ids) in community_buckets {
            member_ids.sort();
            let community_fragment = stable_fragment(&membership_signature(&member_ids));
            let group_id = format!("group:{}:{community_fragment}", stable_fragment(&owner_key));
            let name = community_name(
                community,
                &assignment.display_name,
                &member_ids,
                source_nodes,
                community_labels,
                limits.max_name_evidence,
            );
            let community_ids = community.into_iter().collect::<Vec<_>>();
            let group_cohesion = average_community_score(&community_ids, community_scores);
            let group_source_counts = source_counts_for_ids(&member_ids, nodes, node_index);
            for node_id in &member_ids {
                memberships.push(MembershipBuild {
                    node_id: node_id.clone(),
                    group_id: group_id.clone(),
                });
                node_to_leaf.insert(node_id.clone(), group_id.clone());
                node_to_overview.insert(node_id.clone(), owner_id.clone());
            }
            groups.push(GroupBuild {
                id: group_id,
                parent_id: Some(owner_id.clone()),
                kind: ArchitectureGroupKind::Subsystem,
                name,
                owner_key: owner_key.clone(),
                community_ids,
                node_ids: member_ids,
                relationship_count: 0,
                neighbors: BTreeSet::new(),
                cohesion: group_cohesion,
                source_scopes: group_source_counts,
                pinned: false,
                rank: 0,
            });
        }
    }
    check_limit("max_groups", groups.len(), limits.max_groups)?;
    memberships.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let mut build = ScopeBuild {
        groups,
        memberships,
        node_to_leaf,
        node_to_overview,
    };
    let (routes, coverage) = build_routes_and_group_stats(
        &mut build,
        relationships,
        &scoped_node_ids,
        default_lens,
        limits.max_routes,
    )?;
    rank_and_disambiguate(&mut build.groups);
    let (overview_group_ids, overview_route_ids, omissions) = overview_selection(
        &build,
        &routes,
        relationships,
        &scoped_node_ids,
        default_lens,
        limits,
    );
    let quality = architecture_quality(
        scope,
        &build,
        relationships,
        &scoped_node_ids,
        source_counts,
        &coverage,
        &omissions,
        limits.max_diagnostics,
    );
    let group_indexes = build
        .groups
        .iter()
        .enumerate()
        .map(|(index, group)| (group.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let memberships = build
        .memberships
        .iter()
        .map(|membership| {
            let node_index = node_index
                .get(membership.node_id.as_str())
                .copied()
                .ok_or_else(|| {
                    ArchitectureProjectionError::Invariant(format!(
                        "membership node {} is outside the projected node set",
                        membership.node_id
                    ))
                })?;
            let group_index = group_indexes
                .get(membership.group_id.as_str())
                .copied()
                .ok_or_else(|| {
                    ArchitectureProjectionError::Invariant(format!(
                        "membership group {} is outside its scope projection",
                        membership.group_id
                    ))
                })?;
            Ok(ArchitectureMembership {
                node_index,
                group_index,
            })
        })
        .collect::<Result<Vec<_>, ArchitectureProjectionError>>()?;
    drop(group_indexes);
    let groups = build
        .groups
        .into_iter()
        .map(|group| ArchitectureGroup {
            id: group.id,
            parent_id: group.parent_id,
            kind: group.kind,
            rank: group.rank,
            name: group.name,
            owner_key: group.owner_key,
            community_ids: group.community_ids,
            node_count: group.node_ids.len(),
            relationship_count: group.relationship_count,
            neighbor_count: group.neighbors.len(),
            cohesion: group.cohesion,
            source_scopes: group.source_scopes,
            pinned: group.pinned,
        })
        .collect();
    Ok(ArchitectureScopeProjection {
        scope,
        default_lens,
        groups,
        memberships,
        routes,
        overview_group_ids,
        overview_route_ids,
        coverage,
        omissions,
        quality,
    })
}

fn owner_assignment(
    node: &ArchitectureNode,
    overlay: Option<&ArchitectureOverlay>,
) -> OwnerAssignment {
    if let Some(group) = overlay.and_then(|overlay| {
        overlay.groups.iter().find(|group| {
            node.community
                .is_some_and(|community| group.communities.contains(&community))
                || node.source_file.as_deref().is_some_and(|path| {
                    let path = normalized_path(path);
                    group
                        .path_prefixes
                        .iter()
                        .map(|prefix| normalized_path(prefix))
                        .any(|prefix| path_matches_prefix(&path, &prefix))
                })
        })
    }) {
        return OwnerAssignment {
            key: format!("overlay/{}", group.id.trim()),
            display_name: group.name.trim().to_owned(),
            provenance: ArchitectureNameProvenance::Overlay,
            evidence: vec![format!("overlay:{}", group.id.trim())],
            pinned: group.pin,
        };
    }
    let owner_key = node
        .source_file
        .as_deref()
        .map(automatic_owner_key)
        .unwrap_or_else(|| "unknown".to_owned());
    OwnerAssignment {
        display_name: owner_display_name(&owner_key),
        evidence: vec![format!("owner:{owner_key}")],
        key: owner_key,
        provenance: ArchitectureNameProvenance::Owner,
        pinned: false,
    }
}

fn automatic_owner_key(source_file: &str) -> String {
    let normalized = normalized_path(source_file);
    let mut segments = normalized.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next().unwrap_or("root");
    let second = segments.next();
    if matches!(
        first,
        "crates" | "packages" | "apps" | "services" | "editors" | "plugins" | "modules"
    ) && let Some(second) = second
    {
        return format!("{first}/{second}");
    }
    if matches!(first, "src" | "lib" | "app")
        && let Some(second) = second
        && !is_filename(second)
    {
        return format!("{first}/{second}");
    }
    if is_filename(first) {
        "root".to_owned()
    } else {
        first.to_owned()
    }
}

fn is_filename(segment: &str) -> bool {
    segment.rsplit_once('.').is_some()
}

fn scope_includes(scope: ArchitectureScope, source_scope: ArchitectureSourceScope) -> bool {
    matches!(scope, ArchitectureScope::AllCode)
        || matches!(source_scope, ArchitectureSourceScope::Production)
}

fn source_counts_for_ids(
    ids: &[String],
    nodes: &[ArchitectureNode],
    node_index: &BTreeMap<&str, usize>,
) -> ArchitectureSourceCounts {
    let mut counts = ArchitectureSourceCounts::default();
    for id in ids {
        if let Some(index) = node_index.get(id.as_str())
            && let Some(node) = nodes.get(*index)
        {
            counts.increment(node.source_scope);
        }
    }
    counts
}

fn average_community_score(community_ids: &[usize], scores: &BTreeMap<usize, f64>) -> f64 {
    if community_ids.is_empty() {
        return 0.0;
    }
    let sum = community_ids
        .iter()
        .map(|community| scores.get(community).copied().unwrap_or_default())
        .sum::<f64>();
    sum / community_ids.len() as f64
}

fn build_routes_and_group_stats(
    build: &mut ScopeBuild,
    relationships: &[ArchitectureRelationship],
    scoped_node_ids: &BTreeSet<String>,
    default_lens: ArchitectureLens,
    max_routes: usize,
) -> Result<(Vec<ArchitectureRoute>, ArchitectureCoverage), ArchitectureProjectionError> {
    let mut routes =
        BTreeMap::<(ArchitectureRouteLevel, Option<String>, String, String), RouteBuild>::new();
    let mut coverage = ArchitectureCoverage {
        admitted: 0,
        internal: 0,
        cross_group: 0,
        unassigned: 0,
        relation_classes: ArchitectureClassCounts::default(),
    };
    let group_positions = build
        .groups
        .iter()
        .enumerate()
        .map(|(index, group)| (group.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    for relationship in relationships.iter().filter(|relationship| {
        scoped_node_ids.contains(&relationship.source)
            && scoped_node_ids.contains(&relationship.target)
    }) {
        coverage
            .relation_classes
            .increment(relationship.relation_class);
        if !default_lens.admits(relationship.relation_class) {
            continue;
        }
        coverage.admitted += 1;
        let (Some(source_leaf), Some(target_leaf), Some(source_overview), Some(target_overview)) = (
            build.node_to_leaf.get(&relationship.source),
            build.node_to_leaf.get(&relationship.target),
            build.node_to_overview.get(&relationship.source),
            build.node_to_overview.get(&relationship.target),
        ) else {
            coverage.unassigned += 1;
            continue;
        };
        let touched_groups = [source_leaf, target_leaf, source_overview, target_overview]
            .into_iter()
            .collect::<BTreeSet<_>>();
        for group_id in touched_groups {
            if let Some(position) = group_positions.get(group_id)
                && let Some(group) = build.groups.get_mut(*position)
            {
                group.relationship_count += 1;
            }
        }
        if source_leaf == target_leaf {
            coverage.internal += 1;
        } else {
            coverage.cross_group += 1;
            if let (Some(source), Some(target)) = (
                group_positions.get(source_leaf),
                group_positions.get(target_leaf),
            ) {
                if let Some(source_group) = build.groups.get_mut(*source) {
                    source_group.neighbors.insert(target_leaf.clone());
                }
                if let Some(target_group) = build.groups.get_mut(*target) {
                    target_group.neighbors.insert(source_leaf.clone());
                }
            }
            increment_route(
                routes
                    .entry((
                        ArchitectureRouteLevel::Detail,
                        common_owner(source_leaf, target_leaf, &build.groups),
                        source_leaf.clone(),
                        target_leaf.clone(),
                    ))
                    .or_default(),
                relationship,
            );
        }
        if source_overview != target_overview {
            if let (Some(source), Some(target)) = (
                group_positions.get(source_overview),
                group_positions.get(target_overview),
            ) {
                if let Some(source_group) = build.groups.get_mut(*source) {
                    source_group.neighbors.insert(target_overview.clone());
                }
                if let Some(target_group) = build.groups.get_mut(*target) {
                    target_group.neighbors.insert(source_overview.clone());
                }
            }
            increment_route(
                routes
                    .entry((
                        ArchitectureRouteLevel::Overview,
                        None,
                        source_overview.clone(),
                        target_overview.clone(),
                    ))
                    .or_default(),
                relationship,
            );
        }
    }
    check_limit("max_routes", routes.len(), max_routes)?;
    let mut output = routes
        .into_iter()
        .map(
            |((level, owner_id, source_group, target_group), route)| ArchitectureRoute {
                id: format!(
                    "route:{}:{}:{}",
                    match level {
                        ArchitectureRouteLevel::Overview => "overview",
                        ArchitectureRouteLevel::Detail => "detail",
                    },
                    stable_fragment(&source_group),
                    stable_fragment(&target_group)
                ),
                level,
                owner_id,
                source_group,
                target_group,
                relationship_count: route.relationship_count,
                relation_classes: route.relation_classes,
                evidence: route.evidence,
            },
        )
        .collect::<Vec<_>>();
    output.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then_with(|| right.relationship_count.cmp(&left.relationship_count))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok((output, coverage))
}

fn common_owner(source: &str, target: &str, groups: &[GroupBuild]) -> Option<String> {
    let source_parent = groups
        .iter()
        .find(|group| group.id == source)
        .and_then(|group| group.parent_id.clone());
    let target_parent = groups
        .iter()
        .find(|group| group.id == target)
        .and_then(|group| group.parent_id.clone());
    (source_parent.is_some() && source_parent == target_parent)
        .then_some(source_parent)
        .flatten()
}

fn increment_route(route: &mut RouteBuild, relationship: &ArchitectureRelationship) {
    route.relationship_count += 1;
    route
        .relation_classes
        .increment(relationship.relation_class);
    match relationship.confidence.as_str() {
        "inferred" => route.evidence.inferred += 1,
        "ambiguous" => route.evidence.ambiguous += 1,
        _ => route.evidence.extracted += 1,
    }
}

fn rank_and_disambiguate(groups: &mut [GroupBuild]) {
    let mut order = (0..groups.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        groups[*right]
            .pinned
            .cmp(&groups[*left].pinned)
            .then_with(|| {
                groups[*right]
                    .node_ids
                    .len()
                    .cmp(&groups[*left].node_ids.len())
            })
            .then_with(|| {
                groups[*right]
                    .relationship_count
                    .cmp(&groups[*left].relationship_count)
            })
            .then_with(|| {
                groups[*right]
                    .neighbors
                    .len()
                    .cmp(&groups[*left].neighbors.len())
            })
            .then_with(|| groups[*right].cohesion.total_cmp(&groups[*left].cohesion))
            .then_with(|| groups[*left].id.cmp(&groups[*right].id))
    });
    for (rank, index) in order.into_iter().enumerate() {
        groups[index].rank = rank + 1;
    }
    let mut names = groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<Vec<_>>();
    let owners = groups
        .iter()
        .map(|group| group.owner_key.clone())
        .collect::<Vec<_>>();
    disambiguate_names(&mut names, &owners);
    for (group, name) in groups.iter_mut().zip(names) {
        group.name = name;
    }
    groups.sort_by_key(|group| group.rank);
}

#[allow(clippy::too_many_arguments)]
fn overview_selection(
    build: &ScopeBuild,
    routes: &[ArchitectureRoute],
    relationships: &[ArchitectureRelationship],
    scoped_node_ids: &BTreeSet<String>,
    default_lens: ArchitectureLens,
    limits: ArchitectureProjectionLimits,
) -> (Vec<String>, Vec<String>, ArchitectureOmissions) {
    let overview_groups = build
        .groups
        .iter()
        .filter(|group| group.parent_id.is_none())
        .collect::<Vec<_>>();
    let overview_group_ids = overview_groups
        .iter()
        .take(limits.max_overview_groups)
        .map(|group| group.id.clone())
        .collect::<Vec<_>>();
    let shown = overview_group_ids.iter().cloned().collect::<BTreeSet<_>>();
    let overview_route_ids = routes
        .iter()
        .filter(|route| {
            matches!(route.level, ArchitectureRouteLevel::Overview)
                && shown.contains(&route.source_group)
                && shown.contains(&route.target_group)
        })
        .take(limits.max_overview_routes)
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let represented_nodes = scoped_node_ids
        .iter()
        .filter(|node_id| {
            build
                .node_to_overview
                .get(*node_id)
                .is_some_and(|group| shown.contains(group))
        })
        .count();
    let represented_relationships = relationships
        .iter()
        .filter(|relationship| {
            default_lens.admits(relationship.relation_class)
                && scoped_node_ids.contains(&relationship.source)
                && scoped_node_ids.contains(&relationship.target)
                && build
                    .node_to_overview
                    .get(&relationship.source)
                    .is_some_and(|group| shown.contains(group))
                && build
                    .node_to_overview
                    .get(&relationship.target)
                    .is_some_and(|group| shown.contains(group))
        })
        .count();
    let total_relationships = relationships
        .iter()
        .filter(|relationship| {
            default_lens.admits(relationship.relation_class)
                && scoped_node_ids.contains(&relationship.source)
                && scoped_node_ids.contains(&relationship.target)
        })
        .count();
    let witness_group_ids = overview_groups
        .iter()
        .skip(limits.max_overview_groups)
        .take(limits.max_omission_witnesses)
        .map(|group| group.id.clone())
        .collect::<Vec<_>>();
    (
        overview_group_ids,
        overview_route_ids,
        ArchitectureOmissions {
            total_groups: overview_groups.len(),
            shown_groups: shown.len(),
            omitted_groups: overview_groups.len().saturating_sub(shown.len()),
            represented_nodes,
            omitted_nodes: scoped_node_ids.len().saturating_sub(represented_nodes),
            represented_relationships,
            omitted_relationships: total_relationships.saturating_sub(represented_relationships),
            witness_group_ids,
            max_overview_groups: limits.max_overview_groups,
            max_overview_routes: limits.max_overview_routes,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn architecture_quality(
    scope: ArchitectureScope,
    build: &ScopeBuild,
    relationships: &[ArchitectureRelationship],
    scoped_node_ids: &BTreeSet<String>,
    source_scopes: ArchitectureSourceCounts,
    coverage: &ArchitectureCoverage,
    omissions: &ArchitectureOmissions,
    max_diagnostics: usize,
) -> ArchitectureQuality {
    const UNKNOWN_SOURCE_THRESHOLD: f64 = 0.05;
    const REPRESENTED_THRESHOLD: f64 = 0.80;
    const DOMINANT_GROUP_THRESHOLD: f64 = 0.60;
    let total_nodes = scoped_node_ids.len();
    let overview_groups = build
        .groups
        .iter()
        .filter(|group| group.parent_id.is_none())
        .collect::<Vec<_>>();
    let duplicate_names = {
        let mut names = BTreeSet::new();
        overview_groups
            .iter()
            .filter(|group| !names.insert(group.name.value.to_ascii_lowercase()))
            .count()
    };
    let fallback_names = overview_groups
        .iter()
        .filter(|group| matches!(group.name.provenance, ArchitectureNameProvenance::Fallback))
        .count();
    let largest_group = overview_groups
        .iter()
        .map(|group| group.node_ids.len())
        .max()
        .unwrap_or_default();
    let unknown_relations = relationships
        .iter()
        .filter(|relationship| {
            matches!(
                relationship.relation_class,
                ArchitectureRelationClass::Unknown
            ) && scoped_node_ids.contains(&relationship.source)
                && scoped_node_ids.contains(&relationship.target)
        })
        .count();
    let unknown_source_fraction = fraction(source_scopes.unknown, total_nodes);
    let represented_node_fraction = fraction(omissions.represented_nodes, total_nodes);
    let represented_relationship_fraction = fraction(
        omissions.represented_relationships,
        omissions.represented_relationships + omissions.omitted_relationships,
    );
    let largest_group_fraction = fraction(largest_group, total_nodes);
    let generated_vendor_leakage = if matches!(scope, ArchitectureScope::Production) {
        source_scopes.generated + source_scopes.vendor
    } else {
        0
    };
    let unassigned_nodes = scoped_node_ids
        .len()
        .saturating_sub(build.memberships.len());
    let mut diagnostics = Vec::new();
    if generated_vendor_leakage > 0 {
        diagnostics.push(diagnostic(
            "generated_scope_leak",
            ArchitectureDiagnosticSeverity::Error,
            "Generated or vendor nodes leaked into Production.",
            Some(generated_vendor_leakage as f64),
            Some(0.0),
            Vec::new(),
            "Correct source evidence or add an explicit source-scope overlay.",
        ));
    }
    if unknown_source_fraction > UNKNOWN_SOURCE_THRESHOLD {
        diagnostics.push(diagnostic(
            "unknown_source_share",
            ArchitectureDiagnosticSeverity::Warning,
            "The architecture has a material share of nodes without source scope.",
            Some(unknown_source_fraction),
            Some(UNKNOWN_SOURCE_THRESHOLD),
            Vec::new(),
            "Rebuild with complete source inventory or classify paths explicitly.",
        ));
    }
    if duplicate_names > 0 {
        diagnostics.push(diagnostic(
            "duplicate_group_name",
            ArchitectureDiagnosticSeverity::Error,
            "Architecture group names are not unique.",
            Some(duplicate_names as f64),
            Some(0.0),
            Vec::new(),
            "Add owner/path evidence or an explicit project name.",
        ));
    }
    if fallback_names > 0 {
        diagnostics.push(diagnostic(
            "fallback_group_name",
            ArchitectureDiagnosticSeverity::Info,
            "Some groups lack a strong project-specific name.",
            Some(fallback_names as f64),
            Some(0.0),
            overview_groups
                .iter()
                .filter(|group| {
                    matches!(group.name.provenance, ArchitectureNameProvenance::Fallback)
                })
                .take(8)
                .map(|group| group.id.clone())
                .collect(),
            "Add source ownership evidence or an optional architecture overlay.",
        ));
    }
    if total_nodes >= 100 && largest_group_fraction > DOMINANT_GROUP_THRESHOLD {
        diagnostics.push(diagnostic(
            "dominant_group",
            ArchitectureDiagnosticSeverity::Warning,
            "One architecture group dominates the Production projection.",
            Some(largest_group_fraction),
            Some(DOMINANT_GROUP_THRESHOLD),
            overview_groups
                .iter()
                .max_by_key(|group| group.node_ids.len())
                .map(|group| vec![group.id.clone()])
                .unwrap_or_default(),
            "Inspect ownership evidence and community cohesion before trusting the overview.",
        ));
    }
    if represented_node_fraction < REPRESENTED_THRESHOLD {
        diagnostics.push(diagnostic(
            "overview_node_omission",
            ArchitectureDiagnosticSeverity::Warning,
            "The bounded overview omits a material share of nodes.",
            Some(represented_node_fraction),
            Some(REPRESENTED_THRESHOLD),
            omissions.witness_group_ids.clone(),
            "Use the group directory or raise the explicit overview bound.",
        ));
    }
    if represented_relationship_fraction < REPRESENTED_THRESHOLD {
        diagnostics.push(diagnostic(
            "overview_relationship_omission",
            ArchitectureDiagnosticSeverity::Warning,
            "The bounded overview omits a material share of relationships.",
            Some(represented_relationship_fraction),
            Some(REPRESENTED_THRESHOLD),
            omissions.witness_group_ids.clone(),
            "Use the route directory or raise the explicit overview bound.",
        ));
    }
    if unknown_relations > 0 {
        diagnostics.push(diagnostic(
            "unknown_relation",
            ArchitectureDiagnosticSeverity::Info,
            "Some relationship kinds are outside the current architecture policy.",
            Some(unknown_relations as f64),
            Some(0.0),
            Vec::new(),
            "Review the relation vocabulary before admitting new kinds to a lens.",
        ));
    }
    if coverage.unassigned > 0 || unassigned_nodes > 0 {
        diagnostics.push(diagnostic(
            "unassigned_relationship",
            ArchitectureDiagnosticSeverity::Warning,
            "Some nodes or admitted relationships could not be assigned to a group.",
            Some((coverage.unassigned + unassigned_nodes) as f64),
            Some(0.0),
            Vec::new(),
            "Inspect missing community and source ownership evidence.",
        ));
    }
    diagnostics.sort_by(|left, right| left.code.cmp(&right.code));
    diagnostics.truncate(max_diagnostics);
    let status = if total_nodes == 0 || overview_groups.is_empty() {
        ArchitectureQualityStatus::Insufficient
    } else if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            ArchitectureDiagnosticSeverity::Warning | ArchitectureDiagnosticSeverity::Error
        )
    }) {
        ArchitectureQualityStatus::Degraded
    } else {
        ArchitectureQualityStatus::Good
    };
    ArchitectureQuality {
        status,
        metrics: ArchitectureQualityMetrics {
            source_scopes,
            unknown_source_fraction,
            generated_vendor_leakage,
            represented_node_fraction,
            represented_relationship_fraction,
            duplicate_names,
            fallback_names,
            largest_group_fraction,
            unknown_relations,
            unassigned_nodes,
            unassigned_relationships: coverage.unassigned,
        },
        diagnostics,
    }
}

fn diagnostic(
    code: &str,
    severity: ArchitectureDiagnosticSeverity,
    message: &str,
    observed: Option<f64>,
    threshold: Option<f64>,
    witnesses: Vec<String>,
    recommended_action: &str,
) -> ArchitectureQualityDiagnostic {
    ArchitectureQualityDiagnostic {
        code: code.to_owned(),
        severity,
        message: message.to_owned(),
        observed,
        threshold,
        witnesses,
        recommended_action: recommended_action.to_owned(),
    }
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn confidence(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "inferred" => "inferred",
        "ambiguous" => "ambiguous",
        _ => "extracted",
    }
    .to_owned()
}

fn check_limit(
    name: &'static str,
    required: usize,
    limit: usize,
) -> Result<(), ArchitectureProjectionError> {
    if required > limit {
        Err(ArchitectureProjectionError::LimitExceeded {
            name,
            required,
            limit,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    fn fixture() -> Result<(GraphDocument, Communities), Box<dyn Error>> {
        let document = serde_json::from_value(json!({
            "graph": {
                "files": [
                    {"path":"assets/viewer/graph.js","generated":true}
                ]
            },
            "nodes": [
                {"id":"api","label":"ApiRouter","source_file":"crates/app/src/api.rs"},
                {"id":"store","label":"LedgerStore","source_file":"crates/store/src/ledger.rs"},
                {"id":"bundle","label":"x","source_file":"assets/viewer/graph.js"},
                {"id":"test","label":"ApiTest","source_file":"tests/api_test.rs"}
            ],
            "links": [
                {"source":"api","target":"store","relation":"calls","confidence":"EXTRACTED"},
                {"source":"api","target":"store","relation":"contains","confidence":"EXTRACTED"},
                {"source":"bundle","target":"api","relation":"references","confidence":"EXTRACTED"}
            ]
        }))?;
        let communities = BTreeMap::from([
            (
                0,
                vec!["api".to_owned(), "bundle".to_owned(), "test".to_owned()],
            ),
            (1, vec!["store".to_owned()]),
        ]);
        Ok((document, communities))
    }

    #[test]
    fn production_is_scoped_before_grouping_and_relationships_are_typed()
    -> Result<(), Box<dyn Error>> {
        let (document, communities) = fixture()?;
        let model = project_architecture(
            ArchitectureProjectionInput {
                document: &document,
                communities: &communities,
                community_labels: None,
                overlay: None,
                project_name: "Fixture",
                built_at_commit: None,
                generated_at: None,
            },
            &ArchitectureProjectionOptions::default(),
        )?;
        assert_eq!(model.schema, ARCHITECTURE_VIEWER_SCHEMA);
        let production = model
            .projections
            .iter()
            .find(|projection| matches!(projection.scope, ArchitectureScope::Production))
            .ok_or("missing production projection")?;
        assert_eq!(production.memberships.len(), 2);
        assert_eq!(production.coverage.admitted, 1);
        assert_eq!(production.coverage.cross_group, 1);
        assert_eq!(production.coverage.relation_classes.execution, 1);
        assert_eq!(production.coverage.relation_classes.structure, 1);
        assert_eq!(production.quality.metrics.generated_vendor_leakage, 0);
        assert!(
            production
                .groups
                .iter()
                .all(|group| group.name.value != "Other")
        );
        Ok(())
    }

    #[test]
    fn invalid_overlapping_overlay_fails_closed() -> Result<(), Box<dyn Error>> {
        let (document, communities) = fixture()?;
        let overlay: ArchitectureOverlay = serde_json::from_value(json!({
            "schema":"compass.architecture-overlay/1",
            "sourceRules":[
                {"pathPrefix":"src","scope":"production"},
                {"pathPrefix":"src/generated","scope":"generated"}
            ]
        }))?;
        let result = project_architecture(
            ArchitectureProjectionInput {
                document: &document,
                communities: &communities,
                community_labels: None,
                overlay: Some(&overlay),
                project_name: "Fixture",
                built_at_commit: None,
                generated_at: None,
            },
            &ArchitectureProjectionOptions::default(),
        );
        assert!(matches!(
            result,
            Err(ArchitectureProjectionError::InvalidOverlay(_))
        ));
        Ok(())
    }

    #[test]
    fn unsafe_overlay_path_fails_closed() -> Result<(), Box<dyn Error>> {
        let (document, communities) = fixture()?;
        for path_prefix in ["../outside", "/absolute", "C:/outside", "src//hidden"] {
            let overlay: ArchitectureOverlay = serde_json::from_value(json!({
                "schema":"compass.architecture-overlay/1",
                "sourceRules":[{"pathPrefix":path_prefix,"scope":"generated"}]
            }))?;
            let result = project_architecture(
                ArchitectureProjectionInput {
                    document: &document,
                    communities: &communities,
                    community_labels: None,
                    overlay: Some(&overlay),
                    project_name: "Fixture",
                    built_at_commit: None,
                    generated_at: None,
                },
                &ArchitectureProjectionOptions::default(),
            );
            assert!(matches!(
                result,
                Err(ArchitectureProjectionError::InvalidOverlay(_))
            ));
        }
        Ok(())
    }

    #[test]
    fn input_limits_fail_explicitly_instead_of_returning_an_empty_projection()
    -> Result<(), Box<dyn Error>> {
        let (document, communities) = fixture()?;
        let default_options = ArchitectureProjectionOptions::default();
        let options = ArchitectureProjectionOptions {
            limits: ArchitectureProjectionLimits {
                max_nodes: document.nodes.len().saturating_sub(1),
                ..default_options.limits
            },
            ..default_options
        };
        let result = project_architecture(
            ArchitectureProjectionInput {
                document: &document,
                communities: &communities,
                community_labels: None,
                overlay: None,
                project_name: "Fixture",
                built_at_commit: None,
                generated_at: None,
            },
            &options,
        );
        assert!(matches!(
            result,
            Err(ArchitectureProjectionError::LimitExceeded {
                name: "max_nodes",
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn equivalent_input_order_produces_byte_equivalent_projection() -> Result<(), Box<dyn Error>> {
        let (document, communities) = fixture()?;
        let mut reordered = document.clone();
        reordered.nodes.reverse();
        reordered.links.reverse();
        let project = |document: &GraphDocument| {
            project_architecture(
                ArchitectureProjectionInput {
                    document,
                    communities: &communities,
                    community_labels: None,
                    overlay: None,
                    project_name: "Fixture",
                    built_at_commit: Some("abc123"),
                    generated_at: Some("2026-08-23T00:00:00Z"),
                },
                &ArchitectureProjectionOptions::default(),
            )
        };
        let first = serde_json::to_vec(&project(&document)?)?;
        let second = serde_json::to_vec(&project(&reordered)?)?;
        assert_eq!(first, second);
        Ok(())
    }
}
