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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalResolutionLimits {
    pub declarations: usize,
    pub bindings: usize,
    pub occurrences: usize,
    pub candidates: usize,
    pub candidates_per_lookup: usize,
}

impl Default for UniversalResolutionLimits {
    fn default() -> Self {
        Self {
            declarations: 1_000_000,
            bindings: 1_000_000,
            occurrences: 5_000_000,
            candidates: 5_000_000,
            candidates_per_lookup: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionRule {
    ExactSourceDeclaration,
    ExactLexicalDeclaration,
    ExplicitBinding,
    ProjectModuleBinding,
    MemberBinding,
    DeferredReceiver,
    WildcardBinding,
    UniqueModuleOrPackage,
    ExactHierarchyBase,
    DirectReceiverSuccessorDispatch,
    LinearizedReceiverDispatch,
    ClosedWorldReceiverDispatch,
    IncompleteHierarchyReceiverDispatch,
    RustAssociatedType,
    ExactSourceInventory,
    QualifiedExternal,
}

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
    members: Vec<String>,
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
type TypeScriptProjectModuleIndex = AHashMap<(String, String), Vec<String>>;

struct TypeScriptExportWalk<'a> {
    candidate: &'a RelationshipCandidate,
    allow_type_owner: bool,
    visiting: BTreeSet<(String, String, String)>,
    slots: BTreeSet<DeclarationSlot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionEvidence {
    pub rule: ResolutionRule,
    pub candidate_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionDecision {
    Resolved {
        declaration_id: String,
        evidence: ResolutionEvidence,
    },
    ResolvedInventory {
        graph_node_id: String,
        evidence: ResolutionEvidence,
    },
    QualifiedExternal {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    DeferredReceiver {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    Ambiguous {
        candidate_count: usize,
    },
    Unresolved,
}

pub struct UniversalResolutionIndex {
    declarations: AHashMap<String, DeclarationFact>,
    declaration_ids: Vec<String>,
    occurrences: AHashMap<String, OccurrenceFact>,
    bindings: AHashMap<String, compass_languages::BindingFact>,
    candidates: AHashMap<String, RelationshipCandidate>,
    scopes: AHashMap<String, compass_languages::ScopeFact>,
    definition_ranges: BTreeMap<String, EvidenceRange>,
    by_qualified: AHashMap<(String, String), Vec<DeclarationSlot>>,
    by_module_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    by_scope_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    by_source_directory_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    /// TypeScript/JavaScript source modules are indexed by the normalized
    /// repository-relative module path rather than by a basename. This keeps
    /// relative imports project-aware without allowing terminal-name lookup
    /// to select a same-spelled declaration from another directory.
    typescript_modules: TypeScriptModuleIndex,
    /// Export aliases such as `export default Foo` and `export { Foo as Bar }`
    /// retain the exact source declaration selected by the adapter. Re-export
    /// chains without a local declaration remain unresolved until a later
    /// module hop can prove one target.
    typescript_export_aliases: TypeScriptModuleIndex,
    /// Cross-file re-exports retain their normalized target module and export
    /// spelling. Resolution follows this bounded table rather than selecting
    /// a terminal name from an unrelated source file.
    typescript_reexport_targets: TypeScriptReexportIndex,
    /// Project/module resolver decisions keyed by importer and raw module
    /// specifier. Values are normalized source-module keys and are retained
    /// only when the existing bounded project resolver admitted a target.
    typescript_project_modules: TypeScriptProjectModuleIndex,
    root: std::path::PathBuf,
    direct_bases: AHashMap<(String, String), DirectBaseSet>,
    direct_subtypes: AHashMap<(String, String), DirectSubtypeSet>,
    members_by_owner: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    rust_impl_associated_types: AHashMap<(String, String, String), AssociatedTypeSet>,
    rust_impl_associated_trait_names: AHashMap<(String, String), WildcardModuleSet>,
    rust_impl_traits: AHashMap<(String, String), RustImplTraitSet>,
    inventory_by_qualified: AHashMap<(String, String), Vec<String>>,
    aliases: AHashMap<(String, String), Vec<String>>,
    rust_source_wildcard_targets: AHashSet<String>,
    wildcard_bindings_by_scope: AHashMap<(String, String), WildcardModuleSet>,
    wildcard_bindings_by_module: AHashMap<(String, String), WildcardModuleSet>,
    wildcard_reexports_by_module: AHashMap<(String, String), WildcardModuleSet>,
    members: AHashMap<(String, String, String), Vec<String>>,
    return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
    outer_return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
    go_module_path: Option<String>,
    limits: UniversalResolutionLimits,
}

struct PreparedTarget<'a> {
    candidate_id: &'a str,
    target: String,
    rule: ResolutionRule,
    target_kind: Option<String>,
    declaration_id: Option<String>,
    external_qualified_name: Option<String>,
    deferred_qualified_name: Option<String>,
}

impl UniversalResolutionIndex {
    pub fn new(
        batches: &[SemanticEvidenceBatch],
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory(batches, &[], Path::new("."), limits)
    }

    pub fn new_with_inventory(
        batches: &[SemanticEvidenceBatch],
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned(batches.to_vec(), inventory_nodes, root, limits)
    }

    pub(crate) fn new_with_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(batches, inventory_nodes, &[], root, limits, true)
    }

    pub(crate) fn new_with_project_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            true,
        )
    }

    pub(crate) fn new_with_prevalidated_project_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            false,
        )
    }

