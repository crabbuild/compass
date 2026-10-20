use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_model::source_query::{SOURCE_QUERY_FILE, SOURCE_QUERY_PROFILE, SourceQueryArtifact};
use compass_query::{EngineSelection, RankedExcerptKind, RelevanceBand, open_with_engine};
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
    let mut options = BuildOptions::new(root);
    options.output_root = Some(output.to_path_buf());
    options.graph_storage = storage;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    options
}
fn artifact(output: &Path) -> Result<SourceQueryArtifact, Box<dyn Error>> {
    let value: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(output.join(SOURCE_QUERY_FILE))?)?;
    value.validate()?;
    assert_eq!(value.profile, SOURCE_QUERY_PROFILE);
    assert_eq!(
        value.graph_digest,
        format!("{:x}", Sha256::digest(fs::read(output.join("graph.json"))?))
    );
    Ok(value)
}
fn same_publication(left: &Path, right: &Path) -> Result<(), Box<dyn Error>> {
    for file in ["graph.json", SOURCE_QUERY_FILE] {
        assert_eq!(
            Sha256::digest(fs::read(left.join(file))?),
            Sha256::digest(fs::read(right.join(file))?),
            "{file}"
        );
    }
    Ok(())
}

#[test]
fn later_callable_comments_remain_searchable_without_leaking_to_siblings()
-> Result<(), Box<dyn Error>> {
    for (file, opening, comment, ending) in [
        (
            "sample.py",
            "def opaque():\n",
            "    #",
            "    return 0\n\ndef sibling():\n    return 1\n",
        ),
        ("sample.rs", "fn opaque() {\n", "//", "}\nfn sibling() {}\n"),
        (
            "sample.go",
            "package p\nfunc opaque() {\n",
            "//",
            "}\nfunc sibling() {}\n",
        ),
        (
            "sample.ts",
            "function opaque() {\n",
            "//",
            "}\nfunction sibling() {}\n",
        ),
        (
            "sample.tsx",
            "function opaque() {\n",
            "//",
            "}\nfunction sibling() {}\n",
        ),
    ] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        let mut source = opening.to_owned();
        for index in 0..24 {
            source.push_str(&format!("{comment} bookkeeping detail {index}\n"));
        }
        source.push_str(&format!(
            "{comment} Capacity reservations protect uncommon search terms.\n{ending}"
        ));
        fs::write(root.join(file), &source)?;
        let built =
            build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
        let path = built.output_dir.join("graph.json");
        let graph = GraphDocument::load(&path)?;
        let target = graph
            .nodes
            .iter()
            .find(|n| n.kind.is_callable() && n.name.contains("opaque"))
            .ok_or("missing native callable")?;
        let evidence = artifact(&built.output_dir)?;
        let owner = evidence
            .owners
            .iter()
            .find(|o| o.node_id == target.id)
            .ok_or("missing source owner")?;
        let site = owner
            .comments
            .iter()
            .find(|s| s.text.contains("Capacity reservations"))
            .ok_or("late callable comment was discarded")?;
        assert_eq!(
            site.source.start_line,
            26 + u32::from(file.ends_with(".go")),
            "{file}"
        );
        assert_eq!(
            source
                .as_bytes()
                .get(site.source.start_byte as usize..site.source.end_byte as usize),
            Some(site.text.as_bytes()),
            "{file}: retained evidence must quote the original bytes"
        );
        assert!(
            evidence
                .owners
                .iter()
                .filter(|o| o.node_id != target.id)
                .all(|o| o
                    .comments
                    .iter()
                    .all(|s| !s.text.contains("Capacity reservations")))
        );
        let mut previous = None;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine =
                open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
            let answer = engine.discover_ranked(request(
                "Where do capacity reservations protect uncommon search terms?",
            ))?;
            let candidate = answer
                .candidates
                .first()
                .ok_or("late-comment candidate missing")?;
            assert_eq!(candidate.node.id, target.id, "{file}: {answer:?}");
            assert_eq!(candidate.relevance, RelevanceBand::Low);
            assert!(candidate.matched_fields.iter().any(|f| f == "body_comment"));
            if let Some(previous) = &previous {
                assert_eq!(previous, &answer.candidates, "{file}: backend parity");
            } else {
                previous = Some(answer.candidates);
            }
        }
    }
    Ok(())
}

