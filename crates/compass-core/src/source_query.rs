//! Coherent, graph-bound publication of query-only native source evidence.
use crate::CoreError;
use compass_languages::{
    Extraction,
    source_query::{self as raw, SourceQueryExtraction},
};
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, SourceAnchor};
use compass_model::source_query::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Default)]
pub(crate) struct Inputs {
    files: BTreeMap<String, SourceQueryExtraction>,
    aggregate_truncated: bool,
}
fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidBuildState(message.into())
}

fn inventory_mismatch(path: &str, extracted: u64, published: u64) -> CoreError {
    // Bound before escaping: graph paths are validated but have no length cap.
    let mut characters = path.chars();
    let context: String = characters.by_ref().take(128).collect();
    let suffix = if characters.next().is_some() {
        "..."
    } else {
        ""
    };
    invalid(format!(
        "source-query extraction digest does not match published file inventory for {context:?}{suffix}: extracted {extracted} bytes, published {published} bytes; run compass update --force"
    ))
}

fn count_site_text<'a>(
    total: &mut usize,
    sites: impl Iterator<Item = &'a raw::Site>,
    site_cap: usize,
    byte_cap: usize,
) -> Result<(), CoreError> {
    for site in sites {
        *total = total.saturating_add(site.text.len());
        if site.text.len() > site_cap || *total > byte_cap {
            return Err(invalid(
                "oversized cached source-query text accounting; run compass update --force",
            ));
        }
    }
    Ok(())
}

/// Validate counters before any channel can be dropped by aggregate pruning.
fn validate_accounting(file: &SourceQueryExtraction) -> Result<(), CoreError> {
    let mut generic = 0_usize;
    let mut parameters = 0_usize;
    let mut code = 0_usize;
    for scope in &file.projection.scopes {
        if scope.names.len() > 256
            || scope.names.values().any(|sites| sites.len() > 4)
            || scope.comments.len() > 8
            || scope.code_windows.len() > 16
            || scope.parameter_bindings.sites.len() > SOURCE_QUERY_MAX_PARAMETER_SITES
        {
            return Err(invalid(
                "oversized cached source-query scope accounting; run compass update --force",
            ));
        }
        count_site_text(
            &mut generic,
            scope.names.values().flatten(),
            256,
            SOURCE_QUERY_MAX_FILE_TEXT_BYTES,
        )?;
        count_site_text(
            &mut generic,
            scope.comments.iter(),
            4096,
            SOURCE_QUERY_MAX_FILE_TEXT_BYTES,
        )?;
        count_site_text(
            &mut parameters,
            scope.parameter_bindings.sites.iter(),
            256,
            SOURCE_QUERY_MAX_PARAMETER_FILE_BYTES,
        )?;
        count_site_text(&mut code, scope.code_windows.iter(), 512, 256 * 1024)?;
    }
    if generic != file.projection.retained_bytes
        || parameters != file.projection.retained_parameter_bytes
        || code != file.projection.retained_code_bytes
        || generic.saturating_add(parameters) > SOURCE_QUERY_MAX_FILE_TEXT_BYTES
    {
        return Err(invalid(
            "invalid cached source-query text accounting; run compass update --force",
        ));
    }
    Ok(())
}