    fn new_with_inventory_owned_impl(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
        validate_batches: bool,
    ) -> Result<Self, String> {
        let mut profile_started = Instant::now();
        if validate_batches {
            batches.par_iter().try_for_each(|batch| {
                validate_evidence(batch, EvidenceLimits::default())
                    .map_err(|error| format!("invalid universal evidence: {error}"))
            })?;
        }

        // Check aggregate input sizes before reserving hash-map capacity. A
        // valid batch is bounded on its own, but an unbounded number of valid
        // batches could otherwise make the old aggregate reservation attempt
        // a very large allocation before the resolver's configured limits are
        // consulted. The scope table has no separate public limit; sharing the
        // candidate ceiling keeps the public limit shape stable while still
        // bounding every resolver-owned primary table.
        let capacities = batches.iter().try_fold(
            [0_usize; 5],
            |mut counts, batch| -> Result<[usize; 5], String> {
                for (index, (name, value)) in [
                    ("declarations", batch.declarations.len()),
                    ("occurrences", batch.occurrences.len()),
                    ("bindings", batch.bindings.len()),
                    ("candidates", batch.candidates.len()),
                    ("scopes", batch.scopes.len()),
                ]
                .into_iter()
                .enumerate()
                {
                    counts[index] = counts[index].checked_add(value).ok_or_else(|| {
                        format!("universal aggregate {name} count overflows usize")
                    })?;
                }
                Ok(counts)
            },
        )?;
        for (name, count, limit) in [
            ("declarations", capacities[0], limits.declarations),
            ("occurrences", capacities[1], limits.occurrences),
            ("bindings", capacities[2], limits.bindings),
            ("candidates", capacities[3], limits.candidates),
            ("scopes", capacities[4], limits.candidates),
        ] {
            if count > limit {
                return Err(format!(
                    "universal aggregate {name} count {count} exceeds limit {limit}"
                ));
            }
        }

        let go_module_path = read_go_module_path(root);
        // Reserve the checked aggregate fact counts before consuming the
        // batches. A corpus-scale evidence index otherwise grows each primary
        // map by repeated rehashes while the old and new tables overlap in
        // memory.
        let mut declarations = AHashMap::with_capacity(capacities[0]);
        let mut occurrences = AHashMap::with_capacity(capacities[1]);
        let mut bindings = AHashMap::with_capacity(capacities[2]);
        let mut candidates = AHashMap::with_capacity(capacities[3]);
        let mut scopes = AHashMap::with_capacity(capacities[4]);
        profile_internal("universal evidence validation", &mut profile_started);
        for batch in batches {
            for fact in batch.declarations {
                insert_unique(&mut declarations, fact.id.clone(), fact)?;
            }
            for fact in batch.occurrences {
                insert_unique(&mut occurrences, fact.id.clone(), fact)?;
            }
            for fact in batch.bindings {
                insert_unique(&mut bindings, fact.id.clone(), fact)?;
            }
            for fact in batch.candidates {
                insert_unique(&mut candidates, fact.id.clone(), fact)?;
            }
            for fact in batch.scopes {
                insert_unique(&mut scopes, fact.id.clone(), fact)?;
            }
        }
        profile_internal("universal fact collection", &mut profile_started);
        let rust_source_wildcard_targets = declarations
            .values()
            .filter(|declaration| {
                declaration.language == "rust"
                    && matches!(declaration.kind.as_str(), "file" | "module" | "enum")
            })
            .map(|declaration| declaration.qualified_name.clone())
            .collect::<AHashSet<_>>();
        let mut declaration_ids = declarations.keys().cloned().collect::<Vec<_>>();
        declaration_ids.sort_unstable();
        if u32::try_from(declaration_ids.len()).is_err() {
            return Err("universal declaration slot count exceeds u32".to_owned());
        }
        let definition_ranges = unique_definition_ranges(&declarations, &scopes);
        let typescript_project_modules = typescript_project_module_index(
            project_edges,
            inventory_nodes,
            root,
            limits.candidates,
            limits.candidates_per_lookup,
        )?;
        let (typescript_modules, typescript_export_aliases, typescript_reexport_targets) =
            typescript_module_indices(
                &declarations,
                &declaration_ids,
                &bindings,
                &scopes,
                root,
                &typescript_project_modules,
            );
        for (name, count, limit) in [
            ("declarations", declarations.len(), limits.declarations),
            ("bindings", bindings.len(), limits.bindings),
            ("occurrences", occurrences.len(), limits.occurrences),
            ("candidates", candidates.len(), limits.candidates),
        ] {
            if count > limit {
                return Err(format!(
                    "universal {name} count {count} exceeds limit {limit}"
                ));
            }
        }
        let (by_qualified, (by_module_name, (by_scope_name, by_source_directory_name))) =
            rayon::join(
                || {
                    let mut index = AHashMap::<(String, String), Vec<DeclarationSlot>>::new();
                    for declaration in declarations.values() {
                        let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                            continue;
                        };
                        index
                            .entry((
                                declaration.language.clone(),
                                declaration.qualified_name.clone(),
                            ))
                            .or_default()
                            .push(slot);
                    }
                    sort_declaration_index(
                        &mut index,
                        &declaration_ids,
                        limits.candidates_per_lookup,
                    );
                    index
                },
                || {
                    let (by_module_name, (by_scope_name, by_source_directory_name)) = rayon::join(
                        || {
                            let mut index =
                                AHashMap::<(String, String, String), Vec<DeclarationSlot>>::new();
                            for declaration in declarations.values() {
                                // Go methods live in the receiver method set, not in
                                // the package block. Keeping them in the package-name
                                // index makes an unqualified call ambiguous whenever a
                                // package function shares the method name (for example,
                                // a method forwarding to its package-level helper).
                                if declaration.language == "go" && declaration.kind == "method" {
                                    continue;
                                }
                                let Some(slot) =
                                    declaration_slot(&declaration_ids, &declaration.id)
                                else {
                                    continue;
                                };
                                let Some(module) = declaration.module_or_package.as_ref() else {
                                    continue;
                                };
                                index
                                    .entry((
                                        declaration.language.clone(),
                                        module.clone(),
                                        declaration.name.clone(),
                                    ))
                                    .or_default()
                                    .push(slot);
                            }
                            sort_declaration_index(
                                &mut index,
                                &declaration_ids,
                                limits.candidates_per_lookup,
                            );
                            index
                        },
                        || {
                            let (by_scope_name, by_source_directory_name) = rayon::join(
                                || {
                                    let mut index = AHashMap::<
                                        (String, String, String),
                                        Vec<DeclarationSlot>,
                                    >::new();
                                    for declaration in declarations.values() {
                                        // Go methods are selected through a receiver or
                                        // method expression; they are not lexical names
                                        // in the package/file scope.
                                        if declaration.language == "go"
                                            && declaration.kind == "method"
                                        {
                                            continue;
                                        }
                                        let Some(slot) =
                                            declaration_slot(&declaration_ids, &declaration.id)
                                        else {
                                            continue;
                                        };
                                        let Some(scope) = declaration.scope_id.as_ref() else {
                                            continue;
                                        };
                                        index
                                            .entry((
                                                declaration.language.clone(),
                                                scope.clone(),
                                                declaration.name.clone(),
                                            ))
                                            .or_default()
                                            .push(slot);
                                    }
                                    sort_declaration_index(
                                        &mut index,
                                        &declaration_ids,
                                        limits.candidates_per_lookup,
                                    );
                                    index
                                },
                                || {
                                    let mut index = AHashMap::<
                                        (String, String, String),
                                        Vec<DeclarationSlot>,
                                    >::new();
                                    for declaration in declarations.values() {
                                        let Some(slot) =
                                            declaration_slot(&declaration_ids, &declaration.id)
                                        else {
                                            continue;
                                        };
                                        let Some(directory) =
                                            source_directory(&declaration.range.source_file, root)
                                        else {
                                            continue;
                                        };
                                        index
                                            .entry((
                                                declaration.language.clone(),
                                                directory,
                                                declaration.name.clone(),
                                            ))
                                            .or_default()
                                            .push(slot);
                                    }
                                    sort_declaration_index(
                                        &mut index,
                                        &declaration_ids,
                                        limits.candidates_per_lookup,
                                    );
                                    index
                                },
                            );
                            (by_scope_name, by_source_directory_name)
                        },
                    );
                    (by_module_name, (by_scope_name, by_source_directory_name))
                },
            );
        profile_internal("universal declaration indices", &mut profile_started);
        let mut inventory_by_qualified = AHashMap::<_, Vec<_>>::new();
        for node in inventory_nodes {
            if node.string("symbol_kind") != "file" || node.string("source_file").is_empty() {
                continue;
            }
            let language = node.string("language");
            let qualified = match language.as_str() {
                "python" => python_module_name(&node.string("source_file"), root),
                "go" => {
                    let package = node.string("package");
                    (!package.is_empty()).then_some(package)
                }
                _ => None,
            };
            if let Some(qualified) = qualified {
                inventory_by_qualified
                    .entry((language, qualified))
                    .or_default()
                    .push(node.id.clone());
            }
        }
        for values in inventory_by_qualified.values_mut() {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        profile_internal("universal source inventory index", &mut profile_started);
        let (aliases, direct_bases) = rayon::join(
            || {
                let mut aliases = AHashMap::<_, Vec<_>>::new();
                for binding in bindings.values() {
                    let Some(owner) = binding
                        .scope_id
                        .as_deref()
                        .and_then(|id| scopes.get(id))
                        .and_then(|scope| scope.owner_declaration_id.as_deref())
                        .and_then(|id| declarations.get(id))
                    else {
                        continue;
                    };
                    for separator in [".", "::"] {
                        aliases
                            .entry((
                                binding.language.clone(),
                                format!("{}{separator}{}", owner.qualified_name, binding.spelling),
                            ))
                            .or_default()
                            .push(binding.qualified_target.clone());
                    }
                }
                for targets in aliases.values_mut() {
                    targets.sort_unstable();
                    targets.dedup();
                }
                aliases
            },
            || {
                let mut direct_bases = AHashMap::<(String, String), DirectBaseSet>::new();
                for candidate in candidates.values() {
                    let Some(owner) = declarations.get(&candidate.source_declaration_id) else {
                        continue;
                    };
                    let base_set_complete = match candidate.constraints.hierarchy.as_ref() {
                        Some(HierarchyConstraint::DirectBase { base_set_complete }) => {
                            *base_set_complete
                        }
                        None if candidate.language == "java"
                            && matches!(
                                candidate.relation,
                                CandidateRelation::Extends | CandidateRelation::Implements
                            ) =>
                        {
                            owner.direct_bases_complete
                        }
                        _ => continue,
                    };
                    let range = candidate
                        .occurrence_id
                        .as_deref()
                        .and_then(|id| occurrences.get(id))
                        .map(|occurrence| &occurrence.range);
                    let entry = direct_bases
                        .entry((candidate.language.clone(), owner.qualified_name.clone()))
                        .or_insert_with(|| DirectBaseSet {
                            links: Vec::new(),
                            complete: true,
                        });
                    entry.complete &= base_set_complete;
                    if entry.links.len() <= limits.candidates_per_lookup {
                        entry.links.push(DirectBaseLink {
                            qualified_name: candidate.constraints.qualified_name.clone(),
                            source_file: range
                                .map_or_else(String::new, |range| range.source_file.clone()),
                            start_byte: range.map_or(u64::MAX, |range| range.start_byte),
                            end_byte: range.map_or(u64::MAX, |range| range.end_byte),
                            candidate_id: candidate.id.clone(),
                        });
                    } else {
                        entry.complete = false;
                    }
                }
                for bases in direct_bases.values_mut() {
                    bases.links.sort_unstable_by(|left, right| {
                        left.source_file
                            .cmp(&right.source_file)
                            .then_with(|| left.start_byte.cmp(&right.start_byte))
                            .then_with(|| left.end_byte.cmp(&right.end_byte))
                            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
                    });
                    if bases.links.len() > limits.candidates_per_lookup {
                        bases.complete = false;
                        bases.links.truncate(limits.candidates_per_lookup);
                    }
                }
                direct_bases
            },
        );
        profile_internal(
            "universal alias and hierarchy indices",
            &mut profile_started,
        );
        let mut direct_subtypes = AHashMap::<(String, String), DirectSubtypeSet>::new();
        for ((language, subtype), bases) in &direct_bases {
            for link in &bases.links {
                let Some(base) = link.qualified_name.as_ref() else {
                    continue;
                };
                let entry = direct_subtypes
                    .entry((language.clone(), base.clone()))
                    .or_insert_with(|| DirectSubtypeSet {
                        types: Vec::new(),
                        complete: true,
                    });
                if entry.types.len() <= limits.candidates_per_lookup {
                    entry.types.push(subtype.clone());
                } else {
                    entry.complete = false;
                }
            }
        }
        for subtypes in direct_subtypes.values_mut() {
            subtypes.types.sort_unstable();
            subtypes.types.dedup();
            if subtypes.types.len() > limits.candidates_per_lookup {
                subtypes.complete = false;
                subtypes.types.truncate(limits.candidates_per_lookup);
            }
        }
        let mut members_by_owner =
            AHashMap::<(String, String, String), Vec<DeclarationSlot>>::new();
        for declaration in declarations.values() {
            let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                continue;
            };
            let Some(owner) = declaration
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members_by_owner
                .entry((
                    declaration.language.clone(),
                    owner.qualified_name.clone(),
                    declaration.name.clone(),
                ))
                .or_default()
                .push(slot);
        }
        for members in members_by_owner.values_mut() {
            members.sort_unstable_by(|left, right| {
                declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
            });
            members.dedup();
            if members.len() > limits.candidates_per_lookup {
                members.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        let rust_impl_associated_types = rust_impl_associated_type_index(
            &declarations,
            &declaration_ids,
            &scopes,
            &candidates,
            &occurrences,
            limits.candidates_per_lookup,
        );
        let rust_impl_associated_trait_names = rust_impl_associated_trait_name_index(
            &rust_impl_associated_types,
            limits.candidates_per_lookup,
        );
        let rust_impl_traits = rust_impl_trait_index(&candidates, limits.candidates_per_lookup);
        profile_internal("universal hierarchy indices", &mut profile_started);
        let mut wildcard_bindings_by_scope = AHashMap::<(String, String), WildcardModuleSet>::new();
        let mut wildcard_bindings_by_module =
            AHashMap::<(String, String), WildcardModuleSet>::new();
        let mut wildcard_reexports_by_module =
            AHashMap::<(String, String), WildcardModuleSet>::new();
        for binding in bindings.values().filter(|binding| binding.spelling == "*") {
            let Some(scope_id) = binding.scope_id.as_ref() else {
                continue;
            };
            let entry = wildcard_bindings_by_scope
                .entry((binding.language.clone(), scope_id.clone()))
                .or_insert_with(|| WildcardModuleSet {
                    modules: Vec::new(),
                    complete: true,
                });
            let target_is_internal = scopes
                .get(scope_id)
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
                .is_some_and(|owner| {
                    qualified_root(&owner.qualified_name)
                        == qualified_root(&binding.qualified_target)
                })
                || (binding.language == "rust"
                    && rust_source_wildcard_targets.contains(&binding.qualified_target));
            entry.complete &= target_is_internal;
            entry.modules.push(binding.qualified_target.clone());
            if let Some(owner) = scopes
                .get(scope_id)
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            {
                let module_entry = wildcard_bindings_by_module
                    .entry((binding.language.clone(), owner.qualified_name.clone()))
                    .or_insert_with(|| WildcardModuleSet {
                        modules: Vec::new(),
                        complete: true,
                    });
                module_entry.complete &= target_is_internal;
                module_entry.modules.push(binding.qualified_target.clone());
            }
            if binding.kind == compass_languages::BindingKind::Reexport
                && let Some(owner) = scopes
                    .get(scope_id)
                    .and_then(|scope| scope.owner_declaration_id.as_deref())
                    .and_then(|id| declarations.get(id))
            {
                wildcard_reexports_by_module
                    .entry((binding.language.clone(), owner.qualified_name.clone()))
                    .or_insert_with(|| WildcardModuleSet {
                        modules: Vec::new(),
                        complete: true,
                    })
                    .modules
                    .push(binding.qualified_target.clone());
            }
        }
        for bindings in wildcard_bindings_by_scope.values_mut() {
            bindings.modules.sort_unstable();
            bindings.modules.dedup();
            if bindings.modules.len() > limits.candidates_per_lookup {
                bindings.complete = false;
                bindings.modules.truncate(limits.candidates_per_lookup);
            }
        }
        for bindings in wildcard_bindings_by_module.values_mut() {
            bindings.modules.sort_unstable();
            bindings.modules.dedup();
            if bindings.modules.len() > limits.candidates_per_lookup {
                bindings.complete = false;
                bindings.modules.truncate(limits.candidates_per_lookup);
            }
        }
        for reexports in wildcard_reexports_by_module.values_mut() {
            reexports.modules.sort_unstable();
            reexports.modules.dedup();
            if reexports.modules.len() > limits.candidates_per_lookup {
                reexports.complete = false;
                reexports.modules.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal wildcard index", &mut profile_started);
        let mut members = AHashMap::<_, Vec<_>>::new();
        for binding in bindings
            .values()
            .filter(|binding| binding.kind == compass_languages::BindingKind::Member)
        {
            let Some(owner) = binding
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members
                .entry((
                    binding.language.clone(),
                    owner.qualified_name.clone(),
                    binding.spelling.clone(),
                ))
                .or_default()
                .push(binding.qualified_target.clone());
        }
        for targets in members.values_mut() {
            targets.sort_unstable();
            targets.dedup();
        }
        let mut return_entries = AHashMap::<_, Vec<_>>::new();
        let mut outer_return_entries = AHashMap::<_, Vec<_>>::new();
        for candidate in candidates
            .values()
            .filter(|candidate| candidate.relation == CandidateRelation::Returns)
        {
            let Some(callable) = declarations.get(&candidate.source_declaration_id) else {
                continue;
            };
            if candidate.constraints.qualified_name.is_none() {
                continue;
            }
            let start_byte = candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .map_or(u64::MAX, |occurrence| occurrence.range.start_byte);
            return_entries
                .entry((candidate.language.clone(), callable.qualified_name.clone()))
                .or_default()
                .push((start_byte, candidate.id.clone()));
            if candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .and_then(|occurrence| occurrence.context.as_deref())
                == Some("rust-outer-nominal-return")
            {
                outer_return_entries
                    .entry((candidate.language.clone(), callable.qualified_name.clone()))
                    .or_default()
                    .push((start_byte, candidate.id.clone()));
            }
        }
        let return_candidates_by_callable = return_entries
            .into_iter()
            .map(|(key, mut entries)| {
                entries.sort_unstable();
                entries.truncate(candidate_storage_limit(limits.candidates_per_lookup));
                (
                    key,
                    entries
                        .into_iter()
                        .map(|(_, candidate)| candidate)
                        .collect(),
                )
            })
            .collect();
        let outer_return_candidates_by_callable = outer_return_entries
            .into_iter()
            .map(|(key, mut entries)| {
                entries.sort_unstable();
                entries.truncate(limits.candidates_per_lookup);
                (
                    key,
                    entries
                        .into_iter()
                        .map(|(_, candidate)| candidate)
                        .collect(),
                )
            })
            .collect();
        profile_internal("universal member index", &mut profile_started);
        Ok(Self {
            declarations,
            declaration_ids,
            occurrences,
            bindings,
            candidates,
            scopes,
            definition_ranges,
            by_qualified,
            by_module_name,
            by_scope_name,
            by_source_directory_name,
            typescript_modules,
            typescript_export_aliases,
            typescript_reexport_targets,
            typescript_project_modules,
            root: root.to_path_buf(),
            direct_bases,
            direct_subtypes,
            members_by_owner,
            rust_impl_associated_types,
            rust_impl_associated_trait_names,
            rust_impl_traits,
            inventory_by_qualified,
            aliases,
            rust_source_wildcard_targets,
            wildcard_bindings_by_scope,
            wildcard_bindings_by_module,
            wildcard_reexports_by_module,
            members,
            return_candidates_by_callable,
            outer_return_candidates_by_callable,
            go_module_path,
            limits,
        })
    }

    #[must_use]
    pub fn candidate_ids(&self) -> Vec<&str> {
        let mut ordered = self
            .candidates
            .iter()
            .map(|(id, candidate)| {
                let range = self
                    .occurrence(candidate)
                    .map(|occurrence| &occurrence.range)
                    .or_else(|| {
                        self.declarations
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

    fn declaration_id(&self, slot: DeclarationSlot) -> Option<&str> {
        self.declaration_ids
            .get(usize::try_from(slot).ok()?)
            .map(String::as_str)
    }

    fn declaration(&self, slot: DeclarationSlot) -> Option<&DeclarationFact> {
        self.declaration_id(slot)
            .and_then(|id| self.declarations.get(id))
    }

    fn declaration_allowed_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        self.declaration_id(slot)
            .is_some_and(|id| self.declaration_allowed(id, candidate))
    }

    #[must_use]
    pub fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        let Some(candidate) = self.candidates.get(candidate_id) else {
            return ResolutionDecision::Unresolved;
        };
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        if let Some(target) = candidate.constraints.exact_target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceDeclaration,
                    candidate_count: 1,
                },
            };
        }
        if let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_c3_receiver_dispatch(
                language,
                receiver_qualified_name,
                *strategy,
                candidate,
            );
        }
        if let Some(HierarchyConstraint::RustAssociatedType {
            receiver_declaration_id,
            receiver_qualified_name,
            trait_qualified_name,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_rust_associated_type(
                language,
                receiver_declaration_id,
                receiver_qualified_name,
                trait_qualified_name,
                candidate,
            );
        }
        if let Some(decision) = self.resolve_typescript_import_candidate(language, candidate) {
            return decision;
        }
        if let Some(decision) = self.resolve_explicit_binding(language, candidate) {
            return decision;
        }
        let fallback_candidate = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            .filter(|binding| binding.kind == compass_languages::BindingKind::CallResult)
            .map(|binding| {
                let mut fallback = candidate.clone();
                fallback.binding_id.clone_from(&binding.fallback_binding_id);
                fallback
            });
        let candidate = fallback_candidate.as_ref().unwrap_or(candidate);
        let occurrence = self.occurrence(candidate);
        let qualifier = occurrence.and_then(|occurrence| occurrence.qualifier.as_deref());
        let has_unbound_qualified_receiver = qualifier.is_some_and(|qualifier| {
            candidate.binding_id.is_none()
                && !matches!((language, qualifier), ("python", "self" | "cls"))
        });
        let allows_lexical_lookup = qualifier.is_none()
            || qualifier.is_some_and(|qualifier| {
                candidate.binding_id.is_none()
                    && matches!((language, qualifier), ("python", "self" | "cls"))
            });
        if matches!(
            candidate.constraints.hierarchy.as_ref(),
            Some(HierarchyConstraint::DirectBase { .. })
        ) {
            return self.resolve_direct_base(language, candidate);
        }

        if matches!(
            candidate.relation,
            CandidateRelation::Contains | CandidateRelation::Owns
        ) && let Some(qualified) = candidate.constraints.qualified_name.as_ref()
        {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            if let Some(decision) = self.unique_decision(
                self.by_qualified
                    .get(&(language.to_owned(), qualified.clone())),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
        }

        if allows_lexical_lookup && let Some(scope) = candidate.constraints.scope_id.as_deref() {
            let mut cursor = Some(scope);
            let mut visited = BTreeSet::new();
            while let Some(scope) = cursor.filter(|scope| visited.insert((*scope).to_owned())) {
                let key = (
                    language.to_owned(),
                    scope.to_owned(),
                    candidate.target_spelling.clone(),
                );
                if let Some(decision) = self.unique_decision(
                    self.by_scope_name.get(&key),
                    candidate,
                    ResolutionRule::ExactLexicalDeclaration,
                ) {
                    return decision;
                }
                cursor = self
                    .scopes
                    .get(scope)
                    .and_then(|scope| scope.parent_scope_id.as_deref());
            }
        }

        if let Some(qualified) = candidate.constraints.qualified_name.as_ref() {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            let key = (language.to_owned(), qualified.clone());
            if let Some(decision) = self.unique_decision(
                self.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
            if let Some(decision) = self.member_decision(language, &qualified, candidate) {
                return decision;
            }
            if let Some(decision) = self.wildcard_reexport_decision(language, &qualified, candidate)
            {
                return decision;
            }
            if let Some(decision) = self.imported_member_decision(language, &qualified, candidate) {
                return decision;
            }
            if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
                return decision;
            }
        }

        if !has_unbound_qualified_receiver
            && let Some(module) = candidate.constraints.module_or_package.as_ref()
        {
            let key = (
                language.to_owned(),
                module.clone(),
                candidate.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.by_module_name.get(&key),
                candidate,
                ResolutionRule::UniqueModuleOrPackage,
            ) {
                return decision;
            }
        }

        if let Some(decision) = self.resolve_wildcard_binding(language, candidate) {
            return decision;
        }
        if let Some(decision) = self.resolve_visible_wildcard_bindings(language, candidate) {
            return decision;
        }

        if candidate.constraints.allow_external
            && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
        {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        if matches!(
            candidate.relation,
            CandidateRelation::Calls | CandidateRelation::IndirectCalls | CandidateRelation::Tests
        ) && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
            && qualifier.is_some_and(is_deferred_receiver)
        {
            return ResolutionDecision::DeferredReceiver {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DeferredReceiver,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }

    fn resolve_typescript_import_candidate(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if !matches!(language, "typescript" | "javascript") {
            return None;
        }
        let occurrence = self.occurrence(candidate)?;
        let mut module_and_export = None;
        if let Some(binding) = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            && matches!(
                binding.kind,
                compass_languages::BindingKind::Import
                    | compass_languages::BindingKind::ImportAlias
                    | compass_languages::BindingKind::Reexport
            )
            && let Some((module, exported)) = binding.qualified_target.rsplit_once("::")
            && !module.is_empty()
            && !exported.is_empty()
        {
            module_and_export = Some((module.to_owned(), exported.to_owned()));
        } else if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) && let Some(qualified) = candidate.constraints.qualified_name.as_deref()
            && let Some((module, exported)) = qualified.rsplit_once("::")
            && !module.is_empty()
            && !exported.is_empty()
        {
            module_and_export = Some((module.to_owned(), exported.to_owned()));
        }

        let (module, exported) = if let Some(module_and_export) = module_and_export {
            module_and_export
        } else if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) {
            let module = candidate
                .constraints
                .module_or_package
                .as_deref()
                .filter(|module| !module.is_empty())?
                .to_owned();
            (module, "module".to_owned())
        } else {
            return None;
        };
        let (keys, project_resolved) = self
            .typescript_project_modules
            .get(&(
                typescript_project_importer_key(&occurrence.range.source_file, &self.root)?,
                module.clone(),
            ))
            .cloned()
            .map_or_else(
                || {
                    (
                        typescript_import_module_keys(
                            &occurrence.range.source_file,
                            &module,
                            &self.root,
                        ),
                        false,
                    )
                },
                |keys| (keys, true),
            );
        if keys.is_empty() {
            return None;
        }
        let member_path = if matches!(
            candidate.relation,
            CandidateRelation::Calls
                | CandidateRelation::IndirectCalls
                | CandidateRelation::AccessesMember
        ) {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .and_then(|qualified| typescript_member_path(qualified, &candidate.target_spelling))
        } else {
            None
        };
        let member_owner_export = member_path
            .as_ref()
            .filter(|path| path.members.len() == 1)
            .map(|path| path.root_export.clone());
        let is_member_candidate = member_path.is_some();
        let mut targets = BTreeSet::new();
        for key in keys {
            if let Some(path) = member_path.as_ref()
                && (path.members.len() > 1 || !path.type_arguments.is_empty())
            {
                targets.extend(self.typescript_member_chain_slots(language, &key, path, candidate));
                continue;
            }
            let owner_export = member_owner_export
                .as_deref()
                .unwrap_or(&exported)
                .to_owned();
            // A member candidate constrains the final target to
            // callable/value kinds, but the exported owner can be a
            // type-only declaration such as an interface. Widen only this
            // internal owner lookup; member filtering below still uses the
            // original candidate, so an interface itself is never published
            // as the callable/access target.
            let owner_targets = self.typescript_export_slots(
                language,
                &key,
                &owner_export,
                candidate,
                member_owner_export.is_some(),
            );
            if member_owner_export.is_some() {
                for owner in owner_targets {
                    let Some(owner) = self.declaration(owner) else {
                        continue;
                    };
                    if let Some(members) = self.members_by_owner.get(&(
                        owner.language.clone(),
                        owner.qualified_name.clone(),
                        candidate.target_spelling.clone(),
                    )) {
                        targets.extend(
                            members
                                .iter()
                                .filter(|slot| {
                                    self.typescript_declaration_allowed_slot(**slot, candidate)
                                })
                                .copied(),
                        );
                    }
                }
            } else {
                targets.extend(owner_targets);
            }
        }
        let targets = targets.into_iter().collect::<Vec<_>>();
        self.unique_typescript_decision(
            Some(&targets),
            candidate,
            if is_member_candidate {
                ResolutionRule::MemberBinding
            } else if project_resolved {
                ResolutionRule::ProjectModuleBinding
            } else {
                ResolutionRule::ExplicitBinding
            },
        )
    }

    /// Resolve a bounded TypeScript member-value chain such as
    /// `Box<Item>.item.inspect`. The root export and generic arguments come
    /// from the import binding; each intermediate member must publish a
    /// direct type signature, and every hop must resolve uniquely. Missing,
    /// structural, or competing type evidence remains unresolved.
    fn typescript_member_chain_slots(
        &self,
        language: &str,
        module: &str,
        path: &TypeScriptMemberPath,
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<DeclarationSlot> {
        const MAX_MEMBER_CHAIN_DEPTH: usize = 16;
        if path.members.is_empty() || path.members.len() > MAX_MEMBER_CHAIN_DEPTH {
            return BTreeSet::new();
        }
        let root_targets =
            self.typescript_export_slots(language, module, &path.root_export, candidate, true);
        let mut targets = BTreeSet::new();
        for root_slot in root_targets {
            let Some(root) = self.declaration(root_slot) else {
                continue;
            };
            let generic_parameters = root
                .signature
                .as_deref()
                .map(typescript_generic_parameter_names)
                .unwrap_or_default();
            let mut owners = BTreeSet::from([root_slot]);
            for (index, member_name) in path.members.iter().enumerate() {
                let final_member = index.saturating_add(1) == path.members.len();
                let mut next_owners = BTreeSet::new();
                for owner_slot in owners.iter().copied() {
                    let Some(owner) = self.declaration(owner_slot) else {
                        continue;
                    };
                    let Some(members) = self.members_by_owner.get(&(
                        owner.language.clone(),
                        owner.qualified_name.clone(),
                        member_name.clone(),
                    )) else {
                        continue;
                    };
                    if final_member {
                        next_owners.extend(
                            members
                                .iter()
                                .filter(|slot| {
                                    self.typescript_declaration_allowed_slot(**slot, candidate)
                                })
                                .copied(),
                        );
                    } else {
                        let typed_members = members
                            .iter()
                            .filter_map(|slot| self.declaration(*slot))
                            .filter(|member| member.kind == "property")
                            .filter_map(|member| member.signature.as_deref())
                            .collect::<Vec<_>>();
                        let [type_name] = typed_members.as_slice() else {
                            continue;
                        };
                        next_owners.extend(self.typescript_member_type_slots(
                            language,
                            module,
                            type_name,
                            &generic_parameters,
                            &path.type_arguments,
                            candidate,
                        ));
                    }
                }
                if next_owners.is_empty() {
                    owners.clear();
                    break;
                }
                owners = next_owners;
            }
            targets.extend(owners);
            if targets.len() > self.limits.candidates_per_lookup {
                break;
            }
        }
        targets
    }

    fn typescript_member_type_slots(
        &self,
        language: &str,
        module: &str,
        type_name: &str,
        generic_parameters: &[String],
        type_arguments: &[String],
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<DeclarationSlot> {
        let type_name = type_name.trim();
        if type_name.is_empty() || type_name.len() > 1024 {
            return BTreeSet::new();
        }
        let substituted = generic_parameters
            .iter()
            .position(|parameter| parameter == type_name)
            .and_then(|index| type_arguments.get(index))
            .map_or_else(|| type_name.to_owned(), |argument| argument.clone());
        let (base, _) = typescript_generic_type_parts(&substituted)
            .unwrap_or((substituted.as_str(), Vec::new()));
        let (qualified_module, exported) = split_typescript_module_qualified(base);
        if exported.is_empty() {
            return BTreeSet::new();
        }
        let mut modules = Vec::new();
        if let Some(qualified_module) = qualified_module {
            modules.push(qualified_module.to_owned());
            modules.extend(typescript_import_module_keys(
                module,
                qualified_module,
                &self.root,
            ));
        } else {
            modules.push(module.to_owned());
        }
        modules.sort_unstable();
        modules.dedup();
        let mut slots = BTreeSet::new();
        for module in modules {
            slots
                .extend(self.typescript_export_slots(language, &module, exported, candidate, true));
            if slots.len() > self.limits.candidates_per_lookup {
                break;
            }
        }
        slots
    }

    fn typescript_export_slots(
        &self,
        language: &str,
        module: &str,
        exported: &str,
        candidate: &RelationshipCandidate,
        allow_type_owner: bool,
    ) -> BTreeSet<DeclarationSlot> {
        const MAX_TYPESCRIPT_REEXPORT_DEPTH: usize = 64;
        let mut walk = TypeScriptExportWalk {
            candidate,
            allow_type_owner,
            visiting: BTreeSet::new(),
            slots: BTreeSet::new(),
        };
        self.collect_typescript_export_slots(
            language,
            module,
            exported,
            0,
            MAX_TYPESCRIPT_REEXPORT_DEPTH,
            &mut walk,
        );
        walk.slots
    }

    fn collect_typescript_export_slots(
        &self,
        language: &str,
        module: &str,
        exported: &str,
        depth: usize,
        max_depth: usize,
        walk: &mut TypeScriptExportWalk<'_>,
    ) {
        if depth >= max_depth
            || walk.slots.len() > self.limits.candidates_per_lookup
            || !walk
                .visiting
                .insert((language.to_owned(), module.to_owned(), exported.to_owned()))
        {
            return;
        }
        for target_language in typescript_language_family(language) {
            let target_language = *target_language;
            for index in [&self.typescript_modules, &self.typescript_export_aliases] {
                if let Some(values) = index.get(&(
                    target_language.to_owned(),
                    module.to_owned(),
                    exported.to_owned(),
                )) {
                    let remaining = candidate_storage_limit(self.limits.candidates_per_lookup)
                        .saturating_sub(walk.slots.len());
                    if remaining > 0 {
                        walk.slots.extend(
                            values
                                .iter()
                                .filter(|slot| {
                                    if walk.allow_type_owner {
                                        self.typescript_declaration_allowed_owner_slot(
                                            **slot,
                                            walk.candidate,
                                        )
                                    } else {
                                        self.typescript_declaration_allowed_slot(
                                            **slot,
                                            walk.candidate,
                                        )
                                    }
                                })
                                .take(remaining)
                                .copied(),
                        );
                    }
                }
            }
            if exported != "default"
                && let Some(values) = self.typescript_reexport_targets.get(&(
                    target_language.to_owned(),
                    module.to_owned(),
                    "*".to_owned(),
                ))
            {
                for target in values {
                    self.collect_typescript_export_slots(
                        target_language,
                        &target.module,
                        exported,
                        depth.saturating_add(1),
                        max_depth,
                        walk,
                    );
                }
            }
            if let Some(values) = self.typescript_reexport_targets.get(&(
                target_language.to_owned(),
                module.to_owned(),
                exported.to_owned(),
            )) {
                for target in values {
                    self.collect_typescript_export_slots(
                        target_language,
                        &target.module,
                        &target.exported,
                        depth.saturating_add(1),
                        max_depth,
                        walk,
                    );
                }
            }
        }
        walk.visiting
            .remove(&(language.to_owned(), module.to_owned(), exported.to_owned()));
    }

    fn resolve_wildcard_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) {
            return None;
        }
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            .filter(|binding| binding.spelling == "*")?;
        let qualifier = self
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        let declarations = match self.wildcard_declarations(
            language,
            std::slice::from_ref(&binding.qualified_target),
            qualifier,
            candidate,
        ) {
            Ok(declarations) => declarations,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        match declarations.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::WildcardBinding,
                    candidate_count: 1,
                },
            }),
            [] if !self.binding_target_is_internal(binding)
                && (language != "rust"
                    || rust_external_wildcard_target_is_explicit(qualifier, candidate)) =>
            {
                Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: wildcard_qualified_names(
                        language,
                        &binding.qualified_target,
                        qualifier,
                        &candidate.target_spelling,
                    )
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| candidate.target_spelling.clone()),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                })
            }
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn resolve_visible_wildcard_bindings(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let qualifier = self
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        if language != "rust"
            || candidate.binding_id.is_some()
            || matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            )
            || qualifier.is_some_and(|qualifier| {
                !rust_external_wildcard_target_is_explicit(Some(qualifier), candidate)
            })
        {
            return None;
        }
        let mut scope_id = candidate.constraints.scope_id.as_deref();
        let mut visited_scopes = BTreeSet::new();
        while let Some(current) = scope_id.filter(|scope| visited_scopes.insert(*scope)) {
            if visited_scopes.len() > self.limits.candidates_per_lookup {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: visited_scopes.len(),
                });
            }
            if let Some(bindings) = self
                .wildcard_bindings_by_scope
                .get(&(language.to_owned(), current.to_owned()))
            {
                if !bindings.complete {
                    return Some(ResolutionDecision::Ambiguous {
                        candidate_count: bindings.modules.len().saturating_add(1),
                    });
                }
                let declarations = match self.wildcard_declarations(
                    language,
                    &bindings.modules,
                    qualifier,
                    candidate,
                ) {
                    Ok(declarations) => declarations,
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                };
                match declarations.into_iter().collect::<Vec<_>>().as_slice() {
                    [only] => {
                        return Some(ResolutionDecision::Resolved {
                            declaration_id: self.declaration_id(*only)?.to_owned(),
                            evidence: ResolutionEvidence {
                                rule: ResolutionRule::WildcardBinding,
                                candidate_count: 1,
                            },
                        });
                    }
                    [] => {}
                    many => {
                        return Some(ResolutionDecision::Ambiguous {
                            candidate_count: many.len(),
                        });
                    }
                }
            }
            scope_id = self
                .scopes
                .get(current)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        None
    }

    fn wildcard_declarations(
        &self,
        language: &str,
        initial_modules: &[String],
        qualifier: Option<&str>,
        candidate: &RelationshipCandidate,
    ) -> Result<BTreeSet<DeclarationSlot>, usize> {
        let mut modules = initial_modules.to_vec();
        let mut visited = BTreeSet::new();
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        while let Some(module) = modules.pop() {
            if !visited.insert(module.clone()) {
                continue;
            }
            if visited.len() > self.limits.candidates_per_lookup {
                return Err(visited.len());
            }
            if qualifier.is_none()
                && let Some(ids) = self.by_module_name.get(&(
                    language.to_owned(),
                    module.clone(),
                    candidate.target_spelling.clone(),
                ))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
            for raw_qualified in
                wildcard_qualified_names(language, &module, qualifier, &candidate.target_spelling)
            {
                let qualified = self.follow_alias(language, &raw_qualified)?;
                let mut qualified_declarations = BTreeSet::new();
                if let Some(ids) = self
                    .by_qualified
                    .get(&(language.to_owned(), qualified.clone()))
                {
                    qualified_declarations.extend(
                        ids.iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .cloned(),
                    );
                }
                if let Some(ids) = self.member_declarations(language, &qualified, candidate) {
                    qualified_declarations.extend(ids);
                }
                if qualified_declarations.is_empty()
                    && qualifier.is_some_and(|qualifier| {
                        rust_external_wildcard_target_is_explicit(Some(qualifier), candidate)
                    })
                    && qualified == raw_qualified
                {
                    let expanded =
                        self.follow_wildcard_qualified_alias(language, &raw_qualified)?;
                    if expanded != qualified {
                        if let Some(ids) = self
                            .by_qualified
                            .get(&(language.to_owned(), expanded.clone()))
                        {
                            qualified_declarations.extend(
                                ids.iter()
                                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                                    .cloned(),
                            );
                        }
                        if let Some(ids) = self.member_declarations(language, &expanded, candidate)
                        {
                            qualified_declarations.extend(ids);
                        }
                    }
                }
                declarations.extend(qualified_declarations);
            }
            if declarations.len() > self.limits.candidates_per_lookup {
                return Err(declarations.len());
            }
            if let Some(reexports) = self
                .wildcard_reexports_by_module
                .get(&(language.to_owned(), module.clone()))
            {
                if !reexports.complete {
                    return Err(reexports.modules.len().saturating_add(1));
                }
                modules.extend(reexports.modules.iter().cloned());
            }
            if language == "rust"
                && candidate
                    .constraints
                    .module_or_package
                    .as_deref()
                    .is_some_and(|source_module| rust_module_is_descendant(source_module, &module))
                && let Some(bindings) = self
                    .wildcard_bindings_by_module
                    .get(&(language.to_owned(), module))
            {
                if !bindings.complete {
                    return Err(bindings.modules.len().saturating_add(1));
                }
                modules.extend(bindings.modules.iter().cloned());
            }
        }
        Ok(declarations)
    }

    fn binding_target_is_internal(&self, binding: &BindingFact) -> bool {
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| self.scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| self.declarations.get(declaration_id))
        else {
            return false;
        };
        qualified_root(&owner.qualified_name) == qualified_root(&binding.qualified_target)
            || (binding.language == "rust"
                && self
                    .rust_source_wildcard_targets
                    .contains(&binding.qualified_target))
    }

    fn resolve_explicit_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))?;
        // Wildcards are a search scope, not an exact spelling. Let lexical and
        // module resolution run before the dedicated wildcard stage below.
        if binding.spelling == "*" {
            return None;
        }
        let qualified_occurrence = self
            .occurrence(candidate)
            .is_some_and(|occurrence| occurrence.qualifier.is_some());
        if !qualified_occurrence
            && let Some(target) = binding.target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return Some(ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExplicitBinding,
                    candidate_count: 1,
                },
            });
        }
        match self.bound_member_target(language, binding, candidate) {
            Ok(Some(qualified)) => {
                let key = (language.to_owned(), qualified.clone());
                if let Some(decision) = self.unique_decision(
                    self.by_qualified.get(&key),
                    candidate,
                    ResolutionRule::MemberBinding,
                ) {
                    return Some(decision);
                }
                if let Some(decision) = self.member_decision(language, &qualified, candidate) {
                    return Some(decision);
                }
                match self.rust_trait_member_declarations(language, &qualified, candidate) {
                    Ok(declarations) => {
                        if let Some(decision) = self.unique_decision(
                            Some(&declarations),
                            candidate,
                            ResolutionRule::MemberBinding,
                        ) {
                            return Some(decision);
                        }
                    }
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                }
                if let Some(decision) =
                    self.imported_member_decision(language, &qualified, candidate)
                {
                    return Some(decision);
                }
                if let Some(decision) =
                    self.wildcard_reexport_decision(language, &qualified, candidate)
                {
                    return Some(decision);
                }
                return Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: qualified,
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                });
            }
            Ok(None) if binding.kind == compass_languages::BindingKind::CallResult => {
                if let Some(fallback_binding_id) = binding.fallback_binding_id.as_ref() {
                    let mut fallback = candidate.clone();
                    fallback.binding_id = Some(fallback_binding_id.clone());
                    if let Some(decision) = self.resolve_explicit_binding(language, &fallback)
                        && decision != ResolutionDecision::Unresolved
                    {
                        return Some(decision);
                    }
                }
                // The call-result evidence is an optional refinement. If the
                // project-wide callable or return type is unavailable, keep
                // the candidate on its prior qualified/deferred path instead
                // of suppressing source-valid fallback evidence.
                return None;
            }
            Ok(None) => {}
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        }
        let binding_lookup = if qualified_occurrence
            || matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            ) {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .unwrap_or(&binding.qualified_target)
        } else {
            &binding.qualified_target
        };
        let qualified = match self.follow_alias(language, binding_lookup) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        let key = (language.to_owned(), qualified.clone());
        let qualified_rule = if language == "rust"
            && qualified_occurrence
            && matches!(
                candidate.relation,
                CandidateRelation::Calls
                    | CandidateRelation::IndirectCalls
                    | CandidateRelation::Constructs
                    | CandidateRelation::AccessesMember
            ) {
            ResolutionRule::MemberBinding
        } else {
            ResolutionRule::ExplicitBinding
        };
        if let Some(decision) =
            self.unique_decision(self.by_qualified.get(&key), candidate, qualified_rule)
        {
            return Some(decision);
        }
        if let Some(decision) = self.member_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        if let Some(decision) = self.wildcard_reexport_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        let imported = self.imported_declarations(
            language,
            &binding.qualified_target,
            &candidate.target_spelling,
        );
        (!imported.is_empty())
            .then(|| {
                self.unique_decision(Some(&imported), candidate, ResolutionRule::ExplicitBinding)
            })
            .flatten()
    }

    pub fn materialize(&self, nodes: &mut Vec<NodeRecord>, edges: &mut Vec<EdgeRecord>) {
        let mut profile_started = Instant::now();
        let overloads = declaration_overloads(self.declarations.values());
        let graph_ids = materialized_declaration_ids(self.declarations.values());
        let existing_positions = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<AHashMap<_, _>>();
        let mut existing_nodes = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<AHashSet<_>>();
        let mut declarations = self.declarations.values().collect::<Vec<_>>();
        declarations.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        const DECLARATION_BATCH_SIZE: usize = 8_192;
        for declaration_batch in declarations.chunks(DECLARATION_BATCH_SIZE) {
            let prepared = declaration_batch
                .par_iter()
                .map(|declaration| {
                    let graph_node_id = &graph_ids[&declaration.id];
                    let definition_range = self.definition_ranges.get(&declaration.id);
                    let node = declaration_node(declaration, definition_range, graph_node_id);
                    let discriminator = overloads.get(&declaration.id).cloned();
                    (node, discriminator)
                })
                .collect::<Vec<_>>();
            for (mut node, discriminator) in prepared {
                if let Some(index) = existing_positions.get(&node.id) {
                    nodes[*index].attributes.extend(node.attributes);
                    if let Some(discriminator) = discriminator {
                        nodes[*index].attributes.insert(
                            "overload_discriminator".to_owned(),
                            Value::String(discriminator),
                        );
                    }
                } else if existing_nodes.insert(node.id.clone()) {
                    if let Some(discriminator) = discriminator {
                        node.attributes.insert(
                            "overload_discriminator".to_owned(),
                            Value::String(discriminator),
                        );
                    }
                    nodes.push(node);
                }
            }
        }
        profile_internal("universal declaration projection", &mut profile_started);
        let inventory_kinds = nodes
            .iter()
            .map(|node| (node.id.clone(), node.string("symbol_kind")))
            .collect::<AHashMap<_, _>>();
        let mut external_positions = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.attributes.get("external").and_then(Value::as_bool) == Some(true)
            })
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<AHashMap<_, _>>();
        let candidate_ids = self.candidate_ids();
        profile_internal("universal candidate ordering", &mut profile_started);
        let decisions = candidate_ids
            .into_par_iter()
            .map(|candidate_id| {
                let decision = self.resolve(candidate_id);
                let exact_declaration_id = match &decision {
                    ResolutionDecision::Resolved { declaration_id, .. } => {
                        Some(declaration_id.clone())
                    }
                    _ => None,
                };
                let allow_possible = !matches!(decision, ResolutionDecision::Ambiguous { .. });
                let mut decisions = vec![(candidate_id, decision)];
                if allow_possible {
                    decisions.extend(
                        self.possible_receiver_dispatches(
                            candidate_id,
                            exact_declaration_id.as_deref(),
                        )
                        .into_iter()
                        .map(|(declaration_id, rule)| {
                            (
                                candidate_id,
                                ResolutionDecision::Resolved {
                                    declaration_id,
                                    evidence: ResolutionEvidence {
                                        rule,
                                        candidate_count: 1,
                                    },
                                },
                            )
                        }),
                    );
                }
                decisions
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        profile_internal("universal candidate decisions", &mut profile_started);
        let prepared_targets = decisions
            .into_par_iter()
            .map(|(candidate_id, decision)| match decision {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let target = self.declarations.get(&declaration_id)?;
                    Some(PreparedTarget {
                        candidate_id,
                        target: graph_ids[&target.id].clone(),
                        rule: evidence.rule,
                        target_kind: Some(target.kind.clone()),
                        declaration_id: Some(target.id.clone()),
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::ResolvedInventory {
                    graph_node_id,
                    evidence,
                } => {
                    let kind = inventory_kinds.get(&graph_node_id).cloned();
                    Some(PreparedTarget {
                        candidate_id,
                        target: graph_node_id,
                        rule: evidence.rule,
                        target_kind: kind,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    let candidate = &self.candidates[candidate_id];
                    let id = make_id(&["external", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_id,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: Some(qualified_name),
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::DeferredReceiver {
                    qualified_name,
                    evidence,
                } => {
                    let candidate = &self.candidates[candidate_id];
                    let id = make_id(&["deferred", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_id,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: Some(qualified_name),
                    })
                }
                ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut resolved_targets = Vec::with_capacity(prepared_targets.len());
        for mut prepared in prepared_targets {
            let candidate = &self.candidates[prepared.candidate_id];
            if let Some(qualified_name) = prepared.external_qualified_name.take() {
                if let Some(position) = external_positions.get(&prepared.target).copied() {
                    merge_external_node(&mut nodes[position], candidate);
                } else if !existing_nodes.contains(&prepared.target) {
                    let position = nodes.len();
                    nodes.push(external_node(
                        &prepared.target,
                        &qualified_name,
                        &candidate.language,
                        candidate,
                    ));
                    existing_nodes.insert(prepared.target.clone());
                    external_positions.insert(prepared.target.clone(), position);
                }
                let fallback = external_kind(candidate).to_owned();
                prepared.target_kind = Some(
                    external_positions
                        .get(&prepared.target)
                        .map(|position| nodes[*position].string("symbol_kind"))
                        .filter(|kind| !kind.is_empty())
                        .unwrap_or(fallback),
                );
            } else if let Some(qualified_name) = prepared.deferred_qualified_name.take() {
                if existing_nodes.insert(prepared.target.clone()) {
                    nodes.push(deferred_receiver_node(
                        &prepared.target,
                        &qualified_name,
                        &candidate.language,
                        candidate,
                    ));
                }
                prepared.target_kind = Some(external_kind(candidate).to_owned());
            }
            let target_site = prepared
                .declaration_id
                .as_deref()
                .and_then(|id| self.declarations.get(id))
                .map(|declaration| &declaration.range);
            resolved_targets.push((
                prepared.candidate_id,
                prepared.target,
                prepared.rule,
                prepared.target_kind,
                target_site,
            ));
        }
        profile_internal("universal target projection", &mut profile_started);
        let materialized = resolved_targets
            .into_par_iter()
            .filter_map(
                |(candidate_id, target, resolution_rule, target_kind, target_site)| {
                    let candidate = &self.candidates[candidate_id];
                    let source = self
                        .declarations
                        .get(&candidate.source_declaration_id)
                        .map(|declaration| graph_ids[&declaration.id].clone())?;
                    let (source, target) = if candidate.relation == CandidateRelation::Contains {
                        (source, target)
                    } else if self.occurrence(candidate).is_some_and(|occurrence| {
                        occurrence.role == compass_languages::SemanticRole::Receiver
                    }) {
                        (target, source)
                    } else {
                        (source, target)
                    };
                    let exact_target = candidate
                        .constraints
                        .exact_target_declaration_id
                        .as_deref()
                        .and_then(|id| self.declarations.get(id));
                    let relation = if self.occurrence(candidate).is_some_and(|occurrence| {
                        occurrence.role == compass_languages::SemanticRole::Receiver
                    }) {
                        "method"
                    } else if candidate.language == "go"
                        && candidate.relation == CandidateRelation::Calls
                        && target_kind.as_deref().is_some_and(|kind| {
                            matches!(kind, "struct" | "interface" | "type_alias")
                        })
                    {
                        "references"
                    } else {
                        relation_name(candidate.relation)
                    };
                    let site = self
                        .occurrence(candidate)
                        .map(|occurrence| &occurrence.range)
                        .or_else(|| exact_target.map(|target| &target.range))
                        .or_else(|| {
                            matches!(
                                candidate.relation,
                                CandidateRelation::Contains | CandidateRelation::Owns
                            )
                            .then_some(target_site)
                            .flatten()
                        })
                        .or_else(|| {
                            self.declarations
                                .get(&candidate.source_declaration_id)
                                .map(|declaration| &declaration.range)
                        });
                    let site = site?;
                    // Exact resolution and bounded possible dispatches can project
                    // more than one target for a candidate. Downstream publication
                    // performs contract-level semantic edge coalescing.
                    if source == target && relation != "calls" {
                        return None;
                    }
                    Some(materialized_edge(
                        source,
                        target,
                        relation,
                        candidate,
                        self.occurrence(candidate),
                        candidate
                            .binding_id
                            .as_deref()
                            .and_then(|binding_id| self.bindings.get(binding_id)),
                        target_kind.as_deref(),
                        target_site.map(|range| range.source_file.as_str()),
                        site,
                        resolution_rule,
                        &candidate.language,
                    ))
                },
            )
            .collect::<Vec<_>>();
        edges.extend(materialized);
        profile_internal("universal edge materialization", &mut profile_started);
    }

    fn resolve_rust_associated_type(
        &self,
        language: &str,
        receiver_declaration_id: &str,
        receiver_qualified_name: &str,
        trait_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let receiver = self.declarations.get(receiver_declaration_id);
        if language != "rust"
            || receiver.is_none_or(|receiver| {
                receiver.language != "rust"
                    || receiver.qualified_name != receiver_qualified_name
                    || !matches!(receiver.kind.as_str(), "struct" | "enum" | "type_alias")
            })
        {
            return ResolutionDecision::Unresolved;
        }
        let trait_qualified_name =
            match self.canonical_rust_impl_trait(receiver_declaration_id, trait_qualified_name) {
                Ok(qualified_name) => qualified_name,
                Err(0) => return ResolutionDecision::Unresolved,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
        let lineage = match self.rust_trait_lineage(&trait_qualified_name) {
            Ok(lineage) => lineage,
            Err(0) => return ResolutionDecision::Unresolved,
            Err(candidate_count) => {
                return ResolutionDecision::Ambiguous { candidate_count };
            }
        };
        let mut targets = BTreeSet::new();
        let associated_trait_names = self.rust_impl_associated_trait_names.get(&(
            receiver_declaration_id.to_owned(),
            candidate.target_spelling.clone(),
        ));
        let Some(associated_trait_names) = associated_trait_names else {
            return ResolutionDecision::Unresolved;
        };
        if !associated_trait_names.complete {
            return ResolutionDecision::Ambiguous {
                candidate_count: associated_trait_names.modules.len().saturating_add(1),
            };
        }
        for raw_trait_name in &associated_trait_names.modules {
            let trait_name =
                match self.canonical_rust_impl_trait(receiver_declaration_id, raw_trait_name) {
                    Ok(qualified_name) => qualified_name,
                    Err(0) => return ResolutionDecision::Unresolved,
                    Err(candidate_count) => {
                        return ResolutionDecision::Ambiguous { candidate_count };
                    }
                };
            if !lineage.contains(&trait_name) {
                continue;
            }
            let Some(associated) = self.rust_impl_associated_types.get(&(
                receiver_declaration_id.to_owned(),
                raw_trait_name.clone(),
                candidate.target_spelling.clone(),
            )) else {
                continue;
            };
            if !associated.complete {
                return ResolutionDecision::Ambiguous {
                    candidate_count: associated.declarations.len().saturating_add(1),
                };
            }
            targets.extend(
                associated
                    .declarations
                    .iter()
                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                    .copied(),
            );
            if targets.len() > self.limits.candidates_per_lookup {
                return ResolutionDecision::Ambiguous {
                    candidate_count: targets.len(),
                };
            }
        }
        match targets.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => {
                let Some(declaration_id) = self.declaration_id(*only) else {
                    return ResolutionDecision::Unresolved;
                };
                ResolutionDecision::Resolved {
                    declaration_id: declaration_id.to_owned(),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::RustAssociatedType,
                        candidate_count: 1,
                    },
                }
            }
            [] => ResolutionDecision::Unresolved,
            many => ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            },
        }
    }

    fn canonical_rust_impl_trait(
        &self,
        receiver_declaration_id: &str,
        raw_trait_name: &str,
    ) -> Result<String, usize> {
        if let Ok(declaration) = self.exact_rust_declaration(raw_trait_name, &["trait"]) {
            return Ok(declaration.qualified_name.clone());
        }
        let Some(candidates) = self.rust_impl_traits.get(&(
            receiver_declaration_id.to_owned(),
            raw_trait_name.to_owned(),
        )) else {
            return Err(0);
        };
        if !candidates.complete {
            return Err(candidates.candidate_ids.len().saturating_add(1));
        }
        let mut qualified_names = BTreeSet::new();
        for candidate_id in &candidates.candidate_ids {
            let Some(candidate) = self.candidates.get(candidate_id) else {
                return Err(0);
            };
            match self.resolve_rust_impl_trait_candidate(candidate, raw_trait_name) {
                ResolutionDecision::Resolved { declaration_id, .. } => {
                    let Some(declaration) = self.declarations.get(&declaration_id) else {
                        return Err(0);
                    };
                    if declaration.language != "rust" || declaration.kind != "trait" {
                        return Err(0);
                    }
                    qualified_names.insert(declaration.qualified_name.clone());
                }
                ResolutionDecision::Ambiguous { candidate_count } => {
                    return Err(candidate_count);
                }
                _ => return Err(0),
            }
            if qualified_names.len() > self.limits.candidates_per_lookup {
                return Err(qualified_names.len());
            }
        }
        match qualified_names.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(0),
            many => Err(many.len()),
        }
    }

    fn resolve_rust_impl_trait_candidate(
        &self,
        candidate: &RelationshipCandidate,
        raw_trait_name: &str,
    ) -> ResolutionDecision {
        let mut lookup = candidate.clone();
        lookup.target_spelling = raw_trait_name
            .rsplit("::")
            .next()
            .unwrap_or(raw_trait_name)
            .to_owned();
        lookup.constraints.qualified_name = Some(raw_trait_name.to_owned());
        lookup.constraints.allow_external = false;

        if let Some(decision) = self.resolve_explicit_binding("rust", &lookup) {
            return decision;
        }
        if let Some(module) = lookup.constraints.module_or_package.as_ref() {
            let key = (
                "rust".to_owned(),
                module.clone(),
                lookup.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.by_module_name.get(&key),
                &lookup,
                ResolutionRule::UniqueModuleOrPackage,
            ) {
                return decision;
            }
        }
        if let Some(decision) = self.resolve_wildcard_binding("rust", &lookup) {
            return decision;
        }
        if let Some(decision) = self.resolve_visible_wildcard_bindings("rust", &lookup) {
            return decision;
        }
        ResolutionDecision::Unresolved
    }

    fn rust_trait_lineage(&self, root: &str) -> Result<BTreeSet<String>, usize> {
        let mut pending = vec![root.to_owned()];
        let mut lineage = BTreeSet::new();
        while let Some(qualified_name) = pending.pop() {
            let trait_declaration = self.exact_rust_declaration(&qualified_name, &["trait"])?;
            if !lineage.insert(trait_declaration.qualified_name.clone()) {
                continue;
            }
            if lineage.len() > self.limits.candidates_per_lookup {
                return Err(lineage.len());
            }
            if !trait_declaration.direct_bases_complete {
                return Err(lineage.len().saturating_add(1));
            }
            let key = ("rust".to_owned(), trait_declaration.qualified_name.clone());
            let Some(bases) = self.direct_bases.get(&key) else {
                continue;
            };
            if !bases.complete || bases.links.len() > self.limits.candidates_per_lookup {
                return Err(bases.links.len().saturating_add(1));
            }
            for base in bases.links.iter().rev() {
                let Some(base_name) = base.qualified_name.as_deref() else {
                    return Err(bases.links.len().saturating_add(1));
                };
                match self.exact_rust_declaration(base_name, &["trait"]) {
                    Ok(base) => pending.push(base.qualified_name.clone()),
                    Err(0) if self.rust_builtin_marker_base(base) => {}
                    Err(candidate_count) => return Err(candidate_count),
                }
            }
        }
        Ok(lineage)
    }

    fn rust_builtin_marker_base(&self, link: &DirectBaseLink) -> bool {
        let Some(candidate) = self.candidates.get(&link.candidate_id) else {
            return false;
        };
        if !matches!(candidate.target_spelling.as_str(), "Send" | "Sized")
            || self
                .occurrence(candidate)
                .is_none_or(|occurrence| occurrence.qualifier.is_some())
        {
            return false;
        }
        match candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
        {
            Some(binding) if binding.spelling == "*" => {
                self.resolve_wildcard_binding("rust", candidate).is_none()
            }
            Some(_) => false,
            None => self
                .resolve_visible_wildcard_bindings("rust", candidate)
                .is_none(),
        }
    }

    fn exact_rust_declaration<'a>(
        &'a self,
        qualified_name: &str,
        allowed_kinds: &[&str],
    ) -> Result<&'a DeclarationFact, usize> {
        let exact = self
            .by_qualified
            .get(&("rust".to_owned(), qualified_name.to_owned()))
            .into_iter()
            .flat_map(|slots| slots.iter())
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| allowed_kinds.contains(&declaration.kind.as_str()))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [only] => return Ok(*only),
            [] => {}
            many => return Err(many.len()),
        }
        let aliased = self.follow_alias("rust", qualified_name)?;
        if aliased == qualified_name {
            return Err(0);
        }
        let declarations = self
            .by_qualified
            .get(&("rust".to_owned(), aliased))
            .into_iter()
            .flat_map(|slots| slots.iter())
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| allowed_kinds.contains(&declaration.kind.as_str()))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .collect::<Vec<_>>();
        match declarations.as_slice() {
            [only] => Ok(*only),
            [] => Err(0),
            many => Err(many.len()),
        }
    }

    fn resolve_c3_receiver_dispatch(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        strategy: ReceiverDispatchStrategy,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        match strategy {
            ReceiverDispatchStrategy::C3FromReceiver => {
                if let Some(decision) =
                    self.resolve_exact_receiver_member(language, receiver_qualified_name, candidate)
                {
                    return decision;
                }
                // A direct member on the first base is source-proven even
                // when a later external base prevents construction of the
                // complete linearization. The same remains true through a
                // chain of single inheritance: no sibling can precede the
                // next class until the chain reaches a multiple-base fork.
                if let Some(decision) = self.resolve_source_proven_receiver_prefix(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
                if let Some(decision) = self.resolve_source_proven_later_direct_base(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
            ReceiverDispatchStrategy::C3AfterReceiver => {
                if let Some(decision) = self.resolve_direct_receiver_successor(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
                if let Some(decision) = self.resolve_source_proven_later_direct_base(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
        }
        let mut memo = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let Ok(linearization) = self.c3_linearization(
            language,
            receiver_qualified_name,
            &mut memo,
            &mut visiting,
            0,
        ) else {
            return ResolutionDecision::Unresolved;
        };
        if linearization.first().map(String::as_str) != Some(receiver_qualified_name) {
            return ResolutionDecision::Unresolved;
        }
        for owner in linearization.iter().skip(1) {
            let key = (
                language.to_owned(),
                owner.clone(),
                candidate.target_spelling.clone(),
            );
            let Some(members) = self.members_by_owner.get(&key) else {
                continue;
            };
            let eligible = members
                .iter()
                .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                .take(self.limits.candidates_per_lookup.saturating_add(1))
                .cloned()
                .collect::<Vec<_>>();
            match eligible.as_slice() {
                [only] => {
                    let Some(only) = self.declaration_id(*only) else {
                        continue;
                    };
                    return ResolutionDecision::Resolved {
                        declaration_id: only.to_owned(),
                        evidence: ResolutionEvidence {
                            rule: ResolutionRule::LinearizedReceiverDispatch,
                            candidate_count: 1,
                        },
                    };
                }
                [] => {}
                many => {
                    return ResolutionDecision::Ambiguous {
                        candidate_count: many.len(),
                    };
                }
            }
        }
        ResolutionDecision::Unresolved
    }

    fn possible_receiver_dispatches(
        &self,
        candidate_id: &str,
        exact_declaration_id: Option<&str>,
    ) -> Vec<(String, ResolutionRule)> {
        let Some(candidate) = self.candidates.get(candidate_id) else {
            return Vec::new();
        };
        let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        else {
            return Vec::new();
        };
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        let Some(receiver) = self.exact_hierarchy_type(language, receiver_qualified_name) else {
            return Vec::new();
        };
        let mut possible = BTreeMap::<String, ResolutionRule>::new();
        if let Some(declaration_id) =
            self.possible_incomplete_hierarchy_member(language, &receiver, candidate)
            && exact_declaration_id != Some(declaration_id.as_str())
        {
            possible.insert(
                declaration_id,
                ResolutionRule::IncompleteHierarchyReceiverDispatch,
            );
        }
        let Some(descendants) = self.closed_world_descendants(language, &receiver) else {
            return possible.into_iter().collect();
        };
        for descendant in descendants {
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            let linearization =
                match self.c3_linearization(language, &descendant, &mut memo, &mut visiting, 0) {
                    Ok(linearization) => linearization,
                    Err(()) => {
                        if *strategy == ReceiverDispatchStrategy::C3FromReceiver
                            && self.hierarchy_has_unresolved_base(language, &descendant)
                        {
                            let decision = self
                                .resolve_exact_receiver_member(language, &descendant, candidate)
                                .or_else(|| {
                                    self.resolve_source_proven_receiver_prefix(
                                        language,
                                        &descendant,
                                        candidate,
                                    )
                                });
                            match decision {
                                Some(ResolutionDecision::Resolved { declaration_id, .. }) => {
                                    if exact_declaration_id != Some(declaration_id.as_str()) {
                                        possible.entry(declaration_id).or_insert(
                                            ResolutionRule::IncompleteHierarchyReceiverDispatch,
                                        );
                                    }
                                }
                                Some(ResolutionDecision::Ambiguous { .. }) => return Vec::new(),
                                Some(_) | None => {}
                            }
                        }
                        continue;
                    }
                };
            let start = match strategy {
                ReceiverDispatchStrategy::C3FromReceiver => 0,
                ReceiverDispatchStrategy::C3AfterReceiver => {
                    let Some(position) = linearization.iter().position(|owner| owner == &receiver)
                    else {
                        continue;
                    };
                    position.saturating_add(1)
                }
            };
            if !linearization.iter().any(|owner| owner == &receiver) {
                continue;
            }
            for owner in linearization.iter().skip(start) {
                match self.unique_receiver_member_id(language, owner, candidate) {
                    Ok(Some(declaration_id)) => {
                        if exact_declaration_id != Some(declaration_id.as_str()) {
                            possible
                                .entry(declaration_id)
                                .or_insert(ResolutionRule::ClosedWorldReceiverDispatch);
                        }
                        break;
                    }
                    Ok(None) => {}
                    Err(()) => return Vec::new(),
                }
            }
            if possible.len() > self.limits.candidates_per_lookup {
                return Vec::new();
            }
        }
        possible.into_iter().collect()
    }

    fn hierarchy_has_unresolved_base(&self, language: &str, root: &str) -> bool {
        let mut visiting = BTreeSet::new();
        self.hierarchy_incompleteness(language, root, &mut visiting, 0)
            .unwrap_or(false)
    }

    fn hierarchy_incompleteness(
        &self,
        language: &str,
        qualified_name: &str,
        visiting: &mut BTreeSet<(String, String)>,
        depth: usize,
    ) -> Result<bool, ()> {
        if depth >= self.limits.candidates_per_lookup {
            return Err(());
        }
        let canonical = self
            .exact_hierarchy_type(language, qualified_name)
            .ok_or(())?;
        let key = (language.to_owned(), canonical);
        if !visiting.insert(key.clone()) {
            return Err(());
        }
        let result = (|| {
            let Some(bases) = self.direct_bases.get(&key) else {
                return Ok(false);
            };
            if bases.links.len() > self.limits.candidates_per_lookup {
                return Err(());
            }
            let mut incomplete = !bases.complete;
            for link in &bases.links {
                let Some(base) = link
                    .qualified_name
                    .as_deref()
                    .and_then(|name| self.exact_hierarchy_type(language, name))
                else {
                    incomplete = true;
                    continue;
                };
                incomplete |= self.hierarchy_incompleteness(
                    language,
                    &base,
                    visiting,
                    depth.saturating_add(1),
                )?;
            }
            Ok(incomplete)
        })();
        visiting.remove(&key);
        result
    }

    fn closed_world_descendants(&self, language: &str, receiver: &str) -> Option<Vec<String>> {
        let mut discovered = BTreeSet::new();
        let mut frontier = vec![receiver.to_owned()];
        let mut cursor = 0usize;
        while let Some(current) = frontier.get(cursor).cloned() {
            cursor = cursor.saturating_add(1);
            let Some(direct) = self.direct_subtypes.get(&(language.to_owned(), current)) else {
                continue;
            };
            if !direct.complete {
                return None;
            }
            for subtype in &direct.types {
                if discovered.insert(subtype.clone()) {
                    if discovered.len() > self.limits.candidates_per_lookup {
                        return None;
                    }
                    frontier.push(subtype.clone());
                }
            }
        }
        Some(discovered.into_iter().collect())
    }

    fn possible_incomplete_hierarchy_member(
        &self,
        language: &str,
        receiver: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<String> {
        let bases = self
            .direct_bases
            .get(&(language.to_owned(), receiver.to_owned()))?;
        if !bases.complete
            || bases.links.len() < 2
            || bases.links.len() > self.limits.candidates_per_lookup
        {
            return None;
        }
        let mut preceding_hierarchy_unknown = false;
        for link in &bases.links {
            let Some(base) = link
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))
            else {
                preceding_hierarchy_unknown = true;
                continue;
            };
            match self.unique_receiver_member_id(language, &base, candidate) {
                Ok(Some(declaration_id)) => {
                    return preceding_hierarchy_unknown.then_some(declaration_id);
                }
                Ok(None) => {}
                Err(()) => return None,
            }
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            if self
                .c3_linearization(language, &base, &mut memo, &mut visiting, 0)
                .is_err()
            {
                preceding_hierarchy_unknown = true;
            }
        }
        None
    }

    fn unique_receiver_member_id(
        &self,
        language: &str,
        receiver: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<Option<String>, ()> {
        match self.resolve_exact_receiver_member(language, receiver, candidate) {
            Some(ResolutionDecision::Resolved { declaration_id, .. }) => Ok(Some(declaration_id)),
            Some(ResolutionDecision::Ambiguous { .. }) => Err(()),
            Some(_) | None => Ok(None),
        }
    }

    fn resolve_source_proven_receiver_prefix(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let mut receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let mut visited = BTreeSet::new();
        for _ in 0..self.limits.candidates_per_lookup {
            if !visited.insert(receiver.clone()) {
                return None;
            }
            let base_set = self
                .direct_bases
                .get(&(language.to_owned(), receiver.clone()))?;
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.limits.candidates_per_lookup
            {
                return None;
            }
            let first_base = base_set.links[0]
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))?;
            if let Some(decision) =
                self.resolve_exact_receiver_member(language, &first_base, candidate)
            {
                return Some(decision);
            }
            if base_set.links.len() != 1 {
                return None;
            }
            receiver = first_base;
        }
        None
    }

    fn resolve_exact_receiver_member(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let key = (
            language.to_owned(),
            receiver,
            candidate.target_spelling.clone(),
        );
        let mut eligible = BTreeSet::new();
        if let Some(members) = self.members_by_owner.get(&key) {
            eligible.extend(
                members
                    .iter()
                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                    .copied(),
            );
        }
        if let Some(targets) = self.members.get(&key) {
            for target in targets {
                let Some(declarations) = self
                    .by_qualified
                    .get(&(language.to_owned(), target.clone()))
                else {
                    continue;
                };
                eligible.extend(
                    declarations
                        .iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .copied(),
                );
            }
        }
        let eligible = eligible
            .into_iter()
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::LinearizedReceiverDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn resolve_source_proven_later_direct_base(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let base_set = self.direct_bases.get(&(language.to_owned(), receiver))?;
        if !base_set.complete
            || base_set.links.len() < 2
            || base_set.links.len() > self.limits.candidates_per_lookup
        {
            return None;
        }
        for (index, link) in base_set.links.iter().enumerate() {
            let base = link
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))?;
            if index > 0
                && let Some(decision) =
                    self.resolve_exact_receiver_member(language, &base, candidate)
            {
                return Some(decision);
            }
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            let linearization = self
                .c3_linearization(language, &base, &mut memo, &mut visiting, 0)
                .ok()?;
            if linearization.as_slice() != [base.as_str()] {
                return None;
            }
        }
        None
    }

    fn resolve_direct_base(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let Some(qualified_name) = candidate.constraints.qualified_name.as_deref() else {
            return ResolutionDecision::Unresolved;
        };
        let qualified_name = match self.follow_alias(language, qualified_name) {
            Ok(qualified_name) => qualified_name,
            Err(candidate_count) => {
                return ResolutionDecision::Ambiguous { candidate_count };
            }
        };
        let key = (language.to_owned(), qualified_name.clone());
        if let Some(decision) = self.unique_decision(
            self.by_qualified.get(&key),
            candidate,
            ResolutionRule::ExactHierarchyBase,
        ) {
            return decision;
        }
        if candidate.constraints.allow_external {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }

    fn resolve_direct_receiver_successor(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let base_set = self.direct_bases.get(&(language.to_owned(), receiver))?;
        if !base_set.complete
            || base_set.links.is_empty()
            || base_set.links.len() > self.limits.candidates_per_lookup
        {
            return None;
        }
        let direct_successor = base_set.links[0]
            .qualified_name
            .as_deref()
            .and_then(|name| self.exact_hierarchy_type(language, name))?;
        let members = self.members_by_owner.get(&(
            language.to_owned(),
            direct_successor,
            candidate.target_spelling.clone(),
        ))?;
        let eligible = members
            .iter()
            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DirectReceiverSuccessorDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn c3_linearization(
        &self,
        language: &str,
        qualified_name: &str,
        memo: &mut BTreeMap<(String, String), Result<Vec<String>, ()>>,
        visiting: &mut BTreeSet<(String, String)>,
        depth: usize,
    ) -> Result<Vec<String>, ()> {
        if depth >= self.limits.candidates_per_lookup {
            return Err(());
        }
        let canonical = self
            .exact_hierarchy_type(language, qualified_name)
            .ok_or(())?;
        let key = (language.to_owned(), canonical.clone());
        if let Some(cached) = memo.get(&key) {
            return cached.clone();
        }
        if !visiting.insert(key.clone()) {
            return Err(());
        }
        let result = (|| {
            let Some(base_set) = self.direct_bases.get(&key) else {
                return Ok(vec![canonical.clone()]);
            };
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.limits.candidates_per_lookup
            {
                return Err(());
            }
            let mut bases = Vec::with_capacity(base_set.links.len());
            let mut sequences = Vec::with_capacity(base_set.links.len().saturating_add(1));
            for link in &base_set.links {
                let base = link
                    .qualified_name
                    .as_deref()
                    .and_then(|name| self.exact_hierarchy_type(language, name))
                    .ok_or(())?;
                bases.push(base.clone());
                sequences.push(self.c3_linearization(
                    language,
                    &base,
                    memo,
                    visiting,
                    depth.saturating_add(1),
                )?);
            }
            sequences.push(bases);
            let mut linearization = vec![canonical.clone()];
            linearization.extend(c3_merge(sequences, self.limits.candidates_per_lookup)?);
            Ok(linearization)
        })();
        visiting.remove(&key);
        memo.insert(key, result.clone());
        result
    }

    fn exact_hierarchy_type(&self, language: &str, qualified_name: &str) -> Option<String> {
        let qualified_name = self.follow_alias(language, qualified_name).ok()?;
        let declarations = self
            .by_qualified
            .get(&(language.to_owned(), qualified_name))?;
        let eligible = declarations
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| declaration.kind == "class")
            .take(2)
            .collect::<Vec<_>>();
        let [declaration] = eligible.as_slice() else {
            return None;
        };
        Some(declaration.qualified_name.clone())
    }

    fn occurrence(&self, candidate: &RelationshipCandidate) -> Option<&OccurrenceFact> {
        candidate
            .occurrence_id
            .as_deref()
            .and_then(|id| self.occurrences.get(id))
    }

    fn unique_decision(
        &self,
        ids: Option<&Vec<DeclarationSlot>>,
        candidate: &RelationshipCandidate,
        rule: ResolutionRule,
    ) -> Option<ResolutionDecision> {
        let ids = ids?;
        let eligible = ids
            .iter()
            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
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

    fn typescript_declaration_allowed_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        let Some(target) = self.declaration(slot) else {
            return false;
        };
        typescript_declaration_basic_allowed(target, candidate)
    }

    fn typescript_declaration_allowed_owner_slot(
        &self,
        slot: DeclarationSlot,
        candidate: &RelationshipCandidate,
    ) -> bool {
        let Some(target) = self.declaration(slot) else {
            return false;
        };
        typescript_declaration_basic_allowed_with_type_owner(target, candidate)
    }

    fn unique_typescript_decision(
        &self,
        ids: Option<&Vec<DeclarationSlot>>,
        candidate: &RelationshipCandidate,
        rule: ResolutionRule,
    ) -> Option<ResolutionDecision> {
        let ids = ids?;
        let eligible = ids
            .iter()
            .filter(|slot| self.typescript_declaration_allowed_slot(**slot, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
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

    fn inventory_decision(
        &self,
        language: &str,
        qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if !matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) || (!candidate.constraints.allowed_target_kinds.is_empty()
            && !candidate
                .constraints
                .allowed_target_kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), "file" | "module" | "package")))
        {
            return None;
        }
        let candidates = self
            .inventory_by_qualified
            .get(&(language.to_owned(), qualified_name.to_owned()))?;
        match candidates.as_slice() {
            [graph_node_id] => Some(ResolutionDecision::ResolvedInventory {
                graph_node_id: graph_node_id.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceInventory,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn imported_declarations(
        &self,
        language: &str,
        import_path: &str,
        spelling: &str,
    ) -> Vec<DeclarationSlot> {
        if language != "go" {
            return Vec::new();
        }
        let components = import_path
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect::<Vec<_>>();
        for start in 0..components.len().min(64) {
            let directory = components[start..].join("/");
            let key = (language.to_owned(), directory, spelling.to_owned());
            if let Some(candidates) = self.by_source_directory_name.get(&key) {
                let imported = candidates
                    .iter()
                    .filter_map(|slot| {
                        self.declaration(*slot)
                            .filter(|declaration| !declaration.qualified_name.contains("::"))
                            .map(|_| *slot)
                    })
                    .take(self.limits.candidates_per_lookup.saturating_add(1))
                    .collect::<BTreeSet<_>>();
                if !imported.is_empty() {
                    return imported.into_iter().collect();
                }
            }
        }
        if self.go_module_path.as_deref() != Some(import_path) {
            return Vec::new();
        }
        let Some(package) = components.last() else {
            return Vec::new();
        };
        self.by_module_name
            .get(&(
                language.to_owned(),
                (*package).to_owned(),
                spelling.to_owned(),
            ))
            .into_iter()
            .flatten()
            .filter_map(|slot| {
                self.declaration(*slot)
                    .filter(|declaration| !declaration.qualified_name.contains("::"))
                    .map(|_| *slot)
            })
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn imported_member_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if language != "go" {
            return None;
        }
        let (owner, member) = qualified.rsplit_once("::")?;
        let (import_path, owner_spelling) = owner.rsplit_once('.')?;
        let owner_ids = self.imported_declarations(language, import_path, owner_spelling);
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        for owner_id in owner_ids
            .into_iter()
            .take(candidate_storage_limit(self.limits.candidates_per_lookup))
        {
            let owner = &self.declaration(owner_id)?.qualified_name;
            if let Some(ids) = self
                .by_qualified
                .get(&(language.to_owned(), format!("{owner}::{member}")))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
        }
        self.unique_decision(
            Some(&declarations.into_iter().collect::<Vec<_>>()),
            candidate,
            ResolutionRule::MemberBinding,
        )
    }

    fn bound_member_target(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
    ) -> Result<Option<String>, usize> {
        if binding.kind == compass_languages::BindingKind::CallResult {
            let Some(return_type) = self.call_result_return_type(
                language,
                binding,
                candidate,
                &mut BTreeSet::new(),
                0,
            )?
            else {
                return Ok(None);
            };
            return Ok(Some(format!(
                "{return_type}::{}",
                candidate.target_spelling
            )));
        }
        if binding.kind != compass_languages::BindingKind::LocalAlias {
            return Ok(None);
        }
        let Some(qualifier) = self.occurrence(candidate).and_then(|occurrence| {
            occurrence
                .qualifier
                .as_deref()
                .filter(|qualifier| qualifier.contains('.'))
        }) else {
            return Ok(None);
        };
        let mut parts = qualifier.split('.');
        if parts.next() != Some(binding.spelling.as_str()) {
            return Ok(None);
        }
        let mut target = binding.qualified_target.clone();
        for member in parts {
            let Some(targets) =
                self.members
                    .get(&(language.to_owned(), target.clone(), member.to_owned()))
            else {
                return Ok(None);
            };
            let [next] = targets.as_slice() else {
                return Err(targets.len());
            };
            target.clone_from(next);
        }
        Ok(Some(format!("{target}::{}", candidate.target_spelling)))
    }

    fn call_result_return_type(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<Option<String>, usize> {
        const MAX_CALL_RESULT_DEPTH: usize = 64;

        if depth >= MAX_CALL_RESULT_DEPTH || !visiting.insert(binding.id.clone()) {
            return Err(visiting.len().saturating_add(1));
        }
        let result =
            self.call_result_return_type_inner(language, binding, candidate, visiting, depth);
        visiting.remove(&binding.id);
        result
    }

    fn call_result_return_type_inner(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<Option<String>, usize> {
        if let Some(return_type) = binding.result_type_qualified_name.as_ref() {
            return Ok(Some(return_type.clone()));
        }
        let qualified_callable =
            if let Some(receiver_binding_id) = binding.receiver_binding_id.as_ref() {
                let Some(receiver_binding) = self.bindings.get(receiver_binding_id) else {
                    return Ok(None);
                };
                let Some(receiver_type) = self.call_result_return_type(
                    language,
                    receiver_binding,
                    candidate,
                    visiting,
                    depth.saturating_add(1),
                )?
                else {
                    return Ok(None);
                };
                format!("{receiver_type}::{}", binding.qualified_target)
            } else {
                binding.qualified_target.clone()
            };

        let mut callable_ids = BTreeSet::new();
        callable_ids.extend(
            self.callable_declarations(language, &qualified_callable)
                .into_iter()
                .filter(|slot| self.declaration_allowed_slot(*slot, candidate)),
        );
        if callable_ids.is_empty()
            && let Some(member_ids) =
                self.member_declarations(language, &qualified_callable, candidate)
        {
            callable_ids.extend(member_ids);
        }
        if callable_ids.is_empty() {
            callable_ids.extend(self.rust_trait_member_declarations(
                language,
                &qualified_callable,
                candidate,
            )?);
        }
        if callable_ids.is_empty() {
            callable_ids.extend(self.rust_wildcard_callable_declarations(
                language,
                binding,
                &qualified_callable,
                candidate,
            )?);
        }
        if callable_ids.len() > self.limits.candidates_per_lookup {
            return Err(callable_ids.len());
        }
        let callable_ids = callable_ids.into_iter().collect::<Vec<_>>();
        let [callable_id] = callable_ids.as_slice() else {
            return if callable_ids.is_empty() {
                Ok(None)
            } else {
                Err(callable_ids.len())
            };
        };
        let Some(callable) = self.declaration(*callable_id) else {
            return Ok(None);
        };
        let key = (language.to_owned(), callable.qualified_name.clone());
        let return_candidates = if language == "rust" {
            self.outer_return_candidates_by_callable.get(&key)
        } else {
            self.return_candidates_by_callable.get(&key)
        };
        let Some(return_candidates) = return_candidates else {
            return Ok(None);
        };
        if let Some(output_index) = binding.output_index {
            let Ok(output_index) = usize::try_from(output_index) else {
                return Ok(None);
            };
            let Some(return_candidate) = return_candidates.get(output_index) else {
                return Ok(None);
            };
            return self.resolved_return_type(return_candidate);
        }
        let [return_candidate] = return_candidates.as_slice() else {
            return Err(return_candidates.len());
        };
        self.resolved_return_type(return_candidate)
    }

    fn rust_wildcard_callable_declarations(
        &self,
        language: &str,
        call_result_binding: &BindingFact,
        qualified_callable: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<BTreeSet<DeclarationSlot>, usize> {
        if language != "rust" || call_result_binding.receiver_binding_id.is_some() {
            return Ok(BTreeSet::new());
        }
        let Some((qualifier, spelling)) = split_qualified_member(qualified_callable) else {
            return Ok(BTreeSet::new());
        };

        let mut callable_candidate = candidate.clone();
        callable_candidate.target_spelling = spelling.to_owned();
        callable_candidate.constraints.exact_target_declaration_id = None;
        callable_candidate.constraints.qualified_name = Some(qualified_callable.to_owned());
        callable_candidate.constraints.argument_count = None;
        callable_candidate.constraints.argument_types.clear();
        callable_candidate.constraints.allowed_target_kinds =
            vec!["function".to_owned(), "method".to_owned()];
        callable_candidate.constraints.hierarchy = None;
        callable_candidate.constraints.allow_external = false;

        if let Some(wildcard_binding) = call_result_binding
            .fallback_binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            .filter(|binding| binding.spelling == "*")
        {
            callable_candidate.binding_id = Some(wildcard_binding.id.clone());
            return self.wildcard_declarations(
                language,
                std::slice::from_ref(&wildcard_binding.qualified_target),
                Some(qualifier),
                &callable_candidate,
            );
        }

        let mut scope_id = call_result_binding.scope_id.as_deref();
        let mut visited_scopes = BTreeSet::new();
        while let Some(current) = scope_id.filter(|scope| visited_scopes.insert(*scope)) {
            if visited_scopes.len() > self.limits.candidates_per_lookup {
                return Err(visited_scopes.len());
            }
            if let Some(bindings) = self
                .wildcard_bindings_by_scope
                .get(&(language.to_owned(), current.to_owned()))
            {
                if !bindings.complete {
                    return Err(bindings.modules.len().saturating_add(1));
                }
                let declarations = self.wildcard_declarations(
                    language,
                    &bindings.modules,
                    Some(qualifier),
                    &callable_candidate,
                )?;
                if !declarations.is_empty() {
                    return Ok(declarations);
                }
            }
            scope_id = self
                .scopes
                .get(current)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        Ok(BTreeSet::new())
    }

    fn resolved_return_type(&self, candidate_id: &str) -> Result<Option<String>, usize> {
        let Some(candidate) = self.candidates.get(candidate_id) else {
            return Ok(None);
        };
        if candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            .is_some_and(|binding| binding.kind == compass_languages::BindingKind::CallResult)
        {
            return Ok(None);
        }
        match self.resolve(candidate_id) {
            ResolutionDecision::Resolved { declaration_id, .. } => Ok(self
                .declarations
                .get(&declaration_id)
                .map(|declaration| declaration.qualified_name.clone())),
            ResolutionDecision::QualifiedExternal { qualified_name, .. } => {
                Ok(Some(qualified_name))
            }
            ResolutionDecision::Ambiguous { candidate_count } => Err(candidate_count),
            ResolutionDecision::ResolvedInventory { .. }
            | ResolutionDecision::DeferredReceiver { .. }
            | ResolutionDecision::Unresolved => Ok(None),
        }
    }

    fn callable_declarations(&self, language: &str, qualified: &str) -> Vec<DeclarationSlot> {
        let qualified = match self.follow_alias(language, qualified) {
            Ok(qualified) => qualified,
            Err(_) => return Vec::new(),
        };
        let qualified = if language == "rust" {
            let Some((owner, member)) = qualified.rsplit_once("::") else {
                return Vec::new();
            };
            let owner = match self.follow_alias(language, owner) {
                Ok(owner) => owner,
                Err(_) => return Vec::new(),
            };
            format!("{owner}::{member}")
        } else {
            qualified
        };
        if let Some(declarations) = self
            .by_qualified
            .get(&(language.to_owned(), qualified.clone()))
        {
            return declarations.clone();
        }
        if language != "go" {
            return Vec::new();
        }
        let mut declarations = BTreeSet::new();
        if let Some((owner, member)) = qualified.rsplit_once("::")
            && let Some((import_path, owner_spelling)) = owner.rsplit_once('.')
        {
            for owner_id in self
                .imported_declarations(language, import_path, owner_spelling)
                .into_iter()
                .take(self.limits.candidates_per_lookup)
            {
                let Some(owner) = self.declaration(owner_id) else {
                    continue;
                };
                if let Some(ids) = self.by_qualified.get(&(
                    language.to_owned(),
                    format!("{}::{member}", owner.qualified_name),
                )) {
                    declarations.extend(ids.iter().cloned());
                }
            }
        } else if let Some((import_path, spelling)) = qualified.rsplit_once('.') {
            declarations.extend(self.imported_declarations(language, import_path, spelling));
        }
        declarations
            .into_iter()
            .take(candidate_storage_limit(self.limits.candidates_per_lookup))
            .collect()
    }

    fn member_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let declarations = self.member_declarations(language, qualified, candidate)?;
        self.unique_decision(
            Some(&declarations),
            candidate,
            ResolutionRule::MemberBinding,
        )
    }

    fn rust_trait_member_declarations(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<Vec<DeclarationSlot>, usize> {
        if language != "rust" {
            return Ok(Vec::new());
        }
        let Some((receiver, member)) = split_qualified_member(qualified) else {
            return Ok(Vec::new());
        };
        let receiver_slots = self
            .by_qualified
            .get(&(language.to_owned(), receiver.to_owned()))
            .into_iter()
            .flatten()
            .filter(|slot| {
                self.declaration(**slot).is_some_and(|declaration| {
                    matches!(
                        declaration.kind.as_str(),
                        "class" | "enum" | "struct" | "trait" | "type_alias"
                    )
                })
            })
            .copied()
            .collect::<Vec<_>>();
        let [receiver_slot] = receiver_slots.as_slice() else {
            return if receiver_slots.is_empty() {
                Ok(Vec::new())
            } else {
                Err(receiver_slots.len())
            };
        };
        let Some(receiver_id) = self.declaration_id(*receiver_slot) else {
            return Ok(Vec::new());
        };
        let mut matching_trait_sets = self
            .rust_impl_traits
            .iter()
            .filter(|((implementer_id, _), _)| implementer_id == receiver_id)
            .collect::<Vec<_>>();
        matching_trait_sets.sort_by(|left, right| left.0.1.cmp(&right.0.1));
        if matching_trait_sets.len() > self.limits.candidates_per_lookup {
            return Err(matching_trait_sets.len());
        }
        let mut declarations = BTreeSet::new();
        for ((_, raw_trait_name), traits) in matching_trait_sets {
            if !traits.complete {
                return Err(traits.candidate_ids.len().saturating_add(1));
            }
            for candidate_id in &traits.candidate_ids {
                let Some(implementation) = self.candidates.get(candidate_id) else {
                    return Err(1);
                };
                let trait_name =
                    match self.resolve_rust_impl_trait_candidate(implementation, raw_trait_name) {
                        ResolutionDecision::Resolved { declaration_id, .. } => {
                            let Some(declaration) = self.declarations.get(&declaration_id) else {
                                return Err(1);
                            };
                            declaration.qualified_name.as_str()
                        }
                        ResolutionDecision::Ambiguous { candidate_count } => {
                            return Err(candidate_count);
                        }
                        ResolutionDecision::QualifiedExternal { .. }
                        | ResolutionDecision::DeferredReceiver { .. }
                        | ResolutionDecision::ResolvedInventory { .. }
                        | ResolutionDecision::Unresolved => continue,
                    };
                if let Some(slots) = self
                    .by_qualified
                    .get(&(language.to_owned(), format!("{trait_name}::{member}")))
                {
                    declarations.extend(
                        slots
                            .iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .copied(),
                    );
                }
                if declarations.len() > self.limits.candidates_per_lookup {
                    return Err(declarations.len());
                }
            }
        }
        Ok(declarations.into_iter().collect())
    }

    fn wildcard_reexport_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let (facade, spelling) = qualified.rsplit_once('.')?;
        if !self
            .wildcard_reexports_by_module
            .contains_key(&(language.to_owned(), facade.to_owned()))
        {
            return None;
        }
        let mut modules = vec![facade.to_owned()];
        let mut visited = BTreeSet::new();
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        while let Some(module) = modules.pop() {
            if !visited.insert(module.clone()) {
                continue;
            }
            if visited.len() > self.limits.candidates_per_lookup {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: visited.len(),
                });
            }
            let Some(reexports) = self
                .wildcard_reexports_by_module
                .get(&(language.to_owned(), module))
            else {
                continue;
            };
            if !reexports.complete {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: reexports.modules.len().saturating_add(1),
                });
            }
            for reexport in &reexports.modules {
                let reexported = format!("{reexport}.{spelling}");
                let canonical = match self.follow_alias(language, &reexported) {
                    Ok(canonical) => canonical,
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                };
                if let Some(ids) = self
                    .by_qualified
                    .get(&(language.to_owned(), canonical.clone()))
                {
                    declarations.extend(
                        ids.iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .copied(),
                    );
                }
                if let Some(ids) = self.member_declarations(language, &canonical, candidate) {
                    declarations.extend(ids);
                }
                modules.push(reexport.clone());
            }
            if declarations.len() > self.limits.candidates_per_lookup {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: declarations.len(),
                });
            }
        }
        self.unique_decision(
            Some(&declarations.into_iter().collect::<Vec<_>>()),
            candidate,
            ResolutionRule::WildcardBinding,
        )
    }

    fn member_declarations(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<Vec<DeclarationSlot>> {
        let (owner, spelling) = split_qualified_member(qualified)?;
        let targets =
            self.members
                .get(&(language.to_owned(), owner.to_owned(), spelling.to_owned()))?;
        let mut declarations = BTreeSet::new();
        for target in targets {
            if let Some(ids) = self
                .by_qualified
                .get(&(language.to_owned(), target.clone()))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
        }
        Some(declarations.into_iter().collect())
    }

    fn declaration_allowed(&self, declaration_id: &str, candidate: &RelationshipCandidate) -> bool {
        let Some(target) = self.declarations.get(declaration_id) else {
            return false;
        };
        if !declaration_basic_allowed(target, candidate) {
            return false;
        }
        let argument_types = &candidate.constraints.argument_types;
        if target.language != "java"
            || argument_types.is_empty()
            || argument_types.iter().any(Option::is_none)
        {
            return true;
        }
        let Some(overloads) = self
            .by_qualified
            .get(&(target.language.clone(), target.qualified_name.clone()))
        else {
            return true;
        };
        let eligible = overloads
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| declaration_basic_allowed(declaration, candidate))
            .collect::<Vec<_>>();
        let exact = eligible
            .iter()
            .copied()
            .filter(|declaration| {
                declaration.parameter_types.len() == argument_types.len()
                    && declaration
                        .parameter_types
                        .iter()
                        .zip(argument_types)
                        .all(|(parameter, argument)| argument.as_deref() == Some(parameter))
            })
            .take(2)
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [only] => only.id == target.id,
            [] => self
                .unique_java_applicable_overload(&eligible, argument_types)
                .is_none_or(|only| only == target.id),
            _ => true,
        }
    }

    fn unique_java_applicable_overload<'a>(
        &self,
        overloads: &[&'a DeclarationFact],
        argument_types: &[Option<String>],
    ) -> Option<&'a str> {
        let mut proven = Vec::new();
        for declaration in overloads {
            if declaration.parameter_types.len() != argument_types.len() {
                return None;
            }
            let mut applicability = JavaApplicability::Proven;
            for (parameter, argument) in declaration.parameter_types.iter().zip(argument_types) {
                let argument = argument.as_deref()?;
                match self.java_conversion(argument, parameter) {
                    JavaConversion::Proven => {}
                    JavaConversion::Disproven => {
                        applicability = JavaApplicability::Disproven;
                        break;
                    }
                    JavaConversion::Unknown => applicability = JavaApplicability::Unknown,
                }
            }
            match applicability {
                JavaApplicability::Proven => proven.push(*declaration),
                JavaApplicability::Unknown => return None,
                JavaApplicability::Disproven => {}
            }
        }
        if let [only] = proven.as_slice() {
            return Some(only.id.as_str());
        }
        let mut most_specific = proven.iter().copied().filter(|candidate| {
            proven.iter().copied().all(|other| {
                candidate.id == other.id
                    || self.java_parameter_vector_more_specific(candidate, other)
            })
        });
        let only = most_specific.next()?;
        most_specific.next().is_none().then_some(only.id.as_str())
    }

    fn java_parameter_vector_more_specific(
        &self,
        candidate: &DeclarationFact,
        other: &DeclarationFact,
    ) -> bool {
        candidate.parameter_types.len() == other.parameter_types.len()
            && candidate
                .parameter_types
                .iter()
                .zip(&other.parameter_types)
                .all(|(candidate, other)| {
                    self.java_conversion(candidate, other) == JavaConversion::Proven
                })
            && candidate.parameter_types != other.parameter_types
    }

    fn java_conversion(&self, argument: &str, parameter: &str) -> JavaConversion {
        if argument == parameter {
            return JavaConversion::Proven;
        }
        if argument == "null" {
            return if java_primitive_type(parameter) {
                JavaConversion::Disproven
            } else {
                JavaConversion::Proven
            };
        }
        if java_primitive_type(argument) {
            if java_primitive_type(parameter) {
                return if java_primitive_widens_to(argument, parameter) {
                    JavaConversion::Proven
                } else {
                    JavaConversion::Disproven
                };
            }
            let Some(boxed) = java_boxed_type(argument) else {
                return JavaConversion::Disproven;
            };
            return self.java_reference_conversion(boxed, parameter);
        }
        if java_primitive_type(parameter) {
            let Some(unboxed) = java_unboxed_type(argument) else {
                return JavaConversion::Disproven;
            };
            return if java_primitive_widens_to(unboxed, parameter) {
                JavaConversion::Proven
            } else {
                JavaConversion::Disproven
            };
        }
        self.java_reference_conversion(argument, parameter)
    }

    fn java_reference_conversion(&self, argument: &str, parameter: &str) -> JavaConversion {
        if argument == parameter || parameter == "java.lang.Object" {
            return JavaConversion::Proven;
        }
        if let Some(argument_component) = argument.strip_suffix("[]") {
            if let Some(parameter_component) = parameter.strip_suffix("[]") {
                return if java_primitive_type(argument_component)
                    || java_primitive_type(parameter_component)
                {
                    if argument_component == parameter_component {
                        JavaConversion::Proven
                    } else {
                        JavaConversion::Disproven
                    }
                } else {
                    self.java_reference_conversion(argument_component, parameter_component)
                };
            }
            return if matches!(parameter, "java.lang.Cloneable" | "java.io.Serializable") {
                JavaConversion::Proven
            } else {
                JavaConversion::Disproven
            };
        }
        if parameter.ends_with("[]") {
            return JavaConversion::Disproven;
        }

        let mut pending = vec![argument.to_owned()];
        let mut visited = BTreeSet::new();
        let mut complete = true;
        while let Some(current) = pending.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if visited.len() > self.limits.candidates_per_lookup {
                return JavaConversion::Unknown;
            }
            for base in java_known_direct_bases(&current) {
                if *base == parameter {
                    return JavaConversion::Proven;
                }
                pending.push((*base).to_owned());
            }
            if java_known_direct_bases(&current).is_empty() && current != "java.lang.Object" {
                let Some(declaration) = self.exact_java_type_declaration(&current) else {
                    complete = false;
                    continue;
                };
                if !declaration.direct_bases_complete {
                    complete = false;
                    continue;
                }
                if let Some(bases) = self.direct_bases.get(&("java".to_owned(), current.clone())) {
                    if !bases.complete {
                        complete = false;
                        continue;
                    }
                    for link in &bases.links {
                        let Some(base) = link.qualified_name.as_ref() else {
                            complete = false;
                            continue;
                        };
                        if base == parameter {
                            return JavaConversion::Proven;
                        }
                        pending.push(base.clone());
                    }
                }
                let implicit = match declaration.kind.as_str() {
                    "enum" => "java.lang.Enum",
                    "record" => "java.lang.Record",
                    "class" | "interface" | "annotation_type" => "java.lang.Object",
                    _ => {
                        complete = false;
                        continue;
                    }
                };
                if implicit == parameter {
                    return JavaConversion::Proven;
                }
                pending.push(implicit.to_owned());
            }
        }
        if complete {
            JavaConversion::Disproven
        } else {
            JavaConversion::Unknown
        }
    }

    fn exact_java_type_declaration(&self, qualified_name: &str) -> Option<&DeclarationFact> {
        let declarations = self
            .by_qualified
            .get(&("java".to_owned(), qualified_name.to_owned()))?;
        let mut eligible = declarations.iter().filter_map(|id| {
            self.declaration(*id).filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "record" | "annotation_type"
                )
            })
        });
        let only = eligible.next()?;
        eligible.next().is_none().then_some(only)
    }

    fn follow_alias(&self, language: &str, qualified: &str) -> Result<String, usize> {
        let mut current = qualified.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..64 {
            if !seen.insert(current.clone()) {
                return Err(seen.len());
            }
            let Some(targets) = self.aliases.get(&(language.to_owned(), current.clone())) else {
                return Ok(current);
            };
            let [target] = targets.as_slice() else {
                return Err(targets.len());
            };
            if target == &current {
                return Ok(current);
            }
            current.clone_from(target);
        }
        Err(64)
    }

    fn follow_wildcard_qualified_alias(
        &self,
        language: &str,
        qualified: &str,
    ) -> Result<String, usize> {
        if language != "rust" {
            return self.follow_alias(language, qualified);
        }

        const MAX_ALIAS_DEPTH: usize = 64;
        let mut current = qualified.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_ALIAS_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(seen.len());
            }
            if let Some(targets) = self.aliases.get(&(language.to_owned(), current.clone())) {
                let [target] = targets.as_slice() else {
                    return Err(targets.len());
                };
                if target == &current {
                    return Ok(current);
                }
                current.clone_from(target);
                continue;
            }

            let separators = current
                .match_indices("::")
                .map(|(index, _)| index)
                .take(MAX_ALIAS_DEPTH.saturating_add(1))
                .collect::<Vec<_>>();
            if separators.len() > MAX_ALIAS_DEPTH {
                return Err(separators.len());
            }
            let mut expanded = None;
            for separator in separators.into_iter().rev() {
                let prefix = &current[..separator];
                let Some(targets) = self.aliases.get(&(language.to_owned(), prefix.to_owned()))
                else {
                    continue;
                };
                let [target] = targets.as_slice() else {
                    return Err(targets.len());
                };
                if target == prefix {
                    return Ok(current);
                }
                expanded = Some(format!("{target}{}", &current[separator..]));
                break;
            }
            let Some(next) = expanded else {
                return Ok(current);
            };
            current = next;
        }
        Err(MAX_ALIAS_DEPTH)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JavaApplicability {
    Proven,
    Disproven,
    Unknown,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JavaConversion {
    Proven,
    Disproven,
    Unknown,
}

fn java_primitive_type(kind: &str) -> bool {
    matches!(
        kind,
        "byte" | "short" | "int" | "long" | "float" | "double" | "boolean" | "char"
    )
}

fn java_primitive_widens_to(argument: &str, parameter: &str) -> bool {
    argument == parameter
        || matches!(
            (argument, parameter),
            ("byte", "short" | "int" | "long" | "float" | "double")
                | ("short" | "char", "int" | "long" | "float" | "double")
                | ("int", "long" | "float" | "double")
                | ("long", "float" | "double")
                | ("float", "double")
        )
}

fn java_boxed_type(primitive: &str) -> Option<&'static str> {
    match primitive {
        "byte" => Some("java.lang.Byte"),
        "short" => Some("java.lang.Short"),
        "int" => Some("java.lang.Integer"),
        "long" => Some("java.lang.Long"),
        "float" => Some("java.lang.Float"),
        "double" => Some("java.lang.Double"),
        "boolean" => Some("java.lang.Boolean"),
        "char" => Some("java.lang.Character"),
        _ => None,
    }
}

