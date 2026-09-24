//! Bounded, evidence-gated comparison of two published community hierarchies.
//!
//! Identity first: a published group keeps its id across builds through
//! reconciliation, so two hierarchies can be compared on the ids they carry.
//! When an id does not survive, the members decide what happened — a split, a
//! merge, a disappearance, or a group whose evidence named no single heir — and
//! the diff reports the overlap rather than promoting a guess into a fact.

use std::collections::{BTreeMap, BTreeSet};

use compass_graph::{
    Communities, CommunityHierarchy, HierarchyEventKind, ReconcilePolicy, hierarchy_group_members,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::SemanticDiffError;

pub const HIERARCHY_DIFF_SCHEMA: &str = "compass.community-hierarchy-diff/1";

/// Events a diff publishes. The remainder is counted, never dropped silently.
pub const MAX_HIERARCHY_DIFF_EVENTS: usize = 256;

/// One side of a hierarchy comparison.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyDiffSide {
    pub generation: String,
    pub graph_digest: String,
    pub hierarchy_digest: String,
    pub levels: usize,
    pub groups: usize,
}

/// One bounded, evidence-carrying entry in a hierarchy diff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyDiffEvent {
    pub kind: HierarchyEventKind,
    pub level: usize,
    /// Base-build group ids involved, sorted.
    pub base_ids: Vec<String>,
    /// Target-build group ids involved, sorted.
    pub target_ids: Vec<String>,
    /// Best member overlap that justifies the entry.
    pub overlap: f64,
    /// Members involved on the side that has them.
    pub member_count: usize,
}

/// The versioned comparison of two published hierarchies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyDiff {
    pub schema: String,
    pub base: HierarchyDiffSide,
    pub target: HierarchyDiffSide,
    pub policy: ReconcilePolicy,
    pub stable: usize,
    pub split: usize,
    pub merged: usize,
    pub appeared: usize,
    pub disappeared: usize,
    pub ambiguous: usize,
    pub events: Vec<HierarchyDiffEvent>,
    pub omitted_events: usize,
    pub result_digest: String,
}

