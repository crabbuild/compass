use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::{GraphDocument, NodeKind};
use compass_model::provenance::SourceAnchor;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_model::source_query::{
    SOURCE_QUERY_FILE, SourceQueryArtifact, SourceQueryOwner, SourceQuerySite,
};
use compass_query::{EngineSelection, QueryErrorKind, open_with_engine};
use std::error::Error;
use std::fs;
use std::path::Path;
fn request() -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "Where is payload handled?".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}
fn options(root: &Path, output: &Path) -> BuildOptions {
    let mut o = BuildOptions::new(root);
    o.output_root = Some(output.to_path_buf());
    o.graph_storage = GraphStorage::Json;
    o.no_cluster = true;
    o.no_viz = true;
    o.max_workers = Some(1);
    o
}

#[test]
fn source_channel_completeness_guards_operation_and_compound_preferences()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::SOURCE_QUERY_PROFILE_V4;
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let mut cases = Vec::new();
    for compound in [false, true] {
        for mode in [
            "complete",
            "code_omitted",
            "identifier_clipped",
            "comment_clipped",
            "mixed",
        ] {
            let file = format!(
                "case_{}_{mode}.py",
                if compound { "compound" } else { "operation" }
            );
            let mut source = if compound {
                "def opaque(fields):\n    read_only = fields\n".to_owned()
            } else {
                "def opaque(session, values):\n    encoded = codec.encode(values)\n".to_owned()
            };
            match mode {
                "code_omitted" => {
                    for index in 0..24 {
                        source.push_str(&format!("    marker_{index} = None\n"));
                    }
                }
                "identifier_clipped" => {
                    for index in 0..64 {
                        source
                            .push_str(&format!("    marker_{index}_{} = None\n", "a".repeat(180)));
                    }
                }
                "comment_clipped" => {
                    for _ in 0..8 {
                        source.push_str(&format!("    # {}\n", "padding ".repeat(200)));
                    }
                }
                "mixed" => source.push_str("    tag = 'coral reef'\n"),
                _ => {}
            }
            source.push_str(if compound {
                "    return read_only\n"
            } else {
                "    return encoded\n"
            });
            fs::write(root.join(&file), source)?;
            let topic = if mode == "mixed" { "coral reef " } else { "" };
            let question = if compound {
                format!("Where are {topic}read-only fields prepared?")
            } else {
                format!("Where are {topic}session values encoded?")
            };
            cases.push((file, mode, compound, question));
        }
    }
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph_path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let body: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    let mut expected = Vec::new();
    for engine_mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph_path,
            None,
            &d.path().join(format!("query-{engine_mode:?}")),
            engine_mode,
        )?;
        for (at, (file, mode, compound, question)) in cases.iter().enumerate() {
            let node = graph
                .nodes
                .iter()
                .find(|n| {
                    n.source.as_ref().is_some_and(|s| s.file == *file)
                        && n.name.trim_end_matches("()") == "opaque"
                })
                .ok_or("missing native owner")?;
            let owner = body
                .owners
                .iter()
                .find(|o| o.node_id == node.id)
                .ok_or("missing body owner")?;
            let old = owner.document_for_profile(SOURCE_QUERY_PROFILE_V4)?;
            let clipped = matches!(*mode, "identifier_clipped" | "comment_clipped");
            assert!(
                !owner.truncated,
                "{file}: fixture must exercise projected field limits"
            );
            assert_eq!(
                old.truncated, clipped,
                "{file}: original field completeness"
            );
            assert_eq!(
                owner.omitted_code_windows > 0,
                matches!(*mode, "code_omitted" | "identifier_clipped"),
                "{file}"
            );
            if *mode == "mixed" {
                assert!(!old.terms().contains("coral"), "{file}");
                assert!(
                    owner
                        .document()
                        .fields
                        .get("body_code")
                        .is_some_and(|t| t.contains("coral")),
                    "{file}"
                );
            }
            let mut query = request();
            query.question = question.clone();
            let answer = engine.discover_ranked(query)?;
            let candidate = answer
                .candidates
                .iter()
                .find(|c| c.node.id == node.id)
                .ok_or_else(|| format!("missing {file}: {answer:?}"))?;
            let reason = if *compound {
                "Compound identifiers"
            } else {
                "Implementation-local operation identifiers"
            };
            assert_eq!(
                candidate.reasons.iter().any(|r| r.contains(reason)),
                !clipped && *mode != "mixed",
                "{file}: {candidate:?}"
            );
            assert_eq!(
                candidate.relevance,
                compass_query::RelevanceBand::Low,
                "{file}"
            );
            if *mode == "mixed" {
                assert!(
                    candidate.matched_fields.iter().any(|f| f == "body_code"),
                    "{file}"
                );
                assert!(
                    !candidate
                        .reasons
                        .iter()
                        .any(|r| r.contains("inspection preference")),
                    "{file}"
                );
            }
            if owner.omitted_code_windows > 0 {
                assert!(
                    candidate
                        .reasons
                        .iter()
                        .any(|r| r.contains("windows were omitted")),
                    "{file}"
                );
            }
            if engine_mode == EngineSelection::Json {
                expected.push(answer.candidates);
            } else {
                assert_eq!(answer.candidates, expected[at], "{file}");
            }
        }
    }
    Ok(())
}