#[test]
fn comment_capacity_survives_warm_updates_and_reopening() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let settings = options(&root, &d.path().join("warm"), GraphStorage::Sqlite);
    for (step, count) in [64_usize, 65, 64].into_iter().enumerate() {
        let mut source = "fn opaque() {\n".to_owned();
        for index in 0..count {
            source.push_str(&format!("// retained detail {index}\n"));
        }
        source.push_str("}\n");
        fs::write(root.join("sample.rs"), source)?;
        let built = build_local_graph(&settings)?;
        let cold = artifact(&built.output_dir)?;
        let owner = cold.owners.first().ok_or("missing comment owner")?;
        assert_eq!(owner.comments.len(), 64);
        assert_eq!(owner.omitted_sites, (count - 64) as u64);
        assert_eq!(owner.truncated, count > 64);
        let expected = ["graph.json", SOURCE_QUERY_FILE]
            .map(|file| fs::read(built.output_dir.join(file)))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let warm = build_local_graph(&settings)?;
        assert_eq!(cold, artifact(&warm.output_dir)?);
        for (file, bytes) in ["graph.json", SOURCE_QUERY_FILE].into_iter().zip(expected) {
            assert_eq!(
                bytes,
                fs::read(warm.output_dir.join(file))?,
                "{step}: {file}"
            );
        }
        let mut forced = settings.clone();
        forced.force = true;
        forced.output_root = Some(d.path().join(format!("forced-{step}")));
        let clean = build_local_graph(&forced)?;
        same_publication(&warm.output_dir, &clean.output_dir)?;
    }
    Ok(())
}

#[test]
fn literal_only_source_reaches_the_exact_native_owner_on_both_backends()
-> Result<(), Box<dyn Error>> {
    for (file, source) in [
        ("sample.py", "def opaque():\n    return 'session expired'\n"),
        (
            "sample.rs",
            "fn opaque() -> &'static str { \"session expired\" }\n",
        ),
        (
            "sample.go",
            "package p\nfunc Opaque() string { return \"session expired\" }\n",
        ),
        (
            "sample.ts",
            "function opaque() { return 'session expired'; }\n",
        ),
        (
            "sample.tsx",
            "function opaque() { return 'session expired'; }\n",
        ),
        ("arrow.ts", "const opaque = () => 'session expired';\n"),
        ("arrow.tsx", "const opaque = () => 'session expired';\n"),
    ] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        fs::write(root.join(file), source)?;
        let built =
            build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
        let path = built.output_dir.join("graph.json");
        let graph = GraphDocument::load(&path)?;
        let callables: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind.is_callable())
            .collect();
        assert_eq!(callables.len(), 1, "{file}");
        let node = callables[0];
        let body = artifact(&built.output_dir)?;
        let owner = body
            .owners
            .iter()
            .find(|o| o.node_id == node.id)
            .ok_or("native owner missing from publication")?;
        assert_eq!(
            &owner.source,
            node.source.as_ref().ok_or("missing graph source")?
        );
        assert!(
            owner
                .code_windows
                .iter()
                .any(|w| w.text.contains("session expired"))
        );
        let before = fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?;
        let mut previous = None;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine =
                open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
            let response =
                engine.discover_ranked(request("Where are expired sessions handled?"))?;
            let candidate = response
                .candidates
                .first()
                .ok_or("literal-only candidate missing")?;
            assert_eq!(candidate.node.id, node.id, "{file}");
            assert_eq!(candidate.relevance, RelevanceBand::Low);
            assert!(candidate.matched_fields.iter().any(|f| f == "body_code"));
            assert!(
                candidate
                    .excerpts
                    .iter()
                    .any(|q| q.kind == RankedExcerptKind::CallableCode
                        && q.text.contains("session expired")
                        && q.is_coherent())
            );
            for quote in candidate
                .excerpts
                .iter()
                .filter(|q| q.kind == RankedExcerptKind::CallableCode)
            {
                assert_eq!(
                    source.get(quote.source.start_byte as usize..quote.source.end_byte as usize),
                    Some(quote.text.as_str())
                );
                assert!(
                    owner.source.start_byte <= quote.source.start_byte
                        && quote.source.end_byte <= owner.source.end_byte
                );
            }
            assert!(
                engine
                    .discover_ranked(request("Where are orbital plasmids launched?"))?
                    .candidates
                    .is_empty()
            );
            let named = engine.discover_ranked(request(&format!("Where is {}?", node.name)))?;
            assert!(named.candidates.iter().any(|c| c.node.id == node.id));
            // Retrieval and quotations must come from the published artifact.
            fs::remove_file(root.join(file))?;
            assert_eq!(
                engine
                    .discover_ranked(request("Where are expired sessions handled?"))?
                    .candidates,
                response.candidates
            );
            fs::write(root.join(file), source)?;
            if let Some(prior) = &previous {
                assert_eq!(&response.candidates, prior, "{file}: backend mismatch");
            }
            previous = Some(response.candidates);
        }
        assert_eq!(
            Sha256::digest(before),
            Sha256::digest(fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)
        );
    }
    Ok(())
}

