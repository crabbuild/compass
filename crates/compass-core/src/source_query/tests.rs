use super::*;
use crate::{BuildOptions, GraphStorage, build_local_graph};
use compass_languages::Engine;
use std::error::Error;
use std::fs;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn fixture(sources: &[(&str, &str)]) -> TestResult<(tempfile::TempDir, GraphDocument, Inputs)> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    for (file, source) in sources {
        fs::write(root.join(file), source)?;
        extractions.push(engine.extract_source(Path::new(file), source.as_bytes())?);
    }
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(d.path().join("native"));
    options.graph_storage = GraphStorage::Json;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    Ok((d, graph, collect(&mut extractions)?))
}

fn write(
    output: &Path,
    graph: &GraphDocument,
    inputs: &Inputs,
    prior: Option<SourceQueryArtifact>,
) -> TestResult<SourceQueryArtifact> {
    fs::create_dir_all(output)?;
    let graph_bytes = serde_json::to_vec(graph)?;
    fs::write(output.join("graph.json"), &graph_bytes)?;
    publish(
        output,
        graph,
        &format!("{:x}", Sha256::digest(&graph_bytes)),
        inputs,
        prior,
    )?;
    let artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(output.join(SOURCE_QUERY_FILE))?)?;
    artifact.validate()?;
    Ok(artifact)
}

#[test]
fn role_sites_follow_native_method_nested_sibling_and_arrow_owners() -> TestResult {
    let python = "class New:\n    def encode(self, payload):\n        return payload\n\ndef outer(block):\n    def inner(permit):\n        return permit\n    callback = lambda secret: secret\n    return block\n\ndef sibling(ticket):\n    return ticket\n";
    let arrow = "const probe = (block: Block) => block;";
    let (d, graph, inputs) = fixture(&[("sample.py", python), ("sample.ts", arrow)])?;
    let output = d.path().join("proof");
    let published = write(&output, &graph, &inputs, None)?;
    for (file, source, declaration, expected) in [
        ("sample.py", python, "def encode", vec!["self", "payload"]),
        ("sample.py", python, "def outer", vec!["block"]),
        ("sample.py", python, "def inner", vec!["permit"]),
        ("sample.py", python, "def sibling", vec!["ticket"]),
        ("sample.ts", arrow, "probe", vec!["block"]),
    ] {
        let start = source.find(declaration).ok_or("missing declaration")? as u64;
        let node = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind.is_callable()
                    && node
                        .source
                        .as_ref()
                        .is_some_and(|s| s.file == file && s.start_byte == start)
            })
            .ok_or("missing exact native callable")?;
        if declaration == "def encode" {
            assert_eq!(node.name, ".encode()");
            assert!(node.qualified_name.ends_with("New::encode"));
        }
        let owner = published
            .owners
            .iter()
            .find(|o| o.node_id == node.id)
            .ok_or("missing source owner")?;
        assert_eq!(node.source.as_ref(), Some(&owner.source));
        let bindings = owner
            .parameter_bindings
            .as_ref()
            .ok_or("missing role coverage")?;
        assert_eq!(
            bindings
                .sites
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for site in &bindings.sites {
            assert_eq!(
                &source.as_bytes()[site.source.start_byte as usize..site.source.end_byte as usize],
                site.text.as_bytes()
            );
        }
    }
    assert!(
        !published
            .owners
            .iter()
            .flat_map(|o| o.parameter_bindings.iter())
            .flat_map(|b| &b.sites)
            .any(|s| s.text == "secret")
    );
    Ok(())
}