#[test]
fn code_window_omissions_preserve_complete_identifier_preferences() -> Result<(), Box<dyn Error>> {
    use compass_model::source_query::SOURCE_QUERY_PROFILE_V4;
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let mut source =
        "def opaque(session, values):\n    encoded = codec.encode(values)\n".to_owned();
    for index in 0..24 {
        source.push_str(&format!("    marker_{index} = None\n"));
    }
    source.push_str("    return encoded\n");
    fs::write(root.join("sample.py"), source)?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let body: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    let document = GraphDocument::load(&graph)?;
    let node = document
        .nodes
        .iter()
        .find(|n| n.name.trim_end_matches("()") == "opaque")
        .ok_or("missing opaque owner")?;
    let owner = body
        .owners
        .iter()
        .find(|o| o.node_id == node.id)
        .ok_or("missing source owner")?;
    assert!(
        owner.omitted_code_windows > 0,
        "fixture must hit only code-window capacity"
    );
    assert!(owner.document().truncated);
    assert!(
        !owner
            .document_for_profile(SOURCE_QUERY_PROFILE_V4)?
            .truncated,
        "original identifier/comment fields must stay complete"
    );
    let mut partial = body.clone();
    for owner in &mut partial.owners {
        let bindings = owner
            .parameter_bindings
            .as_mut()
            .ok_or("missing current coverage")?;
        bindings.sites.clear();
        bindings.status = compass_model::source_query::ParameterBindingStatus::Partial;
        bindings.omitted_sites = 0;
        bindings.reasons = std::collections::BTreeSet::from([
            compass_model::source_query::ParameterBindingOmission::TraversalLimit,
        ]);
    }
    partial.validate()?;
    let mut historical = body.clone();
    historical.profile = compass_model::source_query::SOURCE_QUERY_PROFILE_V5.to_owned();
    for owner in &mut historical.owners {
        owner.parameter_bindings = None;
    }
    historical.validate()?;
    let variants = [
        serde_json::to_vec(&body)?,
        serde_json::to_vec(&partial)?,
        serde_json::to_vec(&historical)?,
    ];
    let mut expected = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        for bytes in &variants {
            fs::write(built.output_dir.join(SOURCE_QUERY_FILE), bytes)?;
            let engine = open_with_engine(
                &graph,
                None,
                &d.path().join(format!("query-{mode:?}")),
                mode,
            )?;
            let mut query = request();
            query.question = "Where are session values encoded?".to_owned();
            let answer = engine.discover_ranked(query)?;
            let candidate = answer
                .candidates
                .iter()
                .find(|c| c.node.id == node.id)
                .ok_or("source-owned implementation missing")?;
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("Implementation-local operation identifiers")),
                "code-only truncation must not erase complete identifier inspection: {candidate:?}"
            );
            assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
            assert!(!candidate.matched_fields.iter().any(|f| f == "body_code"));
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("windows were omitted"))
            );
            if let Some(expected) = &expected {
                assert_eq!(&answer.candidates, expected);
            } else {
                expected = Some(answer.candidates);
            }
        }
    }
    Ok(())
}

