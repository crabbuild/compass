//! Deterministic cross-file resolution over immutable extraction facts.

pub mod evidence;
pub mod frameworks;
mod members;

pub use members::resolve_language_calls;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use compass_languages::{
    Extraction, RawCall, RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord,
    SemanticEvidenceBatch, is_language_builtin_global, make_id,
};
use compass_model::provenance::{
    EndpointRewriteEvidence, EndpointRewriteRule, OCCURRENCE_RULE_ATTRIBUTE,
    append_endpoint_rewrite_evidence, preserve_occurrence_rule,
};
use rayon::prelude::*;
use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

const DECLARATION_SUFFIXES: &[&str] = &["h", "hpp", "hh", "hxx"];
const IMPLEMENTATION_SUFFIXES: &[&str] = &["m", "mm", "cpp", "cc", "cxx", "c"];

/// Collapse a clean sibling header/implementation declaration pair before
/// portable file-prefix remapping would split their shared symbol IDs.
///
/// This mirrors Compass's collection-level C/C++/Objective-C pass. Only an
/// ID collision from one directory/base-stem family with exactly one header
/// is eligible; every other collision is left for conservative disambiguation.
pub fn merge_decl_def_classes(extractions: &mut [Extraction]) {
    let mut groups = HashMap::<String, Vec<(usize, usize, String)>>::new();
    for (extraction_index, extraction) in extractions.iter().enumerate() {
        for (node_index, node) in extraction.nodes.iter().enumerate() {
            let source = string_attribute(node, "source_file");
            if string_attribute(node, "file_type") == "code"
                && !node.id.is_empty()
                && !source.is_empty()
            {
                groups.entry(node.id.clone()).or_default().push((
                    extraction_index,
                    node_index,
                    source,
                ));
            }
        }
    }

    let mut dropped = HashSet::<(usize, usize)>::new();
    let mut definition_hashes = Vec::<((usize, usize), Vec<(String, Value)>)>::new();
    for entries in groups.values().filter(|entries| entries.len() > 1) {
        let mut sibling_keys = HashSet::new();
        let mut headers = Vec::new();
        let mut eligible = true;
        for &(extraction_index, node_index, ref source) in entries {
            let path = Path::new(source);
            let suffix = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !DECLARATION_SUFFIXES.contains(&suffix.as_str())
                && !IMPLEMENTATION_SUFFIXES.contains(&suffix.as_str())
            {
                eligible = false;
                break;
            }
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
            if DECLARATION_SUFFIXES.contains(&suffix.as_str()) {
                headers.push((extraction_index, node_index));
            }
        }
        if eligible && sibling_keys.len() == 1 && headers.len() == 1 {
            let keeper = headers[0];
            if let Some((extraction_index, node_index, _)) = entries
                .iter()
                .filter(|(extraction_index, node_index, source)| {
                    let suffix = Path::new(source)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    IMPLEMENTATION_SUFFIXES.contains(&suffix.as_str())
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
        return;
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
    let mut language_facts = members::collect_language_call_facts(extractions);
    language_facts
        .calls
        .retain(|call| !matches!(call.lang.as_deref(), Some("python" | "go")));
    let evidence_batches = extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.clone())
        .collect::<Vec<_>>();
    let mut merged = Extraction::default();
    for extraction in extractions {
        if extraction.semantic_evidence.is_some() {
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
    finish_resolution(merged, language_facts, evidence_batches, sources, root)
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
    let mut language_facts = members::collect_language_call_facts_owned(&mut extractions);
    language_facts
        .calls
        .retain(|call| !matches!(call.lang.as_deref(), Some("python" | "go")));
    let mut evidence_batches = Vec::new();
    let mut merged = Extraction::default();
    for extraction in &mut extractions {
        let universal = extraction.semantic_evidence.take();
        if universal.is_some() {
            let mut allowed = universal
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
    extractions.into_par_iter().for_each(drop);
    finish_resolution(merged, language_facts, evidence_batches, sources, root)
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
    allowed
}

fn is_source_inventory_node(node: &NodeRecord) -> bool {
    node.string("symbol_kind") == "file" && !node.string("source_file").is_empty()
}

fn finish_resolution(
    mut merged: Extraction,
    mut language_facts: members::LanguageCallFacts,
    evidence_batches: Vec<SemanticEvidenceBatch>,
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    let mut profile_started = Instant::now();
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    resolve_javascript_reexports(&mut merged);
    profile_internal("resolver JavaScript re-exports", &mut profile_started);
    canonicalize_import_targets(&mut merged);
    profile_internal("resolver import canonicalization", &mut profile_started);
    if !evidence_batches.is_empty() {
        match evidence::UniversalResolutionIndex::new_with_inventory(
            &evidence_batches,
            &merged.nodes,
            &canonical_root,
            evidence::UniversalResolutionLimits::default(),
        ) {
            Ok(index) => index.materialize(&mut merged.nodes, &mut merged.edges),
            Err(error) => {
                merged
                    .error
                    .get_or_insert_with(|| format!("universal resolution failed: {error}"));
            }
        }
    }
    profile_internal("resolver universal evidence", &mut profile_started);
    disambiguate_colliding_node_ids_with_calls(
        &mut merged,
        &canonical_root,
        &mut language_facts.calls,
    );
    profile_internal("resolver collision disambiguation", &mut profile_started);
    canonicalize_csharp_namespace_nodes(&mut merged);
    profile_internal("resolver C# namespace normalization", &mut profile_started);
    resolve_php_type_references(&mut merged, sources);
    profile_internal("resolver PHP types", &mut profile_started);
    rewire_unique_family_stubs(&mut merged);
    profile_internal("resolver family stubs", &mut profile_started);
    rewire_unique_stub_nodes(&mut merged);
    profile_internal("resolver unique stubs", &mut profile_started);
    resolve_cross_file_calls_with_root_calls(&mut merged, sources, root, &language_facts.calls);
    profile_internal("resolver cross-file calls", &mut profile_started);
    members::resolve_language_call_facts(language_facts, &mut merged);
    profile_internal("resolver language calls", &mut profile_started);
    let (routes, domains) =
        frameworks::resolve_framework_facts(&merged, compass_languages::FrameworkLimits::default());
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

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
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
        if source.is_empty() {
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

    let mut internal_types = HashMap::new();
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
            .or_insert_with(|| node.id.clone());
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
        let Some((namespace, uses)) = facts.get(&edge.string("source_file")) else {
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
        let target = if let Some(target) = internal_types.get(&fqn.to_ascii_lowercase()) {
            target.clone()
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

fn canonicalize_import_targets(extraction: &mut Extraction) {
    let aliases = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            let source = string_attribute(node, "source_file");
            is_file_node(node, &source).then_some((make_id(&[&source]), node.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    for edge in &mut extraction.edges {
        if matches!(
            relation(edge),
            "imports" | "imports_from" | "exports" | "re_exports"
        ) && let Some(target) = aliases.get(&edge.target)
        {
            edge.target.clone_from(target);
            stamp_endpoint_rewrite(edge, EndpointRewriteRule::CanonicalImportTarget, 1.0);
        }
    }
}

fn rewire_unique_stub_nodes(extraction: &mut Extraction) {
    let mut types = HashMap::<String, Vec<String>>::new();
    let mut types_ci = HashMap::<String, Vec<String>>::new();
    let mut functions = HashMap::<String, Vec<String>>::new();
    let mut source_by_id = HashMap::<String, String>::new();
    let mut stubs = Vec::<(String, String)>::new();
    for node in &extraction.nodes {
        let normalized_label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        let label = normalized_label
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if label.is_empty() {
            continue;
        }
        let source = string_attribute(node, "source_file");
        source_by_id.insert(node.id.clone(), source.clone());
        if source.is_empty() {
            stubs.push((node.id.clone(), label));
        } else if is_type_like_definition(node) {
            types
                .entry(label.clone())
                .or_default()
                .push(node.id.clone());
            if case_insensitive(&source) {
                types_ci
                    .entry(label.to_ascii_lowercase())
                    .or_default()
                    .push(node.id.clone());
            }
        } else if node.label().ends_with("()") && !node.label().starts_with('.') {
            functions.entry(label).or_default().push(node.id.clone());
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
        let candidate = compatible_unique(types.get(&label))
            .or_else(|| compatible_unique(types_ci.get(&label.to_ascii_lowercase())))
            .or_else(|| {
                if supertype_stubs.contains(stub.as_str()) {
                    return None;
                }
                compatible_unique(functions.get(&label))
            });
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
    if string_attribute(node, "type") == "namespace"
        || string_attribute(node, "file_type") != "code"
    {
        return false;
    }
    let label = node.label().trim();
    !label.is_empty() && !label.ends_with(')') && !label.starts_with('.') && !label.contains('.')
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
    let mut groups = HashMap::<String, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if matches!(
            string_attribute(node, "type").as_str(),
            "module" | "namespace"
        ) {
            continue;
        }
        if !node.id.is_empty() {
            groups.entry(node.id.clone()).or_default().push(index);
        }
    }
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

/// Resolve non-member raw calls using unique definitions and import evidence.
pub fn resolve_cross_file_calls(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    resolve_cross_file_calls_with_root(extraction, sources, Path::new("."));
}

fn resolve_cross_file_calls_with_root(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    resolve_cross_file_calls_with_root_calls(extraction, sources, root, &raw_calls);
}

fn resolve_cross_file_calls_with_root_calls(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
    raw_calls: &[RawCall],
) {
    let mut profile_started = Instant::now();
    let python_imports = sources
        .par_iter()
        .filter(|(source_file, _)| extension(source_file) == "py")
        .map(|(source_file, source)| (source_file.clone(), python_symbol_imports(source)))
        .collect::<HashMap<_, _>>();
    let import_edges = resolve_python_import_guided_with_calls(
        extraction,
        sources,
        root,
        raw_calls,
        &python_imports,
    );
    extraction.edges.extend(import_edges);
    profile_internal("resolver Python import-guided calls", &mut profile_started);
    let mut exact = AHashMap::<String, Vec<String>>::new();
    let mut folded = AHashMap::<String, Vec<String>>::new();
    let mut source_by_id = AHashMap::<String, String>::new();
    let mut file_by_source = AHashMap::<String, String>::new();
    let mut callable = AHashSet::<String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
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
            if target != raw.caller_nid
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
                extraction.edges.push(edge);
            }
            continue;
        }
        if target == raw.caller_nid || (!import_evidence && is_javascript(&raw.source_file)) {
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
            extraction.edges.push(edge);
        }
    }
    profile_internal("resolver generic cross-file calls", &mut profile_started);
}

#[cfg(test)]
fn resolve_python_import_guided(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    let python_imports = sources
        .iter()
        .filter(|(source_file, _)| extension(source_file) == "py")
        .map(|(source_file, source)| (source_file.clone(), python_symbol_imports(source)))
        .collect::<HashMap<_, _>>();
    let edges = resolve_python_import_guided_with_calls(
        extraction,
        sources,
        root,
        &raw_calls,
        &python_imports,
    );
    extraction.edges.extend(edges);
}

fn resolve_python_import_guided_with_calls(
    extraction: &Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
    raw_calls: &[RawCall],
    python_imports: &HashMap<String, Vec<PythonImport>>,
) -> Vec<EdgeRecord> {
    let mut resolved_edges = Vec::new();
    let mut definitions = HashMap::<String, Vec<(String, String)>>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if extension(&source) != "py" {
            continue;
        }
        let label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        definitions
            .entry(label)
            .or_default()
            .push((source, node.id.clone()));
    }
    let normalized_sources = sources
        .iter()
        .map(|(source_file, source)| {
            (
                normalize_path(source_file),
                (source_file.as_str(), source.as_str()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut known = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
            )
        })
        .collect::<HashSet<_>>();
    let mut known_import_occurrences = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
                edge.attributes
                    .get(OCCURRENCE_RULE_ATTRIBUTE)
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        })
        .collect::<HashSet<_>>();
    let file_nodes = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            let source = string_attribute(node, "source_file");
            is_file_node(node, &source).then_some((source, node.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let normalized_file_nodes = file_nodes
        .iter()
        .map(|(source, id)| (normalize_path(source), id.clone()))
        .collect::<HashMap<_, _>>();
    for source_file in sources.keys() {
        if extension(source_file) != "py" {
            continue;
        }
        let Some(file_node) = file_nodes.get(source_file) else {
            continue;
        };
        let Some(imports) = python_imports.get(source_file) else {
            continue;
        };
        for imported in imports {
            let module_file = python_module_file(
                Path::new(source_file),
                root,
                &imported.module,
                None,
                &normalized_file_nodes,
            );
            let candidates = python_resolved_definition_candidates(
                Path::new(source_file),
                root,
                &imported.module,
                &imported.imported,
                &definitions,
                &normalized_sources,
                false,
            );
            if candidates.len() == 1 {
                let target = &candidates[0];
                if known_import_occurrences.insert((
                    file_node.clone(),
                    target.clone(),
                    "imports".to_owned(),
                    Some(python_import_occurrence_rule(
                        PythonImportResolution::SymbolImport,
                        imported,
                    )),
                )) {
                    resolved_edges.push(python_import_edge(
                        file_node,
                        target,
                        PythonImportResolution::SymbolImport,
                        source_file,
                        imported,
                    ));
                }
            } else if let Some(target) = python_module_file(
                Path::new(source_file),
                root,
                &imported.module,
                Some(&imported.imported),
                &normalized_file_nodes,
            ) && known_import_occurrences.insert((
                file_node.clone(),
                target.clone(),
                "imports_from".to_owned(),
                Some(python_import_occurrence_rule(
                    PythonImportResolution::SubmoduleImport,
                    imported,
                )),
            )) {
                resolved_edges.push(python_import_edge(
                    file_node,
                    &target,
                    PythonImportResolution::SubmoduleImport,
                    source_file,
                    imported,
                ));
            }
            if Path::new(source_file)
                .file_name()
                .and_then(|name| name.to_str())
                == Some("__init__.py")
                && let Some(target) = module_file
                && known_import_occurrences.insert((
                    file_node.clone(),
                    target.clone(),
                    "re_exports".to_owned(),
                    Some(python_import_occurrence_rule(
                        PythonImportResolution::ModuleReExport,
                        imported,
                    )),
                ))
            {
                resolved_edges.push(python_import_edge(
                    file_node,
                    &target,
                    PythonImportResolution::ModuleReExport,
                    source_file,
                    imported,
                ));
            }
        }
    }
    let aliases_by_source = python_imports
        .iter()
        .map(|(source_file, imports)| {
            (
                source_file.as_str(),
                imports
                    .iter()
                    .map(|import| {
                        (
                            import.local.clone(),
                            (import.module.clone(), import.imported.clone()),
                        )
                    })
                    .collect::<HashMap<_, _>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    for raw in raw_calls {
        if raw.is_member_call == Some(true)
            || extension(&raw.source_file) != "py"
            || raw.extensions.get("indirect").and_then(Value::as_bool) == Some(true)
            || raw
                .extensions
                .get("symbol_import_use")
                .and_then(Value::as_bool)
                != Some(true)
        {
            continue;
        }
        let Some(aliases) = aliases_by_source.get(raw.source_file.as_str()) else {
            continue;
        };
        let Some((module, imported)) = aliases.get(&raw.callee) else {
            continue;
        };
        let candidates = python_resolved_definition_candidates(
            Path::new(&raw.source_file),
            root,
            module,
            imported,
            &definitions,
            &normalized_sources,
            false,
        );
        if candidates.len() != 1 {
            continue;
        }
        let target = &candidates[0];
        if target == &raw.caller_nid
            || !known.insert((raw.caller_nid.clone(), target.clone(), "calls".to_owned()))
        {
            continue;
        }
        let mut edge = resolved_edge(raw, target, "EXTRACTED", 1.0);
        edge.attributes.remove("confidence_score");
        resolved_edges.push(edge);
    }
    resolved_edges
}

#[derive(Clone, Copy)]
enum PythonImportResolution {
    SymbolImport,
    SubmoduleImport,
    ModuleReExport,
}

impl PythonImportResolution {
    const fn relation(self) -> &'static str {
        match self {
            Self::SymbolImport => "imports",
            Self::SubmoduleImport => "imports_from",
            Self::ModuleReExport => "re_exports",
        }
    }

    const fn context(self) -> &'static str {
        match self {
            Self::SymbolImport => "import",
            Self::SubmoduleImport => "submodule_import",
            Self::ModuleReExport => "export",
        }
    }

    const fn rule(self) -> &'static str {
        match self {
            Self::SymbolImport => "python-symbol-import-resolution",
            Self::SubmoduleImport => "python-submodule-import-resolution",
            Self::ModuleReExport => "python-module-re-export-resolution",
        }
    }
}

fn python_import_edge(
    source: &str,
    target: &str,
    resolution: PythonImportResolution,
    source_file: &str,
    imported: &PythonImport,
) -> EdgeRecord {
    let mut attributes = Map::from_iter([
        (
            "relation".to_owned(),
            Value::String(resolution.relation().to_owned()),
        ),
        (
            "context".to_owned(),
            Value::String(resolution.context().to_owned()),
        ),
        (
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        ),
        ("language".to_owned(), Value::String("python".to_owned())),
        (
            "extractor".to_owned(),
            Value::String("compass.resolve.python-imports".to_owned()),
        ),
        ("_origin".to_owned(), Value::String("convention".to_owned())),
        (
            "rule".to_owned(),
            Value::String(resolution.rule().to_owned()),
        ),
        (
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(python_import_occurrence_rule(resolution, imported)),
        ),
        ("module".to_owned(), Value::String(imported.module.clone())),
        (
            "imported_name".to_owned(),
            Value::String(imported.imported.clone()),
        ),
        (
            "local_name".to_owned(),
            Value::String(imported.local.clone()),
        ),
        ("weight".to_owned(), Value::from(1.0)),
    ]);
    insert_python_import_anchor(&mut attributes, source_file, imported);
    EdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes,
    }
}

fn insert_python_import_anchor(
    attributes: &mut Map<String, Value>,
    source_file: &str,
    imported: &PythonImport,
) {
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(format!("L{}", imported.start_line)),
    );
    attributes.insert("start_byte".to_owned(), Value::from(imported.start_byte));
    attributes.insert("end_byte".to_owned(), Value::from(imported.end_byte));
    attributes.insert("line_start".to_owned(), Value::from(imported.start_line));
    attributes.insert("line_end".to_owned(), Value::from(imported.end_line));
    attributes.insert(
        "column_start".to_owned(),
        Value::from(imported.start_column),
    );
    attributes.insert("column_end".to_owned(), Value::from(imported.end_column));
}

fn python_import_occurrence_rule(
    resolution: PythonImportResolution,
    imported: &PythonImport,
) -> String {
    format!(
        "{}@{}:{}:{}:{}:{}",
        resolution.rule(),
        imported.start_byte,
        imported.end_byte,
        imported.occurrence,
        imported.imported,
        imported.local
    )
}

fn python_module_file(
    caller: &Path,
    root: &Path,
    module: &str,
    submodule: Option<&str>,
    file_nodes: &HashMap<String, String>,
) -> Option<String> {
    let depth = module
        .len()
        .saturating_sub(module.trim_start_matches('.').len());
    let bare = module.trim_start_matches('.');
    let mut base = if depth == 0 {
        root.to_path_buf()
    } else {
        let mut base = caller
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for _ in 1..depth {
            base = base.parent().unwrap_or(&base).to_path_buf();
        }
        base
    };
    if !bare.is_empty() {
        base.push(bare.replace('.', "/"));
    }
    if let Some(submodule) = submodule.filter(|value| !value.is_empty()) {
        base.push(submodule.replace('.', "/"));
    }
    let candidates = [base.with_extension("py"), base.join("__init__.py")];
    candidates.iter().find_map(|candidate| {
        file_nodes
            .get(&normalize_path(&candidate.to_string_lossy()))
            .cloned()
    })
}

#[cfg(test)]
fn python_import_aliases(source: &str) -> HashMap<String, (String, String)> {
    python_symbol_imports(source)
        .into_iter()
        .map(|import| (import.local, (import.module, import.imported)))
        .collect()
}

struct PythonImport {
    module: String,
    imported: String,
    local: String,
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    end_line: usize,
    start_column: usize,
    end_column: usize,
    occurrence: usize,
}

struct PythonImportItem {
    occurrence: usize,
    imported: String,
    local: String,
}

fn python_symbol_imports(source: &str) -> Vec<PythonImport> {
    let masked = mask_python_non_code(source);
    let lines = masked.lines().collect::<Vec<_>>();
    let original_lines = source.lines().collect::<Vec<_>>();
    let line_starts = std::iter::once(0)
        .chain(
            masked
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start_matches(is_python_inline_whitespace);
        if !starts_python_from_import(trimmed) {
            index += 1;
            continue;
        }
        let start_line = index + 1;
        let start_column = line.len() - trimmed.len();
        let start_byte = line_starts[index] + start_column;
        let mut statement = String::new();
        let mut depth = 0_usize;
        let mut malformed = false;
        loop {
            let physical = if index + 1 == start_line {
                lines[index].trim_start_matches(is_python_inline_whitespace)
            } else {
                lines[index].trim_matches(is_python_inline_whitespace)
            };
            let trimmed_end = physical.trim_end_matches(is_python_inline_whitespace);
            let continued = trimmed_end.ends_with('\\');
            if continued
                && original_lines
                    .get(index)
                    .is_none_or(|original| !original.ends_with('\\'))
            {
                malformed = true;
                break;
            }
            let logical = if continued {
                trimmed_end.strip_suffix('\\').unwrap_or(trimmed_end)
            } else {
                trimmed_end
            };
            if !statement.is_empty() {
                statement.push(' ');
            }
            statement.push_str(logical.trim_matches(is_python_inline_whitespace));
            for character in logical.chars() {
                match character {
                    '(' => depth = depth.saturating_add(1),
                    ')' if depth == 0 => {
                        malformed = true;
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            if malformed || (depth == 0 && !continued) {
                break;
            }
            if index + 1 >= lines.len()
                || starts_python_from_import(
                    lines[index + 1].trim_start_matches(is_python_inline_whitespace),
                )
            {
                malformed = true;
                break;
            }
            index += 1;
        }
        let end_line = index + 1;
        let end_column = lines[index]
            .trim_end_matches(is_python_inline_whitespace)
            .len();
        let end_byte = line_starts[index] + end_column;
        if !malformed
            && depth == 0
            && let Some((module, items)) = parse_python_from_import(&statement)
        {
            for item in items {
                output.push(PythonImport {
                    module: module.clone(),
                    imported: item.imported,
                    local: item.local,
                    start_byte,
                    end_byte,
                    start_line,
                    end_line,
                    start_column,
                    end_column,
                    occurrence: item.occurrence,
                });
            }
        }
        index += 1;
    }
    output
}

fn starts_python_from_import(statement: &str) -> bool {
    statement
        .strip_prefix("from")
        .and_then(|rest| rest.chars().next())
        .is_some_and(is_python_inline_whitespace)
}

fn parse_python_from_import(statement: &str) -> Option<(String, Vec<PythonImportItem>)> {
    let rest = statement.strip_prefix("from")?;
    let rest = rest
        .strip_prefix(is_python_inline_whitespace)?
        .trim_start_matches(is_python_inline_whitespace);
    let module_end = rest.find(is_python_inline_whitespace)?;
    let module = &rest[..module_end];
    if !valid_python_module(module) {
        return None;
    }
    let imports = rest[module_end..]
        .trim_start_matches(is_python_inline_whitespace)
        .strip_prefix("import")?;
    if imports
        .chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_alphanumeric())
    {
        return None;
    }
    let imports = imports.trim_matches(is_python_inline_whitespace);
    let (imports, parenthesized) = if let Some(imports) = imports.strip_prefix('(') {
        (
            imports
                .strip_suffix(')')?
                .trim_matches(is_python_inline_whitespace),
            true,
        )
    } else {
        (imports, false)
    };
    if imports.is_empty() || imports.contains(['(', ')']) {
        return None;
    }
    let pieces = imports.split(',').collect::<Vec<_>>();
    if !parenthesized
        && pieces
            .last()
            .is_some_and(|item| item.trim_matches(is_python_inline_whitespace).is_empty())
    {
        return None;
    }
    let wildcard_count = pieces
        .iter()
        .filter(|item| item.trim_matches(is_python_inline_whitespace) == "*")
        .count();
    if wildcard_count > 0 {
        return (!parenthesized && pieces.len() == 1).then(|| (module.to_owned(), Vec::new()));
    }
    let piece_count = pieces.len();
    let mut output = Vec::new();
    for (occurrence, item) in pieces.into_iter().enumerate() {
        let item = item.trim_matches(is_python_inline_whitespace);
        if item.is_empty() {
            if parenthesized && occurrence + 1 == piece_count {
                continue;
            }
            return None;
        }
        let (imported, local) = parse_python_import_item(item)?;
        if !valid_python_identifier(imported) || !valid_python_identifier(local) {
            return None;
        }
        output.push(PythonImportItem {
            occurrence,
            imported: imported.to_owned(),
            local: local.to_owned(),
        });
    }
    Some((module.to_owned(), output))
}

fn parse_python_import_item(item: &str) -> Option<(&str, &str)> {
    let imported_end = item.find(is_python_inline_whitespace).unwrap_or(item.len());
    let imported = &item[..imported_end];
    let alias = &item[imported_end..];
    if alias.is_empty() {
        return Some((imported, imported));
    }
    let alias = alias
        .trim_start_matches(is_python_inline_whitespace)
        .strip_prefix("as")?
        .strip_prefix(is_python_inline_whitespace)?
        .trim_start_matches(is_python_inline_whitespace);
    Some((imported, alias))
}

const fn is_python_inline_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\u{000C}')
}

fn valid_python_module(module: &str) -> bool {
    let bare = module.trim_start_matches('.');
    (!bare.is_empty() || module.starts_with('.'))
        && (bare.is_empty() || bare.split('.').all(valid_python_identifier))
}

fn valid_python_identifier(identifier: &str) -> bool {
    let mut characters = identifier.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
        && !is_python_hard_keyword(identifier)
}

fn is_python_hard_keyword(identifier: &str) -> bool {
    matches!(
        identifier,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

fn mask_python_non_code(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let original = source.as_bytes();
    let mut index = 0_usize;
    let mut quote = None::<(u8, bool)>;
    let mut escaped = false;
    while index < original.len() {
        if let Some((delimiter, triple)) = quote {
            if original[index] == b'\n' || original[index] == b'\r' {
                escaped = false;
                index += 1;
                continue;
            }
            if escaped {
                bytes[index] = b' ';
                escaped = false;
                index += 1;
                continue;
            }
            if original[index] == b'\\' {
                bytes[index] = b' ';
                escaped = true;
                index += 1;
                continue;
            }
            let closes = if triple {
                original.get(index..index + 3) == Some(&[delimiter, delimiter, delimiter])
            } else {
                original[index] == delimiter
            };
            if closes {
                let width = if triple { 3 } else { 1 };
                bytes[index..index + width].fill(b' ');
                index += width;
                quote = None;
                continue;
            }
            bytes[index] = b' ';
            index += 1;
            continue;
        }
        if original[index] == b'#' {
            while index < original.len() && !matches!(original[index], b'\n' | b'\r') {
                bytes[index] = b' ';
                index += 1;
            }
            continue;
        }
        if matches!(original[index], b'\'' | b'"') {
            let delimiter = original[index];
            let triple = original.get(index..index + 3) == Some(&[delimiter, delimiter, delimiter]);
            let width = if triple { 3 } else { 1 };
            bytes[index..index + width].fill(b' ');
            index += width;
            quote = Some((delimiter, triple));
            continue;
        }
        index += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned())
}

fn python_definition_candidates(
    caller: &Path,
    root: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<String, Vec<(String, String)>>,
    allow_module_tail: bool,
) -> Vec<String> {
    let bare_module = module.trim_start_matches('.');
    let module_tail = bare_module.rsplit('.').next().unwrap_or_default();
    let relative_candidate = if module.starts_with('.') {
        let depth = module
            .len()
            .saturating_sub(module.trim_start_matches('.').len());
        let mut base = caller.parent().unwrap_or_else(|| Path::new("."));
        for _ in 1..depth {
            base = base.parent().unwrap_or(base);
        }
        base.join(format!("{}.py", bare_module.replace('.', "/")))
    } else {
        root.join(format!("{}.py", bare_module.replace('.', "/")))
    };
    let mut output = Vec::new();
    for (source, id) in definitions.get(imported).into_iter().flatten() {
        let source_path = Path::new(source);
        let exact_relative = source_path == relative_candidate;
        let matching_stem =
            source_path.file_stem().and_then(|value| value.to_str()) == Some(module_tail);
        if exact_relative
            || (!module.starts_with('.')
                && (!module.contains('.') || allow_module_tail)
                && matching_stem)
        {
            output.push(id.clone());
        }
    }
    output
}

fn python_resolved_definition_candidates(
    caller: &Path,
    root: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<String, Vec<(String, String)>>,
    sources: &HashMap<String, (&str, &str)>,
    allow_module_tail: bool,
) -> Vec<String> {
    let mut caller = caller.to_path_buf();
    let mut module = module.to_owned();
    let mut imported = imported.to_owned();
    let mut seen = HashSet::new();
    for _ in 0..16 {
        let direct = python_definition_candidates(
            &caller,
            root,
            &module,
            &imported,
            definitions,
            allow_module_tail,
        );
        if !direct.is_empty() {
            return direct;
        }
        let Some((target_source, target_text)) =
            python_module_source(&caller, root, &module, sources)
        else {
            break;
        };
        let target_key = normalize_path(target_source);
        let in_module = definitions
            .get(&imported)
            .into_iter()
            .flatten()
            .filter(|(source_file, _)| normalize_path(source_file) == target_key)
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>();
        if !in_module.is_empty() {
            return in_module;
        }
        if Path::new(target_source)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("__init__.py")
            || !seen.insert((target_source.to_owned(), imported.clone()))
        {
            break;
        }
        let Some(reexport) = python_symbol_imports(target_text)
            .into_iter()
            .find(|candidate| candidate.local == imported)
        else {
            break;
        };
        caller = Path::new(target_source).to_path_buf();
        module = reexport.module;
        imported = reexport.imported;
    }
    Vec::new()
}

fn python_module_source<'a>(
    caller: &Path,
    root: &Path,
    module: &str,
    sources: &'a HashMap<String, (&'a str, &'a str)>,
) -> Option<(&'a str, &'a str)> {
    let depth = module
        .len()
        .saturating_sub(module.trim_start_matches('.').len());
    let bare = module.trim_start_matches('.');
    let mut base = if depth == 0 {
        root.to_path_buf()
    } else {
        let mut base = caller
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for _ in 1..depth {
            base = base.parent().unwrap_or(&base).to_path_buf();
        }
        base
    };
    if !bare.is_empty() {
        base.push(bare.replace('.', "/"));
    }
    [base.with_extension("py"), base.join("__init__.py")]
        .iter()
        .find_map(|candidate| {
            sources
                .get(&normalize_path(&candidate.to_string_lossy()))
                .copied()
        })
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
        && Path::new(source)
            .file_name()
            .and_then(|value| value.to_str())
            == Some(node.label())
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
    fn python_import_parser_handles_aliases_multiline_comments_and_wildcards() {
        let imports = python_symbol_imports(
            "from pkg.api import (\n  Widget as LocalWidget,\n  helper, # kept\n  *,\n)\nfrom invalid\nimport os\n",
        );
        assert!(imports.is_empty());
        let imports = python_symbol_imports(
            "from pkg.api import (\n  Widget as LocalWidget,\n  helper, # kept\n)\nfrom pkg.api import *\n",
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module, "pkg.api");
        assert_eq!(imports[0].imported, "Widget");
        assert_eq!(imports[0].local, "LocalWidget");
        assert_eq!(imports[0].start_line, 1);
        assert_eq!(imports[1].imported, "helper");
        let aliases = python_import_aliases("from lib import run as execute");
        assert_eq!(
            aliases.get("execute"),
            Some(&("lib".to_owned(), "run".to_owned()))
        );
    }

    #[test]
    fn python_definition_matching_respects_relative_and_module_tail_rules() {
        let definitions = HashMap::from([(
            "Widget".to_owned(),
            vec![
                ("app/models.py".to_owned(), "models-widget".to_owned()),
                ("other/models.py".to_owned(), "other-widget".to_owned()),
            ],
        )]);
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                Path::new("."),
                ".models",
                "Widget",
                &definitions,
                false,
            ),
            vec!["models-widget"]
        );
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                Path::new("."),
                "pkg.models",
                "Widget",
                &definitions,
                false,
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                Path::new("."),
                "pkg.models",
                "Widget",
                &definitions,
                true,
            )
            .len(),
            2
        );
        let qualified = HashMap::from([(
            "edge_data".to_owned(),
            vec![(
                "/repo/compass/build.py".to_owned(),
                "compass_build_edge_data".to_owned(),
            )],
        )]);
        assert_eq!(
            python_definition_candidates(
                Path::new("/repo/compass/analyze.py"),
                Path::new("/repo"),
                "compass.build",
                "edge_data",
                &qualified,
                false,
            ),
            vec!["compass_build_edge_data"]
        );
    }

    #[test]
    fn python_package_initializers_reexport_imported_symbols() {
        let root = Path::new("/repo");
        let caller = "/repo/caller.py";
        let init = "/repo/pkg/__init__.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller.py", caller, "module"),
                node("pkg_init", "__init__.py", init, "module"),
                node("pkg_mod", "mod.py", module, "module"),
                node("pkg_mod_fn", "fn()", module, "function"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (caller.to_owned(), "from pkg import fn\n".to_owned()),
            (init.to_owned(), "from .mod import fn\n".to_owned()),
            (module.to_owned(), "def fn():\n    return 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "pkg_init"
                && edge.target == "pkg_mod"
                && relation(edge) == "re_exports"
                && edge.string("context") == "export"
        }));
        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "caller"
                && edge.target == "pkg_mod_fn"
                && relation(edge) == "imports"
                && edge.string("context") == "import"
        }));
    }

    #[test]
    fn python_package_form_submodule_imports_target_the_submodule_file() {
        let root = Path::new("/repo");
        let caller = "/repo/caller.py";
        let init = "/repo/pkg/__init__.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller.py", caller, "module"),
                node("pkg_init", "__init__.py", init, "module"),
                node("pkg_mod", "mod.py", module, "module"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (caller.to_owned(), "from pkg import mod\n".to_owned()),
            (init.to_owned(), String::new()),
            (module.to_owned(), "VALUE = 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "caller"
                && edge.target == "pkg_mod"
                && relation(edge) == "imports_from"
                && edge.string("context") == "submodule_import"
        }));
    }

    #[test]
    fn python_imports_inside_multiline_strings_are_ignored() {
        let root = Path::new("/repo");
        let caller = "/repo/hooks.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("hooks", "hooks.py", caller, "module"),
                node("pkg_mod", "mod.py", module, "module"),
                node("pkg_mod_fn", "fn()", module, "function"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (
                caller.to_owned(),
                "SCRIPT = \"\"\"\\\nfrom pkg.mod import fn\nfn()\n\"\"\"\n".to_owned(),
            ),
            (module.to_owned(), "def fn():\n    return 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(!extraction.edges.iter().any(|edge| {
            edge.source == "hooks" && edge.target == "pkg_mod_fn" && relation(edge) == "imports"
        }));
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
    fn resolve_adds_import_guided_calls_without_unused_class_edges() {
        let file_a = make_id(&["app/a.py"]);
        let file_b = make_id(&["app/b.py"]);
        let mut import_call = raw("caller", "run", "app/a.py");
        import_call
            .extensions
            .insert("symbol_import_use".to_owned(), Value::Bool(true));
        let extraction = Extraction {
            nodes: vec![
                node(&file_a, "a.py", "app/a.py", "file"),
                node(&file_b, "b.py", "app/b.py", "file"),
                node("caller", "caller()", "app/a.py", "function"),
                node("local-class", "Local", "app/a.py", "class"),
                node("helper", "helper()", "app/b.py", "function"),
                node("widget", "Widget", "app/b.py", "class"),
            ],
            raw_calls: Some(vec![import_call]),
            ..Extraction::default()
        };
        let sources = HashMap::from([(
            "app/a.py".to_owned(),
            "from .b import helper as run\nfrom .b import Widget\nrun()\n".to_owned(),
        )]);
        let resolved = resolve(&[extraction], &sources);
        assert!(resolved.edges.iter().any(|candidate| {
            candidate.source == "caller"
                && candidate.target == "helper"
                && relation(candidate) == "calls"
                && candidate.string("confidence") == "EXTRACTED"
        }));
        assert!(resolved.edges.iter().all(|candidate| {
            candidate.source != "local-class"
                || candidate.target != "widget"
                || relation(candidate) != "uses"
        }));
        assert!(resolved.edges.iter().any(|candidate| {
            candidate.source == file_a
                && candidate.target == "helper"
                && relation(candidate) == "imports"
        }));
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

        merge_decl_def_classes(&mut extractions);

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
    fn unique_stub_rewiring_retargets_edges_and_removes_unreferenced_stubs() {
        let mut extraction = Extraction {
            nodes: vec![
                node("type", "Widget", "src/widget.py", "class"),
                node("stub", "Widget", "", "stub"),
                node("func", "run()", "src/run.py", "function"),
                node("func-stub", "run()", "", "stub"),
            ],
            edges: vec![
                edge("stub", "func-stub", "uses", "src/use.py"),
                edge("type", "stub", "inherits", "src/widget.py"),
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
            EndpointRewriteRule::PythonImportedTypeResolution,
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
}