#[test]
fn role_publication_rejects_envelope_only_sites_and_omits_nonunique_joins() -> TestResult {
    let (d, graph, mut inputs) =
        fixture(&[("sample.ts", "const probe = (block: Block) => block;")])?;
    let output = d.path().join("proof");
    let valid = write(&output, &graph, &inputs, None)?;
    assert_eq!(valid.owners.len(), 1);
    let before = fs::read(output.join(SOURCE_QUERY_FILE))?;
    let scope = inputs
        .files
        .get_mut("sample.ts")
        .ok_or("missing extraction")?
        .projection
        .scopes
        .first_mut()
        .ok_or("missing scope")?;
    let original = scope.parameter_bindings.sites[0].clone();
    assert!(scope.start < scope.callable_start);
    let binding = &mut scope.parameter_bindings.sites[0];
    binding.start = scope.start;
    binding.end = scope.start + 5;
    binding.start_point.column = scope.start;
    binding.end_point.column = scope.start + 5;
    binding.text = "probe".to_owned();
    let error = publish(&output, &graph, &"a".repeat(64), &inputs, None)
        .err()
        .ok_or("invalid envelope binding was accepted")?;
    assert!(error.to_string().contains("escapes its exact callable"));
    assert_eq!(fs::read(output.join(SOURCE_QUERY_FILE))?, before);
    inputs
        .files
        .get_mut("sample.ts")
        .ok_or("missing extraction")?
        .projection
        .scopes[0]
        .parameter_bindings
        .sites[0] = original;
    let owner_id = &valid.owners[0].node_id;
    let mut ambiguous = graph.clone();
    let mut duplicate = graph
        .nodes
        .iter()
        .find(|n| &n.id == owner_id)
        .ok_or("missing native owner")?
        .clone();
    duplicate.id = format!("sha256:{}", "f".repeat(64));
    ambiguous.nodes.push(duplicate);
    let omitted = write(&d.path().join("ambiguous"), &ambiguous, &inputs, None)?;
    assert!(omitted.owners.is_empty());
    assert_eq!(omitted.files[0].omitted_owners, 1);
    let mut missing = graph.clone();
    missing.nodes.retain(|n| &n.id != owner_id);
    assert!(
        write(&d.path().join("missing"), &missing, &inputs, None)?
            .owners
            .is_empty()
    );
    Ok(())
}

#[test]
fn mixed_prior_fresh_deleted_files_preserve_parameter_ownership_and_legacy_upgrade() -> TestResult {
    let kept = "def keep(block):\n    return block\n";
    let changed = "def change(permit):\n    return permit\n";
    let (old_d, old_graph, old_inputs) = fixture(&[
        ("kept.py", kept),
        ("changed.py", changed),
        ("deleted.py", "def remove(ticket):\n    return ticket\n"),
    ])?;
    let prior = write(&old_d.path().join("proof"), &old_graph, &old_inputs, None)?;
    let changed = format!("{changed}# fact neutral edit\n");
    let (d, graph, mut fresh) = fixture(&[("kept.py", kept), ("changed.py", &changed)])?;
    let clean = write(&d.path().join("clean"), &graph, &fresh, None)?;
    fresh.files.remove("kept.py");
    let mixed = write(&d.path().join("mixed"), &graph, &fresh, Some(prior.clone()))?;
    assert_eq!(mixed, clean);
    assert!(mixed.files.iter().all(|f| f.file != "deleted.py"));
    let kept_before = prior
        .owners
        .iter()
        .find(|o| o.source.file == "kept.py")
        .ok_or("missing original keep")?;
    assert_eq!(
        mixed.owners.iter().find(|o| o.source.file == "kept.py"),
        Some(kept_before)
    );
    let mut legacy = prior;
    legacy.profile = SOURCE_QUERY_PROFILE_V5.to_owned();
    for owner in &mut legacy.owners {
        owner.parameter_bindings = None;
    }
    legacy.validate()?;
    let error = publish(
        &d.path().join("mixed"),
        &graph,
        &"a".repeat(64),
        &fresh,
        Some(legacy.clone()),
    )
    .err()
    .ok_or("legacy missing-file provenance was synthesized")?;
    assert!(error.to_string().contains("update --force"));
    fresh.files.insert(
        "kept.py".to_owned(),
        old_inputs
            .files
            .get("kept.py")
            .ok_or("missing kept facts")?
            .clone(),
    );
    assert_eq!(
        write(&d.path().join("upgrade"), &graph, &fresh, Some(legacy))?,
        clean
    );
    Ok(())
}

