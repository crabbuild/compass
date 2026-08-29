//! Deterministic cross-file resolution over immutable extraction facts.

pub mod evidence;
pub mod frameworks;
mod members;
mod program;

pub use evidence::universal_resolution_report;
pub use members::resolve_language_calls;
pub use program::{
    ProgramProjectionSites, apply_program_projection, collect_program_projection_sites,
};

/// Maximum inference that cross-file resolution may materialize.
///
/// This mirrors the graph publication levels at the resolver ownership
/// boundary so discarded relationships do not first allocate full graph
/// records. Existing resolver entry points retain their historical `Max`
/// behavior; build orchestration opts into an explicit level.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResolutionAdmission {
    /// Resolve relationships backed by exact structural evidence only.
    Low,
    /// Also admit inferred relationships between source-backed declarations.
    Medium,
    /// Also admit explicitly qualified external relationships.
    High,
    /// Admit deferred receivers and every other bounded inference.
    #[default]
    Max,
}

impl ResolutionAdmission {
    const fn admits_source_backed_inference(self) -> bool {
        self as u8 >= Self::Medium as u8
    }

    const fn admits_qualified_external(self) -> bool {
        self as u8 >= Self::High as u8
    }

    const fn admits_deferred_receiver(self) -> bool {
        matches!(self, Self::Max)
    }
}

#[derive(Clone, Copy)]
struct ResolutionMode {
    evidence_prevalidated: bool,
    admission: ResolutionAdmission,
}

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use compass_languages::{
    CandidateRelation, Extraction, RawCall, RawEdgeRecord as EdgeRecord,
    RawNodeRecord as NodeRecord, SemanticEvidenceBatch, SemanticRole, file_stem,
    is_language_builtin_global, make_id, parse_jsonc,
};
use compass_model::code_graph::{DiagnosticSeverity, GraphDiagnostic};
use compass_model::provenance::{
    EndpointRewriteEvidence, EndpointRewriteRule, append_endpoint_rewrite_evidence,
    preserve_occurrence_rule,
};
use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

const DECLARATION_SUFFIXES: &[&str] = &["h", "hpp", "hh", "hxx"];
const IMPLEMENTATION_SUFFIXES: &[&str] = &["m", "mm", "cpp", "cc", "cxx", "c"];
const GRAPH_DIAGNOSTICS_EXTENSION: &str = "_compass_v1_graph_diagnostics";

/// Collapse a clean sibling header/implementation declaration pair before
/// portable file-prefix remapping would split their shared symbol IDs.
///
/// This mirrors Compass's collection-level C/C++/Objective-C pass. Only an
/// ID collision from one directory/base-stem family with exactly one header
/// is eligible; every other collision is left for conservative disambiguation.
pub fn merge_decl_def_classes(extractions: &mut [Extraction]) {
    merge_decl_def_classes_changed(extractions);
}

fn merge_decl_def_classes_changed(extractions: &mut [Extraction]) -> bool {
    let has_declaration_definition = extractions
        .iter()
        .flat_map(|extraction| &extraction.nodes)
        .any(|node| {
            node.attributes.get("file_type").and_then(Value::as_str) == Some("code")
                && node
                    .attributes
                    .get("source_file")
                    .and_then(Value::as_str)
                    .is_some_and(|source| {
                        is_declaration_source(source) || is_implementation_source(source)
                    })
        });
    if !has_declaration_definition {
        return false;
    }

    let mut groups = HashMap::<String, Vec<(usize, usize, String)>>::new();
    for (extraction_index, extraction) in extractions.iter().enumerate() {
        for (node_index, node) in extraction.nodes.iter().enumerate() {
            let source = string_attribute(node, "source_file");
            if node.id.is_empty()
                || source.is_empty()
                || string_attribute(node, "file_type") != "code"
                || (!is_declaration_source(&source) && !is_implementation_source(&source))
            {
                continue;
            }
            groups
                .entry(node.id.clone())
                .or_default()
                .push((extraction_index, node_index, source));
        }
    }

    let mut dropped = HashSet::<(usize, usize)>::new();
    let mut definition_hashes = Vec::<((usize, usize), Vec<(String, Value)>)>::new();
    for entries in groups.values().filter(|entries| entries.len() > 1) {
        let mut sibling_keys = HashSet::new();
        let mut headers = Vec::new();
        let mut eligible = true;
        for &(extraction_index, node_index, ref source) in entries {
            if !is_declaration_source(source) && !is_implementation_source(source) {
                eligible = false;
                break;
            }
            let path = Path::new(source);
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .split('+')
                .next()
                .unwrap_or_default();
            if stem.is_empty() {
                eligible = false;
                break;
            }
            sibling_keys.insert((
                path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
                stem.to_owned(),
            ));
            if is_declaration_source(source) {
                headers.push((extraction_index, node_index));
            }
        }
        if eligible && sibling_keys.len() == 1 && headers.len() == 1 {
            let keeper = headers[0];
            if let Some((extraction_index, node_index, _)) = entries
                .iter()
                .filter(|(extraction_index, node_index, source)| {
                    is_implementation_source(source)
                        && extractions[*extraction_index].nodes[*node_index]
                            .attributes
                            .contains_key("implementation_hash")
                })
                .min_by_key(|(_, _, source)| source)
            {
                let definition = &extractions[*extraction_index].nodes[*node_index];
                let hashes = [
                    "_callable",
                    "signature_hash",
                    "implementation_hash",
                    "source_hash",
                ]
                .into_iter()
                .filter_map(|key| {
                    definition
                        .attributes
                        .get(key)
                        .cloned()
                        .map(|value| (key.to_owned(), value))
                })
                .collect::<Vec<_>>();
                definition_hashes.push((keeper, hashes));
            }
            dropped.extend(
                entries
                    .iter()
                    .map(|(extraction, node, _)| (*extraction, *node))
                    .filter(|coordinate| *coordinate != keeper),
            );
        }
    }
    if dropped.is_empty() {
        return false;
    }

    for ((extraction_index, node_index), hashes) in definition_hashes {
        extractions[extraction_index].nodes[node_index]
            .attributes
            .extend(hashes);
    }

    for (extraction_index, extraction) in extractions.iter_mut().enumerate() {
        let mut node_index = 0_usize;
        extraction.nodes.retain(|_| {
            let keep = !dropped.contains(&(extraction_index, node_index));
            node_index += 1;
            keep
        });
    }
    let mut seen_edges = HashSet::new();
    for extraction in extractions {
        extraction.edges.retain(|edge| {
            edge.source != edge.target
                && seen_edges.insert((
                    edge.source.clone(),
                    edge.target.clone(),
                    relation(edge).to_owned(),
                    edge.string("context"),
                ))
        });
    }
    true
}

/// Run declaration/definition merging only when the current source set can
/// contain a native header/implementation pair.
pub fn merge_decl_def_classes_if_needed(extractions: &mut [Extraction], sources: &[PathBuf]) {
    merge_decl_def_classes_if_needed_changed(extractions, sources);
}

/// Merge declaration/definition classes when eligible and report whether facts changed.
///
/// This lets orchestration reuse pre-merge derived state when a corpus merely
/// contains native-looking files but has no mergeable declaration family.
pub fn merge_decl_def_classes_if_needed_changed(
    extractions: &mut [Extraction],
    sources: &[PathBuf],
) -> bool {
    if !sources.iter().any(|source| {
        source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                DECLARATION_SUFFIXES
                    .iter()
                    .chain(IMPLEMENTATION_SUFFIXES)
                    .any(|suffix| extension.eq_ignore_ascii_case(suffix))
            })
    }) {
        return false;
    }
    merge_decl_def_classes_changed(extractions)
}

fn is_declaration_source(source: &str) -> bool {
    let Some(extension) = Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    DECLARATION_SUFFIXES
        .iter()
        .any(|suffix| extension.eq_ignore_ascii_case(suffix))
}

fn is_implementation_source(source: &str) -> bool {
    let Some(extension) = Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    IMPLEMENTATION_SUFFIXES
        .iter()
        .any(|suffix| extension.eq_ignore_ascii_case(suffix))
}

/// Merge per-file facts in source order, then resolve shared cross-file calls.
#[must_use]
pub fn resolve(extractions: &[Extraction], sources: &HashMap<String, String>) -> Extraction {
    resolve_with_root(extractions, sources, Path::new("."))
}

/// Merge and resolve facts with an explicit corpus root for portable collision salts.
#[must_use]
pub fn resolve_with_root(
    extractions: &[Extraction],
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    let language_facts = members::collect_language_call_facts(extractions);
    let evidence_batches = extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.clone())
        .collect::<Vec<_>>();
    let mut project_edges = Vec::new();
    let mut merged = Extraction::default();
    for extraction in extractions {
        if extraction.semantic_evidence.is_some() {
            project_edges.extend(
                extraction
                    .edges
                    .iter()
                    .filter(|edge| relation(edge) == "imports_from")
                    .cloned(),
            );
            let allowed = universal_allowed_node_ids(extraction);
            merged.nodes.extend(
                extraction
                    .nodes
                    .iter()
                    .filter(|node| {
                        allowed.contains(&node.id)
                            || node.string("file_type") != "code"
                            || is_source_inventory_node(node)
                    })
                    .cloned(),
            );
            merged.edges.extend(
                extraction
                    .edges
                    .iter()
                    .filter(|edge| {
                        !evidence::is_replaced_relation(
                            edge.attributes
                                .get("relation")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                    })
                    .cloned(),
            );
        } else {
            merged.nodes.extend(extraction.nodes.iter().cloned());
            merged.edges.extend(extraction.edges.iter().cloned());
        }
        merged
            .hyperedges
            .extend(extraction.hyperedges.iter().cloned());
        merged
            .framework_facts
            .extend(extraction.framework_facts.iter().cloned());
    }
    finish_resolution(
        merged,
        language_facts,
        evidence_batches,
        sources,
        root,
        project_edges,
        ResolutionMode {
            evidence_prevalidated: false,
            admission: ResolutionAdmission::Max,
        },
    )
}

/// Resolve a collection while transferring its node and edge buffers into the
/// merged graph. The build pipeline no longer needs the per-file facts after
/// this boundary, so ownership avoids a full corpus clone at peak RSS.
#[must_use]
pub fn resolve_owned_with_root(
    mut extractions: Vec<Extraction>,
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    resolve_owned_with_root_impl(
        &mut extractions,
        sources,
        root,
        false,
        ResolutionAdmission::Max,
    )
}

/// Resolve owned facts whose universal evidence was validated at its trust boundary.
///
/// This is the build-pipeline entry point: fresh evidence is validated before
/// the language engine publishes it, and cached evidence is validated before
/// cache acceptance. Cross-batch identities, aggregate limits, and all
/// resolution ambiguity checks are still enforced here.
#[must_use]
pub fn resolve_prevalidated_owned_with_root(
    mut extractions: Vec<Extraction>,
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    resolve_owned_with_root_impl(
        &mut extractions,
        sources,
        root,
        true,
        ResolutionAdmission::Max,
    )
}

/// Resolve prevalidated owned facts while suppressing relationships that the
/// selected graph profile cannot publish.
#[must_use]
pub fn resolve_prevalidated_owned_with_root_at_inference(
    mut extractions: Vec<Extraction>,
    sources: &HashMap<String, String>,
    root: &Path,
    admission: ResolutionAdmission,
) -> Extraction {
    resolve_owned_with_root_impl(&mut extractions, sources, root, true, admission)
}

fn resolve_owned_with_root_impl(
    extractions: &mut Vec<Extraction>,
    sources: &HashMap<String, String>,
    root: &Path,
    evidence_prevalidated: bool,
    admission: ResolutionAdmission,
) -> Extraction {
    let mut profile_started = Instant::now();
    let language_facts = members::collect_language_call_facts_owned(extractions);
    profile_internal("resolver language fact collection", &mut profile_started);
    let mut evidence_batches = Vec::new();
    let mut project_edges = Vec::new();
    let mut merged = Extraction::default();
    for extraction in extractions.iter_mut() {
        let universal = extraction.semantic_evidence.take();
        if universal.is_some() {
            project_edges.extend(
                extraction
                    .edges
                    .iter()
                    .filter(|edge| relation(edge) == "imports_from")
                    .cloned(),
            );
            let mut allowed = universal
                .as_ref()
                .into_iter()
                .flat_map(|batch| &batch.declarations)
                .map(|declaration| declaration.graph_node_id.clone())
                .collect::<HashSet<_>>();
            allowed.extend(
                extraction
                    .nodes
                    .iter()
                    .filter(|node| is_framework_owned_node(node))
                    .map(|node| node.id.clone()),
            );
            allowed.extend(
                extraction
                    .edges
                    .iter()
                    .filter(|edge| !evidence::is_replaced_relation(relation(edge)))
                    .flat_map(|edge| [edge.source.clone(), edge.target.clone()]),
            );
            extraction.nodes.retain(|node| {
                allowed.contains(&node.id)
                    || node.string("file_type") != "code"
                    || is_source_inventory_node(node)
            });
            extraction.edges.retain(|edge| {
                !evidence::is_replaced_relation(
                    edge.attributes
                        .get("relation")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
            });
        }
        merged.nodes.append(&mut extraction.nodes);
        merged.edges.append(&mut extraction.edges);
        merged.hyperedges.append(&mut extraction.hyperedges);
        merged
            .framework_facts
            .append(&mut extraction.framework_facts);
        if let Some(batch) = universal {
            evidence_batches.push(batch);
        }
    }
    profile_internal("resolver owned extraction merge", &mut profile_started);
    extractions.clear();
    finish_resolution(
        merged,
        language_facts,
        evidence_batches,
        sources,
        root,
        project_edges,
        ResolutionMode {
            evidence_prevalidated,
            admission,
        },
    )
}

fn universal_allowed_node_ids(extraction: &Extraction) -> HashSet<String> {
    let mut allowed = extraction
        .semantic_evidence
        .as_ref()
        .into_iter()
        .flat_map(|batch| &batch.declarations)
        .map(|declaration| declaration.graph_node_id.clone())
        .collect::<HashSet<_>>();
    allowed.extend(
        extraction
            .edges
            .iter()
            .filter(|edge| !evidence::is_replaced_relation(relation(edge)))
            .flat_map(|edge| [edge.source.clone(), edge.target.clone()]),
    );
    allowed.extend(
        extraction
            .nodes
            .iter()
            .filter(|node| is_framework_owned_node(node))
            .map(|node| node.id.clone()),
    );
    allowed
}

fn is_framework_owned_node(node: &NodeRecord) -> bool {
    node.string("_origin") == "convention"
        || node.string("extractor").starts_with("compass.frameworks.")
}

fn is_source_inventory_node(node: &NodeRecord) -> bool {
    node.string("symbol_kind") == "file" && !node.string("source_file").is_empty()
}

/// Universal JavaScript/TypeScript extraction deliberately publishes only
/// semantic evidence at the language boundary. Project-level package and
/// `tsconfig` resolution still needs a bounded source inventory and an
/// importer/module edge before the evidence index materializes final graph
/// edges. Build those transient resolver facts here so the language layer does
/// not regress to a second raw AST graph.
fn augment_universal_project_inventory(
    merged: &mut Extraction,
    evidence_batches: &[SemanticEvidenceBatch],
    sources: &HashMap<String, String>,
    root: &Path,
    project_edges: &mut Vec<EdgeRecord>,
) {
    let mut source_languages = BTreeMap::<String, String>::new();
    let mut source_evidence_lengths = BTreeMap::<String, usize>::new();
    for batch in evidence_batches
        .iter()
        .filter(|batch| matches!(batch.adapter.language.as_str(), "javascript" | "typescript"))
    {
        for declaration in &batch.declarations {
            if !declaration.range.source_file.is_empty() {
                source_languages
                    .entry(declaration.range.source_file.clone())
                    .or_insert_with(|| batch.adapter.language.clone());
                let normalized_source = source_key(&declaration.range.source_file, root);
                let length = source_evidence_lengths
                    .entry(normalized_source)
                    .or_insert(0);
                *length = (*length).max(declaration.range.end_byte as usize);
            }
        }
    }
    if source_languages.is_empty() {
        return;
    }
    let source_inventory = source_inventory_index(sources, root);

    let mut source_files = BTreeMap::<String, (String, String)>::new();
    for node in &merged.nodes {
        let source = node.string("source_file");
        if !source.is_empty() && is_file_node(node, &source) {
            source_files
                .entry(source_key(&source, root))
                .or_insert_with(|| (node.id.clone(), source));
        }
    }
    for (source, language) in &source_languages {
        let key = source_key(source, root);
        if !is_safe_relative_source(&key) {
            continue;
        }
        let (display_source, inventory_byte_len) = source_inventory
            .get(&key)
            .cloned()
            .unwrap_or_else(|| (source.clone(), 0));
        source_files.entry(key.clone()).or_insert_with(|| {
            let id = make_id(&[&display_source]);
            let label = Path::new(&display_source)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&display_source)
                .to_owned();
            let byte_len =
                inventory_byte_len.max(source_evidence_lengths.get(&key).copied().unwrap_or(0));
            let mut attributes = Map::from_iter([
                ("label".to_owned(), Value::String(label)),
                ("symbol_kind".to_owned(), Value::String("file".to_owned())),
                ("file_type".to_owned(), Value::String("code".to_owned())),
                (
                    "source_file".to_owned(),
                    Value::String(display_source.clone()),
                ),
                ("source_location".to_owned(), Value::String("L1".to_owned())),
                ("start_byte".to_owned(), Value::from(0_u64)),
                ("end_byte".to_owned(), Value::from(byte_len as u64)),
                ("line_start".to_owned(), Value::from(1_u64)),
                ("line_end".to_owned(), Value::from(1_u64)),
                ("column_start".to_owned(), Value::from(0_u64)),
                ("column_end".to_owned(), Value::from(0_u64)),
                ("language".to_owned(), Value::String(language.clone())),
                (
                    "extractor".to_owned(),
                    Value::String(format!("compass.languages.{language}.universal")),
                ),
                (
                    "universal_evidence_source_file".to_owned(),
                    Value::String(source.clone()),
                ),
                (
                    "confidence".to_owned(),
                    Value::String("EXTRACTED".to_owned()),
                ),
                ("_origin".to_owned(), Value::String("ast".to_owned())),
            ]);
            // Keep the inventory node distinguishable from a semantic module
            // declaration while retaining the stable source identity used by
            // the existing package/path resolvers.
            attributes.insert("universal_inventory".to_owned(), Value::Bool(true));
            merged.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
            (id, display_source)
        });
    }

    let declaration_ids = evidence_batches
        .iter()
        .filter(|batch| matches!(batch.adapter.language.as_str(), "javascript" | "typescript"))
        .flat_map(|batch| {
            batch
                .declarations
                .iter()
                .map(|declaration| (declaration.id.as_str(), declaration))
        })
        .collect::<BTreeMap<_, _>>();
    let mut existing = project_edges
        .iter()
        .filter(|edge| edge.attributes.get("_universal_project_edge") == Some(&Value::Bool(true)))
        .map(|edge| {
            edge.attributes
                .get("evidence_candidate_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<HashSet<_>>();
    for batch in evidence_batches
        .iter()
        .filter(|batch| matches!(batch.adapter.language.as_str(), "javascript" | "typescript"))
    {
        let occurrences_by_id = batch
            .occurrences
            .iter()
            .map(|occurrence| (occurrence.id.as_str(), occurrence))
            .collect::<HashMap<_, _>>();
        let bindings_by_id = batch
            .bindings
            .iter()
            .map(|binding| (binding.id.as_str(), binding))
            .collect::<HashMap<_, _>>();
        for candidate in batch.candidates.iter().filter(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            )
        }) {
            if !existing.insert(candidate.id.clone()) {
                continue;
            }
            let Some(occurrence) = candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences_by_id.get(id).copied())
            else {
                continue;
            };
            if !matches!(
                occurrence.role,
                SemanticRole::Import | SemanticRole::Reexport
            ) {
                continue;
            }
            let Some(owner) = declaration_ids.get(candidate.source_declaration_id.as_str()) else {
                continue;
            };
            let module = candidate
                .binding_id
                .as_deref()
                .and_then(|binding_id| {
                    bindings_by_id
                        .get(binding_id)
                        .copied()
                        .and_then(|binding| binding.qualified_target.rsplit_once("::"))
                        .map(|(module, _)| module.to_owned())
                })
                .or_else(|| {
                    candidate
                        .constraints
                        .qualified_name
                        .as_deref()
                        .and_then(|qualified| qualified.rsplit_once("::"))
                        .map(|(module, _)| module.to_owned())
                })
                .or_else(|| candidate.constraints.module_or_package.clone())
                .filter(|module| !module.is_empty())
                .unwrap_or_default();
            let context = occurrence
                .context
                .as_deref()
                .filter(|context| !context.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if candidate.relation == CandidateRelation::Reexports {
                        "re-export".to_owned()
                    } else {
                        "import".to_owned()
                    }
                });
            if module.is_empty() {
                continue;
            }
            let source_key_value = source_key(&occurrence.range.source_file, root);
            let Some((source_id, project_source_file)) = source_files.get(&source_key_value) else {
                continue;
            };
            let attributes = Map::from_iter([
                (
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                ),
                ("module".to_owned(), Value::String(module)),
                (
                    "source_file".to_owned(),
                    Value::String(project_source_file.clone()),
                ),
                (
                    "universal_evidence_source_file".to_owned(),
                    Value::String(occurrence.range.source_file.clone()),
                ),
                (
                    "source_location".to_owned(),
                    Value::String(format!("L{}", occurrence.range.start_line)),
                ),
                (
                    "start_byte".to_owned(),
                    Value::from(occurrence.range.start_byte),
                ),
                (
                    "end_byte".to_owned(),
                    Value::from(occurrence.range.end_byte),
                ),
                (
                    "line_start".to_owned(),
                    Value::from(occurrence.range.start_line),
                ),
                (
                    "line_end".to_owned(),
                    Value::from(occurrence.range.end_line),
                ),
                (
                    "column_start".to_owned(),
                    Value::from(occurrence.range.start_column),
                ),
                (
                    "column_end".to_owned(),
                    Value::from(occurrence.range.end_column),
                ),
                (
                    "language".to_owned(),
                    Value::String(candidate.language.clone()),
                ),
                (
                    "extractor".to_owned(),
                    Value::String(format!(
                        "compass.languages.{}.universal",
                        candidate.language
                    )),
                ),
                (
                    "confidence".to_owned(),
                    Value::String("EXTRACTED".to_owned()),
                ),
                ("_origin".to_owned(), Value::String("ast".to_owned())),
                ("context".to_owned(), Value::String(context)),
                (
                    "evidence_candidate_id".to_owned(),
                    Value::String(candidate.id.clone()),
                ),
                (
                    "evidence_occurrence_id".to_owned(),
                    Value::String(occurrence.id.clone()),
                ),
                ("_universal_project_edge".to_owned(), Value::Bool(true)),
            ]);
            let placeholder = make_id(&[
                "universal-project-import",
                candidate.language.as_str(),
                occurrence.range.source_file.as_str(),
                candidate.id.as_str(),
            ]);
            project_edges.push(EdgeRecord {
                source: source_id.clone(),
                target: placeholder,
                attributes,
            });
            // `owner` is intentionally looked up above to guarantee that the
            // candidate is source-backed; the transient project edge is keyed
            // by its file inventory node so path resolution remains importer-
            // aware and cannot infer a declaration owner from spelling alone.
            let _ = owner;
        }
    }
}