pub(crate) fn collect(extractions: &mut [Extraction]) -> Result<Inputs, CoreError> {
    let mut result = Inputs::default();
    let mut bytes = 0_usize;
    let mut code_bytes = 0_usize;
    for extraction in extractions {
        let Some(value) = extraction.extensions.remove(raw::EXTRACTION_EXTENSION) else {
            continue;
        };
        let mut file: SourceQueryExtraction = serde_json::from_value(value).map_err(|e| {
            invalid(format!(
                "invalid cached source-query evidence: {e}; run compass update --force"
            ))
        })?;
        if file.schema != raw::EXTRACTION_SCHEMA
            || file.profile != SOURCE_QUERY_PROFILE
            || file.projection.scopes.len() > 512
            || file.projection.visits > 200_000
            || file
                .projection
                .retained_bytes
                .saturating_add(file.projection.retained_parameter_bytes)
                > SOURCE_QUERY_MAX_FILE_TEXT_BYTES
            || file.projection.scopes.iter().any(|scope| {
                scope.parameter_bindings.sites.len() > SOURCE_QUERY_MAX_PARAMETER_SITES
            })
        {
            return Err(invalid(
                "unsupported or oversized source-query extraction; run compass update --force",
            ));
        }
        validate_accounting(&file)?;
        if bytes.saturating_add(file.projection.retained_bytes) > SOURCE_QUERY_MAX_TEXT_BYTES
            || result.files.len() >= SOURCE_QUERY_MAX_OWNERS
        {
            file.projection.scopes.clear();
            file.projection.truncated = true;
            result.aggregate_truncated = true;
        } else {
            bytes += file.projection.retained_bytes;
        }
        if file.projection.scopes.is_empty() {
            file.projection.retained_code_bytes = 0;
            file.projection.retained_parameter_bytes = 0;
        }
        if code_bytes.saturating_add(file.projection.retained_code_bytes)
            > SOURCE_QUERY_MAX_CODE_BYTES
        {
            for scope in &mut file.projection.scopes {
                scope.omitted_code_windows = scope
                    .omitted_code_windows
                    .saturating_add(scope.code_windows.len() as u64);
                scope.code_windows.clear();
            }
            file.projection.retained_code_bytes = 0;
        } else {
            code_bytes += file.projection.retained_code_bytes;
        }
        if result.files.len() < SOURCE_QUERY_MAX_OWNERS
            && result
                .files
                .insert(file.source_file.clone(), file)
                .is_some()
        {
            return Err(invalid("duplicate source-query file identity"));
        }
    }
    // Generic lexical evidence receives its original aggregate budget first.
    let mut spare = SOURCE_QUERY_MAX_TEXT_BYTES.saturating_sub(bytes);
    for file in result.files.values_mut() {
        let actual = file
            .projection
            .scopes
            .iter()
            .flat_map(|scope| &scope.parameter_bindings.sites)
            .map(|site| site.text.len())
            .sum::<usize>();
        if actual != file.projection.retained_parameter_bytes
            || actual > SOURCE_QUERY_MAX_PARAMETER_FILE_BYTES
        {
            return Err(invalid("invalid cached parameter text accounting"));
        }
        for scope in &mut file.projection.scopes {
            scope.parameter_bindings.sites.retain(|site| {
                if site.text.len() > spare {
                    scope.parameter_bindings.status = raw::ParameterBindingStatus::Partial;
                    scope.parameter_bindings.omitted_sites =
                        scope.parameter_bindings.omitted_sites.saturating_add(1);
                    scope
                        .parameter_bindings
                        .reasons
                        .insert(raw::ParameterBindingOmission::AggregateLimit);
                    false
                } else {
                    spare -= site.text.len();
                    true
                }
            });
        }
        file.projection.retained_parameter_bytes = file
            .projection
            .scopes
            .iter()
            .flat_map(|scope| &scope.parameter_bindings.sites)
            .map(|site| site.text.len())
            .sum();
    }
    Ok(result)
}

