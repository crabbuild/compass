use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_model::source_query::{SOURCE_QUERY_FILE, SourceQueryArtifact};
use compass_query::{EngineSelection, RelevanceBand, open_with_engine};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::path::Path;
fn request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}
fn options(root: &Path, output: &Path, storage: GraphStorage) -> BuildOptions {
    let mut o = BuildOptions::new(root);
    o.output_root = Some(output.to_path_buf());
    o.no_cluster = true;
    o.no_viz = true;
    o.max_workers = Some(1);
    o.graph_storage = storage;
    o.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    o
}
fn artifact(output: &Path) -> Result<SourceQueryArtifact, Box<dyn Error>> {
    let a: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(output.join(SOURCE_QUERY_FILE))?)?;
    a.validate()?;
    assert_eq!(
        a.graph_digest,
        format!("{:x}", Sha256::digest(fs::read(output.join("graph.json"))?))
    );
    Ok(a)
}

#[test]
fn direct_variable_callable_bodies_bind_and_query_on_both_backends() -> Result<(), Box<dyn Error>> {
    for extension in ["ts", "tsx"] {
        for (form, initializer) in [
            (
                "arrow",
                "(value) => { value = value; // Handle payload normalization here.\n return value; }",
            ),
            (
                "named",
                "function internal(value) { value = value; // Handle payload normalization here.\n return value; }",
            ),
            (
                "generator",
                "function* (value) { value = value; // Handle payload normalization here.\n yield value; }",
            ),
        ] {
            let d = tempfile::tempdir()?;
            let root = d.path().join("repo");
            fs::create_dir(&root)?;
            let file = format!("{form}.{extension}");
            let source = format!("export const opaque = {initializer};\n");
            fs::write(root.join(&file), &source)?;
            let built = build_local_graph(&options(
                &root,
                &d.path().join("artifacts"),
                GraphStorage::Sqlite,
            ))?;
            let a = artifact(&built.output_dir)?;
            let graph_path = built.output_dir.join("graph.json");
            let graph = GraphDocument::load(&graph_path)?;
            let node = graph
                .nodes
                .iter()
                .find(|node| {
                    node.kind.is_callable() && node.name.trim_end_matches("()") == "opaque"
                })
                .ok_or("missing callable binding")?;
            let owner = a
                .owners
                .iter()
                .find(|owner| owner.node_id == node.id)
                .ok_or("initializer body did not bind")?;
            assert_eq!(Some(&owner.source), node.source.as_ref());
            assert_eq!(
                &source[owner.source.start_byte as usize..owner.source.end_byte as usize],
                format!("opaque = {initializer}")
            );
            let mut previous = None;
            for mode in [EngineSelection::Json, EngineSelection::Store] {
                let engine = open_with_engine(
                    &graph_path,
                    None,
                    &d.path().join(format!("cache-{mode:?}")),
                    mode,
                )?;
                let response = engine.discover_ranked(request("Where is payload handled?"))?;
                let candidate = response
                    .candidates
                    .first()
                    .ok_or("missing body candidate")?;
                assert_eq!(candidate.node.id, node.id, "{file}");
                assert_eq!(candidate.relevance, RelevanceBand::Low);
                assert!(
                    candidate
                        .matched_fields
                        .iter()
                        .any(|field| field == "body_comment")
                );
                assert!(candidate.supporting_sources.iter().any(|anchor| {
                    anchor.file == file
                        && source
                            .get(anchor.start_byte as usize..anchor.end_byte as usize)
                            .is_some_and(|text| text.contains("payload normalization"))
                }));
                assert!(!candidate.excerpts.is_empty(), "{file}");
                for quote in &candidate.excerpts {
                    assert!(matches!(
                        quote.kind,
                        compass_query::RankedExcerptKind::CallableComment
                            | compass_query::RankedExcerptKind::CallableCode
                    ));
                    assert!(quote.is_coherent());
                    assert_eq!(
                        source
                            .get(quote.source.start_byte as usize..quote.source.end_byte as usize),
                        Some(quote.text.as_str())
                    );
                    assert!(quote.text.contains("payload"));
                }
                // A newly opened engine must quote the immutable artifact even
                // when current source has changed since graph publication.
                fs::write(root.join(&file), "// modified after publication\n")?;
                let pinned = open_with_engine(
                    &graph_path,
                    None,
                    &d.path().join(format!("cache-mutated-{mode:?}")),
                    mode,
                )?;
                assert_eq!(
                    pinned
                        .discover_ranked(request("Where is payload handled?"))?
                        .candidates,
                    response.candidates
                );
                fs::write(root.join(&file), &source)?;
                if let Some(previous) = &previous {
                    assert_eq!(previous, &response.candidates);
                }
                previous = Some(response.candidates);
            }
        }
    }
    Ok(())
}

