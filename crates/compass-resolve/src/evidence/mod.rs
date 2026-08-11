use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;
use std::path::Path;
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use compass_languages::{
    BindingFact, CandidateRelation, DeclarationFact, EvidenceLimits, EvidenceRange,
    HierarchyConstraint, OccurrenceFact, ReceiverDispatchStrategy, RelationshipCandidate,
    SemanticEvidenceBatch, make_id, validate_evidence,
};
use compass_model::provenance::{NODE_PROVENANCE_ANCHOR_ATTRIBUTE, OCCURRENCE_RULE_ATTRIBUTE};
use rayon::prelude::*;
use serde_json::{Map, Value};

use compass_languages::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};

mod api;
mod budget;
mod facts;
mod index;
mod languages;
mod project;
mod projection;
mod resolve;

use projection::is_deferred_receiver;
pub(crate) use projection::is_replaced_relation;

pub use api::{ResolutionDecision, ResolutionEvidence, ResolutionRule, UniversalResolutionLimits};
use budget::LookupBudget;
use facts::FactStore;
use index::ResolutionIndexes;
use languages::policy::LanguagePolicyKind;
use languages::rust::{
    rust_external_wildcard_target_is_explicit, rust_impl_associated_trait_name_index,
    rust_impl_associated_type_index, rust_impl_trait_index, rust_module_is_descendant,
};
use languages::typescript::{
    typescript_declaration_basic_allowed, typescript_declaration_basic_allowed_with_type_owner,
};
use project::*;
use resolve::context::ResolutionDb;

#[derive(Clone, Debug)]
struct DirectBaseLink {
    qualified_name: Option<String>,
    source_file: String,
    start_byte: u64,
    end_byte: u64,
    candidate_id: String,
}

#[derive(Clone, Debug, Default)]
struct DirectBaseSet {
    links: Vec<DirectBaseLink>,
    complete: bool,
}

#[derive(Clone, Debug, Default)]
struct DirectSubtypeSet {
    types: Vec<String>,
    complete: bool,
}

#[derive(Clone, Debug)]
struct WildcardModuleSet {
    modules: Vec<String>,
    complete: bool,
}

#[derive(Clone, Debug, Default)]
struct AssociatedTypeSet {
    declarations: Vec<DeclarationSlot>,
    complete: bool,
}

#[derive(Clone, Debug)]
struct RustImplTraitSet {
    candidate_ids: Vec<String>,
    complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TypeScriptReexportTarget {
    module: String,
    exported: String,
}

#[derive(Clone, Debug)]
struct TypeScriptMemberPath {
    root_export: String,
    type_arguments: Vec<String>,
    call_result: bool,
    call_argument_types: Vec<String>,
    call_type_arguments: Vec<String>,
    call_member_index: Option<usize>,
    indexed: bool,
    members: Vec<String>,
}

/// A source-proven object spread emitted by the TypeScript/JavaScript
/// candidate adapter. The `*` member spelling is an owner alias marker, not a
/// wildcard export: the destination object inherits only members that the
/// resolver can prove on this source owner.
#[derive(Clone, Debug)]
struct TypeScriptMemberAlias {
    source: String,
    source_slot: Option<DeclarationSlot>,
    start_byte: u64,
}

struct TypeScriptMemberContext<'a> {
    owner_signature: Option<&'a str>,
    type_arguments: &'a [String],
    index_selector: Option<&'a str>,
}

/// Compact slot into the declaration table used by secondary indexes.
///
/// Declaration IDs are long, repeated strings on corpus-scale Java and
/// Python repositories. Keeping those IDs in every lookup vector dominates
/// the universal resolver's transient memory, while the declaration table
/// already provides the canonical ID for each slot.
type DeclarationSlot = u32;
type TypeScriptModuleIndex = AHashMap<(String, String, String), Vec<DeclarationSlot>>;
type TypeScriptReexportIndex = AHashMap<(String, String, String), Vec<TypeScriptReexportTarget>>;
type TypeScriptProjectModuleIndex = AHashMap<(String, String, String), Vec<String>>;
type TypeScriptProjectMetadataIndex =
    AHashMap<(String, String, String, String), BTreeMap<String, String>>;

struct TypeScriptExportWalk<'a> {
    candidate: &'a RelationshipCandidate,
    allow_type_owner: bool,
    visiting: BTreeSet<(String, String, String)>,
    slots: BTreeSet<DeclarationSlot>,
}

pub struct UniversalResolutionIndex {
    facts: FactStore,
    indexes: ResolutionIndexes,
    project: ProjectContext,
    budget: LookupBudget,
    low_test_aliases: AHashMap<String, Vec<String>>,
}