/// Framework routes historically exposed callable identities using the
/// source-oriented `name()@offset` spelling. Universal evidence keeps a
/// module-qualified name for cross-file resolution, so restore the legacy
/// display identity only on files that actually publish framework facts. This
/// preserves existing route/publication identities without weakening the
/// universal declaration index used by ordinary TypeScript/JavaScript code.
fn restore_framework_callable_names(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
    let mut framework_sources = BTreeSet::new();
    let mut framework_handlers = BTreeMap::<String, BTreeSet<String>>::new();
    for fact in &extraction.framework_facts {
        match fact {
            compass_languages::RawFrameworkFact::Route(route) => {
                framework_sources.insert(route.anchor.source_file.clone());
                let handlers = framework_handlers
                    .entry(source_key(&route.anchor.source_file, root))
                    .or_default();
                handlers.insert(route.handler_reference.clone());
                handlers.extend(route.middleware_references.iter().cloned());
                if let Some(handler_source) = route
                    .detail
                    .get("handler_source")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
                {
                    // Convention/framework routes may target a callable in
                    // another TS/JS file. Restore the historical callable
                    // identity on that source as well as on the route file.
                    framework_sources.insert(handler_source.to_owned());
                }
            }
            compass_languages::RawFrameworkFact::Domain(domain) => {
                framework_sources.insert(domain.anchor.source_file.clone());
            }
            compass_languages::RawFrameworkFact::Annotation(annotation) => {
                framework_sources.insert(annotation.anchor.source_file.clone());
            }
        }
    }
    let nodes_by_id = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let framework_source_keys = framework_sources
        .iter()
        .map(|source| source_key(source, root))
        .collect::<BTreeSet<_>>();
    for edge in &extraction.edges {
        if !matches!(
            edge.attributes.get("relation").and_then(Value::as_str),
            Some("references" | "calls" | "constructs" | "imports" | "imports_from")
        ) {
            continue;
        }
        let Some(edge_source) = edge
            .attributes
            .get("source_file")
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
        else {
            continue;
        };
        if !framework_source_keys.contains(&source_key(edge_source, root)) {
            continue;
        }
        let Some(target_source) = nodes_by_id
            .get(edge.target.as_str())
            .and_then(|node| node.attributes.get("source_file"))
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
        else {
            continue;
        };
        let Some(handler_references) = framework_handlers.get(&source_key(edge_source, root))
        else {
            continue;
        };
        let Some(target) = nodes_by_id.get(edge.target.as_str()) else {
            continue;
        };
        if handler_references
            .iter()
            .any(|reference| frameworks::edge_targets_declared_callable(edge, target, reference))
        {
            // Universal binding/reference evidence is the source-backed bridge
            // for framework handlers that live in another TS/JS file. Only a
            // declared handler or middleware may extend the compatibility
            // surface; ordinary references from a framework file must keep
            // their universal qualified names.
            framework_sources.insert(target_source.to_owned());
        }
    }
    if framework_sources.is_empty() {
        return;
    }
    for node in &mut extraction.nodes {
        let source = string_attribute(node, "source_file");
        if !framework_sources.contains(&source)
            || !matches!(
                string_attribute(node, "language").as_str(),
                "typescript" | "javascript" | "tsx" | "jsx"
            )
        {
            continue;
        }
        let Some(legacy) = node
            .attributes
            .get("legacy_qualified_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let mut legacy = legacy.to_owned();
        if string_attribute(node, "symbol_kind") == "function"
            && let Some(contents) = sources.iter().find_map(|(candidate, contents)| {
                (source_key(candidate, root) == source).then_some(contents)
            })
            && let Some(start) = framework_function_start(contents, node)
            && let Some(separator) = legacy.rfind('@')
        {
            legacy.truncate(separator + 1);
            legacy.push_str(&start.to_string());
        }
        node.attributes
            .insert("qualified_name".to_owned(), Value::String(legacy));
        if matches!(
            string_attribute(node, "symbol_kind").as_str(),
            "function" | "method" | "class" | "component"
        ) && let Some(dialect) = framework_source_dialect(&source)
        {
            node.attributes
                .insert("language".to_owned(), Value::String(dialect.to_owned()));
        }
    }
}

fn framework_source_dialect(source: &str) -> Option<&'static str> {
    match Path::new(source)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("tsx") => Some("tsx"),
        Some("jsx") => Some("jsx"),
        _ => None,
    }
}

fn framework_function_start(source: &str, node: &NodeRecord) -> Option<usize> {
    let start = node.attributes.get("start_byte").and_then(Value::as_u64)? as usize;
    if start > source.len() {
        return None;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line = &source[line_start..];
    let indent = line.len() - line.trim_start_matches(char::is_whitespace).len();
    let mut declaration_start = line_start + indent;
    let remainder = &source[declaration_start..];
    if let Some(after_export) = remainder.strip_prefix("export") {
        let consumed = remainder.len() - after_export.len();
        declaration_start += consumed;
        declaration_start += after_export.len() - after_export.trim_start().len();
        if source[declaration_start..].starts_with("default") {
            declaration_start += "default".len();
            declaration_start +=
                source[declaration_start..].len() - source[declaration_start..].trim_start().len();
        }
    }
    Some(declaration_start)
}

fn finish_resolution(
    mut merged: Extraction,
    mut language_facts: members::LanguageCallFacts,
    evidence_batches: Vec<SemanticEvidenceBatch>,
    sources: &HashMap<String, String>,
    root: &Path,
    project_edges: Vec<EdgeRecord>,
    mode: ResolutionMode,
) -> Extraction {
    let ResolutionMode {
        evidence_prevalidated,
        admission,
    } = mode;
    let mut profile_started = Instant::now();
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut project_edges = project_edges;
    augment_universal_project_inventory(
        &mut merged,
        &evidence_batches,
        sources,
        &canonical_root,
        &mut project_edges,
    );
    let has_javascript = sources.keys().any(|source| {
        let extension = extension(source);
        matches!(
            extension.as_str(),
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts"
        )
    });
    let has_csharp = sources
        .keys()
        .any(|source| matches!(extension(source).as_str(), "cs" | "razor" | "cshtml"));
    let has_php = sources.keys().any(|source| extension(source) == "php");
    let mut project_resolution = (!project_edges.is_empty()).then(|| Extraction {
        nodes: merged
            .nodes
            .iter()
            .filter(|node| {
                let source = string_attribute(node, "source_file");
                is_file_node(node, &source)
            })
            .cloned()
            .collect(),
        edges: project_edges,
        ..Extraction::default()
    });
    if has_javascript {
        let mut javascript_profile_started = Instant::now();
        if let Err(error) =
            resolve_javascript_package_conditions(&mut merged, &canonical_root, sources)
        {
            merged.error.get_or_insert_with(|| {
                format!("JavaScript package-condition resolution failed: {error}")
            });
        }
        profile_internal(
            "resolver JavaScript package conditions (merged)",
            &mut javascript_profile_started,
        );
        if let Err(error) = resolve_javascript_workspace_modules(&mut merged, &canonical_root) {
            merged
                .error
                .get_or_insert_with(|| format!("JavaScript workspace resolution failed: {error}"));
        }
        profile_internal(
            "resolver JavaScript workspace exports (merged)",
            &mut javascript_profile_started,
        );
        if let Err(error) =
            resolve_javascript_typescript_paths(&mut merged, &canonical_root, sources)
        {
            merged.error.get_or_insert_with(|| {
                format!("TypeScript/JavaScript path resolution failed: {error}")
            });
        }
        profile_internal(
            "resolver TypeScript paths (merged)",
            &mut javascript_profile_started,
        );
        resolve_javascript_reexports(&mut merged);
        if let Some(project) = project_resolution.as_mut() {
            if let Err(error) =
                resolve_javascript_package_conditions(project, &canonical_root, sources)
            {
                merged.error.get_or_insert_with(|| {
                    format!("JavaScript project package-condition resolution failed: {error}")
                });
            }
            profile_internal(
                "resolver JavaScript package conditions (project)",
                &mut javascript_profile_started,
            );
            if let Err(error) = resolve_javascript_workspace_modules(project, &canonical_root) {
                merged.error.get_or_insert_with(|| {
                    format!("JavaScript project workspace resolution failed: {error}")
                });
            }
            profile_internal(
                "resolver JavaScript workspace exports (project)",
                &mut javascript_profile_started,
            );
            if let Err(error) =
                resolve_javascript_typescript_paths(project, &canonical_root, sources)
            {
                merged.error.get_or_insert_with(|| {
                    format!("JavaScript project path resolution failed: {error}")
                });
            }
            profile_internal(
                "resolver TypeScript paths (project)",
                &mut javascript_profile_started,
            );
            resolve_javascript_reexports(project);
        }
    }
    profile_internal(
        "resolver JavaScript workspace modules",
        &mut profile_started,
    );
    profile_internal("resolver JavaScript re-exports", &mut profile_started);
    if !evidence_batches.is_empty() {
        let project_edges = project_resolution
            .as_ref()
            .map_or(&[][..], |project| project.edges.as_slice());
        let report = evidence::materialize_bounded_owned(
            evidence_batches,
            project_edges,
            &canonical_root,
            evidence::UniversalResolutionLimits::default(),
            admission,
            evidence_prevalidated,
            (&mut merged.nodes, &mut merged.edges),
        );
        append_universal_resolution_report(&mut merged, &report);
    }
    profile_internal("resolver universal evidence", &mut profile_started);
    restore_framework_callable_names(&mut merged, sources, &canonical_root);
    canonicalize_file_targets(&mut merged, root);
    profile_internal(
        "resolver file-target canonicalization",
        &mut profile_started,
    );
    disambiguate_colliding_node_ids_with_calls(
        &mut merged,
        &canonical_root,
        &mut language_facts.calls,
    );
    profile_internal("resolver collision disambiguation", &mut profile_started);
    resolve_document_link_targets(&mut merged, &canonical_root);
    profile_internal("resolver document links", &mut profile_started);
    if has_javascript {
        resolve_javascript_workspace_symbols(&mut merged);
    }
    profile_internal(
        "resolver JavaScript workspace symbols",
        &mut profile_started,
    );
    if has_csharp {
        canonicalize_csharp_namespace_nodes(&mut merged);
    }
    profile_internal("resolver C# namespace normalization", &mut profile_started);
    if has_php {
        resolve_php_type_references(&mut merged, sources);
    }
    profile_internal("resolver PHP types", &mut profile_started);
    rewire_unique_family_stubs(&mut merged);
    profile_internal("resolver family stubs", &mut profile_started);
    rewire_unique_stub_nodes(&mut merged);
    profile_internal("resolver unique stubs", &mut profile_started);
    // Member and non-member call resolution read the same pre-call graph but
    // append independent edge families. Run their read-heavy indexing and
    // candidate selection together, then apply the historical generic-first
    // append order and duplicate suppression at the single mutation point.
    let (mut generic_edges, (language_nodes, mut language_edges)) = rayon::join(
        || resolve_cross_file_call_additions(&merged, &language_facts.calls, admission),
        || members::resolve_language_call_facts_additions(&language_facts, &merged, admission),
    );
    let generic_keys = generic_edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge_occurrence_site(edge),
            )
        })
        .collect::<AHashSet<_>>();
    language_edges.retain(|edge| {
        !generic_keys.contains(&(
            edge.source.clone(),
            edge.target.clone(),
            edge_occurrence_site(edge),
        ))
    });
    merged.edges.append(&mut generic_edges);
    merged.edges.extend(language_edges);
    merged.nodes.extend(language_nodes);
    profile_internal("resolver cross-file calls", &mut profile_started);
    profile_internal("resolver language calls", &mut profile_started);
    if let Err(error) = frameworks::expand_universal_framework_facts(&mut merged) {
        merged
            .error
            .get_or_insert_with(|| format!("universal framework expansion failed: {error}"));
    }
    profile_internal(
        "resolver universal framework expansion",
        &mut profile_started,
    );
    let (routes, domains) = frameworks::resolve_framework_facts(
        &merged,
        compass_languages::FrameworkLimits::default(),
        &canonical_root,
    );
    let route_result = routes.and_then(|routes| {
        frameworks::publish_resolved_routes(&mut merged, &routes)?;
        Ok(routes)
    });
    if let Err(error) = route_result {
        if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
            eprintln!("[compass internal] framework route resolution failed: {error}");
        }
        merged
            .error
            .get_or_insert_with(|| format!("framework resolution failed: {error}"));
    }
    profile_internal("resolver framework routes", &mut profile_started);
    match domains {
        Ok(domains) => frameworks::publish_resolved_domains(&mut merged, &domains),
        Err(error) => {
            merged
                .error
                .get_or_insert_with(|| format!("framework domain resolution failed: {error}"));
        }
    }
    profile_internal("resolver framework domains", &mut profile_started);
    merged
}

fn append_universal_resolution_report(
    extraction: &mut Extraction,
    report: &evidence::UniversalResolutionReport,
) {
    if let Ok(value) = serde_json::to_value(report) {
        extraction.extensions.insert(
            evidence::UNIVERSAL_RESOLUTION_REPORT_EXTENSION.to_owned(),
            value,
        );
    }
    let mut diagnostics = extraction
        .extensions
        .remove(GRAPH_DIAGNOSTICS_EXTENSION)
        .and_then(|value| serde_json::from_value::<Vec<GraphDiagnostic>>(value).ok())
        .unwrap_or_default();
    if report.compacted_declarations > 0 {
        diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: "low_inference_declaration_compaction".to_owned(),
            message: format!(
                "low inference omitted {} unreferenced parameter/property declarations before project resolution",
                report.compacted_declarations
            ),
            anchor: None,
            related_ids: Vec::new(),
        });
    }
    if report.degraded {
        let reason = report
            .reason
            .as_deref()
            .unwrap_or("bounded universal resolution could not complete")
            .chars()
            .take(1_024)
            .collect::<String>();
        diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "universal_resolution_partial".to_owned(),
            message: format!(
                "published bounded partial universal resolution across {} partitions; {} relationship candidates were omitted and {} partitions failed: {reason}",
                report.partitions, report.omitted_candidates, report.failed_partitions
            ),
            anchor: None,
            related_ids: Vec::new(),
        });
    }
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.message.cmp(&right.message))
    });
    diagnostics.dedup();
    if let Ok(value) = serde_json::to_value(diagnostics) {
        extraction
            .extensions
            .insert(GRAPH_DIAGNOSTICS_EXTENSION.to_owned(), value);
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
}

const MAX_JAVASCRIPT_PACKAGE_MANIFESTS: usize = 4_096;
const MAX_JAVASCRIPT_PACKAGE_EXPORTS: usize = 4_096;
const MAX_JAVASCRIPT_PACKAGE_BYTES: u64 = 2 * 1024 * 1024;