fn java_unboxed_type(reference: &str) -> Option<&'static str> {
    match reference {
        "java.lang.Byte" => Some("byte"),
        "java.lang.Short" => Some("short"),
        "java.lang.Integer" => Some("int"),
        "java.lang.Long" => Some("long"),
        "java.lang.Float" => Some("float"),
        "java.lang.Double" => Some("double"),
        "java.lang.Boolean" => Some("boolean"),
        "java.lang.Character" => Some("char"),
        _ => None,
    }
}

fn java_known_direct_bases(reference: &str) -> &'static [&'static str] {
    match reference {
        "java.lang.Byte" | "java.lang.Short" | "java.lang.Integer" | "java.lang.Long"
        | "java.lang.Float" | "java.lang.Double" => &["java.lang.Number"],
        "java.lang.Number" => &["java.lang.Object"],
        "java.lang.Boolean" | "java.lang.Character" | "java.lang.String" => &["java.lang.Object"],
        "java.lang.Class" => &["java.lang.Object", "java.lang.reflect.Type"],
        "java.lang.Enum" | "java.lang.Record" => &["java.lang.Object"],
        "java.lang.StringBuilder" | "java.lang.StringBuffer" => &[
            "java.lang.Object",
            "java.lang.Appendable",
            "java.lang.CharSequence",
        ],
        "java.lang.Object" => &[],
        _ => &[],
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

fn typescript_language_family(language: &str) -> &'static [&'static str] {
    match language {
        "typescript" => &["typescript", "javascript"],
        "javascript" => &["javascript", "typescript"],
        _ => &[],
    }
}

fn typescript_member_path(qualified: &str, target_spelling: &str) -> Option<TypeScriptMemberPath> {
    let (_, path) = split_typescript_module_qualified(qualified);
    let segments = split_typescript_member_segments(path);
    if segments.len() < 2 || segments.last()?.as_str() != target_spelling {
        return None;
    }
    let root_segment = segments.first()?;
    let (root_export, type_arguments) = typescript_generic_type_parts(root_segment).map_or_else(
        || (root_segment.clone(), Vec::new()),
        |(base, arguments)| (base.to_owned(), arguments),
    );
    if root_export.is_empty() {
        return None;
    }
    Some(TypeScriptMemberPath {
        root_export,
        type_arguments,
        members: segments.into_iter().skip(1).collect(),
    })
}

fn split_typescript_module_qualified(value: &str) -> (Option<&str>, &str) {
    let bytes = value.as_bytes();
    let mut angle_depth = 0_u32;
    let mut split = None;
    for index in 0..bytes.len().saturating_sub(1) {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b':' if bytes[index + 1] == b':' && angle_depth == 0 => split = Some(index),
            _ => {}
        }
    }
    split.map_or((None, value), |index| {
        (
            value.get(..index).filter(|module| !module.is_empty()),
            value.get(index.saturating_add(2)..).unwrap_or_default(),
        )
    })
}

