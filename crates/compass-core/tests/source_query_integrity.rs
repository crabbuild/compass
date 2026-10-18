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
    for candidate in &mut expected {
        candidate
            .excerpts
            .retain(|quote| quote.kind != compass_query::RankedExcerptKind::CallableCode);
    }
    for profile in [SOURCE_QUERY_PROFILE_V1, SOURCE_QUERY_PROFILE_V2] {
        artifact.profile = profile.to_owned();
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