#[test]
fn literal_publication_does_not_lend_defaults_decorators_or_type_fields_to_other_owners()
-> Result<(), Box<dyn Error>> {
    let source = concat!(
        "@decorate('decorator_secret')\n",
        "def enclosing():\n",
        "    def inner(value='default_marker'):\n        return 'nested_secret'\n",
        "    class Hidden:\n        field = 'class_secret'\n",
        "        def method(self):\n            return 'type_secret'\n",
        "    return 'outer_marker'\n",
        "def peer():\n    return 'peer_secret'\n",
    );
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(root.join("sample.py"), source)?;
    let built = build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
    let graph = GraphDocument::load(&built.output_dir.join("graph.json"))?;
    let body = artifact(&built.output_dir)?;
    for (name, marker) in [
        ("enclosing", "outer_marker"),
        ("inner", "nested_secret"),
        ("inner", "default_marker"),
        ("method", "type_secret"),
        ("peer", "peer_secret"),
    ] {
        let node = graph
            .nodes
            .iter()
            .find(|n| {
                n.kind.is_callable()
                    && n.name.trim_start_matches('.').trim_end_matches("()") == name
            })
            .ok_or("native callable missing")?;
        let owners: Vec<_> = body
            .owners
            .iter()
            .filter(|o| o.code_windows.iter().any(|w| w.text.contains(marker)))
            .collect();
        assert_eq!(owners.len(), 1, "{marker}");
        assert_eq!(owners[0].node_id, node.id, "{marker}");
    }
    for forbidden in ["decorator_secret", "class_secret"] {
        assert!(
            body.owners
                .iter()
                .flat_map(|o| &o.code_windows)
                .all(|w| !w.text.contains(forbidden)),
            "{forbidden}"
        );
    }
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &built.output_dir.join("graph.json"),
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        let answer = engine.discover_ranked(request("Where is nested secret?"))?;
        let first = answer
            .candidates
            .first()
            .ok_or("nested literal owner lost")?;
        assert_eq!(first.node.name.trim_end_matches("()"), "inner");
        assert!(first.matched_terms.iter().any(|term| term == "nest"));
        // Ordinary partial-topic inspection is supported. It must not borrow
        // the missing topic or its source quotation from the inner callable.
        for candidate in answer.candidates.iter().skip(1) {
            assert_eq!(candidate.relevance, RelevanceBand::Low);
            assert!(!candidate.matched_terms.iter().any(|term| term == "nest"));
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("missing owner-specific context: nest"))
            );
            assert!(
                candidate
                    .excerpts
                    .iter()
                    .all(|quote| !quote.text.contains("nested_secret"))
            );
        }
    }
    Ok(())
}