#[test]
fn role_aggregate_omissions_preserve_lexical_quota_and_disable_lossy_reuse() -> TestResult {
    let parameters = (0..256)
        .map(|i| format!("arg{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let source = (0..128)
        .map(|i| format!("def probe{i}({parameters}):\n    pass\n"))
        .collect::<String>();
    let (d, graph, inputs) = fixture(&[("a.py", &source), ("b.py", &source)])?;
    let output = d.path().join("proof");
    let mut result = write(&output, &graph, &inputs, None)?;
    assert_eq!(result.owners.len(), 256);
    assert_eq!(
        result
            .owners
            .iter()
            .map(|o| o.identifiers.len())
            .sum::<usize>(),
        SOURCE_QUERY_MAX_SITES
    );
    assert!(result.owners.iter().all(|o| !o.truncated));
    for owner in &result.owners {
        let bindings = owner
            .parameter_bindings
            .as_ref()
            .ok_or("missing coverage")?;
        assert!(bindings.sites.is_empty());
        assert_eq!(bindings.status, ParameterBindingStatus::Partial);
        assert_eq!(bindings.omitted_sites, 256);
        assert!(
            bindings
                .reasons
                .contains(&ParameterBindingOmission::SiteLimit)
        );
        assert!(
            bindings
                .reasons
                .contains(&ParameterBindingOmission::AggregateLimit)
        );
    }
    // Global omissions cannot be reused if deletion/edit frees capacity: the
    // omitted sites must be recovered from fresh/cached extraction instead.
    assert!(reusable(&output).is_none());
    for owner in &mut result.owners {
        let bindings = owner
            .parameter_bindings
            .as_mut()
            .ok_or("missing coverage")?;
        bindings
            .reasons
            .remove(&ParameterBindingOmission::AggregateLimit);
        bindings.omitted_sites = 192;
    }
    result.validate()?;
    fs::write(output.join(SOURCE_QUERY_FILE), serde_json::to_vec(&result)?)?;
    assert!(
        reusable(&output).is_some(),
        "per-owner omissions do not depend on other files' quotas"
    );
    Ok(())
}

#[test]
fn underscore_bindings_are_language_specific_in_published_evidence() -> TestResult {
    let sources = [
        ("p.py", "def probe(_):\n    return _\n"),
        ("t.ts", "function probe(_: Value) { return _; }"),
        ("x.tsx", "const probe = (_: Value) => <Panel item={_}/>;"),
        ("g.go", "package p\nfunc probe(_ Value) {}"),
        ("r.rs", "fn probe(_: Value) {}"),
    ];
    let (d, graph, inputs) = fixture(&sources)?;
    let result = write(&d.path().join("proof"), &graph, &inputs, None)?;
    assert_eq!(result.owners.len(), 5);
    for owner in &result.owners {
        let bindings = owner
            .parameter_bindings
            .as_ref()
            .ok_or("missing coverage")?;
        let expected = if matches!(owner.source.file.as_str(), "g.go" | "r.rs") {
            vec![]
        } else {
            vec!["_"]
        };
        assert_eq!(
            bindings
                .sites
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(bindings.status, ParameterBindingStatus::Complete);
        assert_eq!(bindings.omitted_sites, 0);
        assert!(bindings.reasons.is_empty());
        for site in &bindings.sites {
            let source = sources
                .iter()
                .find(|(file, _)| *file == owner.source.file)
                .ok_or("missing input")?
                .1;
            assert_eq!(
                &source.as_bytes()[site.source.start_byte as usize..site.source.end_byte as usize],
                b"_"
            );
        }
    }
    Ok(())
}

fn native_options(root: &Path, output: &Path) -> BuildOptions {
    let mut options = BuildOptions::new(root);
    options.output_root = Some(output.to_path_buf());
    options.graph_storage = GraphStorage::Json;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    options
}

fn read_artifact(output: &Path) -> TestResult<SourceQueryArtifact> {
    let result: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(output.join(SOURCE_QUERY_FILE))?)?;
    result.validate()?;
    Ok(result)
}

fn same_artifact(mut left: SourceQueryArtifact, mut right: SourceQueryArtifact) -> TestResult {
    left.graph_digest = "a".repeat(64);
    right.graph_digest = "a".repeat(64);
    // Compare every serialized artifact byte; keep a failed assertion bounded.
    assert_eq!(
        Sha256::digest(serde_json::to_vec(&left)?),
        Sha256::digest(serde_json::to_vec(&right)?)
    );
    Ok(())
}

#[test]
fn first_shared_site_exhaustion_and_deletion_match_clean_cached_publication() -> TestResult {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let parameters = (0..192)
        .map(|i| format!("arg{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let source = (0..128)
        .map(|i| format!("def probe{i}({parameters}):\n    pass\n"))
        .collect::<String>();
    for file in ["a.py", "b.py"] {
        fs::write(root.join(file), &source)?;
    }
    let options = native_options(&root, &d.path().join("native"));
    let first = build_local_graph(&options)?;
    let prior = read_artifact(&first.output_dir)?;
    assert!(reusable(&first.output_dir).is_some());
    assert_eq!(
        prior
            .owners
            .iter()
            .map(|o| o.identifiers.len()
                + o.comments.len()
                + o.parameter_bindings.as_ref().map_or(0, |b| b.sites.len()))
            .sum::<usize>(),
        SOURCE_QUERY_MAX_SITES
    );
    assert!(
        !prior
            .owners
            .iter()
            .any(|o| o.parameter_bindings.as_ref().is_some_and(|b| b
                .reasons
                .contains(&ParameterBindingOmission::AggregateLimit)))
    );
    let mut owner_files = prior.owners.iter().map(|o| o.source.file.as_str());
    let mut last = owner_files.next();
    let mut reversed = false;
    for file in owner_files {
        reversed |= last.is_some_and(|previous| previous > file);
        last = Some(file);
    }
    assert!(
        reversed,
        "fixture must oppose filename and owner-ID allocation orders"
    );
    let changed = source.replacen("    pass\n", "    pass\n    # neutral\n", 1);
    fs::write(root.join("a.py"), &changed)?;
    let mut engine = Engine::default();
    let mut extraction = [engine.extract_source(Path::new("a.py"), changed.as_bytes())?];
    let fresh = collect(&mut extraction)?;
    assert!(
        !can_reuse_with_inputs(&prior, &fresh),
        "first shared-site exhaustion must use all current cached facts"
    );
    let before = fs::read(first.output_dir.join(SOURCE_QUERY_FILE))?;
    let graph = GraphDocument::load(&first.output_dir.join("graph.json"))?;
    assert!(
        publish(
            &first.output_dir,
            &graph,
            &prior.graph_digest,
            &fresh,
            Some(prior.clone())
        )
        .is_err()
    );
    assert_eq!(
        Sha256::digest(fs::read(first.output_dir.join(SOURCE_QUERY_FILE))?),
        Sha256::digest(before)
    );
    let updated = build_local_graph(&options)?;
    assert!(updated.files_cached > 0);
    let current = read_artifact(&updated.output_dir)?;
    assert!(
        current
            .owners
            .iter()
            .any(|o| o.parameter_bindings.as_ref().is_some_and(|b| b
                .reasons
                .contains(&ParameterBindingOmission::AggregateLimit)))
    );
    let clean = build_local_graph(&native_options(&root, &d.path().join("clean")))?;
    same_artifact(current, read_artifact(&clean.output_dir)?)?;
    fs::remove_file(root.join("b.py"))?;
    let deleted = build_local_graph(&options)?;
    let recovered = read_artifact(&deleted.output_dir)?;
    assert!(
        recovered
            .owners
            .iter()
            .all(|o| o
                .parameter_bindings
                .as_ref()
                .is_some_and(|b| b.sites.len() == 64
                    && !b
                        .reasons
                        .contains(&ParameterBindingOmission::AggregateLimit)))
    );
    let clean = build_local_graph(&native_options(&root, &d.path().join("clean-deleted")))?;
    same_artifact(recovered, read_artifact(&clean.output_dir)?)?;
    Ok(())
}

#[test]
fn first_shared_text_exhaustion_falls_back_before_mixed_publication() -> TestResult {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let parameters = (0..16)
        .map(|i| format!("parameter_{i:02}_{}", "x".repeat(19)))
        .collect::<Vec<_>>()
        .join(", ");
    let comment = format!("    #{}\n", "x".repeat(4095));
    let source = |comments: usize| {
        (0..29)
            .map(|i| {
                format!(
                    "def probe{i}({parameters}):\n{}    pass\n",
                    comment.repeat(comments)
                )
            })
            .collect::<String>()
    };
    let original = source(7);
    for i in 0..19 {
        fs::write(root.join(format!("file{i:02}.py")), &original)?;
    }
    let options = native_options(&root, &d.path().join("native"));
    let first = build_local_graph(&options)?;
    let prior = read_artifact(&first.output_dir)?;
    assert!(reusable(&first.output_dir).is_some());
    assert!(prior.owners.iter().all(|o| {
        o.parameter_bindings
            .as_ref()
            .is_some_and(|b| b.status == ParameterBindingStatus::Complete)
    }));
    let changed = source(8);
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    for i in 0..4 {
        let name = format!("file{i:02}.py");
        fs::write(root.join(&name), &changed)?;
        extractions.push(engine.extract_source(Path::new(&name), changed.as_bytes())?);
    }
    let fresh = collect(&mut extractions)?;
    assert!(!can_reuse_with_inputs(&prior, &fresh));
    let updated = build_local_graph(&options)?;
    assert!(updated.files_cached >= 15);
    let current = read_artifact(&updated.output_dir)?;
    assert!(
        !current.aggregate_truncated,
        "only role text should cross the shared ceiling"
    );
    assert!(
        current
            .owners
            .iter()
            .any(|o| o.parameter_bindings.as_ref().is_some_and(|b| b
                .reasons
                .contains(&ParameterBindingOmission::AggregateLimit)))
    );
    let clean = build_local_graph(&native_options(&root, &d.path().join("clean")))?;
    same_artifact(current, read_artifact(&clean.output_dir)?)?;
    fs::remove_file(root.join("file18.py"))?;
    let deleted = build_local_graph(&options)?;
    let recovered = read_artifact(&deleted.output_dir)?;
    assert!(recovered.owners.iter().all(|o| {
        o.parameter_bindings
            .as_ref()
            .is_some_and(|b| b.status == ParameterBindingStatus::Complete)
    }));
    let clean = build_local_graph(&native_options(&root, &d.path().join("clean-deleted")))?;
    same_artifact(recovered, read_artifact(&clean.output_dir)?)?;

    // Include zero-owner files in the conservative text bound, with exact
    // equality and +1 controls. This proves eligibility, not source recall.
    let mut empty = SourceQueryArtifact::empty("a".repeat(64));
    empty.files = prior.files[..16].to_vec();
    empty.validate()?;
    assert!(can_reuse_with_inputs(&empty, &Inputs::default()));
    empty.files.push(prior.files[16].clone());
    empty.validate()?;
    assert!(!can_reuse_with_inputs(&empty, &Inputs::default()));
    empty.files.pop();
    let mut one_byte =
        [engine.extract_source(Path::new("fresh.py"), b"def f():\n    #\n    pass\n")?];
    let one_byte = collect(&mut one_byte)?;
    assert_eq!(one_byte.files["fresh.py"].projection.retained_bytes, 1);
    assert!(!can_reuse_with_inputs(&empty, &one_byte));
    let mut oversized = prior;
    oversized.owners[0].comments[0].text = "x".repeat(SOURCE_QUERY_MAX_FILE_TEXT_BYTES + 1);
    assert!(!can_reuse_with_inputs(&oversized, &Inputs::default()));
    Ok(())
}

#[test]
fn cached_text_counters_are_verified_before_collection_pruning() -> TestResult {
    let mut engine = Engine::default();
    let extraction = engine.extract_source(
        Path::new("sample.py"),
        b"def probe(block):\n    return block\n",
    )?;
    let inputs = collect(&mut [extraction.clone()])?;
    let file = inputs
        .files
        .get("sample.py")
        .ok_or("missing coherent native input")?;
    assert!(file.projection.retained_bytes > 0);
    assert!(file.projection.retained_parameter_bytes > 0);
    assert!(file.projection.retained_code_bytes > 0);
    for field in [
        "retainedBytes",
        "retainedParameterBytes",
        "retainedCodeBytes",
    ] {
        let declared = extraction.extensions[raw::EXTRACTION_EXTENSION]["projection"][field]
            .as_u64()
            .ok_or("missing native counter")?;
        for wrong in [0, declared + 1] {
            let mut corrupted = extraction.clone();
            corrupted
                .extensions
                .get_mut(raw::EXTRACTION_EXTENSION)
                .ok_or("missing cached native evidence")?["projection"][field] =
                serde_json::json!(wrong);
            let error = collect(&mut [corrupted])
                .err()
                .ok_or("corrupted cached counter was accepted")?;
            assert!(error.to_string().contains("accounting"), "{error}");
        }
    }
    // A correctly declared role counter cannot hide understated generic text.
    let mut corrupted = extraction;
    corrupted
        .extensions
        .get_mut(raw::EXTRACTION_EXTENSION)
        .ok_or("missing native evidence")?["projection"]["retainedBytes"] = serde_json::json!(0);
    assert!(collect(&mut [corrupted]).is_err());
    Ok(())
}

#[test]
fn cached_fallback_keeps_newline_edit_inventory_coherent() -> TestResult {
    use compass_files::{Cache, CacheKind, CacheOptions};
    for storage in [GraphStorage::Json, GraphStorage::Sqlite] {
        for sibling_count in [4, 19] {
            for preexisting_content in [false, true] {
                let d = tempfile::tempdir()?;
                let root = d.path().join("repo");
                fs::create_dir(&root)?;
                for i in 0..sibling_count {
                    fs::write(
                        root.join(format!("sibling{i:02}.py")),
                        format!("def probe{i}(block):\n    return block\n"),
                    )?;
                }
                let edited = root.join("sample.rs");
                let original = b"pub fn probe(block: usize) -> usize { block }\n";
                let mut changed = original.to_vec();
                changed.push(b'\n');
                fs::write(&edited, original)?;
                let cache_root = d.path().join("private-cache");
                let mut options = native_options(&root, &d.path().join("native"));
                options.graph_storage = storage;
                options.cache_root = Some(cache_root.clone());
                let first = build_local_graph(&options)?;
                let original_graph = fs::read(first.output_dir.join("graph.json"))?;
                let prior = read_artifact(&first.output_dir)?;
                assert!(reusable(&first.output_dir).is_some());
                let mut engine = Engine::default();
                let fresh =
                    collect(&mut [engine.extract_source(Path::new("sample.rs"), &changed)?])?;
                assert_eq!(can_reuse_with_inputs(&prior, &fresh), sibling_count <= 16);
                if preexisting_content {
                    fs::write(&edited, &changed)?;
                    let mut warm_options = options.clone();
                    warm_options.output_root = Some(d.path().join("prewarm"));
                    build_local_graph(&warm_options)?;
                    fs::write(&edited, original)?;
                }
                let sibling = root.join("sibling00.py");
                let sibling_cache = {
                    let mut cache = Cache::open(&root, CacheOptions::shared_history(&cache_root))?;
                    assert!(cache.load_portable_ast(&sibling, true)?.is_some());
                    let keys = cache.source_cache_keys(std::slice::from_ref(&sibling))?;
                    let key = keys.first().ok_or("missing sibling cache key")?;
                    let entry = cache
                        .directory(&CacheKind::Ast, None)
                        .join(format!("{key}.msgpack"));
                    fs::remove_file(&entry)?;
                    assert!(cache.load_portable_ast(&sibling, true)?.is_none());
                    entry
                };
                fs::write(&edited, &changed)?;
                {
                    let mut cache = Cache::open(&root, CacheOptions::shared_history(&cache_root))?;
                    assert_eq!(
                        cache.load_portable_ast(&edited, true)?.is_some(),
                        preexisting_content
                    );
                }
                let incremental = build_local_graph(&options)?;
                assert_eq!(incremental.files_extracted, 1);
                // On rejection, only B is a normal-stage miss: A was read and
                // cached by the early stage. With accepted reuse B is untouched.
                assert_eq!(sibling_cache.exists(), sibling_count > 16);
                let graph = GraphDocument::load(&incremental.output_dir.join("graph.json"))?;
                let file = graph
                    .graph
                    .files
                    .iter()
                    .find(|f| f.path == "sample.rs")
                    .ok_or("missing edited file")?;
                assert_eq!(file.byte_size, changed.len() as u64);
                assert_eq!(
                    file.content_digest,
                    format!("sha256:{:x}", Sha256::digest(&changed))
                );
                let mut clean_options = options.clone();
                clean_options.output_root = Some(d.path().join("clean"));
                clean_options.force = true;
                let clean = build_local_graph(&clean_options)?;
                assert_eq!(
                    Sha256::digest(fs::read(incremental.output_dir.join("graph.json"))?),
                    Sha256::digest(fs::read(clean.output_dir.join("graph.json"))?),
                );
                same_artifact(
                    read_artifact(&incremental.output_dir)?,
                    read_artifact(&clean.output_dir)?,
                )?;
                fs::write(&edited, original)?;
                // Exercise restored-content hits with the same other-miss condition.
                if sibling_cache.exists() {
                    fs::remove_file(&sibling_cache)?;
                }
                let restored = build_local_graph(&options)?;
                assert_eq!(
                    Sha256::digest(&original_graph),
                    Sha256::digest(fs::read(restored.output_dir.join("graph.json"))?)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn inventory_mismatch_diagnostic_does_not_render_untrusted_digest_text() -> TestResult {
    let (d, graph, mut inputs) =
        fixture(&[("sample.py", "def probe(block):\n    return block\n")])?;
    let output = d.path().join("proof");
    write(&output, &graph, &inputs, None)?;
    let before = fs::read(output.join(SOURCE_QUERY_FILE))?;
    inputs
        .files
        .get_mut("sample.py")
        .ok_or("missing source input")?
        .source_digest = format!("\u{1b}[31m{}", "untrusted".repeat(131_072));
    let error = write(&output, &graph, &inputs, None)
        .err()
        .ok_or("digest mismatch accepted")?;
    let rendered = error.to_string();
    assert!(
        rendered.len() < 1024,
        "diagnostic length {}",
        rendered.len()
    );
    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains("untrusted"));
    assert!(rendered.contains("sample.py") && rendered.contains("run compass update --force"));
    assert_eq!(
        Sha256::digest(before),
        Sha256::digest(fs::read(output.join(SOURCE_QUERY_FILE))?)
    );
    Ok(())
}

#[test]
fn inventory_mismatch_diagnostic_bounds_and_escapes_path_context() {
    for path in [
        "a".repeat(1_048_576),
        "\u{1b}[31m".repeat(131_072),
        "\u{10ffff}".repeat(131_072),
    ] {
        let rendered = inventory_mismatch(&path, 47, 46).to_string();
        assert!(
            rendered.len() < 4096,
            "diagnostic length {}",
            rendered.len()
        );
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains("\"...: extracted 47 bytes, published 46 bytes"));
        assert!(rendered.contains("run compass update --force"));
    }
    let path = format!("{}🧭end", "a".repeat(127));
    let rendered = inventory_mismatch(&path, 47, 46).to_string();
    assert!(rendered.contains("🧭\"..."));
    assert!(!rendered.contains("end"));
    let rendered = inventory_mismatch("folder/café/🧭.py", 47, 46).to_string();
    assert!(rendered.contains("\"folder/café/🧭.py\": extracted"));
    assert!(!rendered.contains("..."));
}