fn split_typescript_member_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut start = 0_usize;
    let mut angle_depth = 0_u32;
    let bytes = value.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b'.' if angle_depth == 0 => {
                if let Some(segment) = value.get(start..index).map(str::trim)
                    && !segment.is_empty()
                {
                    segments.push(segment.to_owned());
                }
                start = index.saturating_add(1);
            }
            b':' if angle_depth == 0 && bytes.get(index.saturating_add(1)) == Some(&b':') => {
                if let Some(segment) = value.get(start..index).map(str::trim)
                    && !segment.is_empty()
                {
                    segments.push(segment.to_owned());
                }
                start = index.saturating_add(2);
                index = index.saturating_add(1);
            }
            _ => {}
        }
        index = index.saturating_add(1);
    }
    if let Some(segment) = value.get(start..).map(str::trim)
        && !segment.is_empty()
    {
        segments.push(segment.to_owned());
    }
    segments
}

fn typescript_generic_type_parts(value: &str) -> Option<(&str, Vec<String>)> {
    let value = value.trim();
    let bytes = value.as_bytes();
    let open = bytes.iter().position(|byte| *byte == b'<')?;
    if open == 0 || !value.ends_with('>') {
        return None;
    }
    let base = value.get(..open)?.trim();
    let arguments = value.get(open.saturating_add(1)..value.len().saturating_sub(1))?;
    if base.is_empty() || arguments.is_empty() || arguments.len() > 1024 {
        return None;
    }
    let mut parts = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in arguments.char_indices() {
        match character {
            '<' | '[' | '(' | '{' => depth = depth.saturating_add(1),
            '>' | ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = arguments.get(start..index)?.trim();
                if part.is_empty() || part.len() > 1024 {
                    return None;
                }
                parts.push(part.to_owned());
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    let part = arguments.get(start..)?.trim();
    if part.is_empty() || part.len() > 1024 {
        return None;
    }
    parts.push(part.to_owned());
    (parts.len() <= 64).then_some((base, parts))
}

fn typescript_generic_parameter_names(signature: &str) -> Vec<String> {
    let signature = signature.trim();
    if !signature.starts_with('<') || !signature.ends_with('>') {
        return Vec::new();
    }
    let body = &signature[1..signature.len().saturating_sub(1)];
    let mut names = Vec::new();
    for name in body.split(',').map(str::trim) {
        if name.is_empty()
            || name.len() > 128
            || !name.chars().all(|character| {
                character == '_' || character == '$' || character.is_ascii_alphanumeric()
            })
        {
            return Vec::new();
        }
        names.push(name.to_owned());
        if names.len() > 64 {
            return Vec::new();
        }
    }
    names
}

fn typescript_declaration_basic_allowed(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
) -> bool {
    typescript_declaration_basic_allowed_for(target, candidate, false)
}

fn typescript_declaration_basic_allowed_with_type_owner(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
) -> bool {
    typescript_declaration_basic_allowed_for(target, candidate, true)
}

fn typescript_declaration_basic_allowed_for(
    target: &DeclarationFact,
    candidate: &RelationshipCandidate,
    allow_type_owner: bool,
) -> bool {
    typescript_language_family(&candidate.language).contains(&target.language.as_str())
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
                .contains(&target.kind)
            || (allow_type_owner
                && matches!(
                    target.kind.as_str(),
                    "class" | "enum" | "interface" | "namespace" | "type_alias"
                )))
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

fn rust_module_is_descendant(source_module: &str, ancestor_module: &str) -> bool {
    source_module == ancestor_module
        || source_module
            .strip_prefix(ancestor_module)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

fn rust_external_wildcard_target_is_explicit(
    qualifier: Option<&str>,
    candidate: &RelationshipCandidate,
) -> bool {
    qualifier
        .and_then(|value| value.split("::").next())
        .and_then(|value| value.chars().next())
        .is_some_and(char::is_uppercase)
        || (qualifier.is_none()
            && candidate
                .target_spelling
                .chars()
                .next()
                .is_some_and(char::is_uppercase))
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

fn materialized_declaration_ids<'a>(
    declarations: impl Iterator<Item = &'a DeclarationFact>,
) -> AHashMap<String, String> {
    let mut groups = AHashMap::<String, Vec<&DeclarationFact>>::new();
    for declaration in declarations {
        groups
            .entry(declaration.graph_node_id.clone())
            .or_default()
            .push(declaration);
    }
    let mut ids = AHashMap::new();
    for (graph_node_id, declarations) in groups {
        if declarations.len() == 1 {
            ids.insert(declarations[0].id.clone(), graph_node_id);
            continue;
        }
        for declaration in declarations {
            ids.insert(
                declaration.id.clone(),
                make_id(&[&graph_node_id, &declaration.id]),
            );
        }
    }
    ids
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

fn python_module_name(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let source = relative.to_string_lossy().replace('\\', "/");
    let source = source.strip_suffix(".py")?;
    let source = source.strip_suffix("/__init__").unwrap_or(source);
    let module = source
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join(".");
    (!module.is_empty()).then_some(module)
}

fn source_directory(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let directory = relative
        .parent()?
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_owned();
    (!directory.is_empty()).then_some(directory)
}

fn typescript_project_importer_key(source_file: &str, root: &Path) -> Option<String> {
    let relative = typescript_relative_path(source_file, root)?;
    let key = relative.to_str()?.to_owned();
    (!key.is_empty() && key.len() <= 4_096).then_some(key)
}

fn typescript_project_module_keys(
    project_modules: &TypeScriptProjectModuleIndex,
    importer: &str,
    module: &str,
    root: &Path,
) -> Option<Vec<String>> {
    let importer = typescript_project_importer_key(importer, root)?;
    project_modules
        .get(&(importer, module.to_owned()))
        .filter(|keys| !keys.is_empty())
        .cloned()
}

fn typescript_project_module_index(
    edges: &[EdgeRecord],
    inventory_nodes: &[NodeRecord],
    root: &Path,
    entry_limit: usize,
    candidates_per_lookup: usize,
) -> Result<TypeScriptProjectModuleIndex, String> {
    if edges.is_empty() {
        return Ok(TypeScriptProjectModuleIndex::new());
    }
    let mut source_by_node = BTreeMap::<String, BTreeSet<String>>::new();
    for node in inventory_nodes {
        let source = node.string("source_file");
        if !source.is_empty() {
            source_by_node
                .entry(node.id.clone())
                .or_default()
                .insert(source);
        }
    }
    let max_targets = candidate_storage_limit(candidates_per_lookup);
    let mut targets = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for edge in edges {
        if edge.attributes.get("relation").and_then(Value::as_str) != Some("imports_from") {
            continue;
        }
        let module = edge.string("module");
        if module.is_empty() || module.len() > 4_096 || module.contains(['\\', '\0']) {
            continue;
        }
        let importer = {
            let source = edge.string("source_file");
            if source.is_empty() {
                unique_inventory_source(&source_by_node, &edge.source)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                source
            }
        };
        let Some(importer) = typescript_project_importer_key(&importer, root) else {
            continue;
        };
        let target_source = {
            let source = edge.string("target_file");
            if source.is_empty() {
                unique_inventory_source(&source_by_node, &edge.target)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                source
            }
        };
        let Some(target_keys) = (!target_source.is_empty())
            .then(|| typescript_source_module_keys(&target_source, root))
            .filter(|keys| !keys.is_empty())
        else {
            continue;
        };
        let key = (importer, module);
        if !targets.contains_key(&key) && targets.len() >= entry_limit {
            return Err(format!(
                "TypeScript project module entry count exceeds limit {entry_limit}"
            ));
        }
        let values = targets.entry(key).or_default();
        for target_key in target_keys {
            if values.contains(&target_key) || values.len() < max_targets {
                values.insert(target_key);
            }
        }
    }
    Ok(targets
        .into_iter()
        .filter_map(|(key, values)| {
            (!values.is_empty()).then_some((key, values.into_iter().collect()))
        })
        .collect())
}

fn unique_inventory_source<'a>(
    source_by_node: &'a BTreeMap<String, BTreeSet<String>>,
    node_id: &str,
) -> Option<&'a str> {
    let sources = source_by_node.get(node_id)?;
    let mut values = sources.iter();
    let source = values.next()?;
    values.next().is_none().then_some(source.as_str())
}

fn typescript_module_indices(
    declarations: &AHashMap<String, DeclarationFact>,
    declaration_ids: &[String],
    bindings: &AHashMap<String, BindingFact>,
    scopes: &AHashMap<String, compass_languages::ScopeFact>,
    root: &Path,
    project_modules: &TypeScriptProjectModuleIndex,
) -> (
    TypeScriptModuleIndex,
    TypeScriptModuleIndex,
    TypeScriptReexportIndex,
) {
    let mut modules = TypeScriptModuleIndex::new();
    for declaration in declarations
        .values()
        .filter(|declaration| matches!(declaration.language.as_str(), "typescript" | "javascript"))
    {
        let Some(slot) = declaration_slot(declaration_ids, &declaration.id) else {
            continue;
        };
        for module in typescript_source_module_keys(&declaration.range.source_file, root) {
            modules
                .entry((
                    declaration.language.clone(),
                    module.clone(),
                    declaration.name.clone(),
                ))
                .or_default()
                .push(slot);
            if declaration.kind == "module" {
                modules
                    .entry((declaration.language.clone(), module, "module".to_owned()))
                    .or_default()
                    .push(slot);
            }
        }
    }

    let mut export_aliases = TypeScriptModuleIndex::new();
    for binding in bindings.values().filter(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport
            && binding.target_declaration_id.is_some()
    }) {
        let Some(target) = binding.target_declaration_id.as_deref() else {
            continue;
        };
        let Some(slot) = declaration_slot(declaration_ids, target) else {
            continue;
        };
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| declarations.get(declaration_id))
        else {
            continue;
        };
        if !matches!(owner.language.as_str(), "typescript" | "javascript") {
            continue;
        }
        for module in typescript_source_module_keys(&owner.range.source_file, root) {
            export_aliases
                .entry((owner.language.clone(), module, binding.spelling.clone()))
                .or_default()
                .push(slot);
        }
    }
    let mut reexport_targets = TypeScriptReexportIndex::new();
    for binding in bindings.values().filter(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport
            && binding.target_declaration_id.is_none()
    }) {
        let Some((target_module, target_export)) = binding.qualified_target.rsplit_once("::")
        else {
            continue;
        };
        if target_module.is_empty() || target_export.is_empty() {
            continue;
        }
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| declarations.get(declaration_id))
        else {
            continue;
        };
        if !matches!(owner.language.as_str(), "typescript" | "javascript") {
            continue;
        }
        let target_modules = typescript_project_module_keys(
            project_modules,
            &owner.range.source_file,
            target_module,
            root,
        )
        .unwrap_or_else(|| {
            typescript_import_module_keys(&owner.range.source_file, target_module, root)
        });
        if target_modules.is_empty() {
            continue;
        }
        for owner_module in typescript_source_module_keys(&owner.range.source_file, root) {
            let targets = reexport_targets
                .entry((
                    owner.language.clone(),
                    owner_module,
                    binding.spelling.clone(),
                ))
                .or_default();
            for target_module in &target_modules {
                targets.push(TypeScriptReexportTarget {
                    module: target_module.clone(),
                    exported: target_export.to_owned(),
                });
            }
        }
    }
    for targets in reexport_targets.values_mut() {
        targets.sort_unstable_by(|left, right| {
            left.module
                .cmp(&right.module)
                .then_with(|| left.exported.cmp(&right.exported))
        });
        targets.dedup();
    }
    for index in [&mut modules, &mut export_aliases] {
        sort_declaration_index(index, declaration_ids, usize::MAX);
    }
    (modules, export_aliases, reexport_targets)
}