#[test]
fn native_update_body_only_query_is_shared_by_json_and_store() -> Result<(), Box<dyn Error>> {
    for (file, source) in [
        (
            "sample.py",
            "def opaque(value):\n    value = value\n    # Handle payload normalization here.\n    return value\ndef decoy(value):\n    return value\n",
        ),
        (
            "leading_comment.py",
            "def opaque(value):\n    # Handle payload normalization here.\n    return value\ndef decoy(value):\n    return value\n",
        ),
        (
            "sample.go",
            "package sample\nfunc opaque(value string) string { // Handle payload normalization here.\nreturn value }\nfunc decoy(value string) string { return value }\n",
        ),
        (
            "sample.rs",
            "fn opaque(value: &str) -> &str { // Handle payload normalization here.\nvalue }\nfn decoy(value: &str) -> &str { value }\n",
        ),
        (
            "sample.ts",
            "export function opaque(value: unknown) { // Handle payload normalization here.\nreturn value; }\nfunction decoy(value: unknown) { return value; }\n",
        ),
        (
            "sample.tsx",
            "export function opaque(value: unknown) { // Handle payload normalization here.\nreturn <span>{String(value)}</span>; }\nfunction decoy(value: unknown) { return value; }\n",
        ),
    ] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        fs::write(root.join(file), source)?;
        let o = options(&root, &d.path().join("artifacts"), GraphStorage::Sqlite);
        let built = build_local_graph(&o)?;
        let a = artifact(&built.output_dir)?;
        let graph_path = built.output_dir.join("graph.json");
        let graph = GraphDocument::load(&graph_path)?;
        let expected = graph
            .nodes
            .iter()
            .find(|n| n.kind.is_callable() && n.name.trim_end_matches("()") == "opaque")
            .ok_or("missing opaque")?;
        assert!(
            a.owners.iter().any(|owner| owner.node_id == expected.id),
            "{file}: {a:?}"
        );
        let mut previous = None;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine = open_with_engine(
                &graph_path,
                None,
                &d.path().join(format!("cache-{mode:?}")),
                mode,
            )?;
            let response = engine.discover_ranked(request("Where is payload handled?"))?;
            let candidate = response.candidates.first().ok_or_else(|| {
                format!(
                    "{file}: no candidates: {:?}",
                    response.discovery.diagnostics
                )
            })?;
            assert_eq!(candidate.node.id, expected.id, "{file}");
            assert_eq!(
                candidate.relevance,
                RelevanceBand::Low,
                "{file}: {candidate:?}"
            );
            assert!(
                candidate.matched_fields.iter().any(|f| f == "body_comment"),
                "{file}"
            );
            assert!(
                candidate.supporting_sources.iter().any(|s| s.file == file
                    && source
                        .get(s.start_byte as usize..s.end_byte as usize)
                        .is_some_and(|t| t.contains("payload"))),
                "{file}"
            );
            assert!(
                response.candidates.iter().all(|r| r.node.id == expected.id),
                "{file}: irrelevant candidates"
            );
            assert!(
                engine
                    .discover_ranked(request("Where is unicorn handled?"))?
                    .candidates
                    .is_empty()
            );
            assert_eq!(
                response.candidates,
                engine
                    .discover_ranked(request("Where is payload handled?"))?
                    .candidates
            );
            if let Some(previous) = previous {
                assert_eq!(response.candidates, previous, "{file}: backend mismatch");
            }
            previous = Some(response.candidates);
        }
    }
    Ok(())
}

