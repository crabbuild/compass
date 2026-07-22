//! Corpus-level semantic extraction, reconciliation, and caching.

use super::*;
use compass_files::{Cache, CacheKind, bisect_slice, file_hash, prompt_fingerprint, split_file};

/// Load one semantic chunk, call a resolved provider, validate its untrusted
/// graph fragment, and bind code-symbol evidence to the exact text sent to the
/// model.
pub fn extract_semantic_units(
    units: &[SemanticUnit],
    backend: &ResolvedBackend,
    root: &Path,
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<DirectExtractionResult, SemanticError> {
    let mut text_units = Vec::new();
    let mut image_paths = Vec::new();
    for unit in units {
        match unit {
            SemanticUnit::File(path) if is_vision_image(path) => image_paths.push(path.clone()),
            _ => text_units.push(unit.clone()),
        }
    }
    let read = read_semantic_units(&text_units, root);
    let vision = backend.backend.vision
        || (backend.backend.name == "ollama"
            && environment
                .get("GRAPHIFY_OLLAMA_VISION")
                .is_some_and(|value| value.trim() == "1"));
    let inline_images = vision && backend.backend.name != "claude-cli";
    let built_images = build_image_refs(&image_paths, root, inline_images)?;
    let mut fragment = execute_resolved_backend(
        backend,
        &read.prompt,
        &built_images.images,
        deep_mode,
        environment,
    )?;
    let errors = validate_semantic_fragment(&mut fragment);
    if !errors.is_empty() {
        return Err(SemanticError::InvalidFragment(errors.join("; ")));
    }
    let evidence = read.evidence_sources();
    let unverified_nodes = bind_node_evidence(&mut fragment, &evidence, root);
    let mut warnings = read.warnings;
    warnings.extend(built_images.warnings);
    Ok(DirectExtractionResult {
        fragment,
        warnings,
        unverified_nodes,
    })
}

/// Custom-provider counterpart to [`extract_semantic_units`].
pub fn extract_semantic_units_custom(
    units: &[SemanticUnit],
    backend: &ResolvedCustomBackend,
    root: &Path,
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<DirectExtractionResult, SemanticError> {
    let mut text_units = Vec::new();
    let mut image_paths = Vec::new();
    for unit in units {
        match unit {
            SemanticUnit::File(path) if is_vision_image(path) => image_paths.push(path.clone()),
            _ => text_units.push(unit.clone()),
        }
    }
    let read = read_semantic_units(&text_units, root);
    let built_images = build_image_refs(&image_paths, root, backend.vision)?;
    let mut fragment = execute_resolved_custom_backend(
        backend,
        &read.prompt,
        &built_images.images,
        deep_mode,
        environment,
    )?;
    let errors = validate_semantic_fragment(&mut fragment);
    if !errors.is_empty() {
        return Err(SemanticError::InvalidFragment(errors.join("; ")));
    }
    let evidence = read.evidence_sources();
    let unverified_nodes = bind_node_evidence(&mut fragment, &evidence, root);
    let mut warnings = read.warnings;
    warnings.extend(built_images.warnings);
    Ok(DirectExtractionResult {
        fragment,
        warnings,
        unverified_nodes,
    })
}

/// Expand oversized splittable documents into complete, gap-free slices.
#[must_use]
pub fn expand_oversized_semantic_files(paths: &[PathBuf], max_chars: usize) -> Vec<SemanticUnit> {
    let mut units = Vec::new();
    for path in paths {
        let splittable = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["md", "mdx", "markdown", "txt", "rst"]
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            });
        if !splittable {
            units.push(SemanticUnit::File(path.clone()));
            continue;
        }
        match split_file(path, max_chars) {
            Ok(slices) if slices.len() > 1 => {
                units.extend(slices.into_iter().map(SemanticUnit::Slice));
            }
            _ => units.push(SemanticUnit::File(path.clone())),
        }
    }
    units
}

/// Estimate prompt cost using Graphify's deterministic chars-per-token
/// fallback. Raster images use a fixed vision charge.
#[must_use]
pub fn estimate_semantic_unit_tokens(unit: &SemanticUnit) -> usize {
    if matches!(unit, SemanticUnit::File(path) if is_vision_image(path)) {
        return IMAGE_TOKEN_ESTIMATE;
    }
    let chars = match unit {
        SemanticUnit::Slice(slice) => slice.end.saturating_sub(slice.start).min(FILE_CHAR_CAP),
        SemanticUnit::File(path) => match fs::metadata(path) {
            Ok(metadata) => usize::try_from(metadata.len())
                .unwrap_or(usize::MAX)
                .min(FILE_CHAR_CAP),
            Err(_) => return 0,
        },
    };
    chars.saturating_add(160) / 4
}