fn typescript_source_module_keys(source_file: &str, root: &Path) -> Vec<String> {
    let Some(relative) = typescript_relative_path(source_file, root) else {
        return Vec::new();
    };
    let Some(module) = typescript_module_stem(&relative) else {
        return Vec::new();
    };
    let mut keys = vec![module.clone()];
    if relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("index"))
        && let Some(parent) = module.rsplit_once('/').map(|(parent, _)| parent)
    {
        if !parent.is_empty() {
            keys.push(parent.to_owned());
        }
    } else if relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("index"))
    {
        keys.push(String::new());
    }
    keys.sort();
    keys.dedup();
    keys.retain(|key| !key.is_empty());
    keys
}

fn typescript_import_module_keys(importer: &str, module: &str, root: &Path) -> Vec<String> {
    let mut keys = Vec::new();
    if module.starts_with('.') {
        if let Some(importer_path) = typescript_relative_path(importer, root) {
            let base = importer_path.parent().unwrap_or_else(|| Path::new(""));
            if let Some(joined) = normalize_relative_module_path(&base.join(module))
                && let Some(key) = typescript_module_stem(&joined)
            {
                keys.push(key);
                if joined
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("index"))
                    && let Some(parent) = keys[0].rsplit_once('/').map(|(parent, _)| parent)
                    && !parent.is_empty()
                {
                    keys.push(parent.to_owned());
                }
            }
        }
    } else if !module.is_empty() && !module.starts_with('#') {
        let path = Path::new(module);
        if let Some(key) = typescript_module_stem(path) {
            keys.push(key);
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn typescript_relative_path(source_file: &str, root: &Path) -> Option<std::path::PathBuf> {
    let path = Path::new(source_file);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    normalize_relative_module_path(&relative)
}

fn normalize_relative_module_path(path: &Path) -> Option<std::path::PathBuf> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop()?;
            }
            std::path::Component::Normal(value) => components.push(value.to_owned()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    let mut normalized = std::path::PathBuf::new();
    for component in components {
        normalized.push(component);
    }
    Some(normalized)
}

fn typescript_module_stem(path: &Path) -> Option<String> {
    let mut value = path.to_string_lossy().replace('\\', "/");
    value = value.trim_start_matches("./").to_owned();
    for extension in [
        ".d.mts", ".d.cts", ".d.ts", ".mts", ".cts", ".tsx", ".jsx", ".mjs", ".cjs", ".ts", ".js",
    ] {
        if let Some(stem) = value.strip_suffix(extension) {
            value = stem.to_owned();
            break;
        }
    }
    while value.ends_with('/') {
        value.pop();
    }
    (!value.is_empty()
        && !value.starts_with('/')
        && !value
            .split('/')
            .any(|component| component.is_empty() || component == ".."))
    .then_some(value)
}

fn read_go_module_path(root: &Path) -> Option<String> {
    const MAX_GO_MOD_BYTES: u64 = 1024 * 1024;
    let source = compass_files::read_source_lossy(&root.join("go.mod"), MAX_GO_MOD_BYTES).ok()?;
    source.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "module" {
            return None;
        }
        let module = fields.next()?;
        if fields.next().is_some()
            || module.len() > 4096
            || module.starts_with('.')
            || module.contains(['\\', '\0'])
            || module
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return None;
        }
        Some(module.to_owned())
    })
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