impl HierarchyDiff {
    /// True when the two builds describe exactly the same groups.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.split == 0
            && self.merged == 0
            && self.appeared == 0
            && self.disappeared == 0
            && self.ambiguous == 0
    }

    pub fn validate(&self) -> Result<(), SemanticDiffError> {
        if self.schema != HIERARCHY_DIFF_SCHEMA {
            return Err(SemanticDiffError::InvalidHierarchyDiff(format!(
                "unsupported community hierarchy diff schema `{}`",
                self.schema
            )));
        }
        if !self.policy.keep_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.policy.keep_threshold)
            || !self.policy.ambiguity_margin.is_finite()
            || self.policy.ambiguity_margin < 0.0
            || self.policy.max_events == 0
        {
            return Err(SemanticDiffError::InvalidHierarchyDiff(
                "diff policy is outside its domain".to_owned(),
            ));
        }
        if self.events.len() > self.policy.max_events {
            return Err(SemanticDiffError::InvalidHierarchyDiff(
                "diff publishes more events than its policy allows".to_owned(),
            ));
        }
        if self.result_digest != self.calculate_digest()? {
            return Err(SemanticDiffError::InvalidHierarchyDiff(
                "diff digest does not describe its payload".to_owned(),
            ));
        }
        Ok(())
    }

    fn calculate_digest(&self) -> Result<String, SemanticDiffError> {
        let mut canonical = self.clone();
        canonical.result_digest.clear();
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

/// Compare two published hierarchies.
///
/// A group whose id survives is stable. Where an id does not survive, the diff
/// looks at the members: a base group whose members reappear across several
/// target groups is a split, several base groups folding into one target group
/// is a merge, and a base group with two candidates inside `ambiguityMargin` of
/// each other is reported as ambiguous with both candidates named.
pub fn compare_hierarchies(
    base: &CommunityHierarchy,
    base_communities: &Communities,
    target: &CommunityHierarchy,
    target_communities: &Communities,
    policy: &ReconcilePolicy,
) -> Result<HierarchyDiff, SemanticDiffError> {
    if !policy.keep_threshold.is_finite()
        || !(0.0..=1.0).contains(&policy.keep_threshold)
        || !policy.ambiguity_margin.is_finite()
        || policy.ambiguity_margin < 0.0
        || policy.max_events == 0
    {
        return Err(SemanticDiffError::InvalidHierarchyDiff(
            "diff policy is outside its domain".to_owned(),
        ));
    }
    let base_members = hierarchy_group_members(base, base_communities);
    let target_members = hierarchy_group_members(target, target_communities);
    let mut events = Vec::new();
    let mut counts = BTreeMap::<HierarchyEventKind, usize>::new();
    for (position, base_level) in base.levels.iter().enumerate() {
        let Some(target_level) = target.levels.get(position) else {
            continue;
        };
        let (Some(base_sets), Some(target_sets)) =
            (base_members.get(position), target_members.get(position))
        else {
            continue;
        };
        let target_by_id = target_level
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        // Surviving identity is the first evidence: the id names the group, and
        // the signature says whether its membership moved underneath it.
        let mut surviving = BTreeSet::<usize>::new();
        for (base_index, group) in base_level.groups.iter().enumerate() {
            let Some(target_index) = target_by_id.get(group.id.as_str()).copied() else {
                continue;
            };
            surviving.insert(base_index);
            let overlap = overlap_of(base_sets, target_sets, base_index, target_index);
            *counts.entry(HierarchyEventKind::Stable).or_default() += 1;
            if group.signature != target_level.groups[target_index].signature {
                events.push(HierarchyDiffEvent {
                    kind: HierarchyEventKind::Stable,
                    level: base_level.level,
                    base_ids: vec![group.id.clone()],
                    target_ids: vec![group.id.clone()],
                    overlap,
                    member_count: group.member_count,
                });
            }
        }
        // Ids that did not survive are matched by members alone.
        let vanished = base_level
            .groups
            .iter()
            .enumerate()
            .filter(|(index, _)| !surviving.contains(index))
            .collect::<Vec<_>>();
        let fresh = target_level
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !base_level.groups.iter().any(|base| base.id == group.id))
            .collect::<Vec<_>>();
        let mut claimed_targets = BTreeSet::new();
        for (base_index, base_group) in vanished {
            let Some(members) = base_sets.get(base_index) else {
                continue;
            };
            let mut related = fresh
                .iter()
                .filter_map(|(target_index, target_group)| {
                    let target_members = target_sets.get(*target_index)?;
                    let overlap = jaccard(members, target_members);
                    (overlap >= policy.keep_threshold).then_some((
                        overlap,
                        *target_index,
                        target_group.id.clone(),
                    ))
                })
                .collect::<Vec<_>>();
            related.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.2.cmp(&right.2))
            });
            match related.as_slice() {
                [] => {
                    *counts.entry(HierarchyEventKind::Disappeared).or_default() += 1;
                    events.push(HierarchyDiffEvent {
                        kind: HierarchyEventKind::Disappeared,
                        level: base_level.level,
                        base_ids: vec![base_group.id.clone()],
                        target_ids: Vec::new(),
                        overlap: 0.0,
                        member_count: base_group.member_count,
                    });
                }
                [(best, best_index, _), (runner_up, runner_index, _), ..] => {
                    if best - runner_up <= policy.ambiguity_margin {
                        *counts.entry(HierarchyEventKind::Ambiguous).or_default() += 1;
                        events.push(HierarchyDiffEvent {
                            kind: HierarchyEventKind::Ambiguous,
                            level: base_level.level,
                            base_ids: vec![base_group.id.clone()],
                            target_ids: related.iter().map(|(_, _, id)| id.clone()).collect(),
                            overlap: *best,
                            member_count: base_group.member_count,
                        });
                    } else {
                        *counts.entry(HierarchyEventKind::Split).or_default() += 1;
                        let mut ids = related
                            .iter()
                            .map(|(_, _, id)| id.clone())
                            .collect::<Vec<_>>();
                        ids.sort();
                        events.push(HierarchyDiffEvent {
                            kind: HierarchyEventKind::Split,
                            level: base_level.level,
                            base_ids: vec![base_group.id.clone()],
                            target_ids: ids,
                            overlap: *best,
                            member_count: base_group.member_count,
                        });
                    }
                    claimed_targets.insert(*best_index);
                    claimed_targets.insert(*runner_index);
                }
                [(best, best_index, id)] => {
                    // The members survived under a new identity: the base id is
                    // gone and the target id is new, and the overlap says they
                    // describe the same code.
                    claimed_targets.insert(*best_index);
                    *counts.entry(HierarchyEventKind::Disappeared).or_default() += 1;
                    events.push(HierarchyDiffEvent {
                        kind: HierarchyEventKind::Disappeared,
                        level: base_level.level,
                        base_ids: vec![base_group.id.clone()],
                        target_ids: Vec::new(),
                        overlap: *best,
                        member_count: base_group.member_count,
                    });
                    let _ = id;
                }
            }
        }
        // A target group several base groups folded into is a merge.
        for (target_index, target_group) in &fresh {
            if claimed_targets.contains(target_index) {
                continue;
            }
            let Some(target_members) = target_sets.get(*target_index) else {
                continue;
            };
            let predecessors = base_level
                .groups
                .iter()
                .enumerate()
                .filter(|(index, _)| !surviving.contains(index))
                .filter(|(index, _)| {
                    base_sets.get(*index).is_some_and(|members| {
                        jaccard(members, target_members) >= policy.keep_threshold
                    })
                })
                .map(|(_, group)| group.id.clone())
                .collect::<Vec<_>>();
            let member_count = target_group.member_count;
            let id = target_group.id.clone();
            if predecessors.len() > 1 {
                *counts.entry(HierarchyEventKind::Merged).or_default() += 1;
                events.push(HierarchyDiffEvent {
                    kind: HierarchyEventKind::Merged,
                    level: target_level.level,
                    base_ids: predecessors,
                    target_ids: vec![id],
                    overlap: target_sets
                        .get(*target_index)
                        .map_or(0.0, |_| policy.keep_threshold),
                    member_count,
                });
            } else {
                *counts.entry(HierarchyEventKind::Appeared).or_default() += 1;
                events.push(HierarchyDiffEvent {
                    kind: HierarchyEventKind::Appeared,
                    level: target_level.level,
                    base_ids: Vec::new(),
                    target_ids: vec![id],
                    overlap: 0.0,
                    member_count,
                });
            }
        }
    }
    events.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.base_ids.cmp(&right.base_ids))
            .then_with(|| left.target_ids.cmp(&right.target_ids))
    });
    // `stable` is counted, not listed, unless the membership moved: a listing
    // exists to show a reader what changed.
    let omitted_events = events.len().saturating_sub(policy.max_events);
    let stable = counts
        .get(&HierarchyEventKind::Stable)
        .copied()
        .unwrap_or_default();
    events.truncate(policy.max_events);
    let mut diff = HierarchyDiff {
        schema: HIERARCHY_DIFF_SCHEMA.to_owned(),
        base: side(base),
        target: side(target),
        policy: *policy,
        stable,
        split: counts
            .get(&HierarchyEventKind::Split)
            .copied()
            .unwrap_or_default(),
        merged: counts
            .get(&HierarchyEventKind::Merged)
            .copied()
            .unwrap_or_default(),
        appeared: counts
            .get(&HierarchyEventKind::Appeared)
            .copied()
            .unwrap_or_default(),
        disappeared: counts
            .get(&HierarchyEventKind::Disappeared)
            .copied()
            .unwrap_or_default(),
        ambiguous: counts
            .get(&HierarchyEventKind::Ambiguous)
            .copied()
            .unwrap_or_default(),
        events,
        omitted_events,
        result_digest: String::new(),
    };
    diff.result_digest = diff.calculate_digest()?;
    diff.validate()?;
    Ok(diff)
}