impl UniversalResolutionIndex {
    #[must_use]
    pub fn candidate_ids(&self) -> Vec<&str> {
        let db = ResolutionDb::new(self);
        let mut ordered = self
            .facts
            .candidates
            .iter()
            .map(|(id, candidate)| {
                let range = db
                    .occurrence(candidate)
                    .map(|occurrence| &occurrence.range)
                    .or_else(|| {
                        self.facts
                            .declarations
                            .get(&candidate.source_declaration_id)
                            .map(|declaration| &declaration.range)
                    });
                (id.as_str(), range)
            })
            .collect::<Vec<_>>();
        ordered.par_sort_unstable_by(|(left_id, left_range), (right_id, right_range)| {
            left_range
                .map(|range| range.source_file.as_str())
                .unwrap_or_default()
                .cmp(
                    right_range
                        .map(|range| range.source_file.as_str())
                        .unwrap_or_default(),
                )
                .then_with(|| {
                    left_range
                        .map_or(u64::MAX, |range| range.start_byte)
                        .cmp(&right_range.map_or(u64::MAX, |range| range.start_byte))
                })
                .then_with(|| {
                    left_range
                        .map_or(u64::MAX, |range| range.end_byte)
                        .cmp(&right_range.map_or(u64::MAX, |range| range.end_byte))
                })
                .then_with(|| left_id.cmp(right_id))
        });
        ordered.into_iter().map(|(id, _)| id).collect()
    }
}

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn declaration_id(&self, slot: DeclarationSlot) -> Option<&str> {
        self.facts
            .declaration_ids
            .get(usize::try_from(slot).ok()?)
            .map(String::as_str)
    }

    pub(in crate::evidence) fn declaration(
        &self,
        slot: DeclarationSlot,
    ) -> Option<&DeclarationFact> {
        self.declaration_id(slot)
            .and_then(|id| self.facts.declarations.get(id))
    }

    pub(in crate::evidence) fn declaration_allowed_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        self.declaration_id(slot)
            .is_some_and(|id| self.declaration_allowed(id, candidate))
    }

    #[must_use]
    pub(in crate::evidence) fn occurrence(
        &self,
        candidate: &RelationshipCandidate,
    ) -> Option<&OccurrenceFact> {
        candidate
            .occurrence_id
            .as_deref()
            .and_then(|id| self.facts.occurrences.get(id))
    }

    pub(in crate::evidence) fn unique_decision(
        &self,
        ids: Option<&Vec<DeclarationSlot>>,
        candidate: &RelationshipCandidate,
        rule: ResolutionRule,
    ) -> Option<ResolutionDecision> {
        let ids = ids?;
        let eligible = ids
            .iter()
            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .copied()
            .collect::<Vec<_>>();
        if candidate.language == "rust"
            && matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            )
        {
            let modules = eligible
                .iter()
                .filter(|slot| {
                    self.declaration(**slot)
                        .is_some_and(|declaration| declaration.kind == "module")
                })
                .copied()
                .collect::<Vec<_>>();
            let only_module_realizations = eligible.iter().all(|slot| {
                self.declaration(*slot).is_some_and(|declaration| {
                    matches!(declaration.kind.as_str(), "file" | "module")
                })
            });
            if let [module] = modules.as_slice()
                && only_module_realizations
            {
                return Some(ResolutionDecision::Resolved {
                    declaration_id: self.declaration_id(*module)?.to_owned(),
                    evidence: ResolutionEvidence {
                        rule,
                        candidate_count: 1,
                    },
                });
            }
        }
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn typescript_declaration_allowed_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        let Some(target) = self.declaration(slot) else {
            return false;
        };
        typescript_declaration_basic_allowed(target, candidate)
    }

    pub(in crate::evidence) fn typescript_declaration_allowed_owner_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        let Some(target) = self.declaration(slot) else {
            return false;
        };
        typescript_declaration_basic_allowed_with_type_owner(target, candidate)
    }

    pub(in crate::evidence) fn unique_typescript_decision(
        &self,
        ids: Option<&Vec<DeclarationSlot>>,
        candidate: &RelationshipCandidate,
        rule: ResolutionRule,
    ) -> Option<ResolutionDecision> {
        let ids = ids?;
        let eligible = ids
            .iter()
            .filter(|slot| self.typescript_declaration_allowed_slot(**slot, candidate))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .copied()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }
}

fn declaration_basic_allowed(target: &DeclarationFact, candidate: &RelationshipCandidate) -> bool {
    target.language == candidate.language
        && candidate
            .constraints
            .argument_count
            .is_none_or(|arguments| {
                target.parameter_count.is_none_or(|parameters| {
                    arguments == parameters
                        || (target.variadic && arguments >= parameters.saturating_sub(1))
                })
            })
        && (candidate.constraints.allowed_target_kinds.is_empty()
            || candidate
                .constraints
                .allowed_target_kinds
                .contains(&target.kind))
}