fn rust_impl_associated_type_index(
    declarations: &AHashMap<String, DeclarationFact>,
    declaration_ids: &[String],
    scopes: &AHashMap<String, compass_languages::ScopeFact>,
    candidates: &AHashMap<String, RelationshipCandidate>,
    occurrences: &AHashMap<String, OccurrenceFact>,
    candidate_limit: usize,
) -> AHashMap<(String, String, String), AssociatedTypeSet> {
    let mut implementations = AHashMap::<String, Vec<&RelationshipCandidate>>::new();
    for candidate in candidates.values().filter(|candidate| {
        candidate.language == "rust" && candidate.relation == CandidateRelation::Implements
    }) {
        implementations
            .entry(candidate.source_declaration_id.clone())
            .or_default()
            .push(candidate);
    }

    let mut index = AHashMap::<(String, String, String), AssociatedTypeSet>::new();
    for scope in scopes
        .values()
        .filter(|scope| scope.language == "rust" && scope.kind == "impl")
    {
        let Some(owner_id) = scope.owner_declaration_id.as_ref() else {
            continue;
        };
        let Some(_receiver) = declarations.get(owner_id) else {
            continue;
        };
        let traits = implementations
            .get(owner_id)
            .into_iter()
            .flat_map(|values| values.iter())
            .filter_map(|candidate| {
                let occurrence = candidate
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| occurrences.get(id))?;
                range_contains(&scope.range, &occurrence.range)
                    .then_some(candidate.constraints.qualified_name.as_ref())
                    .flatten()
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let [trait_name] = traits.as_slice() else {
            continue;
        };
        for declaration in declarations.values().filter(|declaration| {
            declaration.language == "rust"
                && declaration.kind == "type_alias"
                && declaration.scope_id.as_deref() == Some(scope.id.as_str())
        }) {
            let Some(slot) = declaration_slot(declaration_ids, &declaration.id) else {
                continue;
            };
            let associated = index
                .entry((
                    owner_id.clone(),
                    (*trait_name).clone(),
                    declaration.name.clone(),
                ))
                .or_insert_with(|| AssociatedTypeSet {
                    declarations: Vec::new(),
                    complete: true,
                });
            if associated.declarations.len() < candidate_limit {
                associated.declarations.push(slot);
            } else {
                associated.complete = false;
            }
        }
    }
    for associated in index.values_mut() {
        associated.declarations.sort_unstable_by(|left, right| {
            declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
        });
        associated.declarations.dedup();
        if associated.declarations.len() > candidate_limit {
            associated.complete = false;
            associated.declarations.truncate(candidate_limit);
        }
    }
    index
}

fn rust_impl_associated_trait_name_index(
    associated_types: &AHashMap<(String, String, String), AssociatedTypeSet>,
    candidate_limit: usize,
) -> AHashMap<(String, String), WildcardModuleSet> {
    let mut index = AHashMap::<(String, String), WildcardModuleSet>::new();
    for (receiver, trait_name, associated_name) in associated_types.keys() {
        let entry = index
            .entry((receiver.clone(), associated_name.clone()))
            .or_insert_with(|| WildcardModuleSet {
                modules: Vec::new(),
                complete: true,
            });
        if entry.modules.len() < candidate_limit {
            entry.modules.push(trait_name.clone());
        } else {
            entry.complete = false;
        }
    }
    for traits in index.values_mut() {
        traits.modules.sort_unstable();
        traits.modules.dedup();
        if traits.modules.len() > candidate_limit {
            traits.complete = false;
            traits.modules.truncate(candidate_limit);
        }
    }
    index
}

fn rust_impl_trait_index(
    candidates: &AHashMap<String, RelationshipCandidate>,
    candidate_limit: usize,
) -> AHashMap<(String, String), RustImplTraitSet> {
    let mut index = AHashMap::<(String, String), RustImplTraitSet>::new();
    for candidate in candidates.values().filter(|candidate| {
        candidate.language == "rust" && candidate.relation == CandidateRelation::Implements
    }) {
        let Some(raw_trait_name) = candidate.constraints.qualified_name.as_ref() else {
            continue;
        };
        let entry = index
            .entry((
                candidate.source_declaration_id.clone(),
                raw_trait_name.clone(),
            ))
            .or_insert_with(|| RustImplTraitSet {
                candidate_ids: Vec::new(),
                complete: true,
            });
        if entry.candidate_ids.len() < candidate_limit {
            entry.candidate_ids.push(candidate.id.clone());
        } else {
            entry.complete = false;
        }
    }
    for traits in index.values_mut() {
        traits.candidate_ids.sort_unstable();
        traits.candidate_ids.dedup();
        if traits.candidate_ids.len() > candidate_limit {
            traits.complete = false;
            traits.candidate_ids.truncate(candidate_limit);
        }
    }
    index
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

fn declaration_node(
    declaration: &DeclarationFact,
    definition_range: Option<&EvidenceRange>,
    graph_node_id: &str,
) -> NodeRecord {
    let label = match declaration.kind.as_str() {
        "function" => format!("{}()", declaration.name),
        "method" => format!(".{}()", declaration.name),
        _ => declaration.name.clone(),
    };
    let callable = matches!(declaration.kind.as_str(), "function" | "method");
    let mut attributes = Map::from_iter([
        ("label".to_owned(), Value::String(label)),
        (
            "qualified_name".to_owned(),
            Value::String(declaration.qualified_name.clone()),
        ),
        (
            "symbol_kind".to_owned(),
            Value::String(declaration.kind.clone()),
        ),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        (
            "source_file".to_owned(),
            Value::String(declaration.range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", declaration.range.start_line)),
        ),
        (
            "start_byte".to_owned(),
            Value::from(declaration.range.start_byte),
        ),
        (
            "end_byte".to_owned(),
            Value::from(declaration.range.end_byte),
        ),
        (
            "line_start".to_owned(),
            Value::from(declaration.range.start_line),
        ),
        (
            "line_end".to_owned(),
            Value::from(declaration.range.end_line),
        ),
        (
            "column_start".to_owned(),
            Value::from(declaration.range.start_column),
        ),
        (
            "column_end".to_owned(),
            Value::from(declaration.range.end_column),
        ),
        (
            "language".to_owned(),
            Value::String(declaration.language.clone()),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!(
                "compass.languages.{}.universal",
                declaration.language
            )),
        ),
        (
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        ),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "evidence_declaration_id".to_owned(),
            Value::String(declaration.id.clone()),
        ),
    ]);
    if callable {
        attributes.insert("_callable".to_owned(), Value::Bool(true));
    }
    for (key, value) in [
        ("signature", declaration.signature.as_ref()),
        ("signature_hash", declaration.signature_hash.as_ref()),
        (
            "implementation_hash",
            declaration.implementation_hash.as_ref(),
        ),
        ("source_hash", declaration.source_hash.as_ref()),
    ] {
        if let Some(value) = value {
            attributes.insert(key.to_owned(), Value::String(value.clone()));
        }
    }
    if let Some(module_or_package) = declaration.module_or_package.as_ref() {
        attributes.insert(
            "module".to_owned(),
            Value::String(module_or_package.clone()),
        );
    }
    if let Some(definition_range) = definition_range.filter(|range| *range != &declaration.range) {
        attributes.insert(
            "source_anchor".to_owned(),
            source_anchor_value(definition_range),
        );
        attributes.insert(
            NODE_PROVENANCE_ANCHOR_ATTRIBUTE.to_owned(),
            source_anchor_value(&declaration.range),
        );
    }
    NodeRecord {
        id: graph_node_id.to_owned(),
        attributes,
    }
}

fn source_anchor_value(range: &EvidenceRange) -> Value {
    serde_json::json!({
        "file": range.source_file,
        "startByte": range.start_byte,
        "endByte": range.end_byte,
        "startLine": range.start_line,
        "startColumn": range.start_column,
        "endLine": range.end_line,
        "endColumn": range.end_column,
    })
}

fn relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls | CandidateRelation::Constructs => "calls",
        CandidateRelation::IndirectCalls => "indirect_call",
        CandidateRelation::Decorates => "references",
        CandidateRelation::Annotates | CandidateRelation::References => "references",
        CandidateRelation::TypeOf => "type_of",
        CandidateRelation::Returns => "returns",
        CandidateRelation::Extends => "inherits",
        CandidateRelation::Implements => "implements",
        CandidateRelation::AccessesMember => "accesses",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embeds",
        CandidateRelation::Imports => "imports_from",
        CandidateRelation::Reexports => "re_exports",
        CandidateRelation::InvokesMacro => "references",
        CandidateRelation::Tests => "tests",
    }
}