#[test]
fn string_literal_concepts_recall_direct_owners_on_both_backends() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    for (file, source) in [
        (
            "policy.py",
            "def guard(value):\n    if value == 'retired':\n        raise ValueError('retired account access is rejected')\n    return value\n\ndef enclosing(value):\n    def hidden():\n        raise ValueError('orphan administrator tokens are rejected')\n    return hidden()\n",
        ),
        (
            "policy.rs",
            "fn guard(value: &str) -> Result<(), &'static str> {\n    if value == \"retired\" { return Err(\"retired account access is rejected\"); }\n    Ok(())\n}\n",
        ),
        (
            "policy.go",
            "package policy\nimport \"fmt\"\nfunc guard(value string) error {\n    if value == \"retired\" { return fmt.Errorf(\"retired account access is rejected\") }\n    return nil\n}\n",
        ),
        (
            "policy.ts",
            "function guard(value: string) {\n    if (value === 'retired') { throw new Error('retired account access is rejected'); }\n    return value;\n}\n",
        ),
    ] {
        fs::write(root.join(file), source)?;
    }
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let mut expected = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        let mut query = request();
        query.question = "Where are retired account access attempts rejected?".to_owned();
        let answer = engine.discover_ranked(query)?;
        for file in ["policy.py", "policy.rs", "policy.go", "policy.ts"] {
            let candidate = answer
                .candidates
                .iter()
                .find(|c| {
                    c.node.source.as_ref().is_some_and(|s| s.file == file)
                        && c.node.name.trim_end_matches("()") == "guard"
                })
                .ok_or_else(|| format!("literal-only owner missing for {file}: {answer:?}"))?;
            assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
            assert!(candidate.matched_fields.iter().any(|f| f == "body_code"));
            assert!(!candidate.supporting_sources.is_empty());
            assert!(
                candidate
                    .excerpts
                    .iter()
                    .any(|e| e.text.contains("retired account"))
            );
            assert!(
                !candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("inspection preference"))
            );
        }
        if let Some(expected) = &expected {
            assert_eq!(&answer.candidates, expected);
        } else {
            expected = Some(answer.candidates);
        }
        let mut nested = request();
        nested.question = "Where are orphan administrator tokens rejected?".to_owned();
        let answer = engine.discover_ranked(nested)?;
        assert!(
            answer
                .candidates
                .iter()
                .any(|c| c.node.name.trim_end_matches("()") == "hidden")
        );
        assert!(
            !answer
                .candidates
                .iter()
                .any(|c| c.node.name.trim_end_matches("()") == "enclosing")
        );
    }
    Ok(())
}