#[test]
fn literal_evidence_clean_cached_edit_restore_and_delete_publications_match()
-> Result<(), Box<dyn Error>> {
    for storage in [GraphStorage::Json, GraphStorage::Sqlite] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        let file = root.join("sample.py");
        let original = "def opaque():\n    return 'session expired'\n";
        let edited = "def opaque():\n    return 'credential renewed'\n";
        fs::write(
            root.join("peer.py"),
            "def peer():\n    return 'unrelated marker'\n",
        )?;
        let mut o = options(&root, &d.path().join("out"), storage);
        o.cache_root = Some(d.path().join("shared-cache"));
        for (step, content) in [
            Some(original),
            Some(original),
            Some(edited),
            Some(original),
            None,
            Some(original),
        ]
        .into_iter()
        .enumerate()
        {
            match content {
                Some(text) => fs::write(&file, text)?,
                None => fs::remove_file(&file)?,
            }
            let built = build_local_graph(&o)?;
            let body = artifact(&built.output_dir)?;
            let actual = body.files.iter().find(|f| f.file == "sample.py");
            if let Some(text) = content {
                let actual = actual.ok_or("retained literal file missing")?;
                assert_eq!(
                    actual.source_digest,
                    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
                );
                assert_eq!(actual.byte_size, text.len() as u64);
            } else {
                assert!(actual.is_none());
            }
            let mut clean = o.clone();
            clean.force = true;
            clean.output_root = Some(d.path().join(format!("clean-{step}")));
            let clean = build_local_graph(&clean)?;
            same_publication(&built.output_dir, &clean.output_dir)?;
            let engine = open_with_engine(
                &built.output_dir.join("graph.json"),
                None,
                &d.path().join("same-query-cache"),
                if storage == GraphStorage::Json {
                    EngineSelection::Json
                } else {
                    EngineSelection::Store
                },
            )?;
            let answer = engine.discover_ranked(request("Where are expired sessions handled?"))?;
            assert_eq!(
                answer.candidates.iter().any(|c| c
                    .node
                    .source
                    .as_ref()
                    .is_some_and(|s| s.file == "sample.py")),
                content == Some(original)
            );
        }
    }
    Ok(())
}

#[test]
fn historical_source_profiles_keep_backend_postings_and_scores_without_rewriting()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{
        SOURCE_QUERY_PROFILE_V1, SOURCE_QUERY_PROFILE_V2, SOURCE_QUERY_PROFILE_V3,
        SOURCE_QUERY_PROFILE_V4, SOURCE_QUERY_PROFILE_V5, SOURCE_QUERY_PROFILE_V6,
    };
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    // lookup precedes the string: all retained windows were already possible
    // under the historical identifier-seeded producer, including profile6.
    fs::write(
        root.join("sample.py"),
        "def opaque(block):\n    return lookup('session expired')\n",
    )?;
    let built = build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
    let graph_path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let original = artifact(&built.output_dir)?;
    let target = original
        .owners
        .first()
        .ok_or("missing native owner")?
        .node_id
        .clone();
    let mut expected = None;
    for (profile, code_lexical, bindings) in [
        (SOURCE_QUERY_PROFILE_V5, true, false),
        (SOURCE_QUERY_PROFILE_V6, true, true),
        (SOURCE_QUERY_PROFILE_V1, false, false),
        (SOURCE_QUERY_PROFILE_V2, false, false),
        (SOURCE_QUERY_PROFILE_V3, false, false),
        (SOURCE_QUERY_PROFILE_V4, false, false),
        (SOURCE_QUERY_PROFILE_V6, true, true),
        (SOURCE_QUERY_PROFILE, true, true),
    ] {
        let mut historical = original.clone();
        historical.profile = profile.to_owned();
        historical.additional_document_frequencies.clear();
        for owner in &mut historical.owners {
            if !bindings {
                owner.parameter_bindings = None;
            }
            if matches!(profile, SOURCE_QUERY_PROFILE_V1 | SOURCE_QUERY_PROFILE_V2) {
                owner.code_windows.clear();
                owner.omitted_code_windows = 0;
            }
            let node = graph
                .nodes
                .iter()
                .find(|n| n.id == owner.node_id)
                .ok_or("historical owner missing")?;
            let base = compass_model::search::search_document(node).terms();
            for term in owner.document_for_profile(profile)?.terms() {
                *historical
                    .additional_document_frequencies
                    .entry(term.clone())
                    .or_default() += u64::from(!base.contains(&term));
            }
        }
        historical.validate()?;
        let sidecar = built.output_dir.join(SOURCE_QUERY_FILE);
        let bytes = serde_json::to_vec(&historical)?;
        fs::write(&sidecar, &bytes)?;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine = open_with_engine(
                &graph_path,
                None,
                &d.path().join(format!("same-cache-{mode:?}")),
                mode,
            )?;
            let response =
                engine.discover_ranked(request("Where are expired sessions handled?"))?;
            assert_eq!(
                response.candidates.iter().any(|c| c.node.id == target),
                code_lexical,
                "{profile}"
            );
            if code_lexical {
                if let Some(prior) = &expected {
                    assert_eq!(
                        &response.candidates, prior,
                        "{profile}: score/posting drift"
                    );
                }
                expected = Some(response.candidates);
            }
            let exact = engine.discover_ranked(request(&target))?;
            assert_eq!(exact.candidates.len(), 1);
            assert_eq!(exact.candidates[0].node.id, target);
            assert_eq!(fs::read(&sidecar)?, bytes, "query rewrote {profile}");
        }
    }
    Ok(())
}