/// A bounded digest pass binds the old sidecar before a neutral graph rewrite.
fn graph_digest(path: &Path) -> Option<String> {
    let mut input = File::open(path)
        .ok()?
        .take(compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut bytes = 0_u64;
    loop {
        let n = input.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        };
        bytes = bytes.checked_add(n as u64)?;
        if bytes > compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES {
            return None;
        };
        hasher.update(&buffer[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

pub(crate) fn reusable(output: &Path) -> Option<SourceQueryArtifact> {
    let bytes =
        compass_files::read_bytes_bounded(&output.join(SOURCE_QUERY_FILE), SOURCE_QUERY_MAX_BYTES)
            .ok()?;
    let artifact: SourceQueryArtifact = serde_json::from_slice(&bytes).ok()?;
    artifact.validate().ok()?;
    if artifact.aggregate_truncated
        || artifact.owners.iter().any(|owner| {
            owner.omitted_code_windows > 0
                || owner.omitted_nested_contexts > 0
                || owner.parameter_bindings.as_ref().is_some_and(|bindings| {
                    bindings
                        .reasons
                        .contains(&ParameterBindingOmission::AggregateLimit)
                })
                || owner
                    .nested_contexts
                    .iter()
                    .any(|context| context.omitted_code_windows > 0)
        })
        || graph_digest(&output.join("graph.json"))?.as_str() != artifact.graph_digest
    {
        return None;
    };
    Some(artifact)
}

/// The clean collector accounts unpublished lexical scopes too. A retained
/// artifact cannot recover those counters, so bound each unchanged file by its
/// full native allowance. If shared role pruning is possible, the caller must
/// use the complete cached/fresh extraction universe instead of mixing owners.
/// Code accounting likewise includes unpublished scopes: each unchanged file
/// receives its full 512-scope/16-window and 256 KiB native allowances.
pub(crate) fn can_reuse_with_inputs(prior: &SourceQueryArtifact, inputs: &Inputs) -> bool {
    if prior.profile != SOURCE_QUERY_PROFILE
        || prior.aggregate_truncated
        || inputs.aggregate_truncated
        || prior.owners.iter().any(|owner| {
            owner.omitted_code_windows > 0
                || owner.omitted_nested_contexts > 0
                || owner
                    .nested_contexts
                    .iter()
                    .any(|context| context.omitted_code_windows > 0)
                || owner.parameter_bindings.as_ref().is_some_and(|bindings| {
                    bindings
                        .reasons
                        .contains(&ParameterBindingOmission::AggregateLimit)
                })
        })
    {
        return false;
    }
    let mut bound = 0_usize;
    let mut sites = 0_usize;
    let mut code_sites = 0_usize;
    let mut code_bytes = 0_usize;
    let mut prior_file_bytes = BTreeMap::<&str, usize>::new();
    for owner in &prior.owners {
        let parameter_sites = owner
            .parameter_bindings
            .as_ref()
            .map_or(&[][..], |bindings| bindings.sites.as_slice());
        let bytes = owner
            .identifiers
            .iter()
            .chain(&owner.comments)
            .chain(parameter_sites)
            .fold(0_usize, |sum, site| sum.saturating_add(site.text.len()));
        let file_bytes = prior_file_bytes.entry(&owner.source.file).or_default();
        *file_bytes = file_bytes.saturating_add(bytes);
        if *file_bytes > SOURCE_QUERY_MAX_FILE_TEXT_BYTES {
            return false;
        }
        if !inputs.files.contains_key(&owner.source.file) {
            sites = sites
                .saturating_add(owner.identifiers.len())
                .saturating_add(owner.comments.len())
                .saturating_add(parameter_sites.len());
        }
    }
    for file in &prior.files {
        if !inputs.files.contains_key(&file.file) {
            bound = bound.saturating_add(SOURCE_QUERY_MAX_FILE_TEXT_BYTES);
            code_sites = code_sites.saturating_add(512 * 16);
            code_bytes = code_bytes.saturating_add(256 * 1024);
        }
    }
    for file in inputs.files.values() {
        if file.projection.scopes.iter().any(|scope| {
            scope.omitted_code_windows > 0
                || scope
                    .parameter_bindings
                    .reasons
                    .contains(&raw::ParameterBindingOmission::AggregateLimit)
        }) {
            return false;
        }
        bound = bound
            .saturating_add(file.projection.retained_bytes)
            .saturating_add(file.projection.retained_parameter_bytes);
        for scope in &file.projection.scopes {
            code_sites = code_sites.saturating_add(scope.code_windows.len());
            code_bytes = scope.code_windows.iter().fold(code_bytes, |sum, window| {
                sum.saturating_add(window.text.len())
            });
            sites = sites
                .saturating_add(scope.comments.len())
                .saturating_add(scope.parameter_bindings.sites.len());
            for occurrences in scope.names.values() {
                sites = sites.saturating_add(occurrences.len());
            }
        }
    }
    bound <= SOURCE_QUERY_MAX_TEXT_BYTES
        && sites <= SOURCE_QUERY_MAX_SITES
        && code_sites <= SOURCE_QUERY_MAX_CODE_WINDOWS
        && code_bytes <= SOURCE_QUERY_MAX_CODE_BYTES
}

#[cfg(test)]
mod tests;

fn parameter_bindings(file: &str, scope: &raw::Scope) -> Result<ParameterBindings, CoreError> {
    use raw::ParameterBindingOmission as R;
    let bindings = &scope.parameter_bindings;
    if bindings.sites.len() > SOURCE_QUERY_MAX_PARAMETER_SITES || bindings.omitted_sites > 200_000 {
        return Err(invalid("oversized parameter binding evidence"));
    }
    let mut sites = Vec::new();
    for raw_site in &bindings.sites {
        if raw_site.start < scope.callable_start || raw_site.end > scope.callable_end {
            return Err(invalid(
                "parameter binding escapes its exact callable syntax",
            ));
        }
        sites.push(site(file, raw_site)?);
    }
    Ok(ParameterBindings {
        sites,
        status: match bindings.status {
            raw::ParameterBindingStatus::Complete => ParameterBindingStatus::Complete,
            raw::ParameterBindingStatus::Partial => ParameterBindingStatus::Partial,
            raw::ParameterBindingStatus::Unsupported => ParameterBindingStatus::Unsupported,
        },
        omitted_sites: bindings.omitted_sites,
        reasons: bindings
            .reasons
            .iter()
            .map(|reason| match reason {
                R::UnsupportedPattern => ParameterBindingOmission::UnsupportedPattern,
                R::UnsupportedReceiver => ParameterBindingOmission::UnsupportedReceiver,
                R::UnsupportedSpelling => ParameterBindingOmission::UnsupportedSpelling,
                R::SyntaxRecovery => ParameterBindingOmission::SyntaxRecovery,
                R::TraversalLimit => ParameterBindingOmission::TraversalLimit,
                R::SiteLimit => ParameterBindingOmission::SiteLimit,
                R::TextLimit => ParameterBindingOmission::TextLimit,
                R::AggregateLimit => ParameterBindingOmission::AggregateLimit,
            })
            .collect(),
    })
}

fn enclosed(owner: &SourceAnchor, site: &SourceAnchor) -> bool {
    owner.file == site.file
        && owner.start_byte <= site.start_byte
        && site.end_byte <= owner.end_byte
        && (owner.start_line, owner.start_column) <= (site.start_line, site.start_column)
        && (site.end_line, site.end_column) <= (owner.end_line, owner.end_column)
}
fn exact(node: &NodeRecord) -> bool {
    let Some(source) = &node.source else {
        return false;
    };
    !node.evidence.is_empty()
        && node.evidence.iter().all(|p| {
            p.origin != EvidenceOrigin::Heuristic
                && p.confidence == EvidenceConfidence::Exact
                && p.validate().is_ok()
        })
        && node.evidence.iter().any(|p| {
            p.origin == EvidenceOrigin::Ast && p.anchors.iter().any(|a| enclosed(source, a))
        })
}
fn site(file: &str, site: &raw::Site) -> Result<SourceQuerySite, CoreError> {
    Ok(SourceQuerySite {
        text: site.text.clone(),
        source: SourceAnchor {
            file: file.to_owned(),
            start_byte: site.start as u64,
            end_byte: site.end as u64,
            start_line: u32::try_from(site.start_point.row)
                .ok()
                .and_then(|r| r.checked_add(1))
                .ok_or_else(|| invalid("source-query line overflow"))?,
            end_line: u32::try_from(site.end_point.row)
                .ok()
                .and_then(|r| r.checked_add(1))
                .ok_or_else(|| invalid("source-query line overflow"))?,
            start_column: u32::try_from(site.start_point.column)
                .map_err(|_| invalid("source-query column overflow"))?,
            end_column: u32::try_from(site.end_point.column)
                .map_err(|_| invalid("source-query column overflow"))?,
        },
    })
}

pub(crate) fn publish(
    output: &Path,
    graph: &GraphDocument,
    digest: &str,
    inputs: &Inputs,
    prior: Option<SourceQueryArtifact>,
) -> Result<(), CoreError> {
    let files = graph
        .graph
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect::<BTreeMap<_, _>>();
    let nodes = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect::<BTreeMap<_, _>>();
    let mut ranges = BTreeMap::<(&str, u64, u64), Vec<&NodeRecord>>::new();
    for node in &graph.nodes {
        if node.kind.is_callable()
            && let Some(source) = &node.source
        {
            ranges
                .entry((&source.file, source.start_byte, source.end_byte))
                .or_default()
                .push(node);
        }
    }
    let mut result = SourceQueryArtifact::empty(digest.to_owned());
    result.aggregate_truncated = inputs.aggregate_truncated;
    if let Some(prior) = prior {
        if prior.profile != SOURCE_QUERY_PROFILE
            && prior.files.iter().any(|file| {
                !inputs.files.contains_key(&file.file) && files.contains_key(file.file.as_str())
            })
        {
            return Err(invalid(
                "older source-query evidence uses a different projection profile; run compass update --force",
            ));
        }
        if prior.files.iter().any(|file| {
            !inputs.files.contains_key(&file.file) && files.contains_key(file.file.as_str())
        }) && !can_reuse_with_inputs(&prior, inputs)
        {
            return Err(invalid(
                "mixed source-query evidence may lose shared code or role evidence; run compass update --force",
            ));
        }
        for file in prior.files {
            if !inputs.files.contains_key(&file.file)
                && files.get(file.file.as_str()).is_some_and(|f| {
                    f.content_digest == file.source_digest && f.byte_size == file.byte_size
                })
            {
                result.files.push(file);
            }
        }
        let kept = result
            .files
            .iter()
            .map(|f| f.file.as_str())
            .collect::<BTreeSet<_>>();
        for owner in prior.owners {
            if kept.contains(owner.source.file.as_str()) {
                if !nodes.get(owner.node_id.as_str()).is_some_and(|node| {
                    node.kind.is_callable()
                        && node.source.as_ref() == Some(&owner.source)
                        && exact(node)
                }) {
                    return Err(invalid(
                        "prior source-query owner does not match an exact-AST callable; run compass update --force",
                    ));
                }
                result.owners.push(owner);
            }
        }
    }

    // Role evidence is allocated only after all original lexical channels.
    let mut pending_parameters = BTreeMap::new();
    for owner in &mut result.owners {
        let bindings = owner
            .parameter_bindings
            .take()
            .ok_or_else(|| invalid("current source owner lacks parameter coverage"))?;
        pending_parameters.insert(owner.node_id.clone(), bindings);
    }
    let mut retained_sites = result
        .owners
        .iter()
        .map(|o| o.identifiers.len() + o.comments.len())
        .sum::<usize>();
    let mut retained_text = result
        .owners
        .iter()
        .flat_map(|o| o.identifiers.iter().chain(&o.comments))
        .map(|s| s.text.len())
        .sum::<usize>();
    // Prior nested context is allocated after all direct-owner windows too.
    let mut retained_code_sites = result
        .owners
        .iter()
        .map(|owner| owner.code_windows.len())
        .sum::<usize>();
    let mut retained_code_bytes = result
        .owners
        .iter()
        .flat_map(|owner| &owner.code_windows)
        .map(|window| window.text.len())
        .sum::<usize>();
    for input in inputs.files.values() {
        let Some(file) = files.get(input.source_file.as_str()) else {
            continue;
        };
        if file.content_digest != input.source_digest || file.byte_size != input.byte_size {
            return Err(inventory_mismatch(
                &file.path,
                input.byte_size,
                file.byte_size,
            ));
        }
        let mut summary = SourceQueryFile {
            file: input.source_file.clone(),
            source_digest: input.source_digest.clone(),
            byte_size: input.byte_size,
            language: input.language.clone(),
            truncated: input.projection.truncated || input.omission.is_some(),
            syntax_recovery: input.syntax_recovery,
            omitted_owners: 0,
        };
        for scope in &input.projection.scopes {
            let candidates = ranges.get(&(
                input.source_file.as_str(),
                scope.start as u64,
                scope.end as u64,
            ));
            let node = match candidates.map(Vec::as_slice) {
                Some([node]) if exact(node) => *node,
                _ => {
                    summary.omitted_owners += 1;
                    continue;
                }
            };
            if scope.names.len() > 256
                || scope.comments.len() > 8
                || scope.code_windows.len() > 16
                || scope.names.values().any(|s| s.len() > 4)
            {
                return Err(invalid(
                    "source-query scope exceeds extraction profile limits",
                ));
            }
            let mut identifiers = scope
                .names
                .values()
                .flatten()
                .map(|s| site(&input.source_file, s))
                .collect::<Result<Vec<_>, _>>()?;
            let mut comments = scope
                .comments
                .iter()
                .map(|s| site(&input.source_file, s))
                .collect::<Result<Vec<_>, _>>()?;
            identifiers.sort_by_key(|s| (s.source.start_byte, s.source.end_byte));
            comments.sort_by_key(|s| (s.source.start_byte, s.source.end_byte));
            let count = identifiers.len() + comments.len();
            let bytes = identifiers
                .iter()
                .chain(&comments)
                .map(|s| s.text.len())
                .sum::<usize>();
            if result.owners.len() >= SOURCE_QUERY_MAX_OWNERS
                || retained_sites.saturating_add(count) > SOURCE_QUERY_MAX_SITES
                || retained_text.saturating_add(bytes) > SOURCE_QUERY_MAX_TEXT_BYTES
            {
                summary.omitted_owners += 1;
                summary.truncated = true;
                result.aggregate_truncated = true;
                continue;
            }
            let mut code_windows = scope
                .code_windows
                .iter()
                .map(|s| site(&input.source_file, s))
                .collect::<Result<Vec<_>, _>>()?;
            code_windows.sort_by_key(|s| (s.source.start_byte, s.source.end_byte));
            let mut omitted_code_windows = scope.omitted_code_windows;
            let code_bytes = code_windows.iter().map(|s| s.text.len()).sum::<usize>();
            if retained_code_sites.saturating_add(code_windows.len())
                > SOURCE_QUERY_MAX_CODE_WINDOWS
                || retained_code_bytes.saturating_add(code_bytes) > SOURCE_QUERY_MAX_CODE_BYTES
            {
                omitted_code_windows =
                    omitted_code_windows.saturating_add(code_windows.len() as u64);
                code_windows.clear();
            } else {
                retained_code_sites += code_windows.len();
                retained_code_bytes += code_bytes;
            }
            retained_sites += count;
            retained_text += bytes;
            if pending_parameters
                .insert(
                    node.id.clone(),
                    parameter_bindings(&input.source_file, scope)?,
                )
                .is_some()
            {
                return Err(invalid("duplicate parameter owner identity"));
            }
            result.owners.push(SourceQueryOwner {
                node_id: node.id.clone(),
                source: node
                    .source
                    .clone()
                    .ok_or_else(|| invalid("source-query owner lost its source"))?,
                source_digest: input.source_digest.clone(),
                identifiers,
                comments,
                parameter_bindings: None,
                code_windows,
                omitted_code_windows,
                nested_contexts: Vec::new(),
                omitted_nested_contexts: 0,
                omitted_sites: scope.omitted_sites as u64,
                truncated: scope.truncated,
                syntax_recovery: input.syntax_recovery,
            });
        }
        result.files.push(summary);
    }
    // Direct-owner windows receive the existing quotas first. Only spare
    // shared capacity can retain separate nested lexical context.
    let positions = result
        .owners
        .iter()
        .enumerate()
        .map(|(at, owner)| (owner.node_id.clone(), at))
        .collect::<BTreeMap<_, _>>();
    for input in inputs.files.values() {
        // Scopes arrive in native preorder. Resolve the nearest published
        // owner in one bounded pass, never by guessing from range containment.
        let mut ancestor_owners =
            Vec::<Option<usize>>::with_capacity(input.projection.scopes.len());
        for (at, scope) in input.projection.scopes.iter().enumerate() {
            if scope.callable_start < scope.start
                || scope.callable_end > scope.end
                || scope.callable_start >= scope.callable_end
            {
                return Err(invalid("callable syntax exceeds its definition envelope"));
            }
            let parent_owner = if let Some(parent) = scope.parent_scope {
                if parent >= at {
                    return Err(invalid("invalid source-query parent scope"));
                }
                let outer = &input.projection.scopes[parent];
                if !(outer.start <= scope.start && scope.end <= outer.end) {
                    return Err(invalid("nested callable exceeds explicit parent scope"));
                }
                ancestor_owners[parent]
            } else {
                None
            };
            let key = (
                input.source_file.as_str(),
                scope.start as u64,
                scope.end as u64,
            );
            let published = ranges.get(&key);
            let inherited = match published.map(Vec::as_slice) {
                Some([node]) if exact(node) => positions.get(&node.id).copied(),
                Some(_) => None,
                None if scope.name.is_none() => parent_owner,
                None => None,
            };
            ancestor_owners.push(inherited);
            if scope.name.is_some()
                || (scope.code_windows.is_empty() && scope.omitted_code_windows == 0)
                || !matches!(
                    scope.syntax.as_str(),
                    "lambda"
                        | "closure_expression"
                        | "func_literal"
                        | "arrow_function"
                        | "function_expression"
                        | "generator_function"
                )
                || published.is_some()
            {
                continue;
            }
            let selected = parent_owner;
            let Some(index) = selected else { continue };
            let owner = &mut result.owners[index];
            let used = owner.code_windows.len()
                + owner
                    .nested_contexts
                    .iter()
                    .map(|c| c.code_windows.len())
                    .sum::<usize>();
            if owner.nested_contexts.len() >= SOURCE_QUERY_MAX_NESTED_CONTEXTS || used >= 16 {
                owner.omitted_nested_contexts = owner.omitted_nested_contexts.saturating_add(1);
                continue;
            }
            let scope_anchor = SourceAnchor {
                file: input.source_file.clone(),
                start_byte: scope.callable_start as u64,
                end_byte: scope.callable_end as u64,
                start_line: u32::try_from(scope.start_point.row)
                    .ok()
                    .and_then(|r| r.checked_add(1))
                    .ok_or_else(|| invalid("nested source line overflow"))?,
                end_line: u32::try_from(scope.end_point.row)
                    .ok()
                    .and_then(|r| r.checked_add(1))
                    .ok_or_else(|| invalid("nested source line overflow"))?,
                start_column: u32::try_from(scope.start_point.column)
                    .map_err(|_| invalid("nested source column overflow"))?,
                end_column: u32::try_from(scope.end_point.column)
                    .map_err(|_| invalid("nested source column overflow"))?,
            };
            if !enclosed(&owner.source, &scope_anchor)
                || (scope_anchor.start_byte == owner.source.start_byte
                    && scope_anchor.end_byte == owner.source.end_byte)
            {
                return Err(invalid(
                    "nested source context does not match its published parent",
                ));
            }
            let mut context = SourceQueryNestedContext {
                source: scope_anchor,
                syntax: scope.syntax.clone(),
                code_windows: Vec::new(),
                omitted_code_windows: scope.omitted_code_windows,
            };
            let mut windows = scope.code_windows.iter().collect::<Vec<_>>();
            windows.sort_by_key(|w| (w.start, w.end));
            for window in windows {
                if used + context.code_windows.len() >= 16 {
                    context.omitted_code_windows = context.omitted_code_windows.saturating_add(1);
                    continue;
                }
                let window = site(&input.source_file, window)?;
                if !enclosed(&context.source, &window.source) {
                    return Err(invalid("nested code window escapes its scope"));
                }
                context.code_windows.push(window);
            }
            if context.code_windows.is_empty() {
                owner.omitted_nested_contexts = owner.omitted_nested_contexts.saturating_add(1);
            } else {
                owner.nested_contexts.push(context);
            }
        }
    }
    let mut context_order = (0..result.owners.len()).collect::<Vec<_>>();
    context_order.sort_by(|left, right| {
        let left = &result.owners[*left];
        let right = &result.owners[*right];
        (&left.source.file, left.source.start_byte, &left.node_id).cmp(&(
            &right.source.file,
            right.source.start_byte,
            &right.node_id,
        ))
    });
    for index in context_order {
        let owner = &mut result.owners[index];
        owner
            .nested_contexts
            .sort_by_key(|context| (context.source.start_byte, context.source.end_byte));
        for context in &mut owner.nested_contexts {
            context.code_windows.retain(|window| {
                if retained_code_sites >= SOURCE_QUERY_MAX_CODE_WINDOWS
                    || retained_code_bytes.saturating_add(window.text.len())
                        > SOURCE_QUERY_MAX_CODE_BYTES
                {
                    context.omitted_code_windows = context.omitted_code_windows.saturating_add(1);
                    false
                } else {
                    retained_code_sites += 1;
                    retained_code_bytes += window.text.len();
                    true
                }
            });
        }
        owner.omitted_nested_contexts = owner.omitted_nested_contexts.saturating_add(
            owner
                .nested_contexts
                .iter()
                .filter(|context| context.code_windows.is_empty())
                .count() as u64,
        );
        owner
            .nested_contexts
            .retain(|context| !context.code_windows.is_empty());
    }
    result.files.sort_by(|a, b| a.file.cmp(&b.file));
    result.owners.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    for owner in &mut result.owners {
        let mut bindings = pending_parameters
            .remove(&owner.node_id)
            .ok_or_else(|| invalid("parameter coverage lost its exact owner"))?;
        bindings.sites.retain(|site| {
            if retained_sites >= SOURCE_QUERY_MAX_SITES
                || retained_text.saturating_add(site.text.len()) > SOURCE_QUERY_MAX_TEXT_BYTES
            {
                bindings.status = ParameterBindingStatus::Partial;
                bindings.omitted_sites = bindings.omitted_sites.saturating_add(1);
                bindings
                    .reasons
                    .insert(ParameterBindingOmission::AggregateLimit);
                false
            } else {
                retained_sites += 1;
                retained_text += site.text.len();
                true
            }
        });
        owner.parameter_bindings = Some(bindings);
    }
    let summaries = result
        .files
        .iter()
        .map(|f| f.file.as_str())
        .collect::<BTreeSet<_>>();
    result.omitted_files = files
        .values()
        .filter(|f| {
            f.language
                .as_deref()
                .is_some_and(|l| matches!(l, "python" | "rust" | "go" | "typescript" | "tsx"))
                && !summaries.contains(f.path.as_str())
        })
        .count() as u64;
    result.omitted_files += result
        .files
        .iter()
        .filter(|f| f.truncated || f.syntax_recovery)
        .count() as u64;
    result.omitted_owners = result.files.iter().map(|f| f.omitted_owners).sum();
    let owner_ids = result
        .owners
        .iter()
        .map(|o| o.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut aliases = BTreeMap::<&str, Vec<&str>>::new();
    for edge in &graph.links {
        if edge.kind == EdgeKind::Aliases && owner_ids.contains(edge.target.as_str()) {
            let entries = aliases.entry(&edge.target).or_default();
            if entries.len() < 9 {
                entries.push(
                    if !edge.evidence.is_empty()
                        && compass_model::search::is_exact_nonheuristic_relationship(edge)
                    {
                        nodes
                            .get(edge.source.as_str())
                            .map_or("", |n| n.name.as_str())
                    } else {
                        ""
                    },
                );
            }
        }
    }
    for owner in &result.owners {
        let node = nodes
            .get(owner.node_id.as_str())
            .ok_or_else(|| invalid("source-query owner is missing from graph"))?;
        let base = compass_model::search::search_document_with_aliases(
            node,
            aliases
                .get(owner.node_id.as_str())
                .into_iter()
                .flatten()
                .copied(),
        )
        .terms();
        for term in owner.document().terms() {
            *result
                .additional_document_frequencies
                .entry(term.clone())
                .or_default() += u64::from(!base.contains(&term));
        }
    }
    result.validate().map_err(|e| invalid(e.to_string()))?;
    let path = output.join(SOURCE_QUERY_FILE);
    compass_files::write_atomic_with_digest(&path, |writer| {
        let mut bounded = CappedWriter {
            inner: writer,
            bytes: 0,
        };
        serde_json::to_writer(&mut bounded, &result).map_err(|source| {
            compass_files::FileError::Json {
                path: path.clone(),
                source,
            }
        })
    })?;
    Ok(())
}
struct CappedWriter<W> {
    inner: W,
    bytes: u64,
}
impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.saturating_add(bytes.len() as u64) > SOURCE_QUERY_MAX_BYTES {
            return Err(std::io::Error::other(
                "source-query artifact byte limit exceeded",
            ));
        }
        let n = self.inner.write(bytes)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
