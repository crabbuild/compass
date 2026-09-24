//! Viewer projection of the published community hierarchy.
//!
//! The artifact describes levels over the level below; a viewer needs one
//! drawable graph per level. Each level here carries the bounded projection the
//! standalone workbench renders — one node per group, weighted edges between
//! groups — plus the evidence a reader inspects: the rule that merged the
//! level, the label and its provenance, group quality, and the child indices
//! that make descending a level exact.

use std::collections::BTreeMap;

use compass_graph::{Communities, CommunityHierarchyLevels, HierarchyLabelRule, LevelMerge};
use compass_model::GraphDocument;
use serde::Serialize;

use crate::html::{HtmlOptions, aggregate};
use crate::viewer_model::GraphViewModel;

pub const HIERARCHY_VIEW_SCHEMA: &str = "compass.viewer.hierarchy/1";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityHierarchyView {
    pub schema: &'static str,
    pub budget_identity: String,
    pub merge_policy: String,
    pub root_target: usize,
    pub level_target: usize,
    pub max_levels: usize,
    pub budget_satisfied: bool,
    pub finest_community_count: usize,
    pub finest_signature: String,
    pub boundary_kinds: Vec<String>,
    pub levels: Vec<HierarchyLevelView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyLevelView {
    pub level: usize,
    pub merge: LevelMerge,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<f64>,
    pub group_count: usize,
    pub member_count: usize,
    pub groups: Vec<HierarchyGroupView>,
    /// The bounded projection this level draws, omitted when a level holds more
    /// groups than the export's node budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<GraphViewModel>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyGroupView {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<usize>,
    pub label: String,
    pub label_rule: HierarchyLabelRule,
    pub label_generic: bool,
    pub member_count: usize,
    pub child_indices: Vec<usize>,
    pub cohesion: f64,
    pub conductance: f64,
    pub boundary_kinds: BTreeMap<String, usize>,
    pub detail_available: bool,
}

/// Project the published hierarchy for the viewer.
///
/// `node_budget` bounds a level's projection the same way the overview bounds
/// the symbol canvas: a level that cannot be drawn inside the budget publishes
/// its groups and no model, so the payload stays bounded on a repository with
/// thousands of communities.
#[must_use]
pub fn community_hierarchy_view(
    hierarchy: &CommunityHierarchyLevels<'_>,
    document: &GraphDocument,
    communities: &Communities,
    community_details: &BTreeMap<usize, GraphViewModel>,
    options: &HtmlOptions<'_>,
    title: &str,
    node_budget: usize,
) -> CommunityHierarchyView {
    let members_by_group = level_members(hierarchy, communities);
    let levels = hierarchy
        .levels
        .iter()
        .enumerate()
        .map(|(position, level)| {
            let level_communities = members_by_group.get(position).cloned().unwrap_or_default();
            let model = (level.groups.len() <= node_budget).then(|| {
                let (meta, meta_communities, member_counts) =
                    aggregate(document, &level_communities, options);
                crate::viewer_model::graph_view_model(
                    &meta,
                    &meta_communities,
                    title.to_owned(),
                    &HtmlOptions {
                        community_labels: options.community_labels,
                        member_counts: Some(&member_counts),
                        node_limit: None,
                        learning_overlay: options.learning_overlay,
                    },
                    true,
                )
            });
            HierarchyLevelView {
                level: level.level,
                merge: level.merge,
                resolution: level.resolution,
                group_count: level.group_count,
                member_count: level
                    .groups
                    .iter()
                    .map(|group| group.member_count)
                    .sum::<usize>(),
                groups: level
                    .groups
                    .iter()
                    .map(|group| HierarchyGroupView {
                        index: group.index,
                        community: group.community,
                        label: group.label.text.clone(),
                        label_rule: group.label.rule,
                        label_generic: group.label.generic,
                        member_count: group.member_count,
                        child_indices: group.child_indices.clone(),
                        cohesion: group.quality.cohesion,
                        conductance: group.quality.conductance,
                        boundary_kinds: group.quality.boundary_kinds.clone(),
                        detail_available: group
                            .community
                            .is_some_and(|community| community_details.contains_key(&community)),
                    })
                    .collect(),
                model,
            }
        })
        .collect();
    CommunityHierarchyView {
        schema: HIERARCHY_VIEW_SCHEMA,
        budget_identity: hierarchy.budget_identity.to_owned(),
        merge_policy: hierarchy.merge_policy.to_owned(),
        root_target: hierarchy.budget.root_target,
        level_target: hierarchy.budget.level_target,
        max_levels: hierarchy.budget.max_levels,
        budget_satisfied: hierarchy.budget_satisfied,
        finest_community_count: hierarchy.finest_community_count,
        finest_signature: hierarchy.finest_signature.to_owned(),
        boundary_kinds: hierarchy.boundary_kinds.to_vec(),
        levels,
    }
}

/// Group members per level, coarsest first, without repeating node ids in the
/// artifact: the finest level is the published partition and every level above
/// it is the union of its children.
fn level_members(
    hierarchy: &CommunityHierarchyLevels<'_>,
    communities: &Communities,
) -> Vec<Communities> {
    let mut levels = vec![Communities::new(); hierarchy.levels.len()];
    let Some(finest) = hierarchy.levels.last() else {
        return levels;
    };
    if let Some(slot) = levels.last_mut() {
        for (position, group) in finest.groups.iter().enumerate() {
            let members = group
                .community
                .and_then(|community| communities.get(&community))
                .cloned()
                .unwrap_or_default();
            slot.insert(position, members);
        }
    }
    for position in (0..hierarchy.levels.len().saturating_sub(1)).rev() {
        let Some(finer) = levels.get(position + 1).cloned() else {
            continue;
        };
        let Some(level) = hierarchy.levels.get(position) else {
            continue;
        };
        let mut level_map = Communities::new();
        for (index, group) in level.groups.iter().enumerate() {
            let mut members = Vec::new();
            for child in &group.child_indices {
                if let Some(child_members) = finer.get(child) {
                    members.extend(child_members.iter().cloned());
                }
            }
            members.sort();
            members.dedup();
            level_map.insert(index, members);
        }
        if let Some(slot) = levels.get_mut(position) {
            *slot = level_map;
        }
    }
    levels
}