/// Greedily pack semantically related units by directory while respecting
/// both token and image-count limits.
pub fn pack_semantic_chunks(
    units: &[SemanticUnit],
    token_budget: usize,
) -> Result<Vec<Vec<SemanticUnit>>, SemanticError> {
    if token_budget == 0 {
        return Err(SemanticError::InvalidProviderConfiguration(
            "token_budget must be positive, got 0".to_owned(),
        ));
    }
    let mut by_directory = BTreeMap::<PathBuf, Vec<SemanticUnit>>::new();
    for unit in units {
        let directory = unit
            .path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        by_directory
            .entry(directory)
            .or_default()
            .push(unit.clone());
    }
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_tokens = 0_usize;
    let mut current_images = 0_usize;
    for units in by_directory.into_values() {
        for unit in units {
            let cost = estimate_semantic_unit_tokens(&unit);
            let image = matches!(&unit, SemanticUnit::File(path) if is_vision_image(path));
            let exceeds_budget = current_tokens.saturating_add(cost) > token_budget;
            let exceeds_images = image && current_images >= MAX_IMAGES_PER_CHUNK;
            if !current.is_empty() && (exceeds_budget || exceeds_images) {
                chunks.push(std::mem::take(&mut current));
                current_tokens = 0;
                current_images = 0;
            }
            current.push(unit);
            current_tokens = current_tokens.saturating_add(cost);
            current_images += usize::from(image);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

fn is_vision_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["png", "jpg", "jpeg", "gif", "webp"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// Execute one semantic chunk and recursively bisect known context or output
/// truncation failures. The callback is the provider boundary, which keeps the
/// recovery algorithm deterministic and directly testable.
pub fn extract_with_adaptive_retry<F>(
    chunk: &[SemanticUnit],
    model: Option<&str>,
    max_depth: usize,
    extract: &F,
) -> Result<Value, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError>,
{
    adaptive_extract_at_depth(chunk, model, max_depth, 0, extract)
}

fn adaptive_extract_at_depth<F>(
    chunk: &[SemanticUnit],
    model: Option<&str>,
    max_depth: usize,
    depth: usize,
    extract: &F,
) -> Result<Value, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError>,
{
    let result = match extract(chunk) {
        Ok(result) => result,
        Err(error) if looks_like_context_exceeded(&error.to_string()) => {
            if let Some((left, right)) = split_semantic_chunk(chunk, depth, max_depth) {
                return extract_split(&left, &right, model, max_depth, depth, extract);
            }
            return Ok(empty_semantic_result(model));
        }
        Err(error) => return Err(error),
    };
    if result.get("finish_reason").and_then(Value::as_str) != Some("length") {
        return Ok(result);
    }
    if let Some((left, right)) = split_semantic_chunk(chunk, depth, max_depth) {
        return extract_split(&left, &right, model, max_depth, depth, extract);
    }
    let mut partial = result;
    mark_partial(&mut partial);
    let mut files = partial
        .get("_partial_files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    files.extend(
        chunk
            .iter()
            .map(|unit| unit.path().to_string_lossy().into_owned()),
    );
    partial["_partial_files"] = Value::Array(files.into_iter().map(Value::String).collect());
    Ok(partial)
}

fn split_semantic_chunk(
    chunk: &[SemanticUnit],
    depth: usize,
    max_depth: usize,
) -> Option<(Vec<SemanticUnit>, Vec<SemanticUnit>)> {
    if depth >= max_depth || chunk.is_empty() {
        return None;
    }
    if let [SemanticUnit::Slice(slice)] = chunk {
        let (left, right) = bisect_slice(slice).ok().flatten()?;
        return Some((
            vec![SemanticUnit::Slice(left)],
            vec![SemanticUnit::Slice(right)],
        ));
    }
    if chunk.len() <= 1 {
        return None;
    }
    let midpoint = chunk.len() / 2;
    Some((chunk[..midpoint].to_vec(), chunk[midpoint..].to_vec()))
}

fn extract_split<F>(
    left: &[SemanticUnit],
    right: &[SemanticUnit],
    model: Option<&str>,
    max_depth: usize,
    depth: usize,
    extract: &F,
) -> Result<Value, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError>,
{
    let left = adaptive_extract_at_depth(left, model, max_depth, depth + 1, extract)?;
    let right = adaptive_extract_at_depth(right, model, max_depth, depth + 1, extract)?;
    Ok(merge_semantic_results(&left, &right, model))
}

#[must_use]
pub fn merge_semantic_results(left: &Value, right: &Value, model: Option<&str>) -> Value {
    let merged_bucket = |name: &str| {
        left.get(name)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                right
                    .get(name)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .cloned()
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "nodes": merged_bucket("nodes"),
        "edges": merged_bucket("edges"),
        "hyperedges": merged_bucket("hyperedges"),
        "input_tokens": numeric_u64(left.get("input_tokens")).saturating_add(numeric_u64(right.get("input_tokens"))),
        "output_tokens": numeric_u64(left.get("output_tokens")).saturating_add(numeric_u64(right.get("output_tokens"))),
        "model": model,
        "finish_reason":"stop",
        "_partial_files": merged_partial_files(&[left.clone(), right.clone()]),
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeReconciliation {
    pub out_of_scope_dropped: usize,
    pub dropped_files: Vec<String>,
    pub uncovered_files: Vec<PathBuf>,
}

fn resolved_against_root(path: &Path, root: &Path) -> PathBuf {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    fs::canonicalize(&candidate).unwrap_or(candidate)
}

fn semantic_item_is_out_of_scope(item: &Value, root: &Path, dispatched: &HashSet<PathBuf>) -> bool {
    let Some(source_file) = item.get("source_file").and_then(Value::as_str) else {
        return false;
    };
    let source_path = Path::new(source_file);
    let candidate = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        root.join(source_path)
    };
    candidate.is_file() && !dispatched.contains(&resolved_against_root(source_path, root))
}

/// Remove provider output attributed to real files outside the dispatched
/// corpus and report dispatched files for which the provider returned no node.
pub fn reconcile_semantic_scope(
    fragment: &mut Value,
    dispatched: &[PathBuf],
    root: &Path,
) -> Result<ScopeReconciliation, SemanticError> {
    let object = fragment.as_object_mut().ok_or_else(|| {
        SemanticError::InvalidFragment("semantic corpus result must be an object".to_owned())
    })?;
    let dispatched_resolved = dispatched
        .iter()
        .map(|path| resolved_against_root(path, root))
        .collect::<HashSet<_>>();
    let mut dropped_ids = HashSet::new();
    let mut dropped_files = BTreeSet::new();
    let nodes = object
        .entry("nodes")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| SemanticError::InvalidFragment("nodes must be an array".to_owned()))?;
    let original_node_count = nodes.len();
    nodes.retain(|node| {
        if !semantic_item_is_out_of_scope(node, root, &dispatched_resolved) {
            return true;
        }
        if let Some(identifier) = node.get("id").and_then(Value::as_str) {
            dropped_ids.insert(identifier.to_owned());
        }
        if let Some(source_file) = node.get("source_file").and_then(Value::as_str) {
            dropped_files.insert(source_file.to_owned());
        }
        false
    });
    let dropped = original_node_count.saturating_sub(nodes.len());
    if dropped > 0 {
        let edges = object
            .entry("edges")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| SemanticError::InvalidFragment("edges must be an array".to_owned()))?;
        edges.retain(|edge| {
            !semantic_item_is_out_of_scope(edge, root, &dispatched_resolved)
                && edge
                    .get("source")
                    .and_then(Value::as_str)
                    .is_none_or(|identifier| !dropped_ids.contains(identifier))
                && edge
                    .get("target")
                    .and_then(Value::as_str)
                    .is_none_or(|identifier| !dropped_ids.contains(identifier))
        });
        let hyperedges = object
            .entry("hyperedges")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                SemanticError::InvalidFragment("hyperedges must be an array".to_owned())
            })?;
        hyperedges.retain(|hyperedge| {
            !semantic_item_is_out_of_scope(hyperedge, root, &dispatched_resolved)
                && !hyperedge
                    .get("nodes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|identifier| dropped_ids.contains(identifier))
        });
    }
    object.insert("out_of_scope_dropped".to_owned(), Value::from(dropped));

    let covered = object
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| node.get("source_file").and_then(Value::as_str))
        .map(|path| resolved_against_root(Path::new(path), root))
        .collect::<HashSet<_>>();
    let mut uncovered_files = dispatched
        .iter()
        .filter(|path| !covered.contains(&resolved_against_root(path, root)))
        .cloned()
        .collect::<Vec<_>>();
    uncovered_files.sort();
    uncovered_files.dedup();
    object.insert(
        "uncovered_files".to_owned(),
        Value::Array(
            uncovered_files
                .iter()
                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                .collect(),
        ),
    );
    Ok(ScopeReconciliation {
        out_of_scope_dropped: dropped,
        dropped_files: dropped_files.into_iter().collect(),
        uncovered_files,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct CorpusExtractionOptions {
    pub backend_name: String,
    pub model: Option<String>,
    pub chunk_size: usize,
    pub token_budget: Option<usize>,
    pub max_concurrency: usize,
    pub max_retry_depth: usize,
}

impl Default for CorpusExtractionOptions {
    fn default() -> Self {
        Self {
            backend_name: "kimi".to_owned(),
            model: None,
            chunk_size: 20,
            token_budget: Some(60_000),
            max_concurrency: 4,
            max_retry_depth: 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkFailure {
    pub index: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CorpusExtractionResult {
    pub fragment: Value,
    pub failures: Vec<ChunkFailure>,
    pub reconciliation: ScopeReconciliation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CachedCorpusExtractionOptions {
    pub extraction: CorpusExtractionOptions,
    pub deep_mode: bool,
    pub force: bool,
    pub cache_enabled: bool,
    /// Full live semantic corpus used for orphan pruning. `None` skips pruning;
    /// callers must never substitute an incremental changed-file subset.
    pub prune_live_files: Option<Vec<PathBuf>>,
}

impl Default for CachedCorpusExtractionOptions {
    fn default() -> Self {
        Self {
            extraction: CorpusExtractionOptions::default(),
            deep_mode: false,
            force: false,
            cache_enabled: true,
            prune_live_files: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticCacheIssue {
    pub chunk_index: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CachedCorpusExtractionResult {
    pub fragment: Value,
    pub failures: Vec<ChunkFailure>,
    pub reconciliation: ScopeReconciliation,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub checkpointed_files: usize,
    pub finalized_files: usize,
    pub pruned_entries: usize,
    pub partial_files: Vec<PathBuf>,
    pub cache_issues: Vec<SemanticCacheIssue>,
    pub provider_warnings: Vec<String>,
    pub unverified_nodes: usize,
}

/// Apply provider-specific serialization constraints to a requested worker
/// count while preserving the Python opt-in overrides.
#[must_use]
pub fn effective_semantic_concurrency(
    backend_name: &str,
    requested: usize,
    chunk_count: usize,
    environment: &HashMap<String, String>,
) -> usize {
    let serial = (backend_name == "ollama"
        && environment
            .get("GRAPHIFY_OLLAMA_PARALLEL")
            .is_none_or(|value| value.trim() != "1"))
        || (backend_name == "claude-cli"
            && environment
                .get("GRAPHIFY_CLAUDE_CLI_PARALLEL")
                .is_none_or(|value| value.trim() != "1"));
    if serial {
        1
    } else {
        requested.max(1).min(chunk_count.max(1))
    }
}

fn merge_corpus_chunk(merged: &mut Value, result: &Value) {
    for bucket in ["nodes", "edges", "hyperedges"] {
        let incoming = result
            .get(bucket)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(target) = merged.get_mut(bucket).and_then(Value::as_array_mut) {
            target.extend(incoming);
        }
    }
    for field in ["input_tokens", "output_tokens"] {
        let total = numeric_u64(merged.get(field)).saturating_add(numeric_u64(result.get(field)));
        merged[field] = Value::from(total);
    }
    let partial = merged_partial_files(&[merged.clone(), result.clone()]);
    if !partial.is_empty() {
        merged["_partial_files"] = Value::Array(partial.into_iter().map(Value::String).collect());
    }
}

/// Run corpus chunks concurrently, adaptively bisect truncated chunks, merge
/// in deterministic submission order, and reconcile provider coverage.
pub fn extract_corpus_parallel_with<F>(
    files: &[PathBuf],
    root: &Path,
    options: &CorpusExtractionOptions,
    environment: &HashMap<String, String>,
    extract: &F,
) -> Result<CorpusExtractionResult, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError> + Sync,
{
    extract_corpus_parallel_with_progress(
        files,
        root,
        options,
        environment,
        extract,
        &mut |_, _, _, _| {},
    )
}

/// Progress-aware counterpart to [`extract_corpus_parallel_with`]. Successful
/// top-level chunks invoke the callback in completion order, while the returned
/// graph is always merged in deterministic submission order.
pub fn extract_corpus_parallel_with_progress<F, P>(
    files: &[PathBuf],
    root: &Path,
    options: &CorpusExtractionOptions,
    environment: &HashMap<String, String>,
    extract: &F,
    on_chunk_done: &mut P,
) -> Result<CorpusExtractionResult, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError> + Sync,
    P: FnMut(usize, usize, &[SemanticUnit], &Value) + Send,
{
    let units = expand_oversized_semantic_files(files, FILE_CHAR_CAP);
    let chunks = if let Some(token_budget) = options.token_budget {
        pack_semantic_chunks(&units, token_budget)?
    } else {
        if options.chunk_size == 0 {
            return Err(SemanticError::InvalidProviderConfiguration(
                "chunk_size must be positive, got 0".to_owned(),
            ));
        }
        units
            .chunks(options.chunk_size)
            .map(<[SemanticUnit]>::to_vec)
            .collect::<Vec<_>>()
    };
    let workers = effective_semantic_concurrency(
        &options.backend_name,
        options.max_concurrency,
        chunks.len(),
        environment,
    );
    let total = chunks.len();
    let mut results = if workers == 1 {
        let mut results = Vec::with_capacity(total);
        for (index, chunk) in chunks.iter().enumerate() {
            let result = extract_with_adaptive_retry(
                chunk,
                options.model.as_deref(),
                options.max_retry_depth,
                extract,
            );
            if let Ok(fragment) = &result {
                on_chunk_done(index, total, chunk, fragment);
            }
            results.push((index, result));
        }
        results
    } else {
        let (sender, receiver) = std::sync::mpsc::channel();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let sender = sender.clone();
                let next = &next;
                let chunks = &chunks;
                scope.spawn(move || {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(chunk) = chunks.get(index) else {
                            break;
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            extract_with_adaptive_retry(
                                chunk,
                                options.model.as_deref(),
                                options.max_retry_depth,
                                extract,
                            )
                        }))
                        .unwrap_or_else(|_| {
                            Err(SemanticError::Transport(
                                "semantic chunk worker panicked".to_owned(),
                            ))
                        });
                        if sender.send((index, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);
            let mut results = Vec::with_capacity(total);
            for (index, result) in receiver {
                if let Ok(fragment) = &result {
                    on_chunk_done(index, total, &chunks[index], fragment);
                }
                results.push((index, result));
            }
            results
        })
    };
    results.sort_by_key(|(index, _)| *index);
    let mut merged = serde_json::json!({
        "nodes":[],
        "edges":[],
        "hyperedges":[],
        "input_tokens":0,
        "output_tokens":0,
        "failed_chunks":0,
    });
    let mut failures = Vec::new();
    for (index, result) in results {
        match result {
            Ok(result) => merge_corpus_chunk(&mut merged, &result),
            Err(error) => failures.push(ChunkFailure {
                index,
                message: error.to_string(),
            }),
        }
    }
    merged["failed_chunks"] = Value::from(failures.len());
    let dispatched = units
        .iter()
        .map(|unit| unit.path().to_path_buf())
        .collect::<Vec<_>>();
    let reconciliation = reconcile_semantic_scope(&mut merged, &dispatched, root)?;
    Ok(CorpusExtractionResult {
        fragment: merged,
        failures,
        reconciliation,
    })
}

/// Replay compatible semantic cache entries, checkpoint every successful
/// top-level chunk as it completes, and finalize authoritative per-file cache
/// entries after scope reconciliation. Cache write failures are reported but do
/// not discard successfully extracted graph data.
pub fn extract_corpus_cached_with<F, P>(
    files: &[PathBuf],
    root: &Path,
    cache_root: Option<&Path>,
    options: &CachedCorpusExtractionOptions,
    environment: &HashMap<String, String>,
    extract: &F,
    on_chunk_done: &mut P,
) -> Result<CachedCorpusExtractionResult, SemanticError>
where
    F: Fn(&[SemanticUnit]) -> Result<Value, SemanticError> + Sync,
    P: FnMut(usize, usize, &[SemanticUnit], &Value) + Send,
{
    let prompt = extraction_prompt(options.deep_mode);
    let cache_enabled =
        options.cache_enabled && !environment.contains_key("GRAPHIFY_NO_INCREMENTAL_CACHE");
    let mut cache = cache_enabled
        .then(|| Cache::new(root, cache_root))
        .transpose()?;
    let checked = if options.force || !cache_enabled {
        SemanticCacheCheck {
            uncached: files.to_vec(),
            ..SemanticCacheCheck::default()
        }
    } else {
        check_semantic_cache(
            cache.as_mut().ok_or_else(|| {
                SemanticError::InvalidProviderConfiguration(
                    "semantic cache was unexpectedly unavailable".to_owned(),
                )
            })?,
            files,
            options.deep_mode,
            &prompt,
        )?
    };
    let cache_hits = files.len().saturating_sub(checked.uncached.len());
    let cache_misses = checked.uncached.len();
    let mut checkpointed_files = 0_usize;
    let mut cache_issues = Vec::new();
    let fresh = {
        let mut checkpoint =
            |index: usize, total: usize, chunk: &[SemanticUnit], fragment: &Value| {
                let Some(cache) = cache.as_mut() else {
                    on_chunk_done(index, total, chunk, fragment);
                    return;
                };
                let partial_files = partial_source_files(fragment)
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                let save_options = SemanticCacheSaveOptions {
                    merge_existing: true,
                    allowed_source_files: Some(
                        chunk.iter().map(|unit| unit.path().to_path_buf()).collect(),
                    ),
                    partial_source_files: partial_files,
                    deep_mode: options.deep_mode,
                    prompt: prompt.clone(),
                };
                match save_semantic_cache(cache, root, fragment, &save_options) {
                    Ok(report) => {
                        checkpointed_files = checkpointed_files.saturating_add(report.saved);
                    }
                    Err(error) => cache_issues.push(SemanticCacheIssue {
                        chunk_index: Some(index),
                        message: error.to_string(),
                    }),
                }
                on_chunk_done(index, total, chunk, fragment);
            };
        extract_corpus_parallel_with_progress(
            &checked.uncached,
            root,
            &options.extraction,
            environment,
            extract,
            &mut checkpoint,
        )?
    };

    let partial_files = partial_source_files(&fresh.fragment)
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut finalized_files = 0_usize;
    if let Some(cache) = cache.as_mut() {
        let save_options = SemanticCacheSaveOptions {
            merge_existing: false,
            allowed_source_files: Some(checked.uncached.clone()),
            partial_source_files: partial_files.clone(),
            deep_mode: options.deep_mode,
            prompt: prompt.clone(),
        };
        match save_semantic_cache(cache, root, &fresh.fragment, &save_options) {
            Ok(report) => finalized_files = report.saved,
            Err(error) => cache_issues.push(SemanticCacheIssue {
                chunk_index: None,
                message: error.to_string(),
            }),
        }
    }

    let mut fresh_fragment = fresh.fragment;
    strip_partial_markers(&mut fresh_fragment);
    let mut combined = serde_json::json!({
        "nodes":checked.nodes,
        "edges":checked.edges,
        "hyperedges":checked.hyperedges,
        "input_tokens":numeric_u64(fresh_fragment.get("input_tokens")),
        "output_tokens":numeric_u64(fresh_fragment.get("output_tokens")),
        "failed_chunks":fresh.failures.len(),
    });
    for bucket in ["nodes", "edges", "hyperedges"] {
        if let (Some(target), Some(incoming)) = (
            combined.get_mut(bucket).and_then(Value::as_array_mut),
            fresh_fragment.get(bucket).and_then(Value::as_array),
        ) {
            target.extend(incoming.iter().cloned());
        }
    }

    let pruned_entries = if let (Some(cache), Some(live_files)) =
        (cache.as_ref(), options.prune_live_files.as_ref())
    {
        let live_hashes = live_files
            .iter()
            .filter_map(|path| file_hash(path, root).ok())
            .collect::<BTreeSet<_>>();
        cache.prune_semantic(&live_hashes)
    } else {
        0
    };
    Ok(CachedCorpusExtractionResult {
        fragment: combined,
        failures: fresh.failures,
        reconciliation: fresh.reconciliation,
        cache_hits,
        cache_misses,
        checkpointed_files,
        finalized_files,
        pruned_entries,
        partial_files,
        cache_issues,
        provider_warnings: Vec::new(),
        unverified_nodes: 0,
    })
}

/// Execute the complete cached corpus pipeline for a resolved built-in
/// provider while retaining non-fatal source/image and evidence diagnostics.
pub fn extract_builtin_corpus_cached<P>(
    files: &[PathBuf],
    backend: &ResolvedBackend,
    root: &Path,
    cache_root: Option<&Path>,
    options: &CachedCorpusExtractionOptions,
    environment: &HashMap<String, String>,
    on_chunk_done: &mut P,
) -> Result<CachedCorpusExtractionResult, SemanticError>
where
    P: FnMut(usize, usize, &[SemanticUnit], &Value) + Send,
{
    let mut effective = options.clone();
    effective.extraction.backend_name = backend.backend.name.to_owned();
    effective.extraction.model = Some(backend.model.clone());
    let warnings = std::sync::Mutex::new(Vec::new());
    let unverified = std::sync::atomic::AtomicUsize::new(0);
    let mut result = extract_corpus_cached_with(
        files,
        root,
        cache_root,
        &effective,
        environment,
        &|units| {
            let direct =
                extract_semantic_units(units, backend, root, effective.deep_mode, environment)?;
            unverified.fetch_add(
                direct.unverified_nodes,
                std::sync::atomic::Ordering::Relaxed,
            );
            if let Ok(mut collected) = warnings.lock() {
                collected.extend(direct.warnings);
            }
            Ok(direct.fragment)
        },
        on_chunk_done,
    )?;
    result.provider_warnings = match warnings.into_inner() {
        Ok(warnings) => warnings,
        Err(poisoned) => poisoned.into_inner(),
    };
    result.unverified_nodes = unverified.load(std::sync::atomic::Ordering::Relaxed);
    Ok(result)
}

/// Custom OpenAI-compatible counterpart to
/// [`extract_builtin_corpus_cached`].
pub fn extract_custom_corpus_cached<P>(
    files: &[PathBuf],
    backend: &ResolvedCustomBackend,
    root: &Path,
    cache_root: Option<&Path>,
    options: &CachedCorpusExtractionOptions,
    environment: &HashMap<String, String>,
    on_chunk_done: &mut P,
) -> Result<CachedCorpusExtractionResult, SemanticError>
where
    P: FnMut(usize, usize, &[SemanticUnit], &Value) + Send,
{
    let mut effective = options.clone();
    effective.extraction.backend_name = backend.name.clone();
    effective.extraction.model = Some(backend.model.clone());
    let warnings = std::sync::Mutex::new(Vec::new());
    let unverified = std::sync::atomic::AtomicUsize::new(0);
    let mut result = extract_corpus_cached_with(
        files,
        root,
        cache_root,
        &effective,
        environment,
        &|units| {
            let direct = extract_semantic_units_custom(
                units,
                backend,
                root,
                effective.deep_mode,
                environment,
            )?;
            unverified.fetch_add(
                direct.unverified_nodes,
                std::sync::atomic::Ordering::Relaxed,
            );
            if let Ok(mut collected) = warnings.lock() {
                collected.extend(direct.warnings);
            }
            Ok(direct.fragment)
        },
        on_chunk_done,
    )?;
    result.provider_warnings = match warnings.into_inner() {
        Ok(warnings) => warnings,
        Err(poisoned) => poisoned.into_inner(),
    };
    result.unverified_nodes = unverified.load(std::sync::atomic::Ordering::Relaxed);
    Ok(result)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticCacheCheck {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub hyperedges: Vec<Value>,
    pub uncached: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticCacheSaveOptions {
    pub merge_existing: bool,
    pub allowed_source_files: Option<Vec<PathBuf>>,
    pub partial_source_files: Vec<PathBuf>,
    pub deep_mode: bool,
    pub prompt: String,
}

impl SemanticCacheSaveOptions {
    #[must_use]
    pub fn for_extraction(deep_mode: bool) -> Self {
        Self {
            merge_existing: false,
            allowed_source_files: None,
            partial_source_files: Vec::new(),
            deep_mode,
            prompt: extraction_prompt(deep_mode),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticCacheSaveReport {
    pub saved: usize,
    pub skipped_not_file: usize,
    pub skipped_out_of_scope: usize,
}

#[derive(Default)]
struct SemanticCacheGroup {
    nodes: Vec<Value>,
    edges: Vec<Value>,
    hyperedges: Vec<Value>,
}

fn semantic_cache_kind(deep_mode: bool) -> CacheKind {
    if deep_mode {
        CacheKind::SemanticMode("deep".to_owned())
    } else {
        CacheKind::Semantic
    }
}

/// Replay complete per-file semantic entries produced by the same prompt.
pub fn check_semantic_cache(
    cache: &mut Cache,
    files: &[PathBuf],
    deep_mode: bool,
    prompt: &str,
) -> Result<SemanticCacheCheck, SemanticError> {
    check_semantic_cache_mode(cache, files, deep_mode.then_some("deep"), Some(prompt))
}

/// Compatibility cache reader for agent-managed extraction workflows.
///
/// A missing prompt reads the historical flat namespace. Supplying a prompt
/// selects its fingerprinted namespace while retaining legacy fallback, just
/// like Python's `check_semantic_cache`.
pub fn check_semantic_cache_mode(
    cache: &mut Cache,
    files: &[PathBuf],
    mode: Option<&str>,
    prompt: Option<&str>,
) -> Result<SemanticCacheCheck, SemanticError> {
    let kind = mode.map_or(CacheKind::Semantic, |mode| {
        CacheKind::SemanticMode(mode.to_owned())
    });
    let fingerprint = prompt.map(prompt_fingerprint);
    let mut checked = SemanticCacheCheck::default();
    for file in files {
        let Some(entry) = cache.load(file, &kind, fingerprint.as_deref(), true, false)? else {
            checked.uncached.push(file.clone());
            continue;
        };
        checked.nodes.extend(
            entry
                .get("nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
        checked.edges.extend(
            entry
                .get("edges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
        checked.hyperedges.extend(
            entry
                .get("hyperedges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    Ok(checked)
}

fn cache_group_has_partial_marker(group: &SemanticCacheGroup) -> bool {
    [&group.nodes, &group.edges, &group.hyperedges]
        .into_iter()
        .flatten()
        .any(|item| item.get("_partial").and_then(Value::as_bool) == Some(true))
}

fn prepend_cached_bucket(target: &mut Vec<Value>, entry: &Value, bucket: &str) {
    let mut combined = entry
        .get(bucket)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    combined.append(target);
    *target = combined;
}

/// Save semantic output by real source file, optionally merging slice
/// checkpoints and refusing writes outside the dispatched allowlist.
pub fn save_semantic_cache(
    cache: &mut Cache,
    root: &Path,
    fragment: &Value,
    options: &SemanticCacheSaveOptions,
) -> Result<SemanticCacheSaveReport, SemanticError> {
    let mut groups = BTreeMap::<String, SemanticCacheGroup>::new();
    for (bucket, values) in [
        ("nodes", &fragment["nodes"]),
        ("edges", &fragment["edges"]),
        ("hyperedges", &fragment["hyperedges"]),
    ] {
        for item in values.as_array().into_iter().flatten() {
            let Some(source_file) = item.get("source_file").and_then(Value::as_str) else {
                continue;
            };
            let group = groups.entry(source_file.to_owned()).or_default();
            match bucket {
                "nodes" => group.nodes.push(item.clone()),
                "edges" => group.edges.push(item.clone()),
                _ => group.hyperedges.push(item.clone()),
            }
        }
    }
    let allowed = options.allowed_source_files.as_ref().map(|files| {
        files
            .iter()
            .map(|path| resolved_against_root(path, root))
            .collect::<HashSet<_>>()
    });
    let partial = options
        .partial_source_files
        .iter()
        .map(|path| resolved_against_root(path, root))
        .collect::<HashSet<_>>();
    let present = groups
        .keys()
        .map(|path| resolved_against_root(Path::new(path), root))
        .collect::<HashSet<_>>();
    for path in partial.difference(&present) {
        groups
            .entry(path.to_string_lossy().into_owned())
            .or_default();
    }
    if let Some(allowed) = &allowed {
        let mut skipped_ids = HashSet::new();
        let mut written_ids = HashSet::new();
        for (source_file, group) in &groups {
            let path = resolved_against_root(Path::new(source_file), root);
            let target = if !path.is_file() || !allowed.contains(&path) {
                &mut skipped_ids
            } else {
                &mut written_ids
            };
            target.extend(
                group
                    .nodes
                    .iter()
                    .filter_map(|node| node.get("id").and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
        skipped_ids.retain(|identifier| !written_ids.contains(identifier));
        if !skipped_ids.is_empty() {
            for (source_file, group) in &mut groups {
                let path = resolved_against_root(Path::new(source_file), root);
                if !path.is_file() || !allowed.contains(&path) {
                    continue;
                }
                group.edges.retain(|edge| {
                    edge.get("source")
                        .and_then(Value::as_str)
                        .is_none_or(|identifier| !skipped_ids.contains(identifier))
                        && edge
                            .get("target")
                            .and_then(Value::as_str)
                            .is_none_or(|identifier| !skipped_ids.contains(identifier))
                });
                group.hyperedges.retain(|hyperedge| {
                    !hyperedge
                        .get("nodes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(|identifier| skipped_ids.contains(identifier))
                });
            }
        }
    }
    let kind = semantic_cache_kind(options.deep_mode);
    let fingerprint = prompt_fingerprint(&options.prompt);
    let mut report = SemanticCacheSaveReport::default();
    for (source_file, mut group) in groups {
        let path = resolved_against_root(Path::new(&source_file), root);
        if !path.is_file() {
            report.skipped_not_file += 1;
            continue;
        }
        if allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&path))
        {
            report.skipped_out_of_scope += 1;
            continue;
        }
        let mut previous_partial = false;
        if options.merge_existing
            && let Some(previous) = cache.load(&path, &kind, Some(&fingerprint), false, true)?
        {
            previous_partial = previous.get("partial").and_then(Value::as_bool) == Some(true);
            prepend_cached_bucket(&mut group.nodes, &previous, "nodes");
            prepend_cached_bucket(&mut group.edges, &previous, "edges");
            prepend_cached_bucket(&mut group.hyperedges, &previous, "hyperedges");
        }
        let is_partial =
            partial.contains(&path) || cache_group_has_partial_marker(&group) || previous_partial;
        let mut entry = serde_json::json!({
            "nodes":group.nodes,
            "edges":group.edges,
            "hyperedges":group.hyperedges,
        });
        if is_partial {
            entry["partial"] = Value::Bool(true);
        }
        cache.save(&path, &entry, &kind, Some(&fingerprint))?;
        report.saved += 1;
    }
    cache.flush()?;
    Ok(report)
}

fn empty_semantic_result(model: Option<&str>) -> Value {
    serde_json::json!({
        "nodes":[],
        "edges":[],
        "hyperedges":[],
        "input_tokens":0,
        "output_tokens":0,
        "model":model,
        "finish_reason":"stop"
    })
}