fn declaration_overloads<'a>(
    declarations: impl Iterator<Item = &'a DeclarationFact>,
) -> AHashMap<String, String> {
    let mut groups = AHashMap::<(String, String, String, String), Vec<&DeclarationFact>>::new();
    for declaration in declarations {
        groups
            .entry((
                declaration.language.clone(),
                declaration.range.source_file.clone(),
                declaration.kind.clone(),
                declaration.qualified_name.clone(),
            ))
            .or_default()
            .push(declaration);
    }
    let mut overloads = AHashMap::new();
    for declarations in groups.values_mut().filter(|group| group.len() > 1) {
        declarations.sort_by_key(|declaration| (declaration.range.start_byte, &declaration.id));
        for (position, declaration) in declarations.iter().enumerate() {
            overloads.insert(declaration.id.clone(), format!("overload:{position}"));
        }
    }
    overloads
}

fn wildcard_qualified_names(
    language: &str,
    module: &str,
    qualifier: Option<&str>,
    spelling: &str,
) -> Vec<String> {
    let separator = if language == "rust" || module.contains("::") {
        "::"
    } else {
        "."
    };
    let mut parts = vec![module];
    if let Some(qualifier) = qualifier.filter(|qualifier| !qualifier.is_empty()) {
        parts.push(qualifier);
    }
    parts.push(spelling);
    vec![parts.join(separator)]
}

fn split_qualified_member(qualified: &str) -> Option<(&str, &str)> {
    qualified
        .rsplit_once("::")
        .or_else(|| qualified.rsplit_once('.'))
}

fn qualified_root(qualified: &str) -> &str {
    qualified
        .trim_start_matches('<')
        .split("::")
        .next()
        .unwrap_or(qualified)
}

fn declaration_slot(declaration_ids: &[String], id: &str) -> Option<DeclarationSlot> {
    let index = declaration_ids
        .binary_search_by(|candidate| candidate.as_str().cmp(id))
        .ok()?;
    u32::try_from(index).ok()
}

fn c3_merge(mut sequences: Vec<Vec<String>>, limit: usize) -> Result<Vec<String>, ()> {
    let mut merged = Vec::new();
    loop {
        sequences.retain(|sequence| !sequence.is_empty());
        if sequences.is_empty() {
            return Ok(merged);
        }
        if merged.len() >= limit {
            return Err(());
        }
        let candidate = sequences
            .iter()
            .map(|sequence| &sequence[0])
            .find(|head| {
                sequences
                    .iter()
                    .all(|sequence| !sequence.iter().skip(1).any(|item| item == *head))
            })
            .cloned()
            .ok_or(())?;
        merged.push(candidate.clone());
        for sequence in &mut sequences {
            if sequence.first() == Some(&candidate) {
                sequence.remove(0);
            }
        }
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
        *started = Instant::now();
    }
}

fn insert_unique<T>(map: &mut AHashMap<String, T>, id: String, value: T) -> Result<(), String> {
    match map.entry(id) {
        Entry::Occupied(entry) => Err(format!("duplicate universal evidence id {:?}", entry.key())),
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
    }
}

fn sort_declaration_index<K: Eq + Hash>(
    index: &mut AHashMap<K, Vec<DeclarationSlot>>,
    declaration_ids: &[String],
    candidate_limit: usize,
) {
    for values in index.values_mut() {
        values.sort_unstable_by(|left, right| {
            declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
        });
        values.dedup();
        if values.len() > candidate_limit {
            values.truncate(candidate_storage_limit(candidate_limit));
        }
    }
}

/// Keep one extra candidate as an overflow marker when an index is bounded.
///
/// A lookup is allowed to resolve only when exactly one eligible candidate is
/// present. Truncating a vector of two candidates to a configured limit of one
/// erased the evidence that the result was ambiguous and could manufacture a
/// false unique resolution. The extra slot keeps the operation bounded while
/// making incompleteness observable to the existing `take(limit + 1)` checks.
fn candidate_storage_limit(candidate_limit: usize) -> usize {
    if candidate_limit == 0 {
        0
    } else {
        candidate_limit.saturating_add(1)
    }
}

fn unique_definition_ranges(
    declarations: &AHashMap<String, DeclarationFact>,
    scopes: &AHashMap<String, compass_languages::ScopeFact>,
) -> BTreeMap<String, EvidenceRange> {
    let mut ranges = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for scope in scopes.values() {
        let Some(owner_id) = scope.owner_declaration_id.as_ref() else {
            continue;
        };
        let Some(declaration) = declarations.get(owner_id) else {
            continue;
        };
        if !range_contains(&scope.range, &declaration.range) || ambiguous.contains(owner_id) {
            continue;
        }
        if ranges
            .insert(owner_id.clone(), scope.range.clone())
            .is_some()
        {
            ranges.remove(owner_id);
            ambiguous.insert(owner_id.clone());
        }
    }
    ranges
}

fn range_contains(outer: &EvidenceRange, inner: &EvidenceRange) -> bool {
    outer.source_file == inner.source_file
        && outer.start_byte <= inner.start_byte
        && inner.end_byte <= outer.end_byte
        && (outer.start_line, outer.start_column) <= (inner.start_line, inner.start_column)
        && (inner.end_line, inner.end_column) <= (outer.end_line, outer.end_column)
}