#[test]
fn historical_code_windows_do_not_gain_literal_recall_or_reuse_current_answers()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::SOURCE_QUERY_PROFILE_V4;
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("policy.py"),
        "def guard(value):\n    if value == 'retired':\n        raise ValueError('retired account access is rejected')\n    return value\n",
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let path = built.output_dir.join(SOURCE_QUERY_FILE);
    let graph_path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let original = fs::read(&path)?;
    let mut legacy: SourceQueryArtifact = serde_json::from_slice(&original)?;
    legacy.profile = SOURCE_QUERY_PROFILE_V4.to_owned();
    legacy
        .owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
    legacy.additional_document_frequencies.clear();
    for owner in &legacy.owners {
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == owner.node_id)
            .ok_or("missing source owner")?;
        let base = compass_model::search::search_document(node).terms();
        for term in owner.document_for_profile(SOURCE_QUERY_PROFILE_V4)?.terms() {
            *legacy
                .additional_document_frequencies
                .entry(term.clone())
                .or_default() += u64::from(!base.contains(&term));
        }
    }
    legacy.validate()?;
    let legacy_bytes = serde_json::to_vec(&legacy)?;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let cache = d.path().join(format!("cache-{mode:?}"));
        for (bytes, should_recall) in [(&original, true), (&legacy_bytes, false), (&original, true)]
        {
            fs::write(&path, bytes)?;
            let engine = open_with_engine(&graph_path, None, &cache, mode)?;
            let mut query = request();
            query.question = "Where are retired account access attempts rejected?".to_owned();
            assert_eq!(
                !engine.discover_ranked(query)?.candidates.is_empty(),
                should_recall
            );
            assert_eq!(
                &fs::read(&path)?,
                bytes,
                "reader rewrote immutable evidence"
            );
        }
    }
    Ok(())
}
#[test]
fn outside_edit_does_not_widen_a_declaration_that_ends_at_eof() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source =
        "fn opaque(value: &str) -> &str {\nlet result = value;\n// Handle payload here.\nresult }";
    fs::write(root.join("sample.rs"), source)?;
    let o = options(&root, &d.path().join("artifacts"));
    let first = build_local_graph(&o)?;
    let a = GraphDocument::load(&first.output_dir.join("graph.json"))?;
    let owner = a
        .nodes
        .iter()
        .find(|n| n.kind.is_callable() && n.name.trim_end_matches("()") == "opaque")
        .ok_or("missing owner")?;
    let expected = owner.source.clone();
    assert_eq!(
        expected.as_ref().map(|s| s.end_byte),
        Some(source.len() as u64)
    );
    fs::write(
        root.join("sample.rs"),
        format!("{source}\n// outside declaration\n"),
    )?;
    let second = build_local_graph(&o)?;
    let b = GraphDocument::load(&second.output_dir.join("graph.json"))?;
    let owner = b
        .nodes
        .iter()
        .find(|n| n.kind.is_callable() && n.name.trim_end_matches("()") == "opaque")
        .ok_or("missing owner")?;
    assert_eq!(
        owner.source, expected,
        "a coincident file/declaration range is not a file envelope"
    );
    let engine = open_with_engine(
        &second.output_dir.join("graph.json"),
        None,
        &d.path().join("query"),
        EngineSelection::Json,
    )?;
    assert!(
        engine
            .discover_ranked(request())?
            .candidates
            .iter()
            .any(|c| c.node.id == owner.id)
    );
    Ok(())
}
#[test]
fn schema_valid_non_callable_body_owners_are_rejected() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = "struct Holder {\n// Handle payload.\nvalue: u8\n}\nfn opaque() {}\n";
    fs::write(root.join("sample.rs"), source)?;
    let o = options(&root, &d.path().join("artifacts"));
    let built = build_local_graph(&o)?;
    let path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&path)?;
    let node = graph
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.name.trim_end_matches("()") == "Holder")
        .ok_or("missing struct")?;
    let mut artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    let text = "// Handle payload.";
    let start = source.find(text).ok_or("missing comment")?;
    let owner = SourceQueryOwner {
        node_id: node.id.clone(),
        source: node.source.clone().ok_or("missing struct source")?,
        source_digest: artifact.files[0].source_digest.clone(),
        identifiers: Vec::new(),
        parameter_bindings: Some(Default::default()),
        comments: vec![SourceQuerySite {
            source: SourceAnchor {
                file: "sample.rs".into(),
                start_byte: start as u64,
                end_byte: (start + text.len()) as u64,
                start_line: 2,
                start_column: 0,
                end_line: 2,
                end_column: text.len() as u32,
            },
            text: text.into(),
        }],
        code_windows: Vec::new(),
        omitted_code_windows: 0,
        nested_contexts: Vec::new(),
        omitted_nested_contexts: 0,
        omitted_sites: 0,
        truncated: false,
        syntax_recovery: false,
    };
    let base = compass_model::search::search_document(node).terms();
    artifact.additional_document_frequencies = owner
        .document()
        .terms()
        .into_iter()
        .map(|term| {
            let df = u64::from(!base.contains(&term));
            (term, df)
        })
        .collect();
    artifact.owners = vec![owner];
    artifact.validate()?;
    fs::write(
        built.output_dir.join(SOURCE_QUERY_FILE),
        serde_json::to_vec(&artifact)?,
    )?;
    let engine = open_with_engine(&path, None, &d.path().join("query"), EngineSelection::Json)?;
    assert!(
        engine.discover_ranked(request()).is_err(),
        "callable source evidence cannot be attached to a struct"
    );
    Ok(())
}
#[test]
fn unknown_major_is_a_typed_error_even_without_current_version_fields() -> Result<(), Box<dyn Error>>
{
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(root.join("sample.rs"), "fn opaque() {}")?;
    let o = options(&root, &d.path().join("artifacts"));
    let built = build_local_graph(&o)?;
    fs::write(
        built.output_dir.join(SOURCE_QUERY_FILE),
        br#"{"schema":"compass.source-query/2"}"#,
    )?;
    let result = open_with_engine(
        &built.output_dir.join("graph.json"),
        None,
        &d.path().join("query"),
        EngineSelection::Json,
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => return Err("unknown major was admitted".into()),
    };
    assert_eq!(error.kind(), QueryErrorKind::UnsupportedSchema);
    assert_eq!(error.code(), "unsupported_source_query_schema");
    Ok(())
}