#[test]
fn publication_refreshes_neutral_edits_preserves_other_files_and_repairs_corruption()
-> Result<(), Box<dyn Error>> {
    for storage in [GraphStorage::Json, GraphStorage::Sqlite] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        let original = "fn opaque(value: &str) -> &str { // Handle payload.\nvalue }\n";
        fs::write(root.join("sample.rs"), original)?;
        fs::write(
            root.join("other.rs"),
            "fn other() { // Check invariant.\n}\n",
        )?;
        let mut o = options(&root, &d.path().join("artifacts"), storage);
        let cold = build_local_graph(&o)?;
        let first = artifact(&cold.output_dir)?;
        let first_bytes = fs::read(cold.output_dir.join(SOURCE_QUERY_FILE))?;
        let warm = build_local_graph(&o)?;
        assert!(!warm.outputs_changed);
        assert_eq!(
            first_bytes,
            fs::read(warm.output_dir.join(SOURCE_QUERY_FILE))?
        );
        fs::write(
            root.join("sample.rs"),
            format!("{original}// outside declaration\n"),
        )?;
        let updated = build_local_graph(&o)?;
        let after = artifact(&updated.output_dir)?;
        assert_ne!(first.graph_digest, after.graph_digest);
        assert_eq!(
            first
                .owners
                .iter()
                .filter(|o| o.source.file == "other.rs")
                .collect::<Vec<_>>(),
            after
                .owners
                .iter()
                .filter(|o| o.source.file == "other.rs")
                .collect::<Vec<_>>()
        );
        let preserved = after.clone();
        o.force = true;
        let rebuilt = build_local_graph(&o)?;
        assert_eq!(preserved, artifact(&rebuilt.output_dir)?);
        o.force = false;
        fs::write(rebuilt.output_dir.join(SOURCE_QUERY_FILE), b"corrupt")?;
        assert!(
            open_with_engine(
                &rebuilt.output_dir.join("graph.json"),
                None,
                &d.path().join("broken-query"),
                EngineSelection::Default
            )
            .is_err()
        );
        let repaired = build_local_graph(&o)?;
        artifact(&repaired.output_dir)?;
        fs::remove_file(root.join("other.rs"))?;
        let deleted = build_local_graph(&o)?;
        let a = artifact(&deleted.output_dir)?;
        assert!(a.files.iter().all(|f| f.file != "other.rs"));
        assert!(a.owners.iter().all(|f| f.source.file != "other.rs"));
    }
    Ok(())
}