#[test]
fn first_code_window_site_transition_matches_clean_publication() -> Result<(), Box<dyn Error>> {
    for storage in [GraphStorage::Json, GraphStorage::Sqlite] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        let mut bulk = String::new();
        for owner in 0..512 {
            // Avoid creating docstrings. Adding a literal changes callable facts;
            // this exercises full-path code capacity, not fact-neutral reuse.
            bulk.push_str(&format!("def opaque{owner}():\n    pass\n"));
            for _ in 0..16 {
                bulk.push_str(&format!("    '{}'\n", "q".repeat(30)));
            }
        }
        for file in 0..4 {
            fs::write(root.join(format!("bulk{file}.py")), &bulk)?;
        }
        let changed = root.join("aaaa.py");
        fs::write(&changed, "def probe():\n    pass\n")?;
        let o = options(&root, &d.path().join("out"), storage);
        let initial = build_local_graph(&o)?;
        let before = artifact(&initial.output_dir)?;
        let original_graph = fs::read(initial.output_dir.join("graph.json"))?;
        let original_evidence = fs::read(initial.output_dir.join(SOURCE_QUERY_FILE))?;
        assert_eq!(
            before
                .owners
                .iter()
                .map(|o| o.code_windows.len())
                .sum::<usize>(),
            32_768
        );
        assert!(before.owners.iter().all(|o| o.omitted_code_windows == 0));
        fs::write(&changed, "def probe():\n    pass\n    'session expired'\n")?;
        let incremental = build_local_graph(&o)?;
        let mut clean = o.clone();
        clean.force = true;
        clean.output_root = Some(d.path().join("clean"));
        let clean = build_local_graph(&clean)?;
        same_publication(&incremental.output_dir, &clean.output_dir)?;
        fs::write(&changed, "def probe():\n    pass\n")?;
        let restored = build_local_graph(&o)?;
        let mut clean_restored = o.clone();
        clean_restored.force = true;
        clean_restored.output_root = Some(d.path().join("clean-restored"));
        let clean_restored = build_local_graph(&clean_restored)?;
        same_publication(&restored.output_dir, &clean_restored.output_dir)?;
        assert_eq!(
            Sha256::digest(original_graph),
            Sha256::digest(fs::read(restored.output_dir.join("graph.json"))?)
        );
        assert_eq!(
            Sha256::digest(original_evidence),
            Sha256::digest(fs::read(restored.output_dir.join(SOURCE_QUERY_FILE))?)
        );
    }

    Ok(())
}