#[test]
fn immutable_legacy_projection_is_readable_without_rewriting_on_both_backends()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{
        SOURCE_QUERY_PROFILE, SOURCE_QUERY_PROFILE_V1, SOURCE_QUERY_PROFILE_V2,
    };
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.rs"),
        "fn opaque(value: &str) -> &str { let retained = value; // Handle payload here.\n retained }\n",
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let path = built.output_dir.join(SOURCE_QUERY_FILE);
    let original = fs::read(&path)?;
    let mut artifact: SourceQueryArtifact = serde_json::from_slice(&original)?;
    assert_eq!(artifact.profile, SOURCE_QUERY_PROFILE);
    let mut expected = open_with_engine(
        &graph,
        None,
        &d.path().join("current"),
        EngineSelection::Json,
    )?
    .discover_ranked(request())?
    .candidates;
    assert!(!expected.is_empty());
    for owner in &mut artifact.owners {
        owner.code_windows.clear();
        owner.omitted_code_windows = 0;
    }
    let legacy_terms = artifact
        .owners
        .iter()
        .map(|owner| owner.document_for_profile(SOURCE_QUERY_PROFILE_V2))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flat_map(|document| document.terms())
        .collect::<std::collections::BTreeSet<_>>();
    artifact
        .additional_document_frequencies
        .retain(|term, _| legacy_terms.contains(term));
    for candidate in &mut expected {
        candidate
            .excerpts
            .retain(|quote| quote.kind != compass_query::RankedExcerptKind::CallableCode);
    }
    for profile in [SOURCE_QUERY_PROFILE_V1, SOURCE_QUERY_PROFILE_V2] {
        artifact.profile = profile.to_owned();
        if profile != SOURCE_QUERY_PROFILE {
            artifact
                .owners
                .iter_mut()
                .for_each(|owner| owner.parameter_bindings = None);
        }
        artifact.validate()?;
        let legacy = serde_json::to_vec(&artifact)?;
        fs::write(&path, &legacy)?;
        for mode in [EngineSelection::Json, EngineSelection::Store] {
            let engine = open_with_engine(
                &graph,
                None,
                &d.path().join(format!("legacy-{profile}-{mode:?}")),
                mode,
            )?;
            assert_eq!(engine.discover_ranked(request())?.candidates, expected);
            assert_eq!(
                fs::read(&path)?,
                legacy,
                "query rewrote immutable legacy evidence"
            );
        }
    }
    // Version dispatch must precede decoding the current payload fields.
    fs::write(
        &path,
        br#"{"schema":"compass.source-query/1","profile":"source-query/999-future"}"#,
    )?;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let result = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("future-{mode:?}")),
            mode,
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => return Err("unknown profile was admitted".into()),
        };
        assert_eq!(error.kind(), QueryErrorKind::UnsupportedSchema);
        assert_eq!(error.code(), "unsupported_source_query_profile");
    }
    Ok(())
}