#[test]
fn comment_free_implementation_has_immutable_code_excerpt_on_both_backends()
-> Result<(), Box<dyn Error>> {
    for (file, source) in [
        (
            "sample.py",
            "def opaque(items):\n    return normalize_payload(items)\ndef decoy(items):\n    return items\n",
        ),
        (
            "sample.rs",
            "fn opaque(items: usize) -> usize { normalize_payload(items) }\nfn decoy(items: usize) -> usize { items }\n",
        ),
        (
            "sample.go",
            "package sample\nfunc opaque(items int) int { return normalize_payload(items) }\nfunc decoy(items int) int { return items }\n",
        ),
        (
            "sample.ts",
            "export function opaque(items: unknown) { return normalize_payload(items); }\nfunction decoy(items: unknown) { return items; }\n",
        ),
        (
            "sample.tsx",
            "export const opaque = (items: unknown) => normalize_payload(items);\nfunction decoy(items: unknown) { return items; }\n",
        ),
    ] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        fs::write(root.join(file), source)?;
        let built = build_local_graph(&options(
            &root,
            &d.path().join("artifacts"),
            GraphStorage::Sqlite,
        ))?;
        let graph_path = built.output_dir.join("graph.json");
        let mut previous = None;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine = open_with_engine(
                &graph_path,
                None,
                &d.path().join(format!("cache-code-{mode:?}")),
                mode,
            )?;
            let response = engine.discover_ranked(request("where is payload normalized"))?;
            let candidate = response.candidates.first().ok_or("no candidate")?;
            assert!(candidate.node.qualified_name.contains("opaque"));
            assert!(
                candidate.excerpts.iter().any(|quote| {
                    serde_json::to_value(quote)
                        .ok()
                        .is_some_and(|value| value["kind"] == "callable_code")
                        && quote.text.contains("normalize_payload(items)")
                }),
                "comment-free implementation must show retained code: {:?}",
                candidate.excerpts
            );
            for quote in &candidate.excerpts {
                assert!(quote.is_coherent());
                assert_eq!(
                    source.get(quote.source.start_byte as usize..quote.source.end_byte as usize),
                    Some(quote.text.as_str())
                );
            }
            fs::write(root.join(file), "# current source is different\n")?;
            let pinned = open_with_engine(
                &graph_path,
                None,
                &d.path().join(format!("cache-code-pinned-{mode:?}")),
                mode,
            )?;
            assert_eq!(
                pinned
                    .discover_ranked(request("where is payload normalized"))?
                    .candidates,
                response.candidates
            );
            fs::write(root.join(file), source)?;
            if let Some(previous) = &previous {
                assert_eq!(previous, &response.candidates);
            }
            previous = Some(response.candidates);
        }
    }
    Ok(())
}

#[test]
fn nested_formatter_context_is_exact_separate_and_equal_on_json_and_store()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("repo");
    fs::create_dir(&root)?;
    let source = "fn opaque(diagnostics: &[Diagnostic]) -> Vec<String> {\n diagnostics.iter().map(|diagnostic| {\n  format!(\"{}: {}\", diagnostic.code(), diagnostic.message())\n }).collect()\n}\n";
    fs::write(root.join("formatter.rs"), source)?;
    let built = build_local_graph(&options(
        &root,
        &directory.path().join("artifacts"),
        GraphStorage::Sqlite,
    ))?;
    let a = artifact(&built.output_dir)?;
    let graph_path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.kind.is_callable() && node.name.trim_end_matches("()") == "opaque")
        .ok_or("missing formatter")?;
    let owner = a
        .owners
        .iter()
        .find(|owner| owner.node_id == target.id)
        .ok_or("missing owner")?;
    assert!(!owner.nested_contexts.is_empty());
    assert!(
        owner
            .code_windows
            .iter()
            .all(|window| !window.text.contains("format!"))
    );
    assert!(
        owner
            .nested_contexts
            .iter()
            .flat_map(|context| &context.code_windows)
            .any(|window| window.text.contains("format!("))
    );
    let mut prior = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph_path,
            None,
            &directory.path().join(format!("query-cache-{mode:?}")),
            mode,
        )?;
        let response =
            engine.discover_ranked(request("Where are diagnostics formatted in `opaque`?"))?;
        let candidate = response
            .candidates
            .iter()
            .find(|candidate| candidate.node.id == target.id)
            .ok_or("formatter not retrieved")?;
        let quote = candidate
            .excerpts
            .first()
            .ok_or("missing implementation quote")?;
        assert_eq!(
            quote.kind,
            compass_query::RankedExcerptKind::NestedCallableCode
        );
        assert!(
            quote.text.contains("format!(") && quote.text.contains("diagnostic.message()"),
            "{}",
            quote.text
        );
        let scope = quote.nested_scope.as_ref().ok_or("missing lexical scope")?;
        assert!(target.source.as_ref().is_some_and(
            |parent| parent.start_byte < scope.start_byte && scope.end_byte < parent.end_byte
        ));
        assert_eq!(
            &source[quote.source.start_byte as usize..quote.source.end_byte as usize],
            quote.text
        );
        if let Some(old) = prior {
            assert_eq!(old, candidate.excerpts);
        }
        prior = Some(candidate.excerpts.clone());
    }
    Ok(())
}