#[test]
fn ranked_comment_excerpt_retains_later_question_evidence_on_both_backends()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let phrase = "Reject malformed payload before decoding protocol frames.";
    let source = format!(
        "def opaque():\n    # protocol notices: {} {phrase}\n    return 0\n\ndef sibling():\n    return 1\n",
        "protocol bookkeeping ".repeat(40),
    );
    fs::write(root.join("sample.py"), &source)?;
    let built = build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let target = graph
        .nodes
        .iter()
        .find(|n| n.kind.is_callable() && n.qualified_name == "sample.opaque")
        .ok_or("missing target")?;
    let mut previous = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine =
            open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
        let ranked = engine.discover_ranked(request(
            "Where are malformed protocol payloads rejected before decoding frames?",
        ))?;
        let candidate = ranked
            .candidates
            .iter()
            .find(|c| c.node.id == target.id)
            .ok_or("native comment owner missing")?;
        let quote = candidate
            .excerpts
            .iter()
            .find(|q| q.kind == RankedExcerptKind::CallableComment && q.text.contains(phrase))
            .ok_or("later coherent comment passage missing from ranked answer")?;
        assert!(quote.is_coherent());
        assert_eq!(
            source.get(quote.source.start_byte as usize..quote.source.end_byte as usize),
            Some(quote.text.as_str())
        );
        assert_eq!(quote.source.file, "sample.py");
        let declaration = target.source.as_ref().ok_or("missing target source")?;
        assert!(
            declaration.start_byte <= quote.source.start_byte
                && quote.source.end_byte <= declaration.end_byte
        );
        assert!(
            ranked
                .candidates
                .iter()
                .all(|c| c.node.qualified_name != "sample.sibling")
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &candidate.excerpts);
        } else {
            previous = Some(candidate.excerpts.clone());
        }
    }
    Ok(())
}

#[test]
fn final_quotation_order_uses_complete_source_action_tokens() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = format!(
        "def opaque():\n    # payload {}readable\n    # Read payload.\n    return None\n",
        " ".repeat(498),
    );
    fs::write(root.join("sample.py"), &source)?;
    let built = build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
    let path = built.output_dir.join("graph.json");
    let mut previous = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine =
            open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
        let ranked = engine.discover_ranked(request("Where are payloads read?"))?;
        let candidate = ranked
            .candidates
            .iter()
            .find(|c| c.node.qualified_name == "sample.opaque")
            .ok_or("missing source owner")?;
        assert_eq!(candidate.excerpts.len(), 2);
        assert!(
            candidate.excerpts[0].text.contains("# Read payload."),
            "a clipped readable prefix must not tie genuine read action evidence"
        );
        assert!(candidate.excerpts[1].text.ends_with("read"));
        for quote in &candidate.excerpts {
            assert_eq!(quote.kind, RankedExcerptKind::CallableComment);
            assert!(quote.is_coherent());
            assert_eq!(
                source.get(quote.source.start_byte as usize..quote.source.end_byte as usize),
                Some(quote.text.as_str())
            );
        }
        if let Some(previous) = &previous {
            assert_eq!(previous, &candidate.excerpts);
        } else {
            previous = Some(candidate.excerpts.clone());
        }
    }
    Ok(())
}

#[test]
fn broad_comparative_spellings_are_tentative_and_keep_literal_topics() -> Result<(), Box<dyn Error>>
{
    for (word, variant) in [
        ("fast", "fastest"),
        ("large", "largest"),
        ("slow", "slower"),
    ] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join("sample.rs"),
            format!(
                "fn opaque() {{ /* Format the {variant} payloads. */ }}\nfn unrelated() {{ /* Format unrelated invoices. */ }}\n"
            ),
        )?;
        let built =
            build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
        let path = built.output_dir.join("graph.json");
        let graph = GraphDocument::load(&path)?;
        let target = graph
            .nodes
            .iter()
            .find(|n| {
                n.kind.is_callable() && n.qualified_name.rsplit([':', '.']).next() == Some("opaque")
            })
            .ok_or("missing source owner")?;
        let mut previous = None;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine =
                open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
            let question = format!("Where are {word} payloads formatted?");
            let answer = engine.discover_ranked(request(&question))?;
            let candidate = answer
                .candidates
                .iter()
                .find(|c| c.node.id == target.id)
                .ok_or("comparative source lead missing")?;
            assert!(
                candidate.matched_terms.iter().any(|t| t == word),
                "{word}: {candidate:?}"
            );
            assert_eq!(candidate.relevance, RelevanceBand::Low);
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("Vocabulary fallback") && r.contains(variant))
            );
            assert!(
                answer
                    .candidates
                    .iter()
                    .all(|c| c.node.name.trim_end_matches("()") != "unrelated")
            );
            assert!(
                engine
                    .discover_ranked(request(&format!("Where are `{word}` payloads formatted?")))?
                    .candidates
                    .iter()
                    .all(|c| c.node.id != target.id)
            );
            assert!(
                engine
                    .discover_ranked(request(&format!(
                        "Where are {word} LunarRover payloads formatted?"
                    )))?
                    .candidates
                    .is_empty()
            );
            if let Some(previous) = &previous {
                assert_eq!(previous, &answer.candidates);
            } else {
                previous = Some(answer.candidates);
            }
        }
    }
    Ok(())
}