#[test]
fn parameter_binding_publication_preserves_unrelated_v5_queries_and_exact_owners()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{
        ParameterBindingStatus, SOURCE_QUERY_PROFILE, SOURCE_QUERY_PROFILE_V5,
    };
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    for (file, source) in [
        (
            "sample.py",
            "def probe(x=use(block), block: Permit=0):\n    return block\n",
        ),
        ("sample.rs", "fn probe(block: Block, permit: Permit) {}"),
        ("sample.go", "package p\nfunc probe(block, permit Block) {}"),
        (
            "sample.ts",
            "function probe(block: Block = fallback, permit?: Permit) {}",
        ),
        (
            "sample.tsx",
            "const probe = (block: Block, permit = fallback) => block;",
        ),
    ] {
        fs::write(root.join(file), source)?;
    }
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph_path = built.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let sidecar = built.output_dir.join(SOURCE_QUERY_FILE);
    let original = fs::read(&sidecar)?;
    let current: SourceQueryArtifact = serde_json::from_slice(&original)?;
    assert_eq!(current.profile, SOURCE_QUERY_PROFILE);
    current.validate()?;
    for file in [
        "sample.py",
        "sample.rs",
        "sample.go",
        "sample.ts",
        "sample.tsx",
    ] {
        let node = graph
            .nodes
            .iter()
            .find(|n| n.kind.is_callable() && n.source.as_ref().is_some_and(|s| s.file == file))
            .ok_or("missing native owner")?;
        let owner = current
            .owners
            .iter()
            .find(|o| o.node_id == node.id)
            .ok_or("missing exact published owner")?;
        assert_eq!(
            owner.source,
            *node.source.as_ref().ok_or("missing node source")?
        );
        let bindings = owner
            .parameter_bindings
            .as_ref()
            .ok_or("missing explicit parameter coverage")?;
        assert_eq!(
            bindings.status,
            ParameterBindingStatus::Complete,
            "{file}: {bindings:?}"
        );
        let expected = if file == "sample.py" {
            vec!["x", "block"]
        } else {
            vec!["block", "permit"]
        };
        assert_eq!(
            bindings
                .sites
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{file}"
        );
        let source = fs::read(root.join(file))?;
        for site in &bindings.sites {
            assert_eq!(site.source.file, file);
            assert_eq!(
                &source[site.source.start_byte as usize..site.source.end_byte as usize],
                site.text.as_bytes()
            );
        }
        if file == "sample.py" {
            let ordinary = owner
                .identifiers
                .iter()
                .find(|s| s.text == "block")
                .ok_or("missing generic spelling")?;
            assert!(ordinary.source.start_byte < bindings.sites[1].source.start_byte);
        }
    }
    let mut legacy = current.clone();
    legacy.profile = SOURCE_QUERY_PROFILE_V5.to_owned();
    for owner in &mut legacy.owners {
        owner.parameter_bindings = None;
    }
    legacy.validate()?;
    let legacy_bytes = serde_json::to_vec(&legacy)?;
    let mut expected_candidates = None;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let mut expected = None;
        for bytes in [&original, &legacy_bytes, &original] {
            fs::write(&sidecar, bytes)?;
            let engine = open_with_engine(
                &graph_path,
                None,
                &d.path().join(format!("cache-{mode:?}")),
                mode,
            )?;
            let mut query = request();
            query.question = "Where is probe implemented?".to_owned();
            let answer = engine.discover_ranked(query)?;
            assert!(!answer.candidates.is_empty());
            if let Some(expected_candidates) = &expected_candidates {
                assert_eq!(&answer.candidates, expected_candidates);
            } else {
                expected_candidates = Some(answer.candidates.clone());
            }
            if let Some(expected) = &expected {
                assert_eq!(&answer, expected);
            } else {
                expected = Some(answer);
            }
            assert_eq!(
                &fs::read(&sidecar)?,
                bytes,
                "reader rewrote historical profile"
            );
        }
    }
    Ok(())
}