#[test]
fn nested_context_respects_named_published_and_type_boundaries() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = "export function opaque(payload: number) {\n const callback = (item: number) => secret_named(item);\n function named() { secret_function(); }\n class Nested { run() { return (() => secret_type(payload))(); } }\n return payload;\n}\n";
    fs::write(root.join("sample.ts"), source)?;
    let built = build_local_graph(&options(
        &root,
        &d.path().join("artifacts"),
        GraphStorage::Sqlite,
    ))?;
    let a = artifact(&built.output_dir)?;
    let graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let callback = graph
        .nodes
        .iter()
        .find(|node| node.kind.is_callable() && node.name.trim_end_matches("()") == "callback")
        .ok_or("fixture requires a published callback binding")?;
    assert!(a.owners.iter().any(|owner| owner.node_id == callback.id));
    let target = graph
        .nodes
        .iter()
        .find(|node| node.kind.is_callable() && node.name.trim_end_matches("()") == "opaque")
        .ok_or("missing parent")?;
    let owner = a
        .owners
        .iter()
        .find(|owner| owner.node_id == target.id)
        .ok_or("missing owner")?;
    assert!(
        owner.nested_contexts.is_empty(),
        "named/published children and class scopes cannot lend code to this parent"
    );
    Ok(())
}

#[test]
fn nested_context_uses_only_spare_direct_window_capacity() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let mut source = "fn opaque(payload: usize) {\n".to_owned();
    for at in 0..24 {
        source.push_str(&format!(" normalize_payload_{at}(payload);\n"));
    }
    source.push_str(" let unused = || hidden_payload_error(payload);\n}\n");
    fs::write(root.join("sample.rs"), source)?;
    let built = build_local_graph(&options(
        &root,
        &d.path().join("artifacts"),
        GraphStorage::Sqlite,
    ))?;
    let a = artifact(&built.output_dir)?;
    let graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.kind.is_callable() && node.name.trim_end_matches("()") == "opaque")
        .ok_or("missing parent")?;
    let owner = a
        .owners
        .iter()
        .find(|owner| owner.node_id == target.id)
        .ok_or("missing owner")?;
    assert_eq!(owner.code_windows.len(), 16);
    assert!(owner.nested_contexts.is_empty());
    assert!(owner.omitted_nested_contexts > 0);
    Ok(())
}

#[test]
fn unused_nested_code_is_only_lexical_context_and_survives_neutral_rebuilds()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = "fn opaque(payload: usize) { let unused = || hidden_payload_error(payload); }\nfn hidden_payload_error(payload: usize) -> usize { payload }\n";
    fs::write(root.join("sample.rs"), source)?;
    let o = options(&root, &d.path().join("incremental"), GraphStorage::Sqlite);
    let built = build_local_graph(&o)?;
    let first = artifact(&built.output_dir)?;
    assert!(
        first
            .owners
            .iter()
            .flat_map(|owner| &owner.nested_contexts)
            .any(|context| context
                .code_windows
                .iter()
                .any(|window| window.text.contains("hidden_payload_error")))
    );
    fs::write(root.join("sample.rs"), format!("{source}\n"))?;
    let incremental = build_local_graph(&o)?;
    let mut incremental = artifact(&incremental.output_dir)?;
    let clean = build_local_graph(&options(
        &root,
        &d.path().join("clean"),
        GraphStorage::Sqlite,
    ))?;
    let mut clean = artifact(&clean.output_dir)?;
    incremental.graph_digest = "a".repeat(64);
    clean.graph_digest = "a".repeat(64);
    assert_eq!(incremental, clean);
    Ok(())
}