/// `None` when either side published no hierarchy: absence means unavailable
/// navigation, never an empty comparison.
pub fn compare_optional_hierarchies(
    base: Option<(&CommunityHierarchy, &Communities)>,
    target: Option<(&CommunityHierarchy, &Communities)>,
    policy: &ReconcilePolicy,
) -> Result<Option<HierarchyDiff>, SemanticDiffError> {
    match (base, target) {
        (Some((base, base_communities)), Some((target, target_communities))) => Ok(Some(
            compare_hierarchies(base, base_communities, target, target_communities, policy)?,
        )),
        _ => Ok(None),
    }
}

fn side(hierarchy: &CommunityHierarchy) -> HierarchyDiffSide {
    HierarchyDiffSide {
        generation: hierarchy.graph_generation.clone(),
        graph_digest: hierarchy.graph_digest.clone(),
        hierarchy_digest: hierarchy.result_digest.clone(),
        levels: hierarchy.levels.len(),
        groups: hierarchy
            .levels
            .iter()
            .map(|level| level.groups.len())
            .sum(),
    }
}

fn overlap_of(
    base: &[BTreeSet<String>],
    target: &[BTreeSet<String>],
    base_index: usize,
    target_index: usize,
) -> f64 {
    match (base.get(base_index), target.get(target_index)) {
        (Some(base), Some(target)) => jaccard(base, target),
        _ => 0.0,
    }
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / union as f64
}