#[test]
fn distinctive_topic_comment_precedes_generic_signature_quotations() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = concat!(
        "fn opaque(search_terms: SearchTerms) {\n",
        "    // Uncommon identifiers receive isolated quotas.\n",
        "    // Read search terms with stable ordering.\n",
        "    let result = search_terms;\n",
        "}\n",
        "fn ordinary_read_search_terms(search_terms: SearchTerms) {}\n",
        "fn other_read_search_terms(search_terms: SearchTerms) {}\n",
    );
    fs::write(root.join("sample.rs"), source)?;
    let built = build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let target = graph
        .nodes
        .iter()
        .find(|n| {
            n.kind.is_callable() && n.qualified_name.rsplit([':', '.']).next() == Some("opaque")
        })
        .ok_or("missing source owner")?;
    let mut previous = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine =
            open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
        let ranked = engine.discover_ranked(request("Where are uncommon search terms?"))?;
        let candidate = ranked
            .candidates
            .iter()
            .find(|c| c.node.id == target.id)
            .ok_or("missing candidate")?;
        let first = candidate
            .excerpts
            .first()
            .ok_or("missing source quotation")?;
        assert_eq!(
            first.kind,
            RankedExcerptKind::CallableComment,
            "{mode:?}: {candidate:?}"
        );
        assert!(
            first
                .text
                .contains("Uncommon identifiers receive isolated quotas.")
        );
        for quote in &candidate.excerpts {
            assert!(quote.is_coherent());
            assert_eq!(
                source
                    .as_bytes()
                    .get(quote.source.start_byte as usize..quote.source.end_byte as usize),
                Some(quote.text.as_bytes())
            );
        }
        let action_ranked =
            engine.discover_ranked(request("Where are uncommon search terms read?"))?;
        let action_candidate = action_ranked
            .candidates
            .iter()
            .find(|c| c.node.id == target.id)
            .ok_or("missing action candidate")?;
        let action_first = action_candidate
            .excerpts
            .first()
            .ok_or("missing action quotation")?;
        assert!(
            action_first.text.contains("Read search terms"),
            "an explicit action precedes rarer topic-only comments: {action_candidate:?}"
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &candidate.excerpts);
        } else {
            previous = Some(candidate.excerpts.clone());
        }
    }
    Ok(())
}

#[test]
fn unrelated_owned_comments_do_not_dilute_a_complete_query_passage() -> Result<(), Box<dyn Error>> {
    let mut baseline = None;
    for noisy in [false, true] {
        let d = tempfile::tempdir()?;
        let root = d.path().join("repo");
        fs::create_dir(&root)?;
        let noise = if noisy {
            (0..120).map(|n| format!("padding{n} ")).collect::<String>()
        } else {
            String::new()
        };
        let source = format!("fn opaque() {{\n    // Format record errors.\n    // {noise}\n}}\n");
        fs::write(root.join("sample.rs"), source)?;
        let built =
            build_local_graph(&options(&root, &d.path().join("out"), GraphStorage::Sqlite))?;
        let path = built.output_dir.join("graph.json");
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine =
                open_with_engine(&path, None, &d.path().join(format!("cache-{mode:?}")), mode)?;
            let answer = engine.discover_ranked(request("Where are record errors formatted?"))?;
            let candidate = answer
                .candidates
                .iter()
                .find(|c| c.node.name.trim_end_matches("()") == "opaque")
                .ok_or("missing comment owner")?;
            assert!(
                candidate
                    .matched_fields
                    .iter()
                    .any(|field| field == "body_comment")
            );
            if let Some((score, relevance)) = baseline {
                assert_eq!(
                    candidate.score, score,
                    "unrelated comments dilute an unchanged complete query passage: {candidate:?}"
                );
                assert_eq!(candidate.relevance, relevance);
            } else {
                baseline = Some((candidate.score, candidate.relevance));
            }
        }
    }
    Ok(())
}
