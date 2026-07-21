use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use trail_files::Cache;
use trail_semantic::{
    CachedCorpusExtractionOptions, CorpusExtractionOptions, SemanticCacheSaveOptions,
    SemanticError, SemanticUnit, check_semantic_cache, effective_semantic_concurrency,
    estimate_semantic_unit_tokens, expand_oversized_semantic_files, extract_corpus_cached_with,
    extract_corpus_parallel_with, extract_corpus_parallel_with_progress,
    extract_with_adaptive_retry, merge_semantic_results, pack_semantic_chunks,
    reconcile_semantic_scope, save_semantic_cache,
};

fn node_for(unit: &SemanticUnit, id: String) -> Value {
    json!({
        "id": id,
        "label": "Fixture",
        "source_file": unit.path().to_string_lossy()
    })
}

#[test]
fn packing_estimation_and_adaptive_retry_cover_slices_images_limits_and_partial_results()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let prose = directory.path().join("guide.md");
    let binary = directory.path().join("archive.pdf");
    let image = directory.path().join("diagram.PNG");
    fs::write(
        &prose,
        "# One\n".to_owned() + &"word ".repeat(80) + "\n# Two\nend\n",
    )?;
    fs::write(&binary, b"binary")?;
    fs::write(&image, b"image")?;

    let units =
        expand_oversized_semantic_files(&[prose.clone(), binary.clone(), image.clone()], 80);
    assert!(units.len() > 3);
    assert!(matches!(units.last(), Some(SemanticUnit::File(path)) if path == &image));
    assert!(estimate_semantic_unit_tokens(units.last().ok_or("missing image")?) > 1_000);
    assert_eq!(
        estimate_semantic_unit_tokens(&SemanticUnit::File(directory.path().join("missing.md"))),
        0
    );
    assert!(pack_semantic_chunks(&units, 0).is_err());
    assert!(pack_semantic_chunks(&units, 40)?.len() > 1);

    let calls = AtomicUsize::new(0);
    let merged = extract_with_adaptive_retry(&units[..4], Some("fixture"), 4, &|chunk| {
        calls.fetch_add(1, Ordering::Relaxed);
        if chunk.len() > 1 {
            return Err(SemanticError::Transport(
                "maximum context length exceeded".to_owned(),
            ));
        }
        Ok(json!({
            "nodes":[node_for(&chunk[0], format!("n{}", calls.load(Ordering::Relaxed)))],
            "edges":[],"hyperedges":[],"input_tokens":1,"output_tokens":2,
            "finish_reason":"stop"
        }))
    })?;
    assert_eq!(merged["nodes"].as_array().map(Vec::len), Some(4));
    assert_eq!(merged["input_tokens"], 4);

    let single = [SemanticUnit::File(binary.clone())];
    let partial = extract_with_adaptive_retry(&single, None, 0, &|_| {
        Ok(json!({
            "nodes":[],"edges":[],"hyperedges":[],"finish_reason":"length"
        }))
    })?;
    assert_eq!(
        partial["_partial_files"][0],
        binary.to_string_lossy().as_ref()
    );

    let swallowed = extract_with_adaptive_retry(&single, None, 0, &|_| {
        Err(SemanticError::Transport(
            "context window exceeded".to_owned(),
        ))
    })?;
    assert_eq!(swallowed["nodes"], json!([]));

    let propagated = extract_with_adaptive_retry(&single, None, 2, &|_| {
        Err(SemanticError::Transport("connection reset".to_owned()))
    });
    assert!(propagated.is_err());

    let explicit = merge_semantic_results(
        &json!({"nodes":[{"id":"left"}],"edges":[],"hyperedges":[],"input_tokens":2,"_partial_files":["a"]}),
        &json!({"nodes":[{"id":"right"}],"edges":[],"hyperedges":[],"output_tokens":3,"_partial_files":["b"]}),
        Some("fixture"),
    );
    assert_eq!(explicit["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(explicit["_partial_files"], json!(["a", "b"]));
    Ok(())
}

#[test]
fn reconciliation_and_parallel_extraction_cover_scope_failures_panics_and_ordering()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first = root.join("first.md");
    let second = root.join("second.md");
    let outside = root.join("outside.md");
    for path in [&first, &second, &outside] {
        fs::write(path, "content")?;
    }

    let mut fragment = json!({
        "nodes":[
            {"id":"keep","source_file":first.to_string_lossy()},
            {"id":"drop","source_file":outside.to_string_lossy()},
            {"id":"virtual"}
        ],
        "edges":[
            {"source":"keep","target":"virtual","source_file":first.to_string_lossy()},
            {"source":"keep","target":"drop","source_file":first.to_string_lossy()}
        ],
        "hyperedges":[
            {"id":"kept","nodes":["keep","virtual"],"source_file":first.to_string_lossy()},
            {"id":"dropped","nodes":["keep","drop"],"source_file":first.to_string_lossy()}
        ]
    });
    let reconciled =
        reconcile_semantic_scope(&mut fragment, &[first.clone(), second.clone()], root)?;
    assert_eq!(reconciled.out_of_scope_dropped, 1);
    assert_eq!(reconciled.dropped_files, [outside.to_string_lossy()]);
    assert_eq!(reconciled.uncovered_files, std::slice::from_ref(&second));
    assert_eq!(fragment["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(fragment["hyperedges"].as_array().map(Vec::len), Some(1));
    assert!(reconcile_semantic_scope(&mut json!([]), &[], root).is_err());
    assert!(
        reconcile_semantic_scope(
            &mut json!({"nodes":"bad","edges":[],"hyperedges":[]}),
            &[],
            root,
        )
        .is_err()
    );

    let options = CorpusExtractionOptions {
        backend_name: "custom".to_owned(),
        model: Some("fixture".to_owned()),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 4,
        max_retry_depth: 0,
    };
    assert_eq!(
        effective_semantic_concurrency("ollama", 8, 4, &HashMap::new()),
        1
    );
    assert_eq!(
        effective_semantic_concurrency(
            "ollama",
            8,
            4,
            &HashMap::from([("GRAPHIFY_OLLAMA_PARALLEL".to_owned(), "1".to_owned())]),
        ),
        4
    );
    assert_eq!(
        effective_semantic_concurrency("custom", 0, 0, &HashMap::new()),
        1
    );

    let progress = AtomicUsize::new(0);
    let mut on_done = |_: usize, _: usize, _: &[SemanticUnit], _: &Value| {
        progress.fetch_add(1, Ordering::Relaxed);
    };
    let result = extract_corpus_parallel_with_progress(
        &[first.clone(), second.clone(), outside.clone()],
        root,
        &options,
        &HashMap::new(),
        &|units| {
            let path = units[0].path();
            if path == second {
                return Err(SemanticError::Transport("fixture failure".to_owned()));
            }
            if path == outside {
                std::panic::resume_unwind(Box::new("fixture panic"));
            }
            Ok(json!({
                "nodes":[node_for(&units[0], "first".to_owned())],
                "edges":[],"hyperedges":[],"input_tokens":2,"output_tokens":1
            }))
        },
        &mut on_done,
    )?;
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    assert_eq!(result.failures.len(), 2);
    assert_eq!(result.fragment["failed_chunks"], 2);

    let invalid = CorpusExtractionOptions {
        chunk_size: 0,
        token_budget: None,
        ..CorpusExtractionOptions::default()
    };
    let mut ignore = |_: usize, _: usize, _: &[SemanticUnit], _: &Value| {};
    assert!(
        extract_corpus_parallel_with_progress(
            &[first],
            root,
            &invalid,
            &HashMap::new(),
            &|_| Ok(json!({})),
            &mut ignore,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn cached_corpus_pipeline_checkpoints_replays_and_filters_out_of_scope_entries()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first = root.join("first.md");
    let second = root.join("second.md");
    let outside = root.join("outside.md");
    fs::write(&first, "first")?;
    fs::write(&second, "second")?;
    fs::write(&outside, "outside")?;

    let options = CachedCorpusExtractionOptions {
        extraction: CorpusExtractionOptions {
            backend_name: "fixture".to_owned(),
            model: Some("fixture".to_owned()),
            chunk_size: 1,
            token_budget: None,
            max_concurrency: 2,
            max_retry_depth: 1,
        },
        prune_live_files: Some(vec![first.clone(), second.clone()]),
        ..CachedCorpusExtractionOptions::default()
    };
    let mut completed = 0;
    let mut progress = |_: usize, _: usize, _: &[SemanticUnit], _: &Value| completed += 1;
    let first_run = extract_corpus_cached_with(
        &[first.clone(), second.clone()],
        root,
        None,
        &options,
        &HashMap::new(),
        &|units| {
            Ok(json!({
                "nodes":[node_for(&units[0], format!("node-{}", units[0].path().display()))],
                "edges":[],"hyperedges":[],"input_tokens":1,"output_tokens":1
            }))
        },
        &mut progress,
    )?;
    assert_eq!(completed, 2);
    assert_eq!(first_run.cache_misses, 2);
    assert_eq!(first_run.finalized_files, 2);
    assert!(first_run.cache_issues.is_empty());

    let mut replay_progress = |_: usize, _: usize, _: &[SemanticUnit], _: &Value| {};
    let replay = extract_corpus_cached_with(
        &[first.clone(), second.clone()],
        root,
        None,
        &options,
        &HashMap::new(),
        &|_| Err(SemanticError::Transport("cache replay failed".to_owned())),
        &mut replay_progress,
    )?;
    assert_eq!(replay.cache_hits, 2);
    assert_eq!(replay.cache_misses, 0);
    assert_eq!(replay.fragment["nodes"].as_array().map(Vec::len), Some(2));

    let mut cache = Cache::new(root, None)?;
    let fragment = json!({
        "nodes":[
            {"id":"inside","source_file":first.to_string_lossy()},
            {"id":"outside","source_file":outside.to_string_lossy()},
            {"id":"missing","source_file":root.join("missing.md").to_string_lossy()}
        ],
        "edges":[{"source":"inside","target":"outside","source_file":first.to_string_lossy()}],
        "hyperedges":[{"id":"flow","nodes":["inside","outside"],"source_file":first.to_string_lossy()}]
    });
    let save_options = SemanticCacheSaveOptions {
        merge_existing: true,
        allowed_source_files: Some(vec![first.clone()]),
        partial_source_files: Vec::new(),
        deep_mode: false,
        prompt: "coverage prompt".to_owned(),
    };
    let report = save_semantic_cache(&mut cache, root, &fragment, &save_options)?;
    assert_eq!(report.saved, 1);
    assert_eq!(report.skipped_not_file, 1);
    assert_eq!(report.skipped_out_of_scope, 1);
    let checked = check_semantic_cache(&mut cache, &[first, outside], false, "coverage prompt")?;
    assert_eq!(checked.nodes.len(), 1);
    assert_eq!(checked.uncached.len(), 1);
    Ok(())
}

#[test]
fn slice_retry_cache_disabled_and_deep_partial_merges_cover_public_orchestration_edges()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let document = root.join("long.md");
    let outside = root.join("outside.md");
    fs::write(
        &document,
        "alpha beta gamma delta epsilon zeta eta theta iota kappa",
    )?;
    fs::write(&outside, "outside")?;

    let slices = trail_files::split_file(&document, 32)?;
    let first = slices.first().cloned().ok_or("missing semantic slice")?;
    let split = extract_with_adaptive_retry(
        &[SemanticUnit::Slice(first)],
        Some("fixture"),
        4,
        &|units| {
            let SemanticUnit::Slice(slice) = &units[0] else {
                return Err(SemanticError::Transport("expected slice".to_owned()));
            };
            if slice.end.saturating_sub(slice.start) > 12 {
                return Err(SemanticError::Transport(
                    "maximum context length exceeded".to_owned(),
                ));
            }
            Ok(json!({
                "nodes": [], "edges": [], "hyperedges": [],
                "input_tokens": 1, "output_tokens": 1,
                "finish_reason": "stop"
            }))
        },
    )?;
    assert!(split["input_tokens"].as_u64().unwrap_or_default() >= 2);

    let options = CorpusExtractionOptions {
        backend_name: "fixture".to_owned(),
        model: Some("fixture".to_owned()),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 0,
    };
    let parallel = extract_corpus_parallel_with(
        std::slice::from_ref(&document),
        root,
        &options,
        &HashMap::new(),
        &|units| {
            Ok(json!({
                "nodes": [node_for(&units[0], "parallel".to_owned())],
                "edges": [], "hyperedges": []
            }))
        },
    )?;
    assert!(parallel.failures.is_empty());

    let cached_options = CachedCorpusExtractionOptions {
        extraction: options,
        deep_mode: true,
        force: false,
        cache_enabled: false,
        prune_live_files: None,
    };
    let mut callbacks = 0;
    let disabled = extract_corpus_cached_with(
        std::slice::from_ref(&document),
        root,
        None,
        &cached_options,
        &HashMap::new(),
        &|units| {
            Ok(json!({
                "nodes": [node_for(&units[0], "uncached".to_owned())],
                "edges": [], "hyperedges": []
            }))
        },
        &mut |_, _, _, _| callbacks += 1,
    )?;
    assert_eq!(
        (disabled.cache_hits, disabled.cache_misses, callbacks),
        (0, 1, 1)
    );

    let mut malformed = json!({
        "nodes": [{"id":"outside","source_file":outside.to_string_lossy()}],
        "edges": [],
        "hyperedges": "invalid"
    });
    assert!(
        reconcile_semantic_scope(&mut malformed, std::slice::from_ref(&document), root).is_err()
    );

    let mut cache = Cache::new(root, None)?;
    let mut save_options = SemanticCacheSaveOptions::for_extraction(true);
    save_options.merge_existing = true;
    save_options.partial_source_files = vec![document.clone()];
    let first_fragment = json!({
        "nodes": [{"id":"first","source_file":document.to_string_lossy(),"_partial":true}],
        "edges": [], "hyperedges": []
    });
    assert_eq!(
        save_semantic_cache(&mut cache, root, &first_fragment, &save_options)?.saved,
        1
    );
    let second_fragment = json!({
        "nodes": [{"id":"second","source_file":document.to_string_lossy()}],
        "edges": [], "hyperedges": []
    });
    assert_eq!(
        save_semantic_cache(&mut cache, root, &second_fragment, &save_options)?.saved,
        1
    );
    let checked = check_semantic_cache(
        &mut cache,
        std::slice::from_ref(&document),
        true,
        &save_options.prompt,
    )?;
    assert_eq!(checked.uncached, vec![document]);
    Ok(())
}