/// Resolve repository-local npm package specifiers through their declared
/// package exports. Only targets that are already present in the source
/// inventory are eligible, and duplicate package names remain unresolved.
fn resolve_javascript_workspace_modules(
    extraction: &mut Extraction,
    root: &Path,
) -> Result<(), String> {
    if !has_javascript_import_edges(extraction) {
        return Ok(());
    }
    let mut file_by_source = BTreeMap::<String, (String, String)>::new();
    let mut manifests = BTreeSet::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if source.is_empty() {
            continue;
        }
        if is_file_node(node, &source) {
            file_by_source
                .entry(source_key(&source, root))
                .or_insert_with(|| (node.id.clone(), source.clone()));
        }
        if Path::new(&source)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("package.json")
        {
            manifests.insert(source);
        }
    }
    if manifests.len() > MAX_JAVASCRIPT_PACKAGE_MANIFESTS {
        return Err(format!(
            "package manifest count {} exceeds limit {MAX_JAVASCRIPT_PACKAGE_MANIFESTS}",
            manifests.len()
        ));
    }

    let mut targets = BTreeMap::<String, BTreeSet<String>>::new();
    let mut export_count = 0_usize;
    for manifest in manifests {
        let manifest_path = rooted_source_path(root, &manifest)?;
        let source =
            match compass_files::read_source_lossy(&manifest_path, MAX_JAVASCRIPT_PACKAGE_BYTES) {
                Ok(source) => source,
                Err(_) => continue,
            };
        let Ok(value) = serde_json::from_str::<Value>(&source) else {
            continue;
        };
        let Some(name) = value.get("name").and_then(Value::as_str).filter(|name| {
            !name.is_empty()
                && name.len() <= 4_096
                && !name.contains(['\\', '\0'])
                && !name
                    .split('/')
                    .any(|part| part.is_empty() || matches!(part, "." | ".."))
        }) else {
            continue;
        };
        let manifest_directory = manifest_path.parent().unwrap_or(root);
        let mut exports = BTreeMap::<String, BTreeSet<String>>::new();
        if let Some(value) = value.get("exports") {
            collect_javascript_package_exports(value, &mut exports, 0)?;
        } else {
            for field in ["module", "main"] {
                if let Some(target) = value.get(field).and_then(Value::as_str) {
                    exports
                        .entry(".".to_owned())
                        .or_default()
                        .insert(target.to_owned());
                }
            }
        }
        for (subpath, candidates) in exports {
            export_count = export_count.saturating_add(candidates.len());
            if export_count > MAX_JAVASCRIPT_PACKAGE_EXPORTS {
                return Err(format!(
                    "package export count exceeds limit {MAX_JAVASCRIPT_PACKAGE_EXPORTS}"
                ));
            }
            let resolved = candidates
                .into_iter()
                .filter_map(|candidate| {
                    javascript_package_target(manifest_directory, root, &candidate)
                })
                .map(|candidate| source_key(&candidate.to_string_lossy(), root))
                .filter(|candidate| file_by_source.contains_key(candidate))
                .collect::<BTreeSet<_>>();
            if resolved.len() != 1 {
                continue;
            }
            let specifier = if subpath == "." {
                name.to_owned()
            } else if let Some(subpath) = subpath.strip_prefix("./") {
                format!("{name}/{subpath}")
            } else {
                continue;
            };
            targets.entry(specifier).or_default().extend(resolved);
        }
    }

    for edge in &mut extraction.edges {
        if relation(edge) != "imports_from" {
            continue;
        }
        // The importer-aware pass runs first and stamps every package edge it
        // owns (including explicit Classic/Node10 misses). Do not let this
        // legacy flattened workspace fallback overwrite a mode-specific
        // decision or reintroduce a conditional branch.
        if edge.attributes.contains_key("resolution_rule")
            || edge.attributes.contains_key("module_resolution")
        {
            continue;
        }
        let module = edge.string("module");
        let Some(candidates) = targets.get(&module) else {
            continue;
        };
        if candidates.len() != 1 {
            continue;
        }
        let Some(target_source) = candidates.iter().next() else {
            continue;
        };
        let Some((target_id, original_source)) = file_by_source.get(target_source) else {
            continue;
        };
        if edge.target != *target_id {
            edge.target.clone_from(target_id);
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(original_source.clone()),
            );
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct JavascriptPackageManifest {
    source: String,
    name: String,
    directory: PathBuf,
    exports: Option<Value>,
    imports: Option<Value>,
    types_versions: Option<Value>,
    types: Option<String>,
    module: Option<String>,
    main: Option<String>,
}

/// Resolve package exports with an importer-aware condition order.
///
/// The previous workspace pass intentionally kept only a unique flattened
/// target. That is safe for simple packages, but it loses the distinction
/// between import, require, types, and default branches. This pass evaluates
/// the documented branch order without unioning mutually exclusive conditions
/// and only publishes an admitted source target.
fn resolve_javascript_package_conditions(
    extraction: &mut Extraction,
    root: &Path,
    sources: &HashMap<String, String>,
) -> Result<(), String> {
    if !has_javascript_import_edges(extraction) {
        return Ok(());
    }
    let (typescript_configs, referenced_configs) =
        collect_typescript_configs(extraction, root, sources)?;
    let mut file_by_source = BTreeMap::<String, (String, String)>::new();
    let mut manifest_sources = BTreeSet::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if source.is_empty() {
            continue;
        }
        if is_file_node(node, &source) {
            file_by_source
                .entry(source_key(&source, root))
                .or_insert_with(|| (node.id.clone(), source.clone()));
        }
        if Path::new(&source)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("package.json")
        {
            manifest_sources.insert(source);
        }
    }
    if manifest_sources.len() > MAX_JAVASCRIPT_PACKAGE_MANIFESTS {
        return Err(format!(
            "package manifest count {} exceeds limit {MAX_JAVASCRIPT_PACKAGE_MANIFESTS}",
            manifest_sources.len()
        ));
    }
    let mut manifests = Vec::new();
    for source in manifest_sources {
        let manifest_path = rooted_source_path(root, &source)?;
        let Ok(contents) =
            compass_files::read_source_lossy(&manifest_path, MAX_JAVASCRIPT_PACKAGE_BYTES)
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&contents) else {
            continue;
        };
        let Some(name) = value.get("name").and_then(Value::as_str).filter(|name| {
            !name.is_empty()
                && name.len() <= 4_096
                && !name.contains(['\\', '\0'])
                && !name
                    .split('/')
                    .any(|part| part.is_empty() || matches!(part, "." | ".."))
        }) else {
            continue;
        };
        let Some(directory) = manifest_path.parent().map(Path::to_path_buf) else {
            continue;
        };
        manifests.push(JavascriptPackageManifest {
            source,
            name: name.to_owned(),
            directory,
            exports: value.get("exports").cloned(),
            imports: value.get("imports").cloned(),
            types_versions: value.get("typesVersions").cloned(),
            types: value
                .get("types")
                .or_else(|| value.get("typings"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            module: value
                .get("module")
                .and_then(Value::as_str)
                .map(str::to_owned),
            main: value.get("main").and_then(Value::as_str).map(str::to_owned),
        });
    }
    manifests.sort_by(|left, right| left.source.cmp(&right.source));

    let mut export_count = 0_usize;
    for edge in &mut extraction.edges {
        if relation(edge) != "imports_from" {
            continue;
        }
        let module = edge.string("module");
        if module.starts_with('#') {
            let importer_path = root.join(source_key(&edge.string("source_file"), root));
            let candidates = manifests
                .iter()
                .filter(|manifest| importer_path.starts_with(&manifest.directory))
                .collect::<Vec<_>>();
            let deepest = candidates
                .iter()
                .map(|manifest| manifest.directory.components().count())
                .max();
            let nearest = deepest.and_then(|depth| {
                let mut nearest = candidates
                    .into_iter()
                    .filter(|manifest| manifest.directory.components().count() == depth)
                    .collect::<Vec<_>>();
                (nearest.len() == 1).then(|| nearest.pop()).flatten()
            });
            let Some(manifest) = nearest else {
                continue;
            };
            let Some(imports) = manifest.imports.as_ref() else {
                continue;
            };
            let importer = edge.string("source_file");
            let config = select_typescript_path_config(
                &typescript_configs,
                &referenced_configs,
                root,
                &importer,
            );
            let resolution_mode = javascript_module_resolution_mode(config);
            if let Some(module_resolution) =
                config.and_then(|config| config.module_resolution.as_ref())
            {
                edge.attributes.insert(
                    "module_resolution".to_owned(),
                    Value::String(module_resolution.clone()),
                );
            }
            if !resolution_mode.supports_conditional_exports() {
                // Classic and Node10 resolution do not search package
                // `imports` maps.
                // Leave the raw import unresolved rather than applying a
                // Node-style package rule to a compiler invocation that did
                // not opt into it.
                edge.attributes.insert(
                    "resolution_rule".to_owned(),
                    Value::String("package-imports-unsupported".to_owned()),
                );
                continue;
            }
            let conditions = javascript_package_condition_order(
                edge.string("context").as_str(),
                config.map_or(&[] as &[String], |config| {
                    config.custom_conditions.as_slice()
                }),
            );
            let condition_refs = conditions.iter().map(String::as_str).collect::<Vec<_>>();
            let candidates =
                resolve_javascript_package_export(imports, &module, &condition_refs, 0, None);
            let Some((_, condition, target_source)) =
                candidates.into_iter().find_map(|(target, condition)| {
                    let target_path =
                        javascript_package_target(&manifest.directory, root, &target)?;
                    let importer_is_typescript = is_typescript_source(&edge.string("source_file"));
                    let candidates = if importer_is_typescript {
                        typescript_target_candidates(&target_path, &[])
                    } else {
                        vec![target_path]
                    };
                    candidates.into_iter().find_map(|candidate| {
                        let key = source_key(&candidate.to_string_lossy(), root);
                        file_by_source.contains_key(&key).then_some((
                            target.clone(),
                            condition.clone(),
                            key,
                        ))
                    })
                })
            else {
                continue;
            };
            export_count = export_count.saturating_add(1);
            if export_count > MAX_JAVASCRIPT_PACKAGE_EXPORTS {
                return Err(format!(
                    "package condition resolution count exceeds {MAX_JAVASCRIPT_PACKAGE_EXPORTS}"
                ));
            }
            let Some((target_id, original_source)) = file_by_source.get(&target_source) else {
                continue;
            };
            if edge.target != *target_id {
                edge.target.clone_from(target_id);
                stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
            }
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(original_source.clone()),
            );
            edge.attributes.insert(
                "resolution_rule".to_owned(),
                Value::String("package-imports".to_owned()),
            );
            edge.attributes
                .insert("package_condition".to_owned(), Value::String(condition));
            edge.attributes.insert(
                "resolution_config".to_owned(),
                Value::String(manifest.source.clone()),
            );
            continue;
        }
        let Some((package_name, subpath)) = javascript_package_specifier(&module) else {
            continue;
        };
        let matching = manifests
            .iter()
            .filter(|manifest| manifest.name == package_name)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            continue;
        }
        let manifest = matching[0];
        let importer = edge.string("source_file");
        let config = select_typescript_path_config(
            &typescript_configs,
            &referenced_configs,
            root,
            &importer,
        );
        let resolution_mode = javascript_module_resolution_mode(config);
        if let Some(module_resolution) = config.and_then(|config| config.module_resolution.as_ref())
        {
            edge.attributes.insert(
                "module_resolution".to_owned(),
                Value::String(module_resolution.clone()),
            );
        }
        if resolution_mode == JavascriptModuleResolution::Classic {
            // Classic resolution has no package-name lookup. A project alias
            // may still resolve this edge in the dedicated paths pass.
            edge.attributes.insert(
                "resolution_rule".to_owned(),
                Value::String("package-classic-unresolved".to_owned()),
            );
            continue;
        }
        let conditions = javascript_package_condition_order(
            edge.string("context").as_str(),
            config.map_or(&[] as &[String], |config| {
                config.custom_conditions.as_slice()
            }),
        );
        let condition_refs = conditions.iter().map(String::as_str).collect::<Vec<_>>();
        let mut selected = if resolution_mode.supports_conditional_exports() {
            manifest
                .exports
                .as_ref()
                .and_then(|exports| {
                    let candidates = resolve_javascript_package_export(
                        exports,
                        &subpath,
                        &condition_refs,
                        0,
                        None,
                    );
                    (!candidates.is_empty()).then_some(candidates)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut resolution_rule = "package-exports";
        if selected.is_empty() && is_typescript_source(&edge.string("source_file")) {
            selected =
                javascript_types_versions_targets(manifest.types_versions.as_ref(), &subpath)
                    .into_iter()
                    .map(|target| (target, "typesVersions".to_owned()))
                    .collect();
            if !selected.is_empty() {
                resolution_rule = "typesVersions";
            }
        }
        if selected.is_empty() {
            selected = javascript_package_legacy_target(manifest, &subpath, &conditions);
            if !selected.is_empty() {
                resolution_rule = "package-legacy";
            }
        }
        let Some((_, condition, target_source)) =
            selected.into_iter().find_map(|(target, condition)| {
                let target_path = javascript_package_target(&manifest.directory, root, &target)?;
                let importer_is_typescript = is_typescript_source(&edge.string("source_file"));
                let candidates = if importer_is_typescript {
                    typescript_target_candidates(&target_path, &[])
                } else {
                    vec![target_path]
                };
                candidates.into_iter().find_map(|candidate| {
                    let key = source_key(&candidate.to_string_lossy(), root);
                    file_by_source.contains_key(&key).then_some((
                        target.clone(),
                        condition.clone(),
                        key,
                    ))
                })
            })
        else {
            // A manifest was present and the compiler-mode decision was
            // evaluated, but no admitted target survived. Preserve the
            // unresolved result so the older flattened workspace pass cannot
            // resurrect a different conditional branch later in the pipeline.
            edge.attributes.insert(
                "resolution_rule".to_owned(),
                Value::String("package-unresolved".to_owned()),
            );
            continue;
        };
        export_count = export_count.saturating_add(1);
        if export_count > MAX_JAVASCRIPT_PACKAGE_EXPORTS {
            return Err(format!(
                "package condition resolution count exceeds {MAX_JAVASCRIPT_PACKAGE_EXPORTS}"
            ));
        }
        let Some((target_id, original_source)) = file_by_source.get(&target_source) else {
            continue;
        };
        if edge.target != *target_id {
            edge.target.clone_from(target_id);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
        edge.attributes.insert(
            "target_file".to_owned(),
            Value::String(original_source.clone()),
        );
        edge.attributes.insert(
            "resolution_rule".to_owned(),
            Value::String(resolution_rule.to_owned()),
        );
        edge.attributes
            .insert("package_condition".to_owned(), Value::String(condition));
        edge.attributes.insert(
            "resolution_config".to_owned(),
            Value::String(manifest.source.clone()),
        );
    }
    Ok(())
}

fn javascript_package_specifier(module: &str) -> Option<(String, String)> {
    if module.is_empty() || module.len() > 4_096 || module.contains(['\\', '\0']) {
        return None;
    }
    let parts = module.split('/').collect::<Vec<_>>();
    let package_len = if parts.first()?.starts_with('@') {
        if parts.len() < 2 {
            return None;
        }
        2
    } else {
        1
    };
    if parts
        .iter()
        .any(|part| part.is_empty() || matches!(*part, "." | ".."))
    {
        return None;
    }
    let package = parts[..package_len].join("/");
    let subpath = if parts.len() == package_len {
        ".".to_owned()
    } else {
        format!("./{}", parts[package_len..].join("/"))
    };
    Some((package, subpath))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JavascriptModuleResolution {
    /// TypeScript's pre-Node package lookup. Non-relative package names are
    /// not resolved through `node_modules` or package `imports`/`exports`.
    Classic,
    /// The Node10 resolver. Legacy `types`/`main`/`module` fields remain
    /// available, but conditional package maps are not part of this mode.
    Node10,
    Node16,
    NodeNext,
    Bundler,
    /// No explicit mode was found in the admitted project. Preserve the
    /// historical Compass behavior while recording no invented config value.
    Inferred,
}

impl JavascriptModuleResolution {
    const fn supports_conditional_exports(self) -> bool {
        matches!(
            self,
            Self::Node16 | Self::NodeNext | Self::Bundler | Self::Inferred
        )
    }
}

fn javascript_module_resolution_mode(
    config: Option<&TypeScriptPathConfig>,
) -> JavascriptModuleResolution {
    let Some(config) = config else {
        return JavascriptModuleResolution::Inferred;
    };
    match config.module_resolution.as_deref() {
        Some("classic") => JavascriptModuleResolution::Classic,
        Some("node") | Some("node10") => JavascriptModuleResolution::Node10,
        Some("node16") => JavascriptModuleResolution::Node16,
        Some("nodenext") => JavascriptModuleResolution::NodeNext,
        Some("bundler") => JavascriptModuleResolution::Bundler,
        // Unknown or omitted values are not silently reinterpreted as a
        // different compiler mode. Omitted/unknown config keeps the existing
        // conservative package behavior until an explicit mode is available.
        _ => JavascriptModuleResolution::Inferred,
    }
}

fn javascript_package_condition_order(context: &str, custom: &[String]) -> Vec<String> {
    let mut conditions = vec!["types".to_owned()];
    for condition in custom {
        if !conditions.iter().any(|candidate| candidate == condition) {
            conditions.push(condition.clone());
        }
    }
    conditions.push(if context == "require" {
        "require".to_owned()
    } else {
        "import".to_owned()
    });
    conditions.push("node".to_owned());
    conditions.push("default".to_owned());
    conditions
}

fn resolve_javascript_package_export(
    value: &Value,
    subpath: &str,
    conditions: &[&str],
    depth: usize,
    wildcard: Option<&str>,
) -> Vec<(String, String)> {
    if depth > 32 {
        return Vec::new();
    }
    match value {
        Value::String(target) => vec![(
            wildcard.map_or_else(|| target.clone(), |wildcard| target.replace('*', wildcard)),
            "default".to_owned(),
        )],
        Value::Array(values) => values
            .iter()
            .flat_map(|value| {
                resolve_javascript_package_export(value, subpath, conditions, depth + 1, wildcard)
            })
            .collect(),
        Value::Object(entries) => {
            let has_subpaths = entries
                .keys()
                .any(|key| key == "." || key.starts_with("./") || key.starts_with('#'));
            if has_subpaths {
                if let Some(value) = entries.get(subpath) {
                    return resolve_javascript_package_export(
                        value,
                        subpath,
                        conditions,
                        depth + 1,
                        wildcard,
                    );
                }
                let mut patterns = entries
                    .iter()
                    .filter_map(|(pattern, value)| {
                        let (prefix, suffix) = pattern.split_once('*')?;
                        if !(pattern.starts_with("./") || pattern.starts_with('#'))
                            || !subpath.starts_with(prefix)
                            || !subpath.ends_with(suffix)
                        {
                            return None;
                        }
                        let end = subpath.len().saturating_sub(suffix.len());
                        (end >= prefix.len()).then_some((
                            prefix.len(),
                            pattern,
                            value,
                            &subpath[prefix.len()..end],
                        ))
                    })
                    .collect::<Vec<_>>();
                patterns
                    .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
                let Some((_, _, value, wildcard)) = patterns.first() else {
                    return Vec::new();
                };
                return resolve_javascript_package_export(
                    value,
                    subpath,
                    conditions,
                    depth + 1,
                    Some(wildcard),
                );
            }
            // Conditional export objects are ordered by the package author;
            // Node/TypeScript select the first active key in that source
            // order, not the first condition in Compass' condition set. A
            // condition set only says whether a key is active.
            for (condition, value) in entries {
                if !conditions.iter().any(|active| *active == condition) {
                    continue;
                }
                let candidates = resolve_javascript_package_export(
                    value,
                    subpath,
                    conditions,
                    depth + 1,
                    wildcard,
                );
                if !candidates.is_empty() {
                    return candidates
                        .into_iter()
                        .map(|(target, _)| (target, condition.clone()))
                        .collect();
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn javascript_package_legacy_target(
    manifest: &JavascriptPackageManifest,
    subpath: &str,
    conditions: &[String],
) -> Vec<(String, String)> {
    if subpath != "." {
        // Node10 resolves a package subpath as a path under the package root
        // when no conditional `exports` map applies. Keep the target relative
        // and let the admitted-inventory probe perform extension/index
        // substitution; never search outside the package directory.
        return subpath
            .strip_prefix("./")
            .filter(|path| !path.is_empty())
            .map(|path| (format!("./{path}"), "package-subpath".to_owned()))
            .into_iter()
            .collect();
    }
    let candidates = if conditions.iter().any(|condition| condition == "require") {
        [
            manifest.main.as_deref(),
            manifest.module.as_deref(),
            manifest.types.as_deref(),
        ]
    } else {
        [
            manifest.types.as_deref(),
            manifest.module.as_deref(),
            manifest.main.as_deref(),
        ]
    };
    candidates
        .into_iter()
        .flatten()
        .map(|target| {
            let condition = if manifest.types.as_deref() == Some(target) {
                "types"
            } else if manifest.module.as_deref() == Some(target) {
                "module"
            } else {
                "main"
            };
            (target.to_owned(), condition.to_owned())
        })
        .collect()
}

fn javascript_types_versions_targets(types_versions: Option<&Value>, subpath: &str) -> Vec<String> {
    let Some(types_versions) = types_versions.and_then(Value::as_object) else {
        return Vec::new();
    };
    // A semver-aware selector would need the compiler's complete version
    // range semantics. Support the deterministic catch-all form and a single
    // explicitly supplied range; multiple unknown ranges remain unresolved.
    let version_map = if let Some(value) = types_versions.get("*") {
        value.as_object()
    } else if types_versions.len() == 1 {
        types_versions.values().next().and_then(Value::as_object)
    } else {
        None
    };
    let Some(version_map) = version_map else {
        return Vec::new();
    };
    let request = subpath.strip_prefix("./").unwrap_or(subpath);
    let request = if request == "." { "" } else { request };
    let mut matching = version_map
        .iter()
        .filter_map(|(pattern, value)| {
            let wildcard = if let Some((prefix, suffix)) = pattern.split_once('*') {
                if !request.starts_with(prefix) || !request.ends_with(suffix) {
                    return None;
                }
                let end = request.len().saturating_sub(suffix.len());
                (end >= prefix.len()).then(|| request[prefix.len()..end].to_owned())
            } else {
                (pattern == request).then(String::new)
            }?;
            Some((pattern.len(), pattern, value, wildcard))
        })
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    if matching
        .get(1)
        .is_some_and(|candidate| candidate.0 == matching[0].0)
    {
        return Vec::new();
    }
    let Some((_, _, value, wildcard)) = matching.first() else {
        return Vec::new();
    };
    let targets = match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .take(256)
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    targets
        .into_iter()
        .filter(|target| {
            !target.is_empty()
                && target.len() <= 4_096
                && !target.contains(['\\', '\0'])
                && !Path::new(target).is_absolute()
        })
        .map(|target| {
            let target = target.replace('*', wildcard);
            if target.starts_with("./") {
                target
            } else {
                format!("./{target}")
            }
        })
        .collect()
}

fn is_typescript_source(source: &str) -> bool {
    matches!(extension(source).as_str(), "ts" | "tsx" | "mts" | "cts")
}

const MAX_TYPESCRIPT_CONFIGS: usize = 256;
const MAX_TYPESCRIPT_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TYPESCRIPT_PATH_RULES: usize = 4_096;
const MAX_TYPESCRIPT_PATH_TARGETS: usize = 4_096;
const MAX_TYPESCRIPT_CONFIG_EXTENDS_DEPTH: usize = 32;
const MAX_TYPESCRIPT_CONFIG_EXTENDS: usize = 64;
const MAX_TYPESCRIPT_CONFIG_REFERENCES: usize = 1_024;
const MAX_TYPESCRIPT_FILE_PATTERNS: usize = 4_096;
const MAX_TYPESCRIPT_TYPE_ROOTS: usize = 256;
const MAX_TYPESCRIPT_CUSTOM_CONDITIONS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypeScriptConfigKind {
    TypeScript,
    JavaScript,
}

#[derive(Clone, Debug)]
struct TypeScriptPathRule {
    pattern: String,
    prefix: String,
    suffix: String,
    targets: Vec<TypeScriptPathTarget>,
}

#[derive(Clone, Debug)]
struct TypeScriptPathTarget {
    value: String,
    base: PathBuf,
}

#[derive(Clone, Debug)]
struct TypeScriptFilePattern {
    value: String,
    base: PathBuf,
}

#[derive(Clone, Debug)]
struct TypeScriptPathConfig {
    source: String,
    directory: PathBuf,
    base_url: Option<PathBuf>,
    rules: Vec<TypeScriptPathRule>,
    root_dirs: Vec<PathBuf>,
    module: Option<String>,
    module_resolution: Option<String>,
    module_suffixes: Vec<String>,
    kind: TypeScriptConfigKind,
    allow_js: bool,
    check_js: bool,
    resolve_json_module: bool,
    references: Vec<PathBuf>,
    extends_sources: Vec<String>,
    files: Option<Vec<TypeScriptFilePattern>>,
    include: Option<Vec<TypeScriptFilePattern>>,
    exclude: Option<Vec<TypeScriptFilePattern>>,
    type_roots: Vec<PathBuf>,
    custom_conditions: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct TypeScriptConfigValues {
    base_url: Option<PathBuf>,
    paths: Option<Vec<TypeScriptPathRule>>,
    root_dirs: Option<Vec<PathBuf>>,
    module: Option<String>,
    module_resolution: Option<String>,
    module_suffixes: Option<Vec<String>>,
    allow_js: Option<bool>,
    check_js: Option<bool>,
    resolve_json_module: Option<bool>,
    references: Vec<PathBuf>,
    extends_sources: Vec<String>,
    files: Option<Vec<TypeScriptFilePattern>>,
    include: Option<Vec<TypeScriptFilePattern>>,
    exclude: Option<Vec<TypeScriptFilePattern>>,
    type_roots: Option<Vec<PathBuf>>,
    custom_conditions: Option<Vec<String>>,
}

impl TypeScriptConfigValues {
    fn overlay(&mut self, child: Self) {
        if child.base_url.is_some() {
            self.base_url = child.base_url;
        }
        if child.paths.is_some() {
            self.paths = child.paths;
        }
        if child.root_dirs.is_some() {
            self.root_dirs = child.root_dirs;
        }
        if child.module.is_some() {
            self.module = child.module;
        }
        if child.module_resolution.is_some() {
            self.module_resolution = child.module_resolution;
        }
        if child.module_suffixes.is_some() {
            self.module_suffixes = child.module_suffixes;
        }
        if child.allow_js.is_some() {
            self.allow_js = child.allow_js;
        }
        if child.check_js.is_some() {
            self.check_js = child.check_js;
        }
        if child.resolve_json_module.is_some() {
            self.resolve_json_module = child.resolve_json_module;
        }
        if child.files.is_some() {
            self.files = child.files;
        }
        if child.include.is_some() {
            self.include = child.include;
        }
        if child.exclude.is_some() {
            self.exclude = child.exclude;
        }
        if child.type_roots.is_some() {
            self.type_roots = child.type_roots;
        }
        if child.custom_conditions.is_some() {
            self.custom_conditions = child.custom_conditions;
        }
        self.references.extend(child.references);
        self.extends_sources.extend(child.extends_sources);
    }
}

/// Resolve TypeScript `compilerOptions.paths` and `baseUrl` using the source
/// inventory already admitted by Compass. This is intentionally separate from
/// package exports: a project alias is a compiler mapping and takes precedence
/// over a same-spelled npm package, while unresolved or ambiguous mappings are
/// left as the extractor emitted them.
fn resolve_javascript_typescript_paths(
    extraction: &mut Extraction,
    root: &Path,
    sources: &HashMap<String, String>,
) -> Result<(), String> {
    if !has_javascript_import_edges(extraction) {
        return Ok(());
    }
    let (configs, referenced_configs) = collect_typescript_configs(extraction, root, sources)?;

    let mut file_by_source = BTreeMap::<String, (String, String)>::new();
    let mut source_by_file = HashMap::<String, String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if !is_file_node(node, &source) {
            continue;
        }
        let key = source_key(&source, root);
        if !is_safe_relative_source(&key) {
            continue;
        }
        file_by_source
            .entry(key)
            .or_insert_with(|| (node.id.clone(), source.clone()));
        source_by_file.entry(node.id.clone()).or_insert(source);
    }

    for edge in &mut extraction.edges {
        if relation(edge) != "imports_from" {
            continue;
        }
        let module = edge.string("module");
        if module.is_empty() || module.len() > 4_096 {
            continue;
        }
        let importer = {
            let source = edge.string("source_file");
            if source.is_empty() {
                source_by_file
                    .get(&edge.source)
                    .cloned()
                    .unwrap_or_default()
            } else {
                source
            }
        };
        let config = select_typescript_path_config(&configs, &referenced_configs, root, &importer);
        let resolved = if module.starts_with('.') {
            resolve_typescript_relative_module(config, &importer, &module, &file_by_source, root)
        } else {
            let Some(config) = config else {
                continue;
            };
            resolve_typescript_module(config, &module, &file_by_source, root)
        };
        let Some((target, rule)) = resolved else {
            continue;
        };
        let Some((target_id, target_source)) = file_by_source.get(&target) else {
            continue;
        };
        if edge.target != *target_id {
            edge.target.clone_from(target_id);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
        edge.attributes.insert(
            "target_file".to_owned(),
            Value::String(target_source.clone()),
        );
        edge.attributes
            .insert("resolution_rule".to_owned(), Value::String(rule.to_owned()));
        if let Some(config) = config {
            edge.attributes.insert(
                "resolution_config".to_owned(),
                Value::String(config.source.clone()),
            );
            if let Some(module_resolution) = &config.module_resolution {
                edge.attributes.insert(
                    "module_resolution".to_owned(),
                    Value::String(module_resolution.clone()),
                );
            }
            if let Some(module) = &config.module {
                edge.attributes
                    .insert("module_kind".to_owned(), Value::String(module.clone()));
            }
            if !config.references.is_empty() {
                let references = config
                    .references
                    .iter()
                    .map(|reference| Value::String(source_key(&reference.to_string_lossy(), root)))
                    .collect::<Vec<_>>();
                edge.attributes.insert(
                    "resolution_project_references".to_owned(),
                    Value::Array(references),
                );
            }
        }
    }
    Ok(())
}

fn has_javascript_import_edges(extraction: &Extraction) -> bool {
    extraction
        .edges
        .iter()
        .any(|edge| relation(edge) == "imports_from")
}

fn collect_typescript_configs(
    extraction: &Extraction,
    root: &Path,
    sources: &HashMap<String, String>,
) -> Result<(Vec<TypeScriptPathConfig>, BTreeSet<String>), String> {
    let mut config_sources = BTreeSet::new();
    for source in sources.keys() {
        let key = source_key(source, root);
        if is_typescript_config_source(&key) && is_safe_relative_source(&key) {
            config_sources.insert(key);
        }
    }
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        let key = source_key(&source, root);
        if is_typescript_config_source(&key) && is_safe_relative_source(&key) {
            config_sources.insert(key);
        }
    }
    if config_sources.len() > MAX_TYPESCRIPT_CONFIGS {
        return Err(format!(
            "TypeScript project configuration count {} exceeds limit {MAX_TYPESCRIPT_CONFIGS}",
            config_sources.len()
        ));
    }

    let mut configs = Vec::new();
    let mut config_cache = HashMap::<String, Option<TypeScriptConfigValues>>::new();
    for source in config_sources {
        let Some(contents) = read_typescript_config_source(root, &source, sources)? else {
            continue;
        };
        if let Some(config) =
            parse_typescript_path_config(root, &source, &contents, sources, &mut config_cache)?
        {
            configs.push(config);
        }
    }
    configs.sort_by(|left, right| left.source.cmp(&right.source));
    let referenced_configs = configs
        .iter()
        .flat_map(|config| config.extends_sources.iter().cloned())
        .collect::<BTreeSet<_>>();
    Ok((configs, referenced_configs))
}

fn is_typescript_config_source(source: &str) -> bool {
    let Some(name) = Path::new(source)
        .file_name()
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    (lower == "tsconfig.json" || (lower.starts_with("tsconfig.") && lower.ends_with(".json")))
        || (lower == "jsconfig.json"
            || (lower.starts_with("jsconfig.") && lower.ends_with(".json")))
}

fn is_safe_relative_source(source: &str) -> bool {
    let path = Path::new(source);
    !path.is_absolute()
        && !path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
}

fn read_typescript_config_source(
    root: &Path,
    source: &str,
    sources: &HashMap<String, String>,
) -> Result<Option<String>, String> {
    let mut in_memory = sources
        .iter()
        .filter(|(candidate, _)| source_key(candidate, root) == source)
        .collect::<Vec<_>>();
    in_memory.sort_by_key(|(candidate, _)| *candidate);
    if let Some((_, contents)) = in_memory.first() {
        if in_memory
            .iter()
            .skip(1)
            .any(|(_, candidate)| *candidate != *contents)
        {
            return Err(format!(
                "multiple in-memory contents disagree for TypeScript config {source:?}"
            ));
        }
        if contents.is_empty() {
            return Ok(None);
        }
        if contents.len() as u64 <= MAX_TYPESCRIPT_CONFIG_BYTES {
            return Ok(Some((*contents).clone()));
        }
        return Err(format!(
            "TypeScript config {source:?} exceeds {MAX_TYPESCRIPT_CONFIG_BYTES} bytes"
        ));
    }
    let path = root.join(source);
    let canonical = match std::fs::canonicalize(&path) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    if !canonical.starts_with(root) {
        return Ok(None);
    }
    match compass_files::read_source_lossy(&canonical, MAX_TYPESCRIPT_CONFIG_BYTES) {
        Ok(contents) => Ok(Some(contents)),
        Err(_) => Ok(None),
    }
}

fn parse_typescript_path_config(
    root: &Path,
    source: &str,
    contents: &str,
    sources: &HashMap<String, String>,
    cache: &mut HashMap<String, Option<TypeScriptConfigValues>>,
) -> Result<Option<TypeScriptPathConfig>, String> {
    let mut stack = Vec::new();
    let Some(values) =
        load_typescript_config_values(root, source, contents, sources, cache, &mut stack, 0)?
    else {
        return Ok(None);
    };
    let config_path = root.join(source);
    let Some(directory) = config_path.parent() else {
        return Ok(None);
    };
    let kind = Path::new(source)
        .file_name()
        .and_then(|value| value.to_str())
        .map_or(TypeScriptConfigKind::TypeScript, |name| {
            if name.to_ascii_lowercase().starts_with("jsconfig.") {
                TypeScriptConfigKind::JavaScript
            } else {
                TypeScriptConfigKind::TypeScript
            }
        });
    let has_project_metadata = values.base_url.is_some()
        || values.paths.as_ref().is_some_and(|paths| !paths.is_empty())
        || values
            .root_dirs
            .as_ref()
            .is_some_and(|roots| !roots.is_empty())
        || values.module.is_some()
        || values.module_resolution.is_some()
        || values.module_suffixes.is_some()
        || values.allow_js.is_some()
        || values.check_js.is_some()
        || values.resolve_json_module.is_some()
        || !values.references.is_empty()
        || values.files.as_ref().is_some_and(|files| !files.is_empty())
        || values
            .include
            .as_ref()
            .is_some_and(|include| !include.is_empty())
        || values
            .exclude
            .as_ref()
            .is_some_and(|exclude| !exclude.is_empty())
        || values
            .type_roots
            .as_ref()
            .is_some_and(|roots| !roots.is_empty())
        || values
            .custom_conditions
            .as_ref()
            .is_some_and(|conditions| !conditions.is_empty());
    if !has_project_metadata {
        return Ok(None);
    }
    let mut references = values.references;
    references.sort();
    references.dedup();
    let mut extends_sources = values.extends_sources;
    extends_sources.sort();
    extends_sources.dedup();
    let mut type_roots = values.type_roots.unwrap_or_default();
    type_roots.sort();
    type_roots.dedup();
    let custom_conditions = values
        .custom_conditions
        .unwrap_or_default()
        .into_iter()
        .fold(Vec::new(), |mut conditions, condition| {
            if !conditions.contains(&condition) {
                conditions.push(condition);
            }
            conditions
        });
    Ok(Some(TypeScriptPathConfig {
        source: source.to_owned(),
        directory: directory.to_path_buf(),
        base_url: values.base_url,
        rules: values.paths.unwrap_or_default(),
        root_dirs: values.root_dirs.unwrap_or_default(),
        module: values.module,
        module_resolution: values.module_resolution,
        module_suffixes: values.module_suffixes.unwrap_or_default(),
        kind,
        allow_js: values.allow_js.unwrap_or(false),
        check_js: values.check_js.unwrap_or(false),
        resolve_json_module: values.resolve_json_module.unwrap_or(false),
        references,
        extends_sources,
        files: values.files,
        include: values.include,
        exclude: values.exclude,
        type_roots,
        custom_conditions,
    }))
}

fn load_typescript_config_values(
    root: &Path,
    source: &str,
    contents: &str,
    sources: &HashMap<String, String>,
    cache: &mut HashMap<String, Option<TypeScriptConfigValues>>,
    stack: &mut Vec<String>,
    depth: usize,
) -> Result<Option<TypeScriptConfigValues>, String> {
    if depth > MAX_TYPESCRIPT_CONFIG_EXTENDS_DEPTH {
        return Err(format!(
            "TypeScript config extends depth exceeds {MAX_TYPESCRIPT_CONFIG_EXTENDS_DEPTH}"
        ));
    }
    if let Some(cached) = cache.get(source) {
        return Ok(cached.clone());
    }
    if stack.iter().any(|candidate| candidate == source) {
        return Err(format!(
            "TypeScript config extends cycle includes {source:?}"
        ));
    }
    let Some(value) = parse_jsonc(contents) else {
        cache.insert(source.to_owned(), None);
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        cache.insert(source.to_owned(), None);
        return Ok(None);
    };
    stack.push(source.to_owned());
    let extends = typescript_config_extends(object)?;
    let mut values = TypeScriptConfigValues::default();
    for base in extends {
        let base_source = resolve_typescript_extends_source(root, source, &base, sources)?
            .ok_or_else(|| {
                format!("TypeScript config {source:?} extends missing config {base:?}")
            })?;
        let base_contents = read_typescript_config_source(root, &base_source, sources)?
            .ok_or_else(|| {
                format!("TypeScript config {source:?} extends unreadable config {base_source:?}")
            })?;
        let base_values = load_typescript_config_values(
            root,
            &base_source,
            &base_contents,
            sources,
            cache,
            stack,
            depth.saturating_add(1),
        )?
        .ok_or_else(|| {
            format!("TypeScript config {source:?} extends invalid config {base_source:?}")
        })?;
        values.overlay(base_values);
        values.extends_sources.push(base_source);
    }
    let local = parse_typescript_config_values(root, source, object, values.base_url.as_ref())?;
    values.overlay(local);
    stack.pop();
    cache.insert(source.to_owned(), Some(values.clone()));
    Ok(Some(values))
}

fn typescript_config_extends(
    object: &serde_json::Map<String, Value>,
) -> Result<Vec<String>, String> {
    let Some(value) = object.get("extends") else {
        return Ok(Vec::new());
    };
    let values = match value {
        Value::String(value) => vec![Some(value.clone())],
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
        _ => return Err("TypeScript config extends must be a string or array".to_owned()),
    };
    if values.len() > MAX_TYPESCRIPT_CONFIG_EXTENDS {
        return Err(format!(
            "TypeScript config extends count exceeds {MAX_TYPESCRIPT_CONFIG_EXTENDS}"
        ));
    }
    values
        .into_iter()
        .map(|value| {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                return Err(
                    "TypeScript config extends entries must be non-empty strings".to_owned(),
                );
            };
            if value.len() > 4_096 || value.contains('\0') {
                return Err(
                    "TypeScript config extends entry is too long or contains NUL".to_owned(),
                );
            }
            Ok(value.to_owned())
        })
        .collect()
}

fn resolve_typescript_extends_source(
    root: &Path,
    source: &str,
    extends: &str,
    sources: &HashMap<String, String>,
) -> Result<Option<String>, String> {
    let source_directory = root
        .join(source)
        .parent()
        .map_or_else(|| root.to_path_buf(), Path::to_path_buf);
    let mut candidates = Vec::new();
    let mut push_candidate = |candidate: PathBuf| {
        let candidate = lexical_path(&candidate);
        if !candidate.starts_with(root) {
            return;
        }
        let key = source_key(&candidate.to_string_lossy(), root);
        if is_safe_relative_source(&key) && !candidates.contains(&key) {
            candidates.push(key);
        }
    };
    if extends.starts_with('.') || extends.starts_with('/') {
        let Some(path) = safe_typescript_config_path(&source_directory, root, extends) else {
            return Ok(None);
        };
        push_candidate(path.clone());
        if path.extension().is_none() {
            push_candidate(path.with_extension("json"));
        }
    } else {
        let mut directory = source_directory;
        for _ in 0..MAX_TYPESCRIPT_CONFIG_EXTENDS_DEPTH {
            let package = directory.join("node_modules").join(extends);
            push_candidate(package.clone());
            if package.extension().is_none() {
                push_candidate(package.with_extension("json"));
            }
            push_candidate(package.join("tsconfig.json"));
            if directory == root {
                break;
            }
            let Some(parent) = directory.parent() else {
                break;
            };
            if !parent.starts_with(root) {
                break;
            }
            directory = parent.to_path_buf();
        }
    }
    for candidate in candidates {
        if read_typescript_config_source(root, &candidate, sources)?.is_some() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn parse_typescript_config_values(
    root: &Path,
    source: &str,
    object: &serde_json::Map<String, Value>,
    inherited_base_url: Option<&PathBuf>,
) -> Result<TypeScriptConfigValues, String> {
    let config_path = root.join(source);
    let Some(directory) = config_path.parent() else {
        return Ok(TypeScriptConfigValues::default());
    };
    let options = object.get("compilerOptions").and_then(Value::as_object);
    let local_base_url = options
        .and_then(|options| options.get("baseUrl"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4_096)
        .and_then(|value| safe_typescript_config_path(directory, root, value));
    let effective_path_base = local_base_url
        .clone()
        .or_else(|| inherited_base_url.cloned())
        .unwrap_or_else(|| directory.to_path_buf());
    let mut values = TypeScriptConfigValues {
        base_url: local_base_url,
        ..TypeScriptConfigValues::default()
    };
    values.files = parse_typescript_file_patterns(object.get("files"), "files", directory)?;
    values.include = parse_typescript_file_patterns(object.get("include"), "include", directory)?;
    values.exclude = parse_typescript_file_patterns(object.get("exclude"), "exclude", directory)?;
    if let Some(options) = options {
        if let Some(paths) = options.get("paths") {
            let Some(paths) = paths.as_object() else {
                return Err("TypeScript compilerOptions.paths must be an object".to_owned());
            };
            if paths.len() > MAX_TYPESCRIPT_PATH_RULES {
                return Err(format!(
                    "TypeScript path rule count {} exceeds limit {MAX_TYPESCRIPT_PATH_RULES}",
                    paths.len()
                ));
            }
            let mut rules = Vec::new();
            let mut target_count = 0_usize;
            for (pattern, targets) in paths {
                if pattern.is_empty() || pattern.len() > 4_096 || pattern.matches('*').count() > 1 {
                    continue;
                }
                let target_values = match targets {
                    Value::Array(values) => values.iter().collect::<Vec<_>>(),
                    Value::String(_) => vec![targets],
                    _ => Vec::new(),
                };
                let mut normalized_targets = Vec::new();
                for target in target_values {
                    let Some(target) = target.as_str() else {
                        continue;
                    };
                    if target.is_empty()
                        || target.len() > 4_096
                        || target.matches('*').count() > 1
                        || safe_typescript_config_path(&effective_path_base, root, target).is_none()
                    {
                        continue;
                    }
                    normalized_targets.push(TypeScriptPathTarget {
                        value: target.to_owned(),
                        base: effective_path_base.clone(),
                    });
                    target_count = target_count.saturating_add(1);
                    if target_count > MAX_TYPESCRIPT_PATH_TARGETS {
                        return Err(format!(
                            "TypeScript path target count exceeds limit {MAX_TYPESCRIPT_PATH_TARGETS}"
                        ));
                    }
                }
                if normalized_targets.is_empty() {
                    continue;
                }
                let (prefix, suffix) = pattern.split_once('*').map_or_else(
                    || (pattern.as_str(), ""),
                    |(prefix, suffix)| (prefix, suffix),
                );
                rules.push(TypeScriptPathRule {
                    pattern: pattern.clone(),
                    prefix: prefix.to_owned(),
                    suffix: suffix.to_owned(),
                    targets: normalized_targets,
                });
            }
            rules.sort_by(|left, right| {
                right
                    .prefix
                    .len()
                    .cmp(&left.prefix.len())
                    .then_with(|| left.pattern.cmp(&right.pattern))
            });
            values.paths = Some(rules);
        }
        values.root_dirs = options.get("rootDirs").map(|value| {
            value
                .as_array()
                .map(|roots| {
                    roots
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(|root_dir| {
                            safe_typescript_config_path(directory, root, root_dir)
                        })
                        .take(256)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        values.module = options
            .get("module")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .map(str::to_ascii_lowercase);
        values.module_resolution = options
            .get("moduleResolution")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .map(str::to_ascii_lowercase);
        values.module_suffixes = options.get("moduleSuffixes").map(|value| {
            value
                .as_array()
                .map(|suffixes| {
                    suffixes
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|suffix| suffix.len() <= 128)
                        .map(str::to_owned)
                        .take(256)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        values.allow_js = options.get("allowJs").and_then(Value::as_bool);
        values.check_js = options.get("checkJs").and_then(Value::as_bool);
        values.resolve_json_module = options.get("resolveJsonModule").and_then(Value::as_bool);
        values.type_roots = parse_typescript_path_array(
            options.get("typeRoots"),
            "compilerOptions.typeRoots",
            directory,
            root,
            MAX_TYPESCRIPT_TYPE_ROOTS,
        )?;
        values.custom_conditions = parse_typescript_string_array(
            options.get("customConditions"),
            "compilerOptions.customConditions",
            MAX_TYPESCRIPT_CUSTOM_CONDITIONS,
        )?;
    }
    if let Some(references) = object.get("references") {
        let Some(references) = references.as_array() else {
            return Err("TypeScript config references must be an array".to_owned());
        };
        if references.len() > MAX_TYPESCRIPT_CONFIG_REFERENCES {
            return Err(format!(
                "TypeScript project reference count exceeds {MAX_TYPESCRIPT_CONFIG_REFERENCES}"
            ));
        }
        for reference in references {
            let Some(path) = reference
                .as_object()
                .and_then(|reference| reference.get("path"))
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty() && path.len() <= 4_096)
            else {
                continue;
            };
            if let Some(path) = safe_typescript_config_path(directory, root, path) {
                values.references.push(path);
            }
        }
    }
    Ok(values)
}

fn parse_typescript_file_patterns(
    value: Option<&Value>,
    key: &str,
    base: &Path,
) -> Result<Option<Vec<TypeScriptFilePattern>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(format!("TypeScript config {key} must be an array"));
    };
    if values.len() > MAX_TYPESCRIPT_FILE_PATTERNS {
        return Err(format!(
            "TypeScript config {key} count exceeds {MAX_TYPESCRIPT_FILE_PATTERNS}"
        ));
    }
    let mut patterns = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = value.as_str() else {
            return Err(format!("TypeScript config {key} entries must be strings"));
        };
        let normalized = value.replace('\\', "/");
        if normalized.is_empty()
            || normalized.len() > 4_096
            || normalized.contains('\0')
            || Path::new(&normalized).is_absolute()
        {
            return Err(format!(
                "TypeScript config {key} contains an invalid pattern"
            ));
        }
        patterns.push(TypeScriptFilePattern {
            value: normalized,
            base: base.to_path_buf(),
        });
    }
    Ok(Some(patterns))
}

fn parse_typescript_path_array(
    value: Option<&Value>,
    key: &str,
    base: &Path,
    root: &Path,
    limit: usize,
) -> Result<Option<Vec<PathBuf>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(format!("TypeScript config {key} must be an array"));
    };
    if values.len() > limit {
        return Err(format!("TypeScript config {key} count exceeds {limit}"));
    }
    let mut paths = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = value.as_str().filter(|value| !value.is_empty()) else {
            return Err(format!("TypeScript config {key} entries must be strings"));
        };
        if value.len() > 4_096 || value.contains('\0') {
            return Err(format!("TypeScript config {key} contains an invalid path"));
        }
        let Some(path) = safe_typescript_config_path(base, root, value) else {
            return Err(format!(
                "TypeScript config {key} path escapes the workspace"
            ));
        };
        paths.push(path);
    }
    Ok(Some(paths))
}

fn parse_typescript_string_array(
    value: Option<&Value>,
    key: &str,
    limit: usize,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(format!("TypeScript config {key} must be an array"));
    };
    if values.len() > limit {
        return Err(format!("TypeScript config {key} count exceeds {limit}"));
    }
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = value.as_str().filter(|value| !value.is_empty()) else {
            return Err(format!("TypeScript config {key} entries must be strings"));
        };
        if value.len() > 256 || value.contains(['\\', '\0']) {
            return Err(format!(
                "TypeScript config {key} contains an invalid condition"
            ));
        }
        output.push(value.to_owned());
    }
    Ok(Some(output))
}

fn safe_typescript_config_path(base: &Path, root: &Path, value: &str) -> Option<PathBuf> {
    if value.contains('\0') {
        return None;
    }
    // TypeScript project files commonly use Windows separators even when the
    // graph is qualified on another host. Normalize only the configuration
    // spelling; source inventory paths remain platform-native and bounded.
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    let candidate = if path.is_absolute() {
        lexical_path(path)
    } else {
        lexical_path(&base.join(path))
    };
    candidate.starts_with(root).then_some(candidate)
}

fn select_typescript_path_config<'a>(
    configs: &'a [TypeScriptPathConfig],
    referenced_configs: &BTreeSet<String>,
    root: &Path,
    importer: &str,
) -> Option<&'a TypeScriptPathConfig> {
    let importer_key = source_key(importer, root);
    if !is_safe_relative_source(&importer_key) {
        return None;
    }
    let importer_path = root.join(&importer_key);
    let importer_extension = extension(importer);
    let mut candidates = configs
        .iter()
        .filter(|config| {
            !referenced_configs.contains(&config.source)
                && importer_path.starts_with(&config.directory)
                && typescript_config_applies(config, &importer_extension)
                && typescript_config_owns_source(config, &importer_key, root, false)
        })
        .collect::<Vec<_>>();
    let deepest = candidates
        .iter()
        .map(|config| config.directory.components().count())
        .max()?;
    candidates.retain(|config| config.directory.components().count() == deepest);
    if candidates.len() == 1 {
        return candidates.pop();
    }
    // Two configs at the same project depth are not interchangeable. A
    // compiler invocation selects one explicitly; Compass has no such command
    // context, so it preserves the import rather than guessing.
    None
}

fn typescript_config_applies(config: &TypeScriptPathConfig, extension: &str) -> bool {
    let javascript = matches!(extension, "js" | "jsx" | "mjs" | "cjs");
    match config.kind {
        TypeScriptConfigKind::JavaScript => javascript,
        TypeScriptConfigKind::TypeScript => !javascript || config.allow_js || config.check_js,
    }
}

fn typescript_config_owns_source(
    config: &TypeScriptPathConfig,
    source: &str,
    root: &Path,
    allow_missing_extension: bool,
) -> bool {
    let source_key = source_key(source, root);
    if !is_safe_relative_source(&source_key) {
        return false;
    }
    let source_path = root.join(&source_key);
    let extension = extension(&source_key);
    if !allow_missing_extension
        && !typescript_config_applies(config, &extension)
        && !extension.is_empty()
    {
        return false;
    }

    let explicitly_included = config.files.as_ref().map_or_else(
        || {
            config.include.as_ref().is_none_or(|patterns| {
                patterns
                    .iter()
                    .any(|pattern| typescript_pattern_matches(pattern, &source_path, false))
            })
        },
        |patterns| {
            patterns
                .iter()
                .any(|pattern| typescript_pattern_matches(pattern, &source_path, true))
        },
    );
    if !explicitly_included {
        return false;
    }

    if config.exclude.as_ref().is_some_and(|patterns| {
        patterns
            .iter()
            .any(|pattern| typescript_pattern_matches(pattern, &source_path, false))
    }) {
        return false;
    }

    // TypeScript's default exclude keeps dependency trees out of a project
    // when the config did not supply an explicit exclude list. Preserve this
    // boundary even when the source inventory contains vendored files.
    if config.exclude.is_none()
        && source_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::Normal(value)
                    if matches!(
                        value.to_str(),
                        Some("node_modules") | Some("bower_components") | Some("jspm_packages")
                    )
            )
        })
    {
        return false;
    }
    true
}

fn typescript_pattern_matches(
    pattern: &TypeScriptFilePattern,
    source: &Path,
    exact_file: bool,
) -> bool {
    let Ok(relative) = source.strip_prefix(&pattern.base) else {
        return false;
    };
    let relative = relative
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_owned();
    let pattern_value = pattern.value.trim_start_matches("./");
    if !exact_file
        && !pattern_value.contains(['*', '?'])
        && (relative == pattern_value || relative.starts_with(&format!("{pattern_value}/")))
    {
        return true;
    }
    glob_path_matches(pattern_value, &relative)
}

fn glob_path_matches(pattern: &str, path: &str) -> bool {
    let pattern_parts = pattern
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let path_parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    fn matches_segment(pattern: &str, value: &str) -> bool {
        let pattern = pattern.as_bytes();
        let value = value.as_bytes();
        let mut table = vec![vec![false; value.len() + 1]; pattern.len() + 1];
        table[0][0] = true;
        for index in 0..pattern.len() {
            for value_index in 0..=value.len() {
                if !table[index][value_index] {
                    continue;
                }
                match pattern[index] {
                    b'*' => {
                        table[index + 1][value_index] = true;
                        if value_index < value.len() {
                            table[index][value_index + 1] = true;
                        }
                    }
                    b'?' if value_index < value.len() => {
                        table[index + 1][value_index + 1] = true;
                    }
                    byte if value_index < value.len() && byte == value[value_index] => {
                        table[index + 1][value_index + 1] = true;
                    }
                    _ => {}
                }
            }
        }
        table[pattern.len()][value.len()]
    }
    fn matches_parts(pattern: &[&str], path: &[&str]) -> bool {
        if pattern.is_empty() {
            return path.is_empty();
        }
        if pattern[0] == "**" {
            return matches_parts(&pattern[1..], path)
                || (!path.is_empty() && matches_parts(pattern, &path[1..]));
        }
        !path.is_empty()
            && matches_segment(pattern[0], path[0])
            && matches_parts(&pattern[1..], &path[1..])
    }
    matches_parts(&pattern_parts, &path_parts)
}

fn resolve_typescript_module(
    config: &TypeScriptPathConfig,
    module: &str,
    file_by_source: &BTreeMap<String, (String, String)>,
    root: &Path,
) -> Option<(String, &'static str)> {
    let mut matching = config
        .rules
        .iter()
        .filter_map(|rule| {
            let wildcard = if rule.pattern.contains('*') {
                if !module.starts_with(&rule.prefix) || !module.ends_with(&rule.suffix) {
                    return None;
                }
                let end = module.len().saturating_sub(rule.suffix.len());
                (end >= rule.prefix.len()).then(|| module[rule.prefix.len()..end].to_owned())
            } else {
                (rule.pattern == module).then(String::new)
            }?;
            Some((rule, wildcard))
        })
        .collect::<Vec<_>>();
    if let Some(longest_prefix) = matching.iter().map(|(rule, _)| rule.prefix.len()).max() {
        matching.retain(|(rule, _)| rule.prefix.len() == longest_prefix);
    }
    if !matching.is_empty() {
        if matching.len() != 1 {
            return None;
        }
        let (rule, wildcard) = matching.pop()?;
        for target in &rule.targets {
            let substituted = if rule.pattern.contains('*') {
                target.value.replace('*', &wildcard)
            } else {
                target.value.clone()
            };
            if let Some(source) = resolve_typescript_target(
                &target.base,
                &substituted,
                file_by_source,
                root,
                config.resolve_json_module,
                &config.module_suffixes,
            ) && typescript_config_owns_source(config, &source, root, true)
            {
                return Some((source, "typescript-paths"));
            }
        }
        return None;
    }
    if let Some(base_url) = config.base_url.as_ref()
        && let Some(source) = resolve_typescript_target(
            base_url,
            module,
            file_by_source,
            root,
            config.resolve_json_module,
            &config.module_suffixes,
        )
        && typescript_config_owns_source(config, &source, root, true)
    {
        return Some((source, "typescript-base-url"));
    }
    resolve_typescript_type_root_module(config, module, file_by_source, root)
}

fn resolve_typescript_type_root_module(
    config: &TypeScriptPathConfig,
    module: &str,
    file_by_source: &BTreeMap<String, (String, String)>,
    root: &Path,
) -> Option<(String, &'static str)> {
    if config.type_roots.is_empty() || module.is_empty() || module.starts_with('#') {
        return None;
    }
    let mut package_names = vec![module.to_owned()];
    if let Some((scope, package)) = module
        .strip_prefix('@')
        .and_then(|module| module.split_once('/'))
        && !scope.is_empty()
        && !package.is_empty()
    {
        package_names.push(format!("{scope}__{package}"));
    }
    for type_root in &config.type_roots {
        for package_name in &package_names {
            if let Some(source) = resolve_typescript_target(
                type_root,
                package_name,
                file_by_source,
                root,
                config.resolve_json_module,
                &config.module_suffixes,
            ) && typescript_config_owns_source(config, &source, root, true)
            {
                return Some((source, "typescript-type-roots"));
            }
            // A typeRoots entry is commonly the parent of an `@types`
            // directory, while some projects point directly at that folder.
            // Try the explicit package path only when it differs from the
            // direct candidate to preserve deterministic target order.
            if !type_root.ends_with("@types") {
                let at_types = format!("@types/{package_name}");
                if let Some(source) = resolve_typescript_target(
                    type_root,
                    &at_types,
                    file_by_source,
                    root,
                    config.resolve_json_module,
                    &config.module_suffixes,
                ) && typescript_config_owns_source(config, &source, root, true)
                {
                    return Some((source, "typescript-type-roots"));
                }
            }
        }
    }
    None
}

fn resolve_typescript_relative_module(
    config: Option<&TypeScriptPathConfig>,
    importer: &str,
    module: &str,
    file_by_source: &BTreeMap<String, (String, String)>,
    root: &Path,
) -> Option<(String, &'static str)> {
    let importer_key = source_key(importer, root);
    if !is_safe_relative_source(&importer_key) {
        return None;
    }
    let importer_path = root.join(&importer_key);
    let importer_directory = importer_path.parent().unwrap_or(root);
    let raw_module = module.replace('\\', "/");
    let direct = lexical_path(&importer_directory.join(&raw_module));
    if !direct.starts_with(root) {
        return None;
    }
    let mut bases = vec![(direct, "typescript-relative")];
    if let Some(config) = config
        && !config.root_dirs.is_empty()
    {
        let mut virtual_relative = None;
        let mut importer_root = None;
        for root_dir in &config.root_dirs {
            if let Ok(relative) = importer_path.strip_prefix(root_dir) {
                virtual_relative = Some(relative.to_path_buf());
                importer_root = Some(root_dir);
                break;
            }
        }
        if let (Some(relative), Some(importer_root)) = (virtual_relative, importer_root) {
            let virtual_parent = relative.parent().unwrap_or_else(|| Path::new(""));
            for root_dir in &config.root_dirs {
                if root_dir == importer_root {
                    continue;
                }
                let candidate = lexical_path(&root_dir.join(virtual_parent).join(&raw_module));
                if candidate.starts_with(root) {
                    bases.push((candidate, "typescript-root-dirs"));
                }
            }
        }
    }
    let module_suffixes =
        config.map_or(&[] as &[String], |config| config.module_suffixes.as_slice());
    for (base, rule) in bases {
        let candidates = typescript_target_candidates(&base, module_suffixes);
        for candidate in candidates {
            if config.is_some_and(|config| {
                !config.resolve_json_module
                    && candidate
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case("json"))
            }) {
                continue;
            }
            let key = source_key(&candidate.to_string_lossy(), root);
            if file_by_source.contains_key(&key)
                && config
                    .is_none_or(|config| typescript_config_owns_source(config, &key, root, true))
            {
                return Some((key, rule));
            }
        }
    }
    None
}

fn resolve_typescript_target(
    base: &Path,
    target: &str,
    file_by_source: &BTreeMap<String, (String, String)>,
    root: &Path,
    resolve_json_module: bool,
    module_suffixes: &[String],
) -> Option<String> {
    let target = safe_typescript_config_path(base, root, target)?;
    let candidates = typescript_target_candidates(&target, module_suffixes);
    for candidate in candidates {
        if !resolve_json_module
            && candidate
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let key = source_key(&candidate.to_string_lossy(), root);
        if file_by_source.contains_key(&key) {
            return Some(key);
        }
    }
    None
}

fn typescript_target_candidates(path: &Path, module_suffixes: &[String]) -> Vec<PathBuf> {
    let mut base_candidates = Vec::new();
    let mut push = |candidate: PathBuf| {
        if !base_candidates.contains(&candidate) {
            base_candidates.push(candidate);
        }
    };
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "js" => {
            push(path.with_extension("ts"));
            push(path.with_extension("tsx"));
            push(path.with_extension("d.ts"));
            push(path.to_path_buf());
            push(path.with_extension("jsx"));
        }
        "jsx" => {
            push(path.with_extension("tsx"));
            push(path.with_extension("d.ts"));
            push(path.to_path_buf());
        }
        "mjs" => {
            push(path.with_extension("mts"));
            push(path.with_extension("d.mts"));
            push(path.to_path_buf());
        }
        "cjs" => {
            push(path.with_extension("cts"));
            push(path.with_extension("d.cts"));
            push(path.to_path_buf());
        }
        _ if !extension.is_empty() => push(path.to_path_buf()),
        _ => {
            push(path.to_path_buf());
            for extension in [
                "ts", "tsx", "d.ts", "mts", "d.mts", "cts", "d.cts", "js", "jsx", "mjs", "cjs",
            ] {
                push(path.with_file_name(format!(
                    "{}.{extension}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                )));
            }
        }
    }
    for index in [
        "index.ts",
        "index.tsx",
        "index.d.ts",
        "index.mts",
        "index.d.mts",
        "index.cts",
        "index.d.cts",
        "index.js",
        "index.jsx",
        "index.mjs",
        "index.cjs",
    ] {
        push(path.join(index));
    }
    let suffixes = if module_suffixes.is_empty() {
        vec![String::new()]
    } else {
        module_suffixes.to_vec()
    };
    let mut candidates = Vec::new();
    for suffix in suffixes {
        for candidate in &base_candidates {
            let candidate = add_typescript_module_suffix(candidate, &suffix);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn add_typescript_module_suffix(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return path.to_path_buf();
    }
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return path.to_path_buf();
    };
    let (stem, extension) = [".d.mts", ".d.cts", ".d.ts"]
        .into_iter()
        .find_map(|extension| {
            file_name
                .strip_suffix(extension)
                .map(|stem| (stem, extension))
        })
        .or_else(|| {
            file_name.rsplit_once('.').map(|(stem, extension)| {
                (
                    stem,
                    &file_name[file_name.len().saturating_sub(extension.len() + 1)..],
                )
            })
        })
        .unwrap_or((file_name, ""));
    let replacement = if extension.is_empty() {
        format!("{stem}{suffix}")
    } else {
        format!("{stem}{suffix}{extension}")
    };
    path.with_file_name(replacement)
}

fn rooted_source_path(root: &Path, source: &str) -> Result<PathBuf, String> {
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let normalized = std::fs::canonicalize(&absolute)
        .map_err(|error| format!("cannot resolve package manifest {source:?}: {error}"))?;
    if !normalized.starts_with(root) {
        return Err(format!(
            "package manifest escapes repository root: {source:?}"
        ));
    }
    Ok(normalized)
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn javascript_package_target(directory: &Path, root: &Path, target: &str) -> Option<PathBuf> {
    if !target.starts_with("./") || target.len() > 4_096 || target.contains(['\\', '\0']) {
        return None;
    }
    let target = lexical_path(&directory.join(target));
    target.starts_with(root).then_some(target)
}

fn collect_javascript_package_exports(
    value: &Value,
    exports: &mut BTreeMap<String, BTreeSet<String>>,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("package exports nesting exceeds limit 16".to_owned());
    }
    match value {
        Value::String(target) => {
            exports
                .entry(".".to_owned())
                .or_default()
                .insert(target.clone());
        }
        Value::Object(entries) => {
            for (key, value) in entries {
                if key == "." || key.starts_with("./") {
                    let mut values = BTreeSet::new();
                    collect_javascript_export_targets(value, &mut values, depth + 1)?;
                    exports.entry(key.clone()).or_default().extend(values);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_javascript_export_targets(
    value: &Value,
    targets: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("package export condition nesting exceeds limit 16".to_owned());
    }
    match value {
        Value::String(target) => {
            targets.insert(target.clone());
        }
        Value::Array(values) => {
            for value in values.iter().take(256) {
                collect_javascript_export_targets(value, targets, depth + 1)?;
            }
            if values.len() > 256 {
                return Err("package export fallback count exceeds limit 256".to_owned());
            }
        }
        Value::Object(values) => {
            if values.len() > 256 {
                return Err("package export condition count exceeds limit 256".to_owned());
            }
            for value in values.values() {
                collect_javascript_export_targets(value, targets, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Repoint named imports to the unique declaration exported by the resolved
/// package entry point. Wildcard barrel traversal is bounded and ambiguity is
/// deliberately left unresolved.
fn resolve_javascript_workspace_symbols(extraction: &mut Extraction) {
    let mut file_by_source = HashMap::new();
    let mut file_ids = HashSet::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if is_file_node(node, &source) {
            file_ids.insert(node.id.clone());
            file_by_source
                .entry(source)
                .or_insert_with(|| node.id.clone());
        }
    }
    let mut package_roots = HashMap::<(String, String), Vec<String>>::new();
    let mut reexports = HashMap::<String, Vec<String>>::new();
    for edge in &extraction.edges {
        if relation(edge) == "imports_from"
            && file_ids.contains(&edge.target)
            && !edge.string("module").is_empty()
        {
            let module = edge.string("module");
            package_roots
                .entry((edge.source.clone(), module))
                .or_default()
                .push(edge.target.clone());
        }
        if relation(edge) == "re_exports"
            && file_ids.contains(&edge.source)
            && file_ids.contains(&edge.target)
        {
            reexports
                .entry(edge.source.clone())
                .or_default()
                .push(edge.target.clone());
        }
    }
    for values in package_roots.values_mut().chain(reexports.values_mut()) {
        values.sort_unstable();
        values.dedup();
    }

    let mut declarations = HashMap::<(String, String), Vec<String>>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        let kind = string_attribute(node, "symbol_kind");
        if !is_javascript(&source) || !javascript_export_candidate(&kind) {
            continue;
        }
        let Some(file_id) = file_by_source.get(&source) else {
            continue;
        };
        let spelling = node
            .label()
            .split_once('(')
            .map_or_else(|| node.label(), |(name, _)| name)
            .trim();
        if !spelling.is_empty() {
            declarations
                .entry((file_id.clone(), spelling.to_owned()))
                .or_default()
                .push(node.id.clone());
        }
    }
    for values in declarations.values_mut() {
        values.sort_unstable();
        values.dedup();
    }

    let mut repointed = HashSet::new();
    for edge in &mut extraction.edges {
        if relation(edge) != "imports" {
            continue;
        }
        let module = edge.string("module");
        let imported_name = edge.string("imported_name");
        if module.is_empty() || imported_name.is_empty() {
            continue;
        }
        let Some(roots) = package_roots.get(&(edge.source.clone(), module)) else {
            continue;
        };
        let mut queue = roots.iter().cloned().collect::<VecDeque<_>>();
        let mut visited = BTreeSet::new();
        let mut candidates = BTreeSet::new();
        while let Some(file) = queue.pop_front() {
            if !visited.insert(file.clone()) || visited.len() > 4_096 {
                continue;
            }
            if let Some(ids) = declarations.get(&(file.clone(), imported_name.clone())) {
                candidates.extend(ids.iter().cloned());
            }
            if let Some(targets) = reexports.get(&file) {
                queue.extend(targets.iter().cloned());
            }
        }
        if candidates.len() != 1 {
            continue;
        }
        let Some(target) = candidates.into_iter().next() else {
            continue;
        };
        if edge.target != target {
            repointed.insert(edge.target.clone());
            edge.target = target;
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
    }
    let mut imported_bindings = HashMap::<(String, String), BTreeSet<String>>::new();
    for edge in &extraction.edges {
        if relation(edge) == "imports" {
            let local_name = edge.string("local_name");
            if !local_name.is_empty() {
                imported_bindings
                    .entry((edge.source.clone(), local_name))
                    .or_default()
                    .insert(edge.target.clone());
            }
        }
    }
    for edge in &mut extraction.edges {
        if relation(edge) != "references" {
            continue;
        }
        let binding_name = edge.string("binding_name");
        let Some(targets) = imported_bindings.get(&(edge.source.clone(), binding_name)) else {
            continue;
        };
        if targets.len() != 1 {
            continue;
        }
        let Some(target) = targets.iter().next() else {
            continue;
        };
        if edge.target != *target {
            repointed.insert(edge.target.clone());
            edge.target.clone_from(target);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
    }
    drop_unreferenced_nodes(extraction, &repointed);
}

fn javascript_export_candidate(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "enum" | "function" | "interface" | "type_alias" | "variable" | "constant"
    )
}

/// Compass's per-file JavaScript extractor emits only the explicit
/// `imports_from` module edge plus named symbol re-exports. Its collection pass
/// then adds the file-level `re_exports` edge used by cycle and facade analysis.
fn resolve_javascript_reexports(extraction: &mut Extraction) {
    let mut existing = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
                edge.string("context"),
            )
        })
        .collect::<HashSet<_>>();
    let additions = extraction
        .edges
        .iter()
        .filter(|edge| relation(edge) == "imports_from" && edge.string("context") == "re-export")
        .filter_map(|edge| {
            let key = (
                edge.source.clone(),
                edge.target.clone(),
                "re_exports".to_owned(),
                "export".to_owned(),
            );
            if !existing.insert(key) {
                return None;
            }
            let mut resolved = edge.clone();
            resolved.attributes.insert(
                "relation".to_owned(),
                Value::String("re_exports".to_owned()),
            );
            resolved
                .attributes
                .insert("context".to_owned(), Value::String("export".to_owned()));
            Some(resolved)
        })
        .collect::<Vec<_>>();
    extraction.edges.extend(additions);
}

/// Match Python's last-writer graph semantics without making the retained C#
/// namespace depend on filesystem traversal order. Namespace IDs are label
/// based, so declarations from multiple files intentionally collide; the
/// lexicographically earliest source/location is the canonical representative.
fn canonicalize_csharp_namespace_nodes(extraction: &mut Extraction) {
    let mut by_label = HashMap::<String, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if string_attribute(node, "type") == "namespace" {
            by_label
                .entry(node.label().to_owned())
                .or_default()
                .push(index);
        }
    }

    let mut dropped = HashSet::new();
    let mut remap = HashMap::new();
    for indexes in by_label.values().filter(|indexes| indexes.len() > 1) {
        let canonical = indexes
            .iter()
            .copied()
            .min_by_key(|index| {
                let node = &extraction.nodes[*index];
                (
                    string_attribute(node, "source_file"),
                    string_attribute(node, "source_location"),
                    node.id.clone(),
                )
            })
            .unwrap_or(indexes[0]);
        let canonical_id = extraction.nodes[canonical].id.clone();
        for &index in indexes {
            if index != canonical {
                dropped.insert(index);
                remap.insert(extraction.nodes[index].id.clone(), canonical_id.clone());
            }
        }
    }
    if dropped.is_empty() {
        return;
    }
    for edge in &mut extraction.edges {
        let mut rewritten = false;
        if let Some(target) = remap.get(&edge.source) {
            edge.source.clone_from(target);
            rewritten = true;
        }
        if let Some(target) = remap.get(&edge.target) {
            edge.target.clone_from(target);
            rewritten = true;
        }
        if rewritten {
            stamp_endpoint_rewrite(
                edge,
                EndpointRewriteRule::CsharpNamespaceCanonicalization,
                1.0,
            );
        }
    }
    let mut index = 0_usize;
    extraction.nodes.retain(|_| {
        let keep = !dropped.contains(&index);
        index += 1;
        keep
    });
}

/// Resolve a sourceless type stub inside the language family of the edge that
/// references it. A globally common name such as `Processor` is ambiguous, but
/// a JVM edge can still have exactly one JVM definition. This is the same
/// conservative boundary used by Compass's Java/Groovy resolver.
fn rewire_unique_family_stubs(extraction: &mut Extraction) {
    let mut definitions = HashMap::<(String, &'static str), Vec<(String, String)>>::new();
    let mut stubs = HashMap::<String, String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        let label = node.label().trim().to_owned();
        if label.is_empty() {
            continue;
        }
        if source.is_empty() && !is_canonical_external_symbol(node) {
            stubs.insert(node.id.clone(), label);
        } else if is_type_like_definition(node)
            && let Some(family @ "jvm") = language_family(&source)
        {
            definitions
                .entry((label, family))
                .or_default()
                .push((node.id.clone(), source));
        }
    }
    if stubs.is_empty() {
        return;
    }
    let imports_by_source = extraction
        .edges
        .iter()
        .filter(|edge| matches!(relation(edge), "imports" | "imports_from"))
        .fold(
            HashMap::<String, HashSet<String>>::new(),
            |mut imports, edge| {
                imports
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
                imports
            },
        );
    let imports_by_file = extraction
        .edges
        .iter()
        .filter(|edge| matches!(relation(edge), "imports" | "imports_from"))
        .fold(
            HashMap::<String, HashSet<String>>::new(),
            |mut imports, edge| {
                imports
                    .entry(edge.string("source_file"))
                    .or_default()
                    .insert(edge.target.clone());
                imports
            },
        );

    let repoint_relations = ["implements", "inherits", "extends", "imports", "references"];
    let mut repointed = HashSet::new();
    for edge in &mut extraction.edges {
        if !repoint_relations.contains(&relation(edge)) {
            continue;
        }
        let Some(label) = stubs.get(&edge.target) else {
            continue;
        };
        let source_file = edge.string("source_file");
        let Some(family @ "jvm") = language_family(&source_file) else {
            continue;
        };
        let Some(candidates) = definitions.get(&(label.clone(), family)) else {
            continue;
        };
        let source_scope = repository_scope(&source_file);
        let compatible = candidates
            .iter()
            .filter(|(id, source)| {
                repository_scope(source) == source_scope
                    || imports_by_source
                        .get(&edge.source)
                        .is_some_and(|targets| targets.contains(id))
                    || imports_by_file
                        .get(&source_file)
                        .is_some_and(|targets| targets.contains(id))
            })
            .collect::<Vec<_>>();
        if let [target] = compatible.as_slice()
            && target.0 != edge.target
        {
            repointed.insert(edge.target.clone());
            edge.target.clone_from(&target.0);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::LanguageFamilyStubResolution, 0.9);
        }
    }
    drop_unreferenced_nodes(extraction, &repointed);
}

fn resolve_php_type_references(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    let namespace_re = Regex::new(r"(?im)^\s*namespace\s+([^;{]+)\s*[;{]")
        .unwrap_or_else(|_| unreachable!("static PHP namespace regex is valid"));
    let use_re = Regex::new(r"(?im)^\s*use\s+([^;]+);")
        .unwrap_or_else(|_| unreachable!("static PHP use regex is valid"));

    let stub_labels = extraction
        .nodes
        .iter()
        .filter(|node| string_attribute(node, "source_file").is_empty())
        .map(|node| (node.id.clone(), node.label().to_owned()))
        .collect::<HashMap<_, _>>();
    if stub_labels.is_empty() {
        return;
    }

    let mut facts = HashMap::<String, (String, HashMap<String, String>)>::new();
    for (source_file, source) in sources {
        if extension(source_file) != "php" {
            continue;
        }
        let namespace = namespace_re
            .captures(source)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim().trim_matches('\\').to_owned())
            .unwrap_or_default();
        let mut uses = HashMap::new();
        for captures in use_re.captures_iter(source) {
            let Some(body) = captures.get(1).map(|value| value.as_str().trim()) else {
                continue;
            };
            if body.starts_with("function ") || body.starts_with("const ") {
                continue;
            }
            for (alias, fqn) in php_use_entries(body) {
                uses.entry(alias.to_ascii_lowercase()).or_insert(fqn);
            }
        }
        facts.insert(source_file.clone(), (namespace, uses));
    }

    let mut internal_types = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    for node in &extraction.nodes {
        let source_file = string_attribute(node, "source_file");
        let Some((namespace, _)) = facts.get(&source_file) else {
            continue;
        };
        let label = node.label().trim();
        if label.is_empty()
            || label.ends_with(')')
            || label.contains('.')
            || is_file_node(node, &source_file)
            || !is_type_like_definition(node)
        {
            continue;
        }
        let fqn = if namespace.is_empty() {
            label.to_owned()
        } else {
            format!("{namespace}\\{label}")
        };
        internal_types
            .entry(fqn.to_ascii_lowercase())
            .or_default()
            .insert((node.id.clone(), source_file));
    }

    let mut created = HashSet::new();
    let mut new_nodes = Vec::new();
    let mut repointed = HashSet::new();
    for edge in &mut extraction.edges {
        if !matches!(
            relation(edge),
            "inherits" | "implements" | "mixes_in" | "imports" | "references"
        ) {
            continue;
        }
        let source_file = edge.string("source_file");
        let Some((namespace, uses)) = facts.get(&source_file) else {
            continue;
        };
        let Some(label) = stub_labels.get(&edge.target) else {
            continue;
        };
        let key = label.trim().to_ascii_lowercase();
        let explicit = uses.contains_key(&key);
        let fqn = uses
            .get(&key)
            .cloned()
            .or_else(|| (!namespace.is_empty()).then(|| format!("{namespace}\\{}", label.trim())));
        let Some(fqn) = fqn else {
            continue;
        };
        let target = if let Some(candidates) = internal_types.get(&fqn.to_ascii_lowercase()) {
            let mut same_file = candidates
                .iter()
                .filter(|(_, candidate_source)| candidate_source == &source_file);
            let first_same_file = same_file.next();
            if let Some((target, _)) = first_same_file {
                if same_file.next().is_some() {
                    continue;
                }
                target.clone()
            } else if let Some((target, _)) = candidates.first().filter(|_| candidates.len() == 1) {
                target.clone()
            } else {
                continue;
            }
        } else if explicit {
            make_id(&[&fqn])
        } else {
            continue;
        };
        if target == edge.target {
            continue;
        }
        if explicit
            && created.insert(target.clone())
            && !extraction.nodes.iter().any(|node| node.id == target)
        {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(fqn));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            new_nodes.push(NodeRecord {
                id: target.clone(),
                attributes,
            });
        }
        repointed.insert(edge.target.clone());
        edge.target = target;
        stamp_endpoint_rewrite(edge, EndpointRewriteRule::PhpQualifiedTypeResolution, 1.0);
    }
    extraction.nodes.extend(new_nodes);
    drop_unreferenced_nodes(extraction, &repointed);
}

fn php_use_entries(body: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let (prefix, members) = body
        .find('{')
        .and_then(|start| {
            body.rfind('}').map(|end| {
                (
                    body[..start].trim().trim_end_matches('\\'),
                    &body[start + 1..end],
                )
            })
        })
        .unwrap_or(("", body));
    for member in members
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (target, alias) = member
            .rsplit_once(" as ")
            .map_or((member, None), |(target, alias)| {
                (target.trim(), Some(alias.trim()))
            });
        let fqn = if prefix.is_empty() {
            target.trim_start_matches('\\').to_owned()
        } else {
            format!("{prefix}\\{}", target.trim_start_matches('\\'))
        };
        let local = alias.unwrap_or_else(|| fqn.rsplit('\\').next().unwrap_or_default());
        if !local.is_empty() && !fqn.is_empty() {
            entries.push((local.to_owned(), fqn));
        }
    }
    entries
}

fn drop_unreferenced_nodes(extraction: &mut Extraction, candidates: &HashSet<String>) {
    if candidates.is_empty() {
        return;
    }
    let referenced = extraction
        .edges
        .iter()
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<HashSet<_>>();
    extraction
        .nodes
        .retain(|node| !candidates.contains(&node.id) || referenced.contains(&node.id));
}

fn canonicalize_file_targets(extraction: &mut Extraction, root: &Path) {
    let mut alias_candidates = HashMap::<String, Vec<String>>::new();
    let node_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if !is_file_node(node, &source) {
            continue;
        }
        let source_path = Path::new(&source);
        let absolute = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            root.join(source_path)
        };
        let mut source_aliases = vec![source_path.to_path_buf(), absolute.clone()];
        let runtime_extension = match source_path.extension().and_then(|value| value.to_str()) {
            Some("ts") => Some("js"),
            Some("tsx") => Some("jsx"),
            Some("mts") => Some("mjs"),
            Some("cts") => Some("cjs"),
            _ => None,
        };
        if let Some(extension) = runtime_extension {
            source_aliases.push(source_path.with_extension(extension));
            source_aliases.push(absolute.with_extension(extension));
        }
        let mut aliases = vec![
            make_id(&[&source]),
            make_id(&[&file_stem(source_path)]),
            make_id(&[&absolute.to_string_lossy()]),
        ];
        aliases.extend(
            source_aliases
                .into_iter()
                .map(|alias| make_id(&[&alias.to_string_lossy().replace('\\', "/")])),
        );
        for alias in aliases {
            alias_candidates
                .entry(alias)
                .or_default()
                .push(node.id.clone());
        }
    }
    let aliases = alias_candidates
        .into_iter()
        .filter_map(|(alias, mut candidates)| {
            candidates.sort();
            candidates.dedup();
            (candidates.len() == 1).then(|| (alias, candidates.pop().unwrap_or_default()))
        })
        .collect::<HashMap<_, _>>();
    for edge in &mut extraction.edges {
        if edge.attributes.contains_key("_document_target_path") {
            continue;
        }
        if node_ids.contains(edge.target.as_str()) {
            continue;
        }
        if let Some(target) = aliases.get(&edge.target) {
            let rule = if matches!(
                relation(edge),
                "imports" | "imports_from" | "exports" | "re_exports"
            ) {
                EndpointRewriteRule::CanonicalImportTarget
            } else {
                EndpointRewriteRule::CanonicalFileTarget
            };
            edge.target.clone_from(target);
            stamp_endpoint_rewrite(edge, rule, 1.0);
        }
    }
}

const DOCUMENT_TARGET_EXTENSIONS: [&str; 5] = ["md", "markdown", "mdx", "qmd", "skill"];

/// Resolve Markdown links only after the complete project inventory is known.
///
/// The per-file extractor preserves the source spelling and an exact wiring
/// site. This stage can therefore resolve cross-file fragments, extensionless
/// links, repository-root links, directory index documents, and unique wiki
/// stems without filesystem-order guesses or cross-language policy.
fn resolve_document_link_targets(extraction: &mut Extraction, root: &Path) {
    let profile = std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some();
    let mut considered = 0_usize;
    let mut rewritten = 0_usize;
    let mut roots_by_source = BTreeMap::<String, Vec<String>>::new();
    let mut wiki_stems = BTreeMap::<String, Vec<(String, String)>>::new();
    let mut headings = BTreeMap::<(String, String), Vec<String>>::new();

    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if source.is_empty() {
            continue;
        }
        let source = source_key(&source, root);
        let document_root =
            node.string("file_type") == "document" && node.string("document_kind") == "document";
        if document_root || is_file_node(node, &source) {
            roots_by_source
                .entry(source.clone())
                .or_default()
                .push(node.id.clone());
            if document_root
                && let Some(stem) = Path::new(&source)
                    .file_stem()
                    .and_then(|value| value.to_str())
            {
                wiki_stems
                    .entry(stem.to_ascii_lowercase())
                    .or_default()
                    .push((source.clone(), node.id.clone()));
            }
        }
        if node.string("file_type") == "document" && node.string("document_kind") == "heading" {
            let mut aliases = BTreeSet::new();
            for attribute in ["anchor_slug", "explicit_id"] {
                let value = node.string(attribute);
                if !value.is_empty() {
                    aliases.insert(value.to_ascii_lowercase());
                }
            }
            for alias in aliases {
                headings
                    .entry((source.clone(), alias))
                    .or_default()
                    .push(node.id.clone());
            }
        }
    }
    for candidates in roots_by_source.values_mut() {
        candidates.sort();
        candidates.dedup();
    }
    for candidates in wiki_stems.values_mut() {
        candidates.sort();
        candidates.dedup();
    }
    for candidates in headings.values_mut() {
        candidates.sort();
        candidates.dedup();
    }

    for edge in &mut extraction.edges {
        let Some(target_path) = edge
            .attributes
            .get("_document_target_path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        considered = considered.saturating_add(1);
        let extension_inferred = edge
            .attributes
            .get("_document_target_extension_inferred")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let wiki_link =
            edge.attributes.get("link_kind").and_then(Value::as_str) == Some("wikilink");

        let mut source_candidates = BTreeSet::new();
        let normalized = document_target_key(&target_path, root);
        if !normalized.is_empty() {
            source_candidates.insert(normalized.clone());
        }
        if extension_inferred {
            let path = Path::new(&normalized);
            for extension in DOCUMENT_TARGET_EXTENSIONS {
                source_candidates.insert(
                    path.with_extension(extension)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                source_candidates.insert(
                    path.join("README")
                        .with_extension(extension)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                source_candidates.insert(
                    path.join("index")
                        .with_extension(extension)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }

        let mut candidates = source_candidates
            .iter()
            .filter_map(|source| {
                roots_by_source
                    .get(source)
                    .filter(|targets| targets.len() == 1)
                    .and_then(|targets| targets.first())
                    .map(|target| (source.clone(), target.clone()))
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() && wiki_link {
            let stem = Path::new(&normalized)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if let Some(targets) = wiki_stems.get(&stem).filter(|targets| targets.len() == 1) {
                candidates.extend(targets.iter().cloned());
            }
        }
        candidates.sort();
        candidates.dedup();

        let Some((target_source, document_target)) =
            (candidates.len() == 1).then(|| candidates.pop()).flatten()
        else {
            let status = if candidates.is_empty() {
                "missing_target"
            } else {
                "ambiguous_target"
            };
            mark_unresolved_document_link(edge, &target_path, status);
            continue;
        };
        let fragment = edge
            .attributes
            .get("fragment")
            .and_then(Value::as_str)
            .filter(|fragment| !fragment.is_empty())
            .map(str::to_owned);
        let target = if let Some(fragment) = fragment.as_deref() {
            let key = decode_markdown_fragment(fragment).to_ascii_lowercase();
            let Some(targets) = headings.get(&(target_source, key)) else {
                mark_unresolved_document_link(edge, &target_path, "missing_fragment");
                continue;
            };
            if targets.len() != 1 {
                mark_unresolved_document_link(edge, &target_path, "ambiguous_fragment");
                continue;
            }
            targets[0].clone()
        } else {
            document_target
        };
        if target != edge.target {
            edge.attributes.insert(
                "_document_original_target".to_owned(),
                Value::String(edge.target.clone()),
            );
            edge.target = target;
            rewritten = rewritten.saturating_add(1);
        }
        edge.attributes.insert(
            "rule".to_owned(),
            Value::String("document-link-exact-target".to_owned()),
        );
        edge.attributes.insert(
            "resolution_rule".to_owned(),
            Value::String("document-link-target-resolution".to_owned()),
        );
    }
    if profile {
        eprintln!(
            "[compass internal] document links considered={considered} rewritten={rewritten}"
        );
    }
}

fn document_target_key(source: &str, root: &Path) -> String {
    let key = source_key(source, root);
    if !Path::new(&key).is_absolute() {
        return key;
    }
    let path = Path::new(source);
    let Some((parent, file_name)) = path.parent().zip(path.file_name()) else {
        return key;
    };
    let Ok(parent) = std::fs::canonicalize(parent) else {
        return key;
    };
    let Ok(parent) = parent.strip_prefix(root) else {
        return key;
    };
    parent.join(file_name).to_string_lossy().replace('\\', "/")
}

fn mark_unresolved_document_link(edge: &mut EdgeRecord, target_path: &str, status: &str) {
    let fragment = edge
        .attributes
        .get("fragment")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_file = edge
        .attributes
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let start_byte = edge
        .attributes
        .get("start_byte")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .to_string();
    edge.target = make_id(&[
        "unresolved_document_link",
        source_file,
        &start_byte,
        target_path,
        fragment,
    ]);
    edge.attributes.insert(
        "_document_target_resolution".to_owned(),
        Value::String(status.to_owned()),
    );
}

fn decode_markdown_fragment(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len().min(512));
    let mut index = 0;
    while index < bytes.len() && decoded.len() < 512 {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (bytes.get(index + 1), bytes.get(index + 2))
            && let (Some(high), Some(low)) = (hex_digit(*high), hex_digit(*low))
        {
            decoded.push(high.saturating_mul(16).saturating_add(low));
            index = index.saturating_add(3);
            continue;
        }
        decoded.push(bytes[index]);
        index = index.saturating_add(1);
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

const fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn rewire_unique_stub_nodes(extraction: &mut Extraction) {
    let normalized_label = |node: &NodeRecord| {
        node.label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
    };
    let stubs = extraction
        .nodes
        .iter()
        .filter(|node| {
            string_attribute(node, "source_file").is_empty() && !is_canonical_external_symbol(node)
        })
        .filter_map(|node| {
            let label = normalized_label(node);
            (!label.is_empty()).then(|| (node.id.clone(), label))
        })
        .collect::<Vec<_>>();
    if stubs.is_empty() {
        return;
    }
    let needed_labels = stubs
        .iter()
        .map(|(_, label)| label.clone())
        .collect::<HashSet<_>>();
    let needed_folded = needed_labels
        .iter()
        .map(|label| label.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut types = HashMap::<String, Vec<String>>::new();
    let mut types_ci = HashMap::<String, Vec<String>>::new();
    let mut callables = HashMap::<String, Vec<String>>::new();
    let mut source_by_id = HashMap::<String, String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if source.is_empty() {
            continue;
        }
        let label = normalized_label(node);
        if label.is_empty() {
            continue;
        }
        let folded = label.to_ascii_lowercase();
        if !needed_labels.contains(label.as_str()) && !needed_folded.contains(&folded) {
            continue;
        }
        source_by_id.insert(node.id.clone(), source.clone());
        if is_generic_call_target(node) && needed_labels.contains(label.as_str()) {
            callables.entry(label).or_default().push(node.id.clone());
        } else if is_type_like_definition(node) {
            if needed_labels.contains(label.as_str()) {
                types
                    .entry(label.clone())
                    .or_default()
                    .push(node.id.clone());
            }
            if case_insensitive(&source) && needed_folded.contains(&folded) {
                types_ci.entry(folded).or_default().push(node.id.clone());
            }
        }
    }
    let supertype_stubs = extraction
        .edges
        .iter()
        .filter(|edge| matches!(relation(edge), "inherits" | "implements" | "extends"))
        .map(|edge| edge.target.as_str())
        .collect::<HashSet<_>>();
    let stub_ids = stubs
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<HashSet<_>>();
    let mut stub_families = HashMap::<String, HashSet<&'static str>>::new();
    let mut stub_scopes = HashMap::<String, HashSet<String>>::new();
    let mut stub_consumers = HashMap::<String, HashSet<String>>::new();
    let mut stub_source_files = HashMap::<String, HashSet<String>>::new();
    let mut imports = HashMap::<String, HashSet<String>>::new();
    let mut imports_by_file = HashMap::<String, HashSet<String>>::new();
    let mut stub_relations = HashMap::<String, HashSet<String>>::new();
    for edge in &extraction.edges {
        if matches!(relation(edge), "imports" | "imports_from") {
            imports
                .entry(edge.source.clone())
                .or_default()
                .insert(edge.target.clone());
            imports_by_file
                .entry(edge.string("source_file"))
                .or_default()
                .insert(edge.target.clone());
        }
        let Some(family) = language_family(&edge.string("source_file")) else {
            continue;
        };
        for endpoint in [&edge.source, &edge.target] {
            if stub_ids.contains(endpoint.as_str()) {
                stub_relations
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(relation(edge).to_owned());
                stub_families
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(family);
                stub_scopes
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(repository_scope(&edge.string("source_file")));
                stub_source_files
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(edge.string("source_file"));
                let counterpart = if endpoint == &edge.source {
                    &edge.target
                } else {
                    &edge.source
                };
                stub_consumers
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(counterpart.clone());
            }
        }
    }
    let mut remap = HashMap::new();
    for (stub, label) in stubs {
        let families = stub_families.get(&stub);
        let scopes = stub_scopes.get(&stub);
        let consumers = stub_consumers.get(&stub);
        let source_files = stub_source_files.get(&stub);
        let compatible_unique = |items: Option<&Vec<String>>| {
            let compatible = items?
                .iter()
                .filter(|candidate| {
                    let Some(candidate_source) = source_by_id.get(*candidate) else {
                        return false;
                    };
                    let Some(candidate_family) = language_family(candidate_source) else {
                        return false;
                    };
                    let family_compatible = families
                        .is_some_and(|set| set.len() == 1 && set.contains(candidate_family));
                    let scope_compatible = scopes.is_some_and(|set| {
                        set.len() == 1 && set.contains(&repository_scope(candidate_source))
                    });
                    let explicitly_imported = consumers.is_some_and(|consumers| {
                        consumers.iter().any(|consumer| {
                            imports
                                .get(consumer)
                                .is_some_and(|targets| targets.contains(*candidate))
                        })
                    }) || source_files.is_some_and(|files| {
                        files.iter().any(|source_file| {
                            imports_by_file
                                .get(source_file)
                                .is_some_and(|targets| targets.contains(*candidate))
                        })
                    });
                    family_compatible && (scope_compatible || explicitly_imported)
                })
                .collect::<Vec<_>>();
            (compatible.len() == 1).then(|| compatible[0].clone())
        };
        let relations = stub_relations.get(&stub);
        let call_only = relations.is_some_and(|relations| {
            !relations.is_empty()
                && relations.iter().all(|relation| {
                    matches!(relation.as_str(), "calls" | "indirect_call" | "tests")
                })
        });
        let candidate = if call_only {
            compatible_unique(callables.get(&label))
        } else {
            compatible_unique(types.get(&label))
                .or_else(|| compatible_unique(types_ci.get(&label.to_ascii_lowercase())))
                .or_else(|| {
                    if supertype_stubs.contains(stub.as_str()) {
                        return None;
                    }
                    compatible_unique(callables.get(&label))
                })
        };
        if let Some(target) = candidate
            && target != stub
        {
            remap.insert(stub, target);
        }
    }
    if remap.is_empty() {
        return;
    }
    for edge in &mut extraction.edges {
        let mut rewritten = false;
        if let Some(target) = remap.get(&edge.source) {
            edge.source.clone_from(target);
            rewritten = true;
        }
        if let Some(target) = remap.get(&edge.target) {
            edge.target.clone_from(target);
            rewritten = true;
        }
        if rewritten {
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::UniqueStubEndpointResolution, 0.8);
        }
    }
    extraction
        .edges
        .retain(|edge| edge.source != edge.target || relation(edge) == "calls");
    let referenced = extraction
        .edges
        .iter()
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<HashSet<_>>();
    extraction
        .nodes
        .retain(|node| !remap.contains_key(&node.id) || referenced.contains(&node.id));
}

fn is_type_like_definition(node: &NodeRecord) -> bool {
    let legacy_kind = string_attribute(node, "type");
    if legacy_kind == "namespace" || string_attribute(node, "file_type") != "code" {
        return false;
    }
    let kind = string_attribute(node, "symbol_kind");
    let effective_kind = if kind.is_empty() { &legacy_kind } else { &kind };
    if !effective_kind.is_empty() {
        return matches!(
            effective_kind.as_str(),
            "class"
                | "component"
                | "enum"
                | "interface"
                | "protocol"
                | "record"
                | "struct"
                | "trait"
                | "type_alias"
        );
    }
    let label = node.label().trim();
    !label.is_empty() && !label.ends_with(')') && !label.starts_with('.') && !label.contains('.')
}

fn is_canonical_external_symbol(node: &NodeRecord) -> bool {
    node.attributes
        .get("_canonical_external_symbol")
        .and_then(Value::as_bool)
        == Some(true)
}

#[cfg(test)]
fn disambiguate_colliding_node_ids(extraction: &mut Extraction, root: &Path) {
    let mut raw_calls = extraction.raw_calls.take();
    if let Some(calls) = raw_calls.as_mut() {
        disambiguate_colliding_node_ids_with_calls(extraction, root, calls);
    } else {
        disambiguate_colliding_node_ids_with_calls(extraction, root, &mut []);
    }
    extraction.raw_calls = raw_calls;
}

fn disambiguate_colliding_node_ids_with_calls(
    extraction: &mut Extraction,
    root: &Path,
    raw_calls: &mut [RawCall],
) {
    let mut first_positions = HashMap::<String, usize>::new();
    let mut groups = HashMap::<String, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if matches!(
            string_attribute(node, "type").as_str(),
            "module" | "namespace"
        ) {
            continue;
        }
        if !node.id.is_empty() {
            if let Some(&first) = first_positions.get(&node.id) {
                groups
                    .entry(node.id.clone())
                    .or_insert_with(|| vec![first])
                    .push(index);
            } else {
                first_positions.insert(node.id.clone(), index);
            }
        }
    }
    drop(first_positions);
    let mut remap = HashMap::<(String, String), String>::new();
    let mut ambiguous = HashSet::new();
    for (old_id, indexes) in &groups {
        let source_keys = indexes
            .iter()
            .map(|index| node_source_key(&extraction.nodes[*index], root))
            .collect::<HashSet<_>>();
        if indexes.len() < 2 || source_keys.len() < 2 {
            continue;
        }
        ambiguous.insert(old_id.clone());
        let naive = source_keys
            .iter()
            .filter(|key| !key.is_empty())
            .map(|key| (key.clone(), make_id(&[key, old_id])))
            .collect::<HashMap<_, _>>();
        let mut counts = HashMap::<String, usize>::new();
        for value in naive.values() {
            *counts.entry(value.clone()).or_default() += 1;
        }
        for index in indexes {
            let source_key = node_source_key(&extraction.nodes[*index], root);
            if source_key.is_empty() {
                continue;
            }
            let naive_id = naive
                .get(&source_key)
                .cloned()
                .unwrap_or_else(|| make_id(&[&source_key, old_id]));
            let new_id = if counts.get(&naive_id).copied().unwrap_or_default() > 1 {
                let digest = Sha1::digest(source_key.as_bytes());
                let salt = format!("{digest:x}");
                make_id(&[&source_key, old_id, &salt[..6]])
            } else {
                naive_id
            };
            remap.insert((old_id.clone(), source_key), new_id.clone());
            extraction.nodes[*index].id = new_id;
        }
    }
    if remap.is_empty() {
        for edge in &mut extraction.edges {
            edge.attributes.remove("target_file");
        }
        return;
    }
    let mut header_remaps = HashMap::new();
    for old_id in &ambiguous {
        if let Some(indexes) = groups.get(old_id) {
            for index in indexes {
                let key = node_source_key(&extraction.nodes[*index], root);
                if matches!(
                    Path::new(&key)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("h" | "hpp" | "hh" | "hxx")
                ) && let Some(new_id) = remap.get(&(old_id.clone(), key))
                {
                    header_remaps.insert(old_id.clone(), new_id.clone());
                    break;
                }
            }
        }
    }
    for edge in &mut extraction.edges {
        let edge_key = source_key(&edge.string("source_file"), root);
        if let Some(new_id) = remap.get(&(edge.source.clone(), edge_key.clone())) {
            edge.source.clone_from(new_id);
            stamp_endpoint_rewrite(
                edge,
                EndpointRewriteRule::SourceScopedNodeDisambiguation,
                1.0,
            );
        }
        let target_file = edge
            .attributes
            .remove("target_file")
            .and_then(|value| value.as_str().map(str::to_owned));
        let relation = relation(edge);
        let target_key = if matches!(
            relation,
            "imports" | "imports_from" | "exports" | "re_exports"
        ) {
            target_file
                .as_deref()
                .map_or(edge_key, |path| source_key(path, root))
        } else {
            edge_key
        };
        if matches!(relation, "imports" | "imports_from")
            && let Some(new_id) = header_remaps.get(&edge.target)
        {
            edge.target.clone_from(new_id);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::HeaderImportDisambiguation, 1.0);
        } else if let Some(new_id) = remap.get(&(edge.target.clone(), target_key)) {
            edge.target.clone_from(new_id);
            stamp_endpoint_rewrite(
                edge,
                EndpointRewriteRule::SourceScopedNodeDisambiguation,
                1.0,
            );
        }
    }
    for raw in raw_calls {
        let key = source_key(&raw.source_file, root);
        if let Some(new_id) = remap.get(&(raw.caller_nid.clone(), key)) {
            raw.caller_nid.clone_from(new_id);
        }
    }
}

fn node_source_key(node: &NodeRecord, root: &Path) -> String {
    let source = string_attribute(node, "source_file");
    if source.is_empty() {
        source_key(&string_attribute(node, "origin_file"), root)
    } else {
        source_key(&source, root)
    }
}

fn source_key(source: &str, root: &Path) -> String {
    if source.is_empty() {
        return String::new();
    }
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if let Ok(relative) = absolute.strip_prefix(root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canonical) = std::fs::canonicalize(&absolute)
        && let Ok(relative) = canonical.strip_prefix(root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    path.to_string_lossy().replace('\\', "/")
}

/// Index exact and suffix spellings for portable universal-evidence paths.
/// The language adapter can shorten an absolute temporary path to a stable
/// `parent/file` spelling; that spelling maps back to an admitted source only
/// when the suffix is unique.
fn source_inventory_index(
    sources: &HashMap<String, String>,
    root: &Path,
) -> BTreeMap<String, (String, usize)> {
    let mut matches = BTreeMap::<String, BTreeMap<String, (String, usize)>>::new();
    for (candidate, contents) in sources {
        let normalized_candidate = candidate.replace('\\', "/");
        let display = source_key(candidate, root);
        let mut spellings = BTreeSet::from([display.clone(), normalized_candidate.clone()]);
        spellings.extend(
            normalized_candidate
                .match_indices('/')
                .map(|(index, _)| normalized_candidate[index + 1..].to_owned())
                .filter(|suffix| !suffix.is_empty()),
        );
        for spelling in spellings {
            matches
                .entry(spelling)
                .or_default()
                .insert(candidate.clone(), (display.clone(), contents.len()));
        }
    }
    matches
        .into_iter()
        .filter_map(|(spelling, mut candidates)| {
            if candidates.len() != 1 {
                return None;
            }
            let (_, (display, byte_len)) = candidates.pop_first()?;
            is_safe_relative_source(&display).then_some((spelling, (display, byte_len)))
        })
        .collect()
}

/// Resolve non-member raw calls using unique definitions and import evidence.
pub fn resolve_cross_file_calls(extraction: &mut Extraction, _sources: &HashMap<String, String>) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    resolve_cross_file_calls_with_root_calls(extraction, &raw_calls);
}

#[cfg(test)]
fn resolve_cross_file_calls_with_root(
    extraction: &mut Extraction,
    _sources: &HashMap<String, String>,
    _root: &Path,
) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    resolve_cross_file_calls_with_root_calls(extraction, &raw_calls);
}

fn resolve_cross_file_calls_with_root_calls(extraction: &mut Extraction, raw_calls: &[RawCall]) {
    let additions =
        resolve_cross_file_call_additions(extraction, raw_calls, ResolutionAdmission::Max);
    extraction.edges.extend(additions);
}

fn resolve_cross_file_call_additions(
    extraction: &Extraction,
    raw_calls: &[RawCall],
    admission: ResolutionAdmission,
) -> Vec<EdgeRecord> {
    let eligible_calls = raw_calls.iter().filter(|raw| {
        !raw.callee.is_empty()
            && raw.is_member_call != Some(true)
            && raw.extensions.get("is_mixin").and_then(Value::as_bool) != Some(true)
    });
    let mut call_families = AHashSet::new();
    let mut has_unscoped_call = false;
    let mut eligible_call_count = 0_usize;
    for raw in eligible_calls {
        eligible_call_count = eligible_call_count.saturating_add(1);
        if let Some(family) = language_family(&raw.source_file) {
            call_families.insert(family);
        } else {
            has_unscoped_call = true;
        }
    }
    if eligible_call_count == 0 {
        return Vec::new();
    }
    let mut profile_started = Instant::now();
    let mut additions = Vec::new();
    let mut exact = AHashMap::<String, Vec<String>>::new();
    let mut folded = AHashMap::<String, Vec<String>>::new();
    let mut source_by_id = AHashMap::<String, String>::new();
    let mut file_by_source = AHashMap::<String, String>::new();
    let mut callable = AHashSet::<String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if !has_unscoped_call
            && language_family(&source).is_some_and(|family| !call_families.contains(family))
        {
            continue;
        }
        source_by_id.insert(node.id.clone(), source.clone());
        if node.attributes.get("_callable").and_then(Value::as_bool) == Some(true) {
            callable.insert(node.id.clone());
        }
        if is_file_node(node, &source) {
            file_by_source
                .entry(source.clone())
                .or_insert_with(|| node.id.clone());
        }
        if !is_generic_call_target(node) {
            continue;
        }
        let label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        if label.is_empty() {
            continue;
        }
        exact
            .entry(label.clone())
            .or_default()
            .push(node.id.clone());
        if case_insensitive(&source) {
            folded
                .entry(label.to_lowercase())
                .or_default()
                .push(node.id.clone());
        }
    }

    let file_by_id = source_by_id
        .iter()
        .filter_map(|(id, source)| {
            file_by_source
                .get(source)
                .map(|file_id| (id.clone(), file_id.clone()))
        })
        .collect::<AHashMap<_, _>>();
    let mut symbol_imports = AHashMap::<String, AHashSet<String>>::new();
    let mut module_imports = AHashMap::<String, AHashSet<String>>::new();
    let mut existing = AHashSet::new();
    let mut call_like = AHashSet::new();
    for edge in &extraction.edges {
        if !source_by_id.contains_key(&edge.source) {
            continue;
        }
        existing.insert((
            edge.source.clone(),
            edge.target.clone(),
            edge_occurrence_site(edge),
        ));
        if matches!(relation(edge), "calls" | "indirect_call") {
            call_like.insert((
                edge.source.clone(),
                edge.target.clone(),
                edge_occurrence_site(edge),
            ));
        }
        match relation(edge) {
            "imports" => {
                symbol_imports
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
            }
            "imports_from" => {
                module_imports
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
            }
            _ => {}
        }
    }

    for raw in raw_calls {
        if raw.callee.is_empty()
            || raw.is_member_call == Some(true)
            || raw.extensions.get("is_mixin").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let candidates = candidate_calls(raw, &exact, &folded, &source_by_id);
        if candidates.is_empty() {
            continue;
        }
        let caller_file = file_by_source
            .get(&raw.source_file)
            .or_else(|| file_by_id.get(&raw.caller_nid));
        let imported_symbols = caller_file.and_then(|id| symbol_imports.get(id));
        let imported_modules = caller_file.and_then(|id| module_imports.get(id));
        let selection = select_candidate(
            &candidates,
            imported_symbols,
            imported_modules,
            &file_by_id,
            &source_by_id,
            &raw.source_file,
        );
        let Some((target, import_evidence)) = selection else {
            continue;
        };
        let same_file = source_by_id
            .get(&target)
            .is_some_and(|source| normalize_path(source) == normalize_path(&raw.source_file));
        let language = raw
            .lang
            .as_deref()
            .or_else(|| language_name_from_source(&raw.source_file));
        if language.is_some_and(|language| {
            is_language_builtin_global(language, &raw.callee) && !same_file && !import_evidence
        }) {
            continue;
        }
        if raw
            .extensions
            .get("symbol_import_use")
            .and_then(Value::as_bool)
            == Some(true)
            && !imported_symbols.is_some_and(|imports| imports.contains(&target))
        {
            continue;
        }
        if raw
            .extensions
            .get("module_import_use")
            .and_then(Value::as_bool)
            == Some(true)
            && !file_by_id.get(&target).is_some_and(|target_file| {
                imported_modules.is_some_and(|imports| imports.contains(target_file))
            })
        {
            continue;
        }
        let indirect = raw.extensions.get("indirect").and_then(Value::as_bool) == Some(true);
        if indirect {
            if admission.admits_source_backed_inference()
                && target != raw.caller_nid
                && callable.contains(&target)
                && call_like.insert((
                    raw.caller_nid.clone(),
                    target.clone(),
                    raw_call_occurrence_site(raw),
                ))
            {
                let mut edge = resolved_edge(raw, &target, "INFERRED", 0.8);
                edge.attributes.insert(
                    "relation".to_owned(),
                    Value::String("indirect_call".to_owned()),
                );
                edge.attributes.insert(
                    "context".to_owned(),
                    raw.extensions
                        .get("context")
                        .cloned()
                        .unwrap_or_else(|| Value::String("argument".to_owned())),
                );
                additions.push(edge);
            }
            continue;
        }
        if target == raw.caller_nid || (!import_evidence && is_javascript(&raw.source_file)) {
            continue;
        }
        if !import_evidence && !admission.admits_source_backed_inference() {
            continue;
        }
        if existing.insert((
            raw.caller_nid.clone(),
            target.clone(),
            raw_call_occurrence_site(raw),
        )) {
            let mut edge = resolved_edge(
                raw,
                &target,
                if import_evidence {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                },
                if import_evidence { 1.0 } else { 0.8 },
            );
            if raw
                .extensions
                .get("symbol_import_use")
                .and_then(Value::as_bool)
                == Some(true)
            {
                edge.attributes.remove("confidence_score");
            }
            additions.push(edge);
        }
    }
    profile_internal("resolver generic cross-file calls", &mut profile_started);
    additions
}

fn candidate_calls(
    raw: &RawCall,
    exact: &AHashMap<String, Vec<String>>,
    folded: &AHashMap<String, Vec<String>>,
    source_by_id: &AHashMap<String, String>,
) -> Vec<String> {
    let mut candidates = exact.get(&raw.callee).cloned().unwrap_or_default();
    if candidates.is_empty() && case_insensitive(&raw.source_file) {
        candidates = folded
            .get(&raw.callee.to_lowercase())
            .cloned()
            .unwrap_or_default();
    }
    if let Some(family) = language_family(&raw.source_file) {
        candidates.retain(|candidate| {
            source_by_id
                .get(candidate)
                .and_then(|source| language_family(source))
                .is_none_or(|candidate_family| candidate_family == family)
        });
    }
    candidates
}

fn select_candidate(
    candidates: &[String],
    symbol_imports: Option<&AHashSet<String>>,
    module_imports: Option<&AHashSet<String>>,
    file_by_id: &AHashMap<String, String>,
    source_by_id: &AHashMap<String, String>,
    call_site_file: &str,
) -> Option<(String, bool)> {
    if candidates.len() == 1 {
        let candidate = candidates[0].clone();
        let evidence = symbol_imports.is_some_and(|imports| imports.contains(&candidate))
            || file_by_id
                .get(&candidate)
                .is_some_and(|file| module_imports.is_some_and(|imports| imports.contains(file)));
        return Some((candidate, evidence));
    }
    let symbol_matches = candidates
        .iter()
        .filter(|candidate| symbol_imports.is_some_and(|imports| imports.contains(*candidate)))
        .cloned()
        .collect::<Vec<_>>();
    if symbol_matches.len() == 1 {
        return Some((symbol_matches[0].clone(), true));
    }
    let module_matches = candidates
        .iter()
        .filter(|candidate| {
            file_by_id
                .get(*candidate)
                .is_some_and(|file| module_imports.is_some_and(|imports| imports.contains(file)))
        })
        .cloned()
        .collect::<Vec<_>>();
    if module_matches.len() == 1 {
        return Some((module_matches[0].clone(), true));
    }
    disambiguate_candidates(candidates, source_by_id, call_site_file)
        .map(|candidate| (candidate, false))
}

fn disambiguate_candidates(
    candidates: &[String],
    source_by_id: &AHashMap<String, String>,
    call_site_file: &str,
) -> Option<String> {
    if candidates.len() == 1 {
        return candidates.first().cloned();
    }
    let call_is_test = is_test_path(call_site_file);
    let test_candidates = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| is_test_path(path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let test_set = test_candidates.iter().collect::<AHashSet<_>>();
    let non_test_candidates = candidates
        .iter()
        .filter(|candidate| !test_set.contains(candidate))
        .cloned()
        .collect::<Vec<_>>();
    let survivors = if call_is_test {
        let normalized_call = normalize_path(call_site_file);
        let same_file = test_candidates
            .iter()
            .filter(|candidate| {
                source_by_id
                    .get(*candidate)
                    .is_some_and(|path| normalize_path(path) == normalized_call)
            })
            .cloned()
            .collect::<Vec<_>>();
        if same_file.len() == 1 {
            return same_file.first().cloned();
        }
        if test_candidates.is_empty() {
            if non_test_candidates.is_empty() {
                candidates.to_vec()
            } else {
                non_test_candidates
            }
        } else {
            test_candidates
        }
    } else {
        non_test_candidates
    };
    if survivors.len() == 1 {
        return survivors.first().cloned();
    }
    path_proximity(&survivors, source_by_id, call_site_file)
}

fn path_proximity(
    candidates: &[String],
    source_by_id: &AHashMap<String, String>,
    call_site_file: &str,
) -> Option<String> {
    if call_site_file.is_empty() {
        return None;
    }
    let call = normalize_path(call_site_file);
    let call_dir = parent_segments(&call);
    let same_file = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| normalize_path(path) == call)
        })
        .cloned()
        .collect::<Vec<_>>();
    if same_file.len() == 1 {
        return same_file.first().cloned();
    }
    if same_file.len() > 1 {
        return None;
    }
    let same_dir = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| parent_segments(&normalize_path(path)) == call_dir)
        })
        .cloned()
        .collect::<Vec<_>>();
    if same_dir.len() == 1 {
        return same_dir.first().cloned();
    }
    if same_dir.len() > 1 {
        return None;
    }
    let scores = candidates
        .iter()
        .map(|candidate| {
            let parts = source_by_id
                .get(candidate)
                .map(|path| parent_segments(&normalize_path(path)))
                .unwrap_or_default();
            let score = call_dir
                .iter()
                .zip(parts.iter())
                .take_while(|(left, right)| left == right)
                .count();
            (candidate, score)
        })
        .collect::<Vec<_>>();
    let best = scores.iter().map(|(_, score)| *score).max()?;
    let winners = scores
        .iter()
        .filter(|(_, score)| *score == best)
        .collect::<Vec<_>>();
    (best > 0 && winners.len() == 1).then(|| (*winners[0].0).clone())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn parent_segments(path: &str) -> Vec<String> {
    path.rsplit_once('/').map_or_else(Vec::new, |(parent, _)| {
        parent.split('/').map(str::to_owned).collect()
    })
}

fn is_test_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "tests" | "test" | "spec" | "specs" | "__tests__"
        )
    }) {
        return true;
    }
    let filename = parts.last().copied().unwrap_or_default();
    let folded = filename.to_ascii_lowercase();
    folded.starts_with("test_")
        || folded.contains("_test.")
        || folded.contains(".test.")
        || folded.contains(".spec.")
        || folded.contains("_spec.")
        || folded.ends_with(".tests.ps1")
        || filename.ends_with("Test.java")
        || filename.ends_with("Tests.java")
        || filename.ends_with("Tests.cs")
}