fn external_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    let role = relation_name(candidate.relation);
    let attributes = Map::from_iter([
        (
            "label".to_owned(),
            Value::String(
                qualified_name
                    .rsplit(['.', '/'])
                    .next()
                    .unwrap_or(qualified_name)
                    .to_owned(),
            ),
        ),
        (
            "qualified_name".to_owned(),
            Value::String(qualified_name.to_owned()),
        ),
        ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        ("source_file".to_owned(), Value::String(String::new())),
        ("source_location".to_owned(), Value::String(String::new())),
        ("language".to_owned(), Value::String(language.to_owned())),
        ("external_role".to_owned(), Value::String(role.to_owned())),
        (
            "external_roles".to_owned(),
            Value::Array(vec![Value::String(role.to_owned())]),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "confidence".to_owned(),
            Value::String(
                if candidate.binding_id.is_some() {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                }
                .to_owned(),
            ),
        ),
        ("external".to_owned(), Value::Bool(true)),
        ("placeholder".to_owned(), Value::Bool(true)),
        ("_canonical_external_symbol".to_owned(), Value::Bool(true)),
    ]);
    NodeRecord {
        id: id.to_owned(),
        attributes,
    }
}

fn deferred_receiver_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    NodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            (
                "label".to_owned(),
                Value::String(
                    qualified_name
                        .rsplit([':', '.'])
                        .find(|component| !component.is_empty())
                        .unwrap_or(qualified_name)
                        .to_owned(),
                ),
            ),
            (
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            ),
            ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("source_file".to_owned(), Value::String(String::new())),
            ("source_location".to_owned(), Value::String(String::new())),
            ("language".to_owned(), Value::String(language.to_owned())),
            (
                "extractor".to_owned(),
                Value::String(format!("compass.resolve.{language}.universal")),
            ),
            (
                "confidence".to_owned(),
                Value::String("INFERRED".to_owned()),
            ),
            ("external".to_owned(), Value::Bool(false)),
            ("placeholder".to_owned(), Value::Bool(true)),
            ("deferred_receiver".to_owned(), Value::Bool(true)),
            (
                "deferred_role".to_owned(),
                Value::String(relation_name(candidate.relation).to_owned()),
            ),
        ]),
    }
}

fn is_deferred_receiver(qualifier: &str) -> bool {
    !qualifier.contains("::") && !qualifier.contains('/')
}

fn merge_external_node(node: &mut NodeRecord, candidate: &RelationshipCandidate) {
    let incoming_kind = external_kind(candidate);
    let current_kind = node.string("symbol_kind");
    if external_kind_rank(incoming_kind) > external_kind_rank(&current_kind) {
        node.attributes.insert(
            "symbol_kind".to_owned(),
            Value::String(incoming_kind.to_owned()),
        );
        node.attributes.insert(
            "external_role".to_owned(),
            Value::String(relation_name(candidate.relation).to_owned()),
        );
    }
    let incoming_role = relation_name(candidate.relation);
    let mut roles = node
        .attributes
        .get("external_roles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    roles.insert(incoming_role.to_owned());
    node.attributes.insert(
        "external_roles".to_owned(),
        Value::Array(roles.into_iter().map(Value::String).collect()),
    );
}

fn external_kind(candidate: &RelationshipCandidate) -> &'static str {
    match candidate.relation {
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "export",
        CandidateRelation::Extends => "class",
        CandidateRelation::Annotates
        | CandidateRelation::Embeds
        | CandidateRelation::TypeOf
        | CandidateRelation::Returns => "type_alias",
        CandidateRelation::Implements => "interface",
        CandidateRelation::AccessesMember => "variable",
        CandidateRelation::Calls | CandidateRelation::IndirectCalls => "function",
        CandidateRelation::Constructs => "class",
        CandidateRelation::Decorates => "function",
        CandidateRelation::References => reference_external_kind(candidate),
        CandidateRelation::Contains | CandidateRelation::Owns => "variable",
        CandidateRelation::InvokesMacro => "macro",
        CandidateRelation::Tests => "function",
    }
}

fn reference_external_kind(candidate: &RelationshipCandidate) -> &'static str {
    let allowed = &candidate.constraints.allowed_target_kinds;
    if allowed.is_empty() {
        return "variable";
    }
    let type_only = allowed.iter().all(|kind| {
        matches!(
            kind.as_str(),
            "class" | "struct" | "enum" | "interface" | "trait" | "type_alias" | "parameter"
        )
    });
    if !type_only {
        return "variable";
    }
    if allowed
        .iter()
        .all(|kind| matches!(kind.as_str(), "interface" | "trait" | "parameter"))
        && allowed
            .iter()
            .any(|kind| matches!(kind.as_str(), "interface" | "trait"))
    {
        "interface"
    } else {
        "type_alias"
    }
}

fn external_kind_rank(kind: &str) -> u8 {
    match kind {
        "interface" => 7,
        "class" => 6,
        "type_alias" => 5,
        "function" => 4,
        "variable" => 3,
        "import" => 2,
        "export" => 1,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn materialized_edge(
    source: String,
    target: String,
    relation: &str,
    candidate: &RelationshipCandidate,
    occurrence: Option<&OccurrenceFact>,
    binding: Option<&BindingFact>,
    target_kind: Option<&str>,
    target_source_file: Option<&str>,
    range: &compass_languages::EvidenceRange,
    resolution_rule: ResolutionRule,
    language: &str,
) -> EdgeRecord {
    let context = match (relation, resolution_rule) {
        ("calls", ResolutionRule::QualifiedExternal) => "external_call",
        ("calls", ResolutionRule::DeferredReceiver) => "deferred_receiver_call",
        ("calls", _) => "call",
        ("indirect_call", _) => occurrence
            .and_then(|occurrence| occurrence.context.as_deref())
            .unwrap_or("reference"),
        ("references", _) if candidate.relation == CandidateRelation::Decorates => "decorator",
        ("references", _)
            if occurrence.is_some_and(|occurrence| {
                occurrence.role == compass_languages::SemanticRole::CallableReference
            }) =>
        {
            occurrence
                .and_then(|occurrence| occurrence.context.as_deref())
                .unwrap_or("reference")
        }
        ("imports_from", _)
            if candidate.relation == CandidateRelation::Imports && target_kind == Some("file") =>
        {
            "submodule_import"
        }
        ("imports_from", _) => "import",
        ("re_exports", _) => "export",
        ("inherits", _) => "base_type",
        ("references", _) => "type_reference",
        ("embeds", _) => "embedding",
        ("method", _) => "receiver",
        _ => "",
    };
    let confidence = if matches!(
        resolution_rule,
        ResolutionRule::QualifiedExternal
            | ResolutionRule::DeferredReceiver
            | ResolutionRule::ClosedWorldReceiverDispatch
            | ResolutionRule::IncompleteHierarchyReceiverDispatch
    ) {
        "INFERRED"
    } else {
        "EXTRACTED"
    };
    let producer_rule = format!(
        "universal-{}-{}",
        candidate_relation_name(candidate.relation),
        resolution_rule_name(resolution_rule)
    );
    let occurrence_rule = binding.map_or_else(
        || producer_rule.clone(),
        |binding| {
            // Import aliases can share one statement anchor and one resolved
            // endpoint. Keep those occurrences distinct without using the
            // candidate ID, whose absolute byte position changes when the
            // statement moves. Binding offsets within the statement remain
            // portable across checkouts and source relocation.
            format!(
                "{producer_rule}:binding:{}:{}:{}",
                binding.spelling,
                binding.range.start_byte.saturating_sub(range.start_byte),
                binding.range.end_byte.saturating_sub(range.start_byte)
            )
        },
    );
    let mut attributes = Map::from_iter([
        ("relation".to_owned(), Value::String(relation.to_owned())),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "confidence".to_owned(),
            Value::String(confidence.to_owned()),
        ),
        (
            "source_file".to_owned(),
            Value::String(range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", range.start_line)),
        ),
        ("start_byte".to_owned(), Value::from(range.start_byte)),
        ("end_byte".to_owned(), Value::from(range.end_byte)),
        ("line_start".to_owned(), Value::from(range.start_line)),
        ("line_end".to_owned(), Value::from(range.end_line)),
        ("column_start".to_owned(), Value::from(range.start_column)),
        ("column_end".to_owned(), Value::from(range.end_column)),
        ("weight".to_owned(), Value::from(1.0)),
        ("language".to_owned(), Value::String(language.to_owned())),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "resolution_rule".to_owned(),
            Value::String(resolution_rule_name(resolution_rule).to_owned()),
        ),
        ("rule".to_owned(), Value::String(producer_rule)),
        (
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(occurrence_rule),
        ),
        (
            "evidence_candidate_id".to_owned(),
            Value::String(candidate.id.clone()),
        ),
    ]);
    if let Some(occurrence_id) = candidate.occurrence_id.as_ref() {
        attributes.insert(
            "evidence_occurrence_id".to_owned(),
            Value::String(occurrence_id.clone()),
        );
    }
    if matches!(
        candidate.relation,
        CandidateRelation::Imports | CandidateRelation::Reexports
    ) && let Some(binding) = binding
    {
        attributes.insert(
            "local_name".to_owned(),
            Value::String(binding.spelling.clone()),
        );
        attributes.insert(
            "imported_name".to_owned(),
            Value::String(candidate.target_spelling.clone()),
        );
        attributes.insert(
            "qualified_target".to_owned(),
            Value::String(binding.qualified_target.clone()),
        );
        attributes.insert(
            "binding_kind".to_owned(),
            Value::String(binding_kind_name(binding.kind).to_owned()),
        );
        if let Some(module) = candidate.constraints.module_or_package.as_ref() {
            attributes.insert(
                "module".to_owned(),
                Value::String(import_module_for_edge(language, &range.source_file, module)),
            );
        }
    }
    if !context.is_empty() {
        attributes.insert("context".to_owned(), Value::String(context.to_owned()));
    }
    if let Some(target_source_file) = target_source_file {
        attributes.insert(
            "target_file".to_owned(),
            Value::String(target_source_file.to_owned()),
        );
    }
    if let Some(binding) = binding {
        attributes.extend([
            (
                "binding_name".to_owned(),
                Value::String(binding.spelling.clone()),
            ),
            (
                "binding_qualified_target".to_owned(),
                Value::String(binding.qualified_target.clone()),
            ),
        ]);
    }
    EdgeRecord {
        source,
        target,
        attributes,
    }
}

fn binding_kind_name(kind: compass_languages::BindingKind) -> &'static str {
    match kind {
        compass_languages::BindingKind::Import => "import",
        compass_languages::BindingKind::ImportAlias => "import_alias",
        compass_languages::BindingKind::Reexport => "reexport",
        compass_languages::BindingKind::LocalAlias => "local_alias",
        compass_languages::BindingKind::CallResult => "call_result",
        compass_languages::BindingKind::Package => "package",
        compass_languages::BindingKind::Member => "member",
    }
}

fn import_module_for_edge(language: &str, source_file: &str, module: &str) -> String {
    if language != "python" {
        return module.to_owned();
    }
    let source_package = Path::new(source_file)
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .filter(|component| !component.is_empty() && *component != ".")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let module_components = module
        .split('.')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let shared = source_package
        .iter()
        .zip(&module_components)
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 {
        return module.to_owned();
    }
    let upward = source_package.len().saturating_sub(shared);
    let suffix = module_components[shared..].join(".");
    format!("{}{}", ".".repeat(upward.saturating_add(1)), suffix)
}

const fn candidate_relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls => "call",
        CandidateRelation::IndirectCalls => "indirect-call",
        CandidateRelation::Constructs => "construction",
        CandidateRelation::Decorates => "decorator",
        CandidateRelation::Annotates => "annotation",
        CandidateRelation::Extends => "extends",
        CandidateRelation::Implements => "implements",
        CandidateRelation::References => "reference",
        CandidateRelation::TypeOf => "type-of",
        CandidateRelation::Returns => "return-type",
        CandidateRelation::AccessesMember => "member-access",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embedding",
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "reexport",
        CandidateRelation::InvokesMacro => "macro-invocation",
        CandidateRelation::Tests => "test-call",
    }
}

const fn resolution_rule_name(rule: ResolutionRule) -> &'static str {
    match rule {
        ResolutionRule::ExactSourceDeclaration => "exact-source-declaration",
        ResolutionRule::ExactLexicalDeclaration => "exact-lexical-declaration",
        ResolutionRule::ExplicitBinding => "explicit-binding",
        ResolutionRule::ProjectModuleBinding => "project-module-binding",
        ResolutionRule::MemberBinding => "member-binding",
        ResolutionRule::DeferredReceiver => "deferred-receiver",
        ResolutionRule::WildcardBinding => "wildcard-binding",
        ResolutionRule::UniqueModuleOrPackage => "unique-module-or-package",
        ResolutionRule::ExactHierarchyBase => "exact-hierarchy-base",
        ResolutionRule::DirectReceiverSuccessorDispatch => "direct-receiver-successor-dispatch",
        ResolutionRule::LinearizedReceiverDispatch => "linearized-receiver-dispatch",
        ResolutionRule::ClosedWorldReceiverDispatch => "closed-world-receiver-dispatch",
        ResolutionRule::IncompleteHierarchyReceiverDispatch => {
            "incomplete-hierarchy-receiver-dispatch"
        }
        ResolutionRule::RustAssociatedType => "rust-associated-type",
        ResolutionRule::ExactSourceInventory => "exact-source-inventory",
        ResolutionRule::QualifiedExternal => "qualified-external",
    }
}

#[must_use]
pub(crate) fn is_replaced_relation(relation: &str) -> bool {
    matches!(
        relation,
        "contains"
            | "method"
            | "calls"
            | "indirect_call"
            | "imports"
            | "imports_from"
            | "re_exports"
            | "inherits"
            | "implements"
            | "references"
            | "embeds"
            | "decorated_by"
            | "owns"
            | "accesses"
    )
}