fn resolved_edge(raw: &RawCall, target: &str, confidence: &str, score: f64) -> EdgeRecord {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String("calls".to_owned()));
    attributes.insert("context".to_owned(), Value::String("call".to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String(confidence.to_owned()),
    );
    attributes.insert("confidence_score".to_owned(), Value::from(score));
    attributes.insert(
        "source_file".to_owned(),
        Value::String(raw.source_file.clone()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(raw.source_location.clone()),
    );
    for key in [
        "language",
        "extractor",
        "source_anchor",
        "start_byte",
        "end_byte",
        "line_start",
        "line_end",
        "column_start",
        "column_end",
    ] {
        if let Some(value) = raw.extensions.get(key) {
            attributes.insert(key.to_owned(), value.clone());
        }
    }
    attributes.insert("weight".to_owned(), Value::from(1.0));
    EdgeRecord {
        source: raw.caller_nid.clone(),
        target: target.to_owned(),
        attributes,
    }
}

fn raw_call_occurrence_site(raw: &RawCall) -> String {
    occurrence_site(&raw.extensions, &raw.source_file, &raw.source_location)
}

fn edge_occurrence_site(edge: &EdgeRecord) -> String {
    occurrence_site(
        &edge.attributes,
        &edge.string("source_file"),
        &edge.string("source_location"),
    )
}

fn occurrence_site(attributes: &Map<String, Value>, source_file: &str, location: &str) -> String {
    serde_json::to_string(&[
        Value::String(source_file.to_owned()),
        attributes
            .get("source_anchor")
            .cloned()
            .unwrap_or(Value::Null),
        attributes.get("start_byte").cloned().unwrap_or(Value::Null),
        attributes.get("end_byte").cloned().unwrap_or(Value::Null),
        attributes.get("line_start").cloned().unwrap_or(Value::Null),
        attributes
            .get("column_start")
            .cloned()
            .unwrap_or(Value::Null),
        attributes.get("line_end").cloned().unwrap_or(Value::Null),
        attributes.get("column_end").cloned().unwrap_or(Value::Null),
        Value::String(location.to_owned()),
    ])
    .unwrap_or_default()
}

fn stamp_endpoint_rewrite(edge: &mut EdgeRecord, rule: EndpointRewriteRule, score: f64) {
    preserve_occurrence_rule(&mut edge.attributes);
    append_endpoint_rewrite_evidence(
        &mut edge.attributes,
        EndpointRewriteEvidence { rule, score },
    );
}

fn repository_scope(source: &str) -> String {
    Path::new(source)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_string_lossy()
        .replace('\\', "/")
}

fn string_attribute(node: &NodeRecord, key: &str) -> String {
    node.attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn is_file_node(node: &NodeRecord, source: &str) -> bool {
    !source.is_empty()
        && (string_attribute(node, "symbol_kind") == "file"
            || Path::new(source)
                .file_name()
                .and_then(|value| value.to_str())
                == Some(node.label()))
}

fn is_generic_call_target(node: &NodeRecord) -> bool {
    if node.attributes.get("_callable").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let kind = string_attribute(node, "symbol_kind");
    let legacy_kind = string_attribute(node, "type");
    matches!(
        kind.as_str(),
        "function" | "method" | "constructor" | "database_procedure"
    ) || matches!(
        legacy_kind.as_str(),
        "function" | "method" | "constructor" | "database_procedure"
    )
}

fn relation(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn case_insensitive(source: &str) -> bool {
    matches!(
        extension(source).as_str(),
        "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" | "sql"
    )
}

fn is_javascript(source: &str) -> bool {
    matches!(
        extension(source).as_str(),
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
    )
}

fn language_name_from_source(source: &str) -> Option<&'static str> {
    match extension(source).as_str() {
        "py" | "pyi" => Some("python"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "go" => Some("go"),
        "rs" => Some("rust"),
        "java" => Some("java"),
        _ => None,
    }
}

fn language_family(source: &str) -> Option<&'static str> {
    match extension(source).as_str() {
        "py" | "pyi" => Some("py"),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte"
        | "astro" => Some("js"),
        "java" | "kt" | "kts" | "scala" | "groovy" | "gradle" => Some("jvm"),
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "cu" | "cuh" | "metal" | "m" | "mm"
        | "swift" => Some("native"),
        "go" => Some("go"),
        "rs" => Some("rs"),
        "rb" | "rake" => Some("rb"),
        "php" => Some("php"),
        "cs" => Some("cs"),
        "lua" | "luau" => Some("lua"),
        "razor" | "cshtml" | "xaml" => Some("cs"),
        "zig" => Some("zig"),
        "ex" | "exs" => Some("elixir"),
        "jl" => Some("julia"),
        "dart" => Some("dart"),
        "sh" | "bash" => Some("shell"),
        "ps1" | "psm1" | "psd1" => Some("powershell"),
        _ => None,
    }
}

fn extension(source: &str) -> String {
    Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use compass_graph::{build_from_extraction, normalize_document_v1};
    use serde_json::json;

    #[test]
    fn degraded_universal_resolution_is_machine_visible() {
        let mut extraction = Extraction::default();
        let report = evidence::UniversalResolutionReport {
            partitioned: true,
            degraded: true,
            partitions: 3,
            failed_partitions: 1,
            omitted_candidates: 17,
            reason: Some("bounded test failure".to_owned()),
            ..evidence::UniversalResolutionReport::default()
        };

        append_universal_resolution_report(&mut extraction, &report);

        assert_eq!(universal_resolution_report(&extraction), Some(report));
        let diagnostics = extraction
            .extensions
            .get(GRAPH_DIAGNOSTICS_EXTENSION)
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<GraphDiagnostic>>(value).ok())
            .unwrap_or_default();
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.code == "universal_resolution_partial"
                && diagnostic.message.contains("17 relationship candidates")
        }));
    }

    fn node(id: &str, label: &str, source_file: &str, kind: &str) -> NodeRecord {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(source_file.to_owned()),
        );
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert("type".to_owned(), Value::String(kind.to_owned()));
        NodeRecord {
            id: id.to_owned(),
            attributes,
        }
    }

    fn edge(source: &str, target: &str, relation: &str, source_file: &str) -> EdgeRecord {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(source_file.to_owned()),
        );
        EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        }
    }

    fn typescript_callable(
        id: &str,
        label: &str,
        qualified_name: &str,
        legacy_qualified_name: &str,
        source_file: &str,
    ) -> NodeRecord {
        let mut callable = node(id, label, source_file, "method");
        callable.attributes.extend([
            (
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            ),
            (
                "legacy_qualified_name".to_owned(),
                Value::String(legacy_qualified_name.to_owned()),
            ),
            ("symbol_kind".to_owned(), Value::String("method".to_owned())),
            (
                "language".to_owned(),
                Value::String("typescript".to_owned()),
            ),
        ]);
        callable
    }

    #[test]
    fn framework_identity_propagation_is_limited_to_declared_handlers() {
        let mut extraction = Extraction {
            nodes: vec![
                typescript_callable(
                    "handler",
                    ".handle()",
                    "service.Service.handle",
                    "Service::handle()@10",
                    "service.ts",
                ),
                typescript_callable(
                    "client-send",
                    ".send()",
                    "client-proxy.ClientProxy.send",
                    "ClientProxy::send()@20",
                    "client-proxy.ts",
                ),
            ],
            edges: vec![
                {
                    let mut edge = edge("controller", "handler", "references", "controller.ts");
                    edge.attributes.insert(
                        compass_model::provenance::OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
                        Value::String(
                            "universal-reference-project-module-binding:binding:HandlerAlias:0:0"
                                .to_owned(),
                        ),
                    );
                    edge
                },
                edge("controller", "client-send", "references", "controller.ts"),
            ],
            framework_facts: vec![compass_languages::RawFrameworkFact::Route(
                compass_languages::RawRouteFact {
                    framework: "nest".to_owned(),
                    operation: "GET".to_owned(),
                    raw_path: "/items".to_owned(),
                    normalized_path: "/items".to_owned(),
                    declaring_scope: "ItemsController".to_owned(),
                    anchor: compass_languages::RawFrameworkAnchor {
                        source_file: "controller.ts".to_owned(),
                        start_byte: 0,
                        end_byte: 1,
                        start_line: 1,
                        start_column: 0,
                        end_line: 1,
                        end_column: 1,
                    },
                    handler_reference: "HandlerAlias".to_owned(),
                    middleware_references: Vec::new(),
                    origin: compass_languages::RawFrameworkOrigin::Ast,
                    rule: None,
                    detail: Map::new(),
                },
            )],
            ..Extraction::default()
        };

        restore_framework_callable_names(&mut extraction, &HashMap::new(), Path::new("."));

        assert_eq!(
            extraction.nodes[0].string("qualified_name"),
            "Service::handle()@10"
        );
        assert_eq!(
            extraction.nodes[1].string("qualified_name"),
            "client-proxy.ClientProxy.send"
        );
    }

    #[test]
    fn javascript_resolution_indices_require_import_edges() {
        let mut extraction = Extraction::default();
        assert!(!has_javascript_import_edges(&extraction));

        extraction
            .edges
            .push(edge("module", "member", "contains", "module.ts"));
        assert!(!has_javascript_import_edges(&extraction));

        extraction
            .edges
            .push(edge("module", "target", "imports_from", "module.ts"));
        assert!(has_javascript_import_edges(&extraction));
    }

    #[test]
    fn portable_file_stem_aliases_require_a_unique_target() {
        let mut extraction = Extraction {
            nodes: vec![node(
                "rust-file",
                "documented.rs",
                "src/documented.rs",
                "file",
            )],
            edges: vec![edge("guide", "src_documented", "documents", "guide.md")],
            ..Extraction::default()
        };

        canonicalize_file_targets(&mut extraction, Path::new("/repo"));
        assert_eq!(extraction.edges[0].target, "rust-file");

        extraction.nodes.push(node(
            "python-file",
            "documented.py",
            "src/documented.py",
            "file",
        ));
        extraction.edges[0].target = "src_documented".to_owned();
        canonicalize_file_targets(&mut extraction, Path::new("/repo"));
        assert_eq!(extraction.edges[0].target, "src_documented");
    }

    fn raw(caller: &str, callee: &str, source_file: &str) -> RawCall {
        RawCall {
            caller_nid: caller.to_owned(),
            callee: callee.to_owned(),
            is_member_call: Some(false),
            source_file: source_file.to_owned(),
            source_location: "L7".to_owned(),
            receiver: None,
            receiver_type: None,
            lang: None,
            extensions: Map::new(),
        }
    }

    #[test]
    fn javascript_collection_pass_adds_module_reexport_once() {
        let mut import = edge("barrel", "module", "imports_from", "barrel.ts");
        import
            .attributes
            .insert("context".to_owned(), Value::String("re-export".to_owned()));
        import.attributes.insert(
            "target_file".to_owned(),
            Value::String("module.ts".to_owned()),
        );
        let mut extraction = Extraction {
            edges: vec![import],
            ..Extraction::default()
        };
        resolve_javascript_reexports(&mut extraction);
        resolve_javascript_reexports(&mut extraction);
        let reexports = extraction
            .edges
            .iter()
            .filter(|edge| relation(edge) == "re_exports")
            .collect::<Vec<_>>();
        assert_eq!(reexports.len(), 1);
        assert_eq!(reexports[0].string("context"), "export");
        assert_eq!(reexports[0].string("target_file"), "module.ts");
    }

    #[test]
    fn python_module_member_calls_require_a_matching_module_import() {
        let caller = "/repo/cli.py";
        let unrelated = "/repo/unrelated.py";
        let mut module_call = raw("dispatch", "log_query", caller);
        module_call
            .extensions
            .insert("module_import_use".to_owned(), Value::Bool(true));
        let mut extraction = Extraction {
            nodes: vec![
                node("cli", "cli.py", caller, "module"),
                node("dispatch", "dispatch()", caller, "function"),
                node("unrelated", "unrelated.py", unrelated, "module"),
                node("unrelated_log_query", "log_query()", unrelated, "function"),
            ],
            raw_calls: Some(vec![module_call]),
            ..Extraction::default()
        };

        resolve_cross_file_calls_with_root(&mut extraction, &HashMap::new(), Path::new("/repo"));

        assert!(!extraction.edges.iter().any(|edge| {
            edge.source == "dispatch"
                && edge.target == "unrelated_log_query"
                && relation(edge) == "calls"
        }));
    }

    #[test]
    fn candidate_disambiguation_prefers_imports_tests_and_nearby_paths() {
        let candidates = vec!["prod".to_owned(), "test".to_owned()];
        let sources = AHashMap::from([
            ("prod".to_owned(), "src/service.py".to_owned()),
            ("test".to_owned(), "tests/test_service.py".to_owned()),
        ]);
        assert_eq!(
            disambiguate_candidates(&candidates, &sources, "tests/test_service.py"),
            Some("test".to_owned())
        );
        assert_eq!(
            disambiguate_candidates(&candidates, &sources, "src/caller.py"),
            Some("prod".to_owned())
        );

        let nearby = vec!["same-dir".to_owned(), "far".to_owned()];
        let nearby_sources = AHashMap::from([
            ("same-dir".to_owned(), "src/api/helper.py".to_owned()),
            ("far".to_owned(), "vendor/helper.py".to_owned()),
        ]);
        assert_eq!(
            path_proximity(&nearby, &nearby_sources, "src/api/caller.py"),
            Some("same-dir".to_owned())
        );
        assert_eq!(path_proximity(&nearby, &nearby_sources, ""), None);

        let symbols = AHashSet::from(["far".to_owned()]);
        assert_eq!(
            select_candidate(
                &nearby,
                Some(&symbols),
                None,
                &AHashMap::new(),
                &nearby_sources,
                "src/api/caller.py",
            ),
            Some(("far".to_owned(), true))
        );
    }

    #[test]
    fn path_and_language_classifiers_cover_supported_families() {
        for path in [
            "tests/a.py",
            "src/test_a.py",
            "src/a_test.go",
            "src/a.spec.ts",
            "src/WidgetTests.cs",
            "spec/unit.rb",
        ] {
            assert!(is_test_path(path), "{path}");
        }
        assert!(!is_test_path("src/widget.rs"));
        assert_eq!(normalize_path(r"src\api\x.py"), "src/api/x.py");
        assert_eq!(parent_segments("src/api/x.py"), vec!["src", "api"]);
        assert_eq!(language_family("x.tsx"), Some("js"));
        assert_eq!(language_family("x.hpp"), Some("native"));
        assert_eq!(language_family("x.psm1"), Some("powershell"));
        assert_eq!(language_family("README"), None);
        assert!(case_insensitive("query.SQL"));
        assert!(is_javascript("view.mjs"));
        assert!(is_language_builtin_global("javascript", "Promise"));
        assert!(!is_language_builtin_global("rust", "Promise"));
        assert!(!is_language_builtin_global(
            "javascript",
            "project_function"
        ));
    }

    #[test]
    fn cross_file_resolution_filters_builtins_members_mixins_and_javascript_guesses() {
        let mut callable = node("target", "work()", "src/b.py", "function");
        callable
            .attributes
            .insert("_callable".to_owned(), Value::Bool(true));
        let mut indirect = raw("caller", "work", "src/a.py");
        indirect
            .extensions
            .insert("indirect".to_owned(), Value::Bool(true));
        indirect
            .extensions
            .insert("context".to_owned(), Value::String("callback".to_owned()));
        let mut mixin = raw("caller", "work", "src/a.py");
        mixin
            .extensions
            .insert("is_mixin".to_owned(), Value::Bool(true));
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller()", "src/a.py", "function"),
                callable,
                node("js-caller", "caller()", "web/a.ts", "function"),
                node("js-target", "work()", "web/b.ts", "function"),
            ],
            raw_calls: Some(vec![
                indirect,
                mixin,
                raw("caller", "len", "src/a.py"),
                RawCall {
                    is_member_call: Some(true),
                    ..raw("caller", "work", "src/a.py")
                },
                raw("js-caller", "work", "web/a.ts"),
            ]),
            ..Extraction::default()
        };
        resolve_cross_file_calls(&mut extraction, &HashMap::new());
        assert!(extraction.edges.iter().any(|candidate| {
            candidate.source == "caller"
                && candidate.target == "target"
                && relation(candidate) == "indirect_call"
                && candidate.string("context") == "callback"
        }));
        assert!(!extraction.edges.iter().any(|candidate| {
            candidate.source == "js-caller"
                && candidate.target == "js-target"
                && relation(candidate) == "calls"
        }));
    }

    #[test]
    fn collision_disambiguation_rewrites_nodes_edges_and_raw_callers() {
        let mut first = node("duplicate", "Thing", "include/thing.h", "class");
        first.attributes.insert(
            "origin_file".to_owned(),
            Value::String("include/thing.h".to_owned()),
        );
        let second = node("duplicate", "Thing", "src/thing.cpp", "class");
        let mut import = edge("source", "duplicate", "imports", "src/use.cpp");
        import.attributes.insert(
            "target_file".to_owned(),
            Value::String("include/thing.h".to_owned()),
        );
        let mut extraction = Extraction {
            nodes: vec![first, second],
            edges: vec![import],
            raw_calls: Some(vec![raw("duplicate", "work", "src/thing.cpp")]),
            extensions: Map::from_iter([("fixture".to_owned(), json!(true))]),
            ..Extraction::default()
        };
        disambiguate_colliding_node_ids(&mut extraction, Path::new("."));
        assert_ne!(extraction.nodes[0].id, extraction.nodes[1].id);
        assert_eq!(extraction.edges[0].target, extraction.nodes[0].id);
        assert_eq!(
            extraction
                .raw_calls
                .as_ref()
                .and_then(|calls| calls.first())
                .map(|call| &call.caller_nid),
            Some(&extraction.nodes[1].id)
        );
        assert!(!extraction.edges[0].attributes.contains_key("target_file"));
    }

    #[test]
    fn declaration_definition_merge_keeps_the_unique_sibling_header() {
        let header = Extraction {
            nodes: vec![node("widget_draw", "draw", "native/Widget.h", "method")],
            edges: vec![edge("widget", "widget_draw", "method", "native/Widget.h")],
            ..Extraction::default()
        };
        let mut implementation_node = node(
            "widget_draw",
            "Widget::draw()",
            "native/Widget.cpp",
            "method",
        );
        implementation_node.attributes.insert(
            "implementation_hash".to_owned(),
            Value::String("body-digest".to_owned()),
        );
        implementation_node.attributes.insert(
            "signature_hash".to_owned(),
            Value::String("signature-digest".to_owned()),
        );
        implementation_node.attributes.insert(
            "source_hash".to_owned(),
            Value::String("source-digest".to_owned()),
        );
        let implementation = Extraction {
            nodes: vec![implementation_node],
            edges: vec![
                edge("widget", "widget_draw", "method", "native/Widget.cpp"),
                edge("widget_draw", "widget_draw", "calls", "native/Widget.cpp"),
            ],
            ..Extraction::default()
        };
        let unrelated = Extraction {
            nodes: vec![
                node("logger", "Logger", "a/Logger.h", "class"),
                node("logger", "Logger", "b/Logger.cpp", "class"),
            ],
            ..Extraction::default()
        };
        let mut extractions = vec![header, implementation, unrelated];

        assert!(merge_decl_def_classes_if_needed_changed(
            &mut extractions,
            &[
                PathBuf::from("native/Widget.h"),
                PathBuf::from("native/Widget.cpp"),
            ],
        ));

        let merged = extractions
            .iter()
            .flat_map(|extraction| &extraction.nodes)
            .filter(|candidate| candidate.id == "widget_draw")
            .collect::<Vec<_>>();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].label(), "draw");
        assert_eq!(merged[0].string("source_file"), "native/Widget.h");
        assert_eq!(merged[0].string("implementation_hash"), "body-digest");
        assert_eq!(merged[0].string("signature_hash"), "signature-digest");
        assert_eq!(merged[0].string("source_hash"), "source-digest");
        assert_eq!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.nodes)
                .filter(|candidate| candidate.id == "logger")
                .count(),
            2
        );
        assert_eq!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.edges)
                .filter(|candidate| candidate.source == "widget")
                .count(),
            1
        );
        assert!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.edges)
                .all(|candidate| candidate.source != candidate.target)
        );
    }

    #[test]
    fn declaration_definition_merge_skips_non_native_source_sets() {
        let mut extractions = vec![Extraction {
            nodes: vec![
                node("duplicate", "first", "package/first.py", "class"),
                node("duplicate", "second", "package/second.py", "class"),
            ],
            ..Extraction::default()
        }];

        assert!(!merge_decl_def_classes_if_needed_changed(
            &mut extractions,
            &[PathBuf::from("package/first.py")],
        ));

        assert_eq!(extractions[0].nodes.len(), 2);
    }

    #[test]
    fn unique_stub_rewiring_retargets_edges_and_removes_unreferenced_stubs() {
        let mut extraction = Extraction {
            nodes: vec![
                node("type", "Widget", "src/widget.py", "class"),
                node("stub", "Widget", "", "stub"),
                node("func", "run()", "src/run.py", "function"),
                node("func-stub", "run()", "", "stub"),
                node("recursive", "recursive()", "src/test.py", "function"),
                node("recursive-stub", "recursive()", "", "stub"),
            ],
            edges: vec![
                edge("stub", "func-stub", "uses", "src/use.py"),
                edge("type", "stub", "inherits", "src/widget.py"),
                edge("recursive", "recursive-stub", "tests", "src/test.py"),
            ],
            ..Extraction::default()
        };
        rewire_unique_stub_nodes(&mut extraction);
        assert!(
            extraction
                .edges
                .iter()
                .any(|candidate| { candidate.source == "type" && candidate.target == "func" })
        );
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "stub")
        );
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "func-stub")
        );
        assert!(
            extraction
                .edges
                .iter()
                .all(|candidate| candidate.source != candidate.target)
        );
        for edge in &extraction.edges {
            assert!(
                edge.attributes["_endpoint_rewrite_rules"]
                    .as_array()
                    .is_some_and(|entries| entries.iter().any(|entry| {
                        entry["rule"] == "unique-stub-endpoint-resolution" && entry["score"] == 0.8
                    }))
            );
        }
    }

    #[test]
    fn every_resolver_rewrite_family_preserves_occurrences_through_v1()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/lib.rs"), vec![b'x'; 500])?;
        let rewrite_rules = [
            EndpointRewriteRule::CsharpNamespaceCanonicalization,
            EndpointRewriteRule::LanguageFamilyStubResolution,
            EndpointRewriteRule::PhpQualifiedTypeResolution,
            EndpointRewriteRule::CanonicalImportTarget,
            EndpointRewriteRule::UniqueStubEndpointResolution,
            EndpointRewriteRule::SourceScopedNodeDisambiguation,
            EndpointRewriteRule::HeaderImportDisambiguation,
        ];

        for rewrite_rule in rewrite_rules {
            let anchor = json!({
                "file":root.join("src/lib.rs"),
                "startByte":50,
                "endByte":54,
                "startLine":6,
                "startColumn":0,
                "endLine":6,
                "endColumn":4
            });
            let mut remapped_same = EdgeRecord {
                source: "pre-rewrite-caller".to_owned(),
                target: "callee".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("calls")),
                    ("rule".to_owned(), json!("producer-a")),
                    ("extractor".to_owned(), json!("test.resolver")),
                    ("_origin".to_owned(), json!("ast")),
                    ("confidence".to_owned(), json!("EXTRACTED")),
                    ("source_anchor".to_owned(), anchor.clone()),
                ]),
            };
            stamp_endpoint_rewrite(&mut remapped_same, rewrite_rule, 0.9);
            assert_eq!(remapped_same.string("_origin"), "ast");
            assert_eq!(remapped_same.string("confidence"), "EXTRACTED");
            assert_eq!(remapped_same.string("rule"), "producer-a");
            assert_eq!(
                remapped_same.string("_occurrence_rule"),
                "producer-a",
                "lost producer identity for {}",
                rewrite_rule.as_str()
            );
            remapped_same.source = "caller".to_owned();
            let mut remapped_distinct = remapped_same.clone();
            remapped_distinct
                .attributes
                .insert("rule".to_owned(), json!("producer-b"));
            remapped_distinct
                .attributes
                .insert("_occurrence_rule".to_owned(), json!("producer-b"));
            let direct_same = EdgeRecord {
                source: "caller".to_owned(),
                target: "callee".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("calls")),
                    ("rule".to_owned(), json!("producer-a")),
                    ("extractor".to_owned(), json!("test.direct")),
                    ("_origin".to_owned(), json!("ast")),
                    ("confidence".to_owned(), json!("EXTRACTED")),
                    ("source_anchor".to_owned(), anchor.clone()),
                ]),
            };
            let node = |id: &str, qualified_name: &str, start_byte: u64| NodeRecord {
                id: id.to_owned(),
                attributes: Map::from_iter([
                    ("label".to_owned(), json!(qualified_name)),
                    ("qualified_name".to_owned(), json!(qualified_name)),
                    ("symbol_kind".to_owned(), json!("function")),
                    ("file_type".to_owned(), json!("code")),
                    ("source_file".to_owned(), json!(root.join("src/lib.rs"))),
                    ("extractor".to_owned(), json!("test.resolver")),
                    ("_origin".to_owned(), json!("ast")),
                    (
                        "source_anchor".to_owned(),
                        json!({
                            "file":root.join("src/lib.rs"),
                            "startByte":start_byte,
                            "endByte":start_byte + 4,
                            "startLine":start_byte / 10 + 1,
                            "startColumn":0,
                            "endLine":start_byte / 10 + 1,
                            "endColumn":4
                        }),
                    ),
                ]),
            };
            let extraction = Extraction {
                nodes: vec![
                    node("caller", "crate::caller", 10),
                    node("callee", "crate::callee", 30),
                ],
                edges: vec![direct_same, remapped_same, remapped_distinct],
                ..Extraction::default()
            };

            let flexible = build_from_extraction(&extraction, true, Some(root));
            let typed = normalize_document_v1(&flexible, root, "sha256:test", None)?;

            assert_eq!(
                flexible.links.len(),
                2,
                "flexible links for {}: {:?}",
                rewrite_rule.as_str(),
                flexible.links
            );
            assert_eq!(
                typed.links.len(),
                2,
                "typed links for {}: {:?}",
                rewrite_rule.as_str(),
                typed.links
            );
            let producer_a = typed
                .links
                .iter()
                .find(|edge| {
                    edge.occurrence_rule
                        .as_ref()
                        .is_some_and(|rule| rule.as_str() == "producer-a")
                })
                .ok_or("missing producer-a occurrence")?;
            assert!(
                producer_a
                    .evidence
                    .iter()
                    .any(|evidence| evidence.rule.as_deref() == Some(rewrite_rule.as_str())),
                "missing {} endpoint evidence: {:?}",
                rewrite_rule.as_str(),
                producer_a.evidence
            );
            assert!(
                producer_a.evidence.iter().any(|evidence| {
                    evidence.origin == compass_model::provenance::EvidenceOrigin::Ast
                        && evidence.rule.as_deref() == Some("producer-a")
                }),
                "missing producer evidence for {}: {:?}",
                rewrite_rule.as_str(),
                producer_a.evidence
            );
        }
        Ok(())
    }

    #[test]
    fn generic_stub_rewiring_never_crosses_language_families() {
        let mut extraction = Extraction {
            nodes: vec![
                node("php-result", "Result", "fixture.php", "class"),
                node("rust-result", "Result", "fixture.rs", "struct"),
                node("result", "Result", "", "stub"),
                node("python-image", "_ImageRef", "llm.py", "class"),
                node("rust-image", "ImageRef", "image.rs", "struct"),
                node("imageref", "ImageRef", "", "stub"),
            ],
            edges: vec![
                edge("caller", "result", "references", "caller.rs"),
                edge("caller", "imageref", "references", "caller.rs"),
            ],
            ..Extraction::default()
        };

        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[0].target, "rust-result");
        assert_eq!(extraction.edges[1].target, "rust-image");
    }

    #[test]
    fn stub_rewiring_requires_compatible_repository_scope() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "package-b-base",
                    "Base",
                    "packages/b/src/Base.java",
                    "class",
                ),
                node("base", "Base", "", "stub"),
                node(
                    "package-a-child",
                    "Child",
                    "packages/a/src/Child.java",
                    "class",
                ),
            ],
            edges: vec![edge(
                "package-a-child",
                "base",
                "extends",
                "packages/a/src/Child.java",
            )],
            ..Extraction::default()
        };

        rewire_unique_family_stubs(&mut extraction);
        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[0].target, "base");
        assert!(extraction.nodes.iter().any(|node| node.id == "base"));
    }

    #[test]
    fn stub_rewiring_retains_unknown_language_and_scope_as_unresolved() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "definition",
                    "Shared",
                    "packages/b/definition.custom",
                    "class",
                ),
                node("shared", "Shared", "", "stub"),
                node(
                    "consumer",
                    "Consumer",
                    "packages/a/consumer.custom",
                    "class",
                ),
            ],
            edges: vec![edge(
                "consumer",
                "shared",
                "references",
                "packages/a/consumer.custom",
            )],
            ..Extraction::default()
        };

        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[0].target, "shared");
        assert!(extraction.nodes.iter().any(|node| node.id == "shared"));
    }

    #[test]
    fn explicit_import_evidence_allows_cross_scope_stub_rewiring() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "package-b-base",
                    "Base",
                    "packages/b/src/Base.java",
                    "class",
                ),
                node("base", "Base", "", "stub"),
                node(
                    "package-a-child",
                    "Child",
                    "packages/a/src/Child.java",
                    "class",
                ),
            ],
            edges: vec![
                edge(
                    "package-a-child",
                    "package-b-base",
                    "imports",
                    "packages/a/src/Child.java",
                ),
                edge(
                    "package-a-child",
                    "base",
                    "extends",
                    "packages/a/src/Child.java",
                ),
            ],
            ..Extraction::default()
        };

        rewire_unique_family_stubs(&mut extraction);
        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[1].target, "package-b-base");
        assert!(extraction.nodes.iter().all(|node| node.id != "base"));
    }

    #[test]
    fn file_scoped_import_evidence_allows_cross_scope_stub_rewiring() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "package-b-base",
                    "Base",
                    "packages/b/src/Base.java",
                    "class",
                ),
                node("base", "Base", "", "stub"),
                node(
                    "package-a-child",
                    "Child",
                    "packages/a/src/Child.java",
                    "class",
                ),
                node(
                    "package-a-import",
                    "BaseImport",
                    "packages/a/src/Child.java",
                    "import",
                ),
            ],
            edges: vec![
                edge(
                    "package-a-import",
                    "package-b-base",
                    "imports",
                    "packages/a/src/Child.java",
                ),
                edge(
                    "package-a-child",
                    "base",
                    "extends",
                    "packages/a/src/Child.java",
                ),
            ],
            ..Extraction::default()
        };

        rewire_unique_family_stubs(&mut extraction);
        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[1].target, "package-b-base");
        assert!(extraction.nodes.iter().all(|node| node.id != "base"));
    }

    #[test]
    fn csharp_namespace_canonicalization_keeps_lexicographically_earliest_source() {
        let mut later = node(
            "namespace-id",
            "Demo.ViewModels",
            "views/ToolkitViewModel.cs",
            "namespace",
        );
        later
            .attributes
            .insert("source_location".to_owned(), Value::String("L4".to_owned()));
        let mut earlier = node(
            "namespace-id",
            "Demo.ViewModels",
            "views/DesignViewModel.cs",
            "namespace",
        );
        earlier
            .attributes
            .insert("source_location".to_owned(), Value::String("L1".to_owned()));
        let mut extraction = Extraction {
            nodes: vec![later, earlier],
            edges: vec![edge("consumer", "namespace-id", "imports", "views/App.cs")],
            ..Extraction::default()
        };

        canonicalize_csharp_namespace_nodes(&mut extraction);

        assert_eq!(extraction.nodes.len(), 1);
        assert_eq!(
            extraction.nodes[0].string("source_file"),
            "views/DesignViewModel.cs"
        );
        assert_eq!(extraction.edges[0].target, "namespace-id");
    }

    #[test]
    fn family_stub_rewiring_does_not_conflate_same_named_cross_language_types() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "java-processor",
                    "Processor",
                    "src/Processor.java",
                    "interface",
                ),
                node("rust-processor", "Processor", "src/processor.rs", "trait"),
                node("processor", "Processor", "", "stub"),
                node(
                    "java-data",
                    "DataProcessor",
                    "src/DataProcessor.java",
                    "class",
                ),
            ],
            edges: vec![edge(
                "java-data",
                "processor",
                "implements",
                "src/DataProcessor.java",
            )],
            ..Extraction::default()
        };

        rewire_unique_family_stubs(&mut extraction);

        assert_eq!(extraction.edges[0].target, "java-processor");
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "processor")
        );
    }

    #[test]
    fn php_use_aliases_retarget_external_type_stubs_to_qualified_nodes() {
        let mut extraction = Extraction {
            nodes: vec![
                node("file", "Client.php", "src/Client.php", "file"),
                node("client", "Client", "src/Client.php", "class"),
                node("authenticator", "Authenticator", "", "stub"),
            ],
            edges: vec![
                edge("file", "authenticator", "imports", "src/Client.php"),
                edge("client", "authenticator", "references", "src/Client.php"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([(
            "src/Client.php".to_owned(),
            "<?php\nnamespace App\\Http;\nuse App\\Auth\\Authenticator;\nclass Client {}\n"
                .to_owned(),
        )]);

        resolve_php_type_references(&mut extraction, &sources);

        let qualified = make_id(&["App\\Auth\\Authenticator"]);
        assert!(
            extraction
                .edges
                .iter()
                .all(|candidate| candidate.target == qualified)
        );
        assert!(extraction.nodes.iter().any(|candidate| {
            candidate.id == qualified && candidate.label() == "App\\Auth\\Authenticator"
        }));
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "authenticator")
        );
        assert_eq!(
            php_use_entries("Vendor\\Package\\{Service, Contract as API}"),
            vec![
                ("Service".to_owned(), "Vendor\\Package\\Service".to_owned()),
                ("API".to_owned(), "Vendor\\Package\\Contract".to_owned()),
            ]
        );
    }

    #[test]
    fn php_type_resolution_is_order_independent_and_rejects_ambiguous_definitions() {
        let sources = HashMap::from([
            (
                "tests/Local.php".to_owned(),
                "<?php\nnamespace Fixtures;\nclass Post {}\n".to_owned(),
            ),
            (
                "tests/Remote.php".to_owned(),
                "<?php\nnamespace Fixtures;\nclass Post {}\n".to_owned(),
            ),
            (
                "tests/Ambiguous.php".to_owned(),
                "<?php\nnamespace Fixtures;\n".to_owned(),
            ),
        ]);
        let make_extraction = || Extraction {
            nodes: vec![
                node("local-post", "Post", "tests/Local.php", "class"),
                node("remote-post", "Post", "tests/Remote.php", "class"),
                node("method-post", "Post", "tests/Local.php", "method"),
                node("post-stub", "Post", "", "stub"),
            ],
            edges: vec![edge(
                "local-consumer",
                "post-stub",
                "references",
                "tests/Local.php",
            )],
            ..Extraction::default()
        };

        let mut forward = make_extraction();
        resolve_php_type_references(&mut forward, &sources);
        assert_eq!(forward.edges[0].target, "local-post");

        let mut reversed = make_extraction();
        reversed.nodes.reverse();
        resolve_php_type_references(&mut reversed, &sources);
        assert_eq!(reversed.edges[0].target, "local-post");

        let mut ambiguous = make_extraction();
        ambiguous.edges[0] = edge(
            "ambiguous-consumer",
            "post-stub",
            "references",
            "tests/Ambiguous.php",
        );
        resolve_php_type_references(&mut ambiguous, &sources);
        assert_eq!(ambiguous.edges[0].target, "post-stub");
    }
}
