use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, CoreError, build_local_graph};
use compass_ir::hex_sha256;
use protobuf::{EnumOrUnknown, Message, MessageField};
use scip::types::{
    Document, Index, Metadata, Occurrence, PositionEncoding, SymbolInformation, SymbolRole,
    TextEncoding, ToolInfo,
};

type ScipFixtureDocument<'a> = (&'a str, &'a str, &'a str, Vec<i32>, Vec<i32>, &'a str);

fn program_options(root: &Path) -> BuildOptions {
    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.program_analysis = true;
    options
}

fn scip_fixture(
    path: &str,
    language: &str,
    source: &str,
    definition_range: Vec<i32>,
    reference_range: Vec<i32>,
    symbol: &str,
) -> Result<Vec<u8>, protobuf::Error> {
    let mut tool = ToolInfo::new();
    tool.name = "fixture-indexer".to_owned();
    tool.version = "1.0".to_owned();
    tool.arguments = vec!["/absolute/path/must/not/escape".to_owned()];
    let mut metadata = Metadata::new();
    metadata.tool_info = MessageField::some(tool);
    metadata.project_root = "file:///absolute/checkout".to_owned();
    metadata.text_document_encoding = EnumOrUnknown::new(TextEncoding::UTF8);
    let document = scip_document(
        path,
        language,
        source,
        definition_range,
        reference_range,
        symbol,
    );
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = vec![document];
    index.write_to_bytes()
}

fn scip_document(
    path: &str,
    language: &str,
    source: &str,
    definition_range: Vec<i32>,
    reference_range: Vec<i32>,
    symbol: &str,
) -> Document {
    let mut definition = Occurrence::new();
    definition.range = definition_range;
    definition.symbol = symbol.to_owned();
    definition.symbol_roles = SymbolRole::Definition as i32;
    let mut reference = Occurrence::new();
    reference.range = reference_range;
    reference.symbol = symbol.to_owned();
    reference.symbol_roles = SymbolRole::ReadAccess as i32;
    let mut information = SymbolInformation::new();
    information.symbol = symbol.to_owned();
    let mut document = Document::new();
    document.language = language.to_owned();
    document.relative_path = path.to_owned();
    document.occurrences = vec![definition, reference];
    document.symbols = vec![information];
    document.text = source.to_owned();
    document.position_encoding =
        EnumOrUnknown::new(PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
    document
}

fn write_scip(
    artifact: &Path,
    source_path: &str,
    language: &str,
    source: &str,
    ranges: (Vec<i32>, Vec<i32>),
    symbol: &str,
    manifest: bool,
) -> Result<(), Box<dyn Error>> {
    let bytes = scip_fixture(source_path, language, source, ranges.0, ranges.1, symbol)?;
    fs::write(artifact, &bytes)?;
    if manifest {
        let index_digest = hex_sha256(&bytes);
        let source_digest = hex_sha256(source.as_bytes());
        let companion = artifact.with_file_name(format!(
            "{}.compass-manifest.json",
            artifact
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("non-UTF-8 artifact name")?
        ));
        fs::write(
            companion,
            format!(
                r#"{{"schema":"compass.scip-manifest/1","index_sha256":"{index_digest}","documents":{{"{source_path}":"{source_digest}"}}}}"#
            ),
        )?;
    }
    Ok(())
}

fn write_multi_scip(
    artifact: &Path,
    documents: &[ScipFixtureDocument<'_>],
) -> Result<(), Box<dyn Error>> {
    let mut tool = ToolInfo::new();
    tool.name = "fixture-indexer".to_owned();
    tool.version = "1.0".to_owned();
    let mut metadata = Metadata::new();
    metadata.tool_info = MessageField::some(tool);
    metadata.project_root = "file:///absolute/checkout".to_owned();
    metadata.text_document_encoding = EnumOrUnknown::new(TextEncoding::UTF8);
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = documents
        .iter()
        .map(|(path, language, source, definition, reference, symbol)| {
            scip_document(
                path,
                language,
                source,
                definition.clone(),
                reference.clone(),
                symbol,
            )
        })
        .collect();
    let bytes = index.write_to_bytes()?;
    fs::write(artifact, &bytes)?;
    let companion = artifact.with_file_name(format!(
        "{}.compass-manifest.json",
        artifact
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("non-UTF-8 artifact name")?
    ));
    let manifest_documents = documents
        .iter()
        .map(|(path, _, source, _, _, _)| {
            (
                path.to_string(),
                serde_json::Value::String(hex_sha256(source.as_bytes())),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    fs::write(
        companion,
        serde_json::to_vec(&serde_json::json!({
            "schema": "compass.scip-manifest/1",
            "index_sha256": hex_sha256(&bytes),
            "documents": manifest_documents,
        }))?,
    )?;
    Ok(())
}

#[test]
fn program_pipeline_is_deterministic_incremental_and_uses_program_json()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    fs::write(
        &source,
        "pub fn helper(value: i32) -> i32 { value + 1 }\npub fn run() { let answer = helper(41); println!(\"{answer}\"); }\n",
    )?;
    let options = program_options(directory.path());

    let cold = build_local_graph(&options)?;
    let output = cold.output_dir.join("program.json");
    assert!(output.is_file());
    assert!(!cold.output_dir.join(".compass_program.json").exists());
    assert_eq!(cold.program_modules, 1);
    assert!(cold.program_summaries >= 2);
    assert_eq!(cold.program_syntax_analyzed, 1);
    assert_eq!(cold.program_syntax_reused, 0);
    let cold_bytes = fs::read(&output)?;
    let document: serde_json::Value = serde_json::from_slice(&cold_bytes)?;
    assert_eq!(
        document["program"]["schema"],
        "http://crab.build/compass/v1"
    );
    assert_eq!(document["analysis_schema_version"], 1);

    let warm = build_local_graph(&options)?;
    assert_eq!(warm.program_syntax_analyzed, 0);
    assert_eq!(warm.program_syntax_reused, 1);
    assert_eq!(fs::read(&output)?, cold_bytes);

    fs::write(&output, serde_json::to_vec_pretty(&document)?)?;
    let repaired = build_local_graph(&options)?;
    assert_eq!(repaired.program_syntax_reused, 1);
    assert_eq!(fs::read(&output)?, cold_bytes);

    fs::write(
        &source,
        "pub fn helper(value: i32) -> i32 { value + 2 }\npub fn run() { let answer = helper(40); println!(\"{answer}\"); }\n",
    )?;
    let changed = build_local_graph(&options)?;
    assert_eq!(changed.program_syntax_analyzed, 1);
    assert_eq!(changed.program_syntax_reused, 0);
    assert_ne!(fs::read(&output)?, cold_bytes);
    Ok(())
}

#[test]
fn program_pipeline_is_opt_in_at_the_core_api() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn visible() {}\n")?;
    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;

    let result = build_local_graph(&options)?;
    assert!(!result.output_dir.join("program.json").exists());
    assert_eq!(result.program_modules, 0);
    assert_eq!(result.program_summaries, 0);
    Ok(())
}

#[test]
fn invalid_explicit_artifact_does_not_replace_existing_program() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn stable() {}\n")?;
    let mut options = program_options(directory.path());
    let first = build_local_graph(&options)?;
    let program_path = first.output_dir.join("program.json");
    let before = fs::read(&program_path)?;

    options
        .program_artifacts
        .push(directory.path().join("missing.scip"));
    assert!(matches!(
        build_local_graph(&options),
        Err(CoreError::InvalidProgramInput(_))
    ));
    assert_eq!(fs::read(program_path)?, before);
    Ok(())
}

#[test]
fn scip_cache_tracks_artifact_manifest_and_source_freshness() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_text = "function work() {}\nfunction run() { work(); }\n";
    fs::create_dir(directory.path().join("src"))?;
    let source = directory.path().join("src/app.ts");
    fs::write(&source, source_text)?;
    let unrelated = directory.path().join("src/unrelated.ts");
    fs::write(&unrelated, "export const unrelated = 1;\n")?;
    let artifact = directory.path().join("index.scip");
    write_scip(
        &artifact,
        "src/app.ts",
        "typescript",
        source_text,
        (vec![0, 9, 13], vec![1, 17, 21]),
        "typescript npm fixture 1.0 work().",
        true,
    )?;
    let options = program_options(directory.path());

    let cold = build_local_graph(&options)?;
    assert_eq!(cold.program_syntax_analyzed, 2);
    assert_eq!(cold.program_artifacts_loaded, 1);
    assert_eq!(cold.program_artifacts_reused, 0);
    assert_eq!(cold.program_artifact_documents_analyzed, 1);
    assert_eq!(cold.program_artifact_documents_reused, 0);
    let program_path = cold.output_dir.join("program.json");
    let first = fs::read(&program_path)?;
    assert!(String::from_utf8_lossy(&first).contains("npm fixture"));

    let warm = build_local_graph(&options)?;
    assert_eq!(warm.program_syntax_reused, 2);
    assert_eq!(warm.program_artifacts_loaded, 0);
    assert_eq!(warm.program_artifacts_reused, 1);
    assert_eq!(warm.program_artifact_documents_analyzed, 0);
    assert_eq!(warm.program_artifact_documents_reused, 0);
    assert_eq!(fs::read(&program_path)?, first);

    fs::write(&unrelated, "export const unrelated = 2;\n")?;
    let unrelated_changed = build_local_graph(&options)?;
    assert_eq!(unrelated_changed.program_artifacts_loaded, 0);
    assert_eq!(unrelated_changed.program_artifacts_reused, 1);
    assert_eq!(unrelated_changed.program_artifact_documents_analyzed, 0);
    assert_eq!(unrelated_changed.program_artifact_documents_reused, 1);

    write_scip(
        &artifact,
        "src/app.ts",
        "typescript",
        source_text,
        (vec![0, 9, 13], vec![1, 17, 21]),
        "typescript npm fixture 2.0 work().",
        true,
    )?;
    let artifact_changed = build_local_graph(&options)?;
    assert_eq!(artifact_changed.program_syntax_reused, 2);
    assert_eq!(artifact_changed.program_artifacts_loaded, 1);
    assert_eq!(artifact_changed.program_artifact_documents_analyzed, 1);
    assert_eq!(artifact_changed.program_artifact_documents_reused, 0);
    let second = fs::read(&program_path)?;
    assert_ne!(second, first);
    assert!(String::from_utf8_lossy(&second).contains("fixture 2.0"));

    let changed_source = "function work() {}\nfunction run() { work(); work(); }\n";
    fs::write(&source, changed_source)?;
    let stale = build_local_graph(&options)?;
    assert_eq!(stale.program_syntax_analyzed, 1);
    assert_eq!(stale.program_artifacts_loaded, 0);
    assert_eq!(stale.program_artifacts_reused, 1);
    assert_eq!(stale.program_artifact_documents_analyzed, 1);
    assert_eq!(stale.program_artifact_documents_reused, 0);
    let stale_bytes = fs::read(&program_path)?;
    assert!(!String::from_utf8_lossy(&stale_bytes).contains("fixture 2.0"));

    fs::remove_file(source)?;
    let deleted = build_local_graph(&options)?;
    assert_eq!(deleted.program_modules, 1);
    assert!(!String::from_utf8_lossy(&fs::read(&program_path)?).contains("src/app.ts"));
    Ok(())
}

#[test]
fn scip_cache_renormalizes_only_the_changed_document() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("src"))?;
    let source_a = "function alpha() {}\nfunction useA() { alpha(); }\n";
    let source_b = "function beta() {}\nfunction useB() { beta(); }\n";
    fs::write(directory.path().join("src/a.ts"), source_a)?;
    fs::write(directory.path().join("src/b.ts"), source_b)?;
    let artifact = directory.path().join("index.scip");
    write_multi_scip(
        &artifact,
        &[
            (
                "src/a.ts",
                "typescript",
                source_a,
                vec![0, 9, 14],
                vec![1, 18, 23],
                "typescript npm fixture 1.0 alpha().",
            ),
            (
                "src/b.ts",
                "typescript",
                source_b,
                vec![0, 9, 13],
                vec![1, 18, 22],
                "typescript npm fixture 1.0 beta().",
            ),
        ],
    )?;
    let options = program_options(directory.path());

    let cold = build_local_graph(&options)?;
    assert_eq!(cold.program_artifacts_loaded, 1);
    assert_eq!(cold.program_artifact_documents_analyzed, 2);
    assert_eq!(cold.program_artifact_documents_reused, 0);

    let warm = build_local_graph(&options)?;
    assert_eq!(warm.program_artifacts_loaded, 0);
    assert_eq!(warm.program_artifacts_reused, 1);
    assert_eq!(warm.program_artifact_documents_analyzed, 0);
    assert_eq!(warm.program_artifact_documents_reused, 0);

    fs::write(
        directory.path().join("src/a.ts"),
        "function alpha() {}\nfunction useA() { alpha(); alpha(); }\n",
    )?;
    let changed = build_local_graph(&options)?;
    assert_eq!(changed.program_artifacts_loaded, 0);
    assert_eq!(changed.program_artifacts_reused, 1);
    assert_eq!(changed.program_artifact_documents_analyzed, 1);
    assert_eq!(changed.program_artifact_documents_reused, 1);
    Ok(())
}

#[test]
fn checkout_roots_and_explicit_artifact_order_do_not_affect_program_bytes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let artifacts = directory.path().join("artifacts");
    fs::create_dir(&artifacts)?;
    let source_a = "pub fn alpha() {}\npub fn call_alpha() { alpha(); }\n";
    let source_b = "pub fn beta() {}\npub fn call_beta() { beta(); }\n";
    let artifact_a = artifacts.join("a.scip");
    let artifact_b = artifacts.join("b.scip");
    write_scip(
        &artifact_a,
        "src/a.rs",
        "rust",
        source_a,
        (vec![0, 7, 12], vec![1, 22, 27]),
        "rust cargo fixture 1.0 alpha().",
        false,
    )?;
    write_scip(
        &artifact_b,
        "src/b.rs",
        "rust",
        source_b,
        (vec![0, 7, 11], vec![1, 21, 25]),
        "rust cargo fixture 1.0 beta().",
        false,
    )?;
    let roots = [
        directory.path().join("first"),
        directory.path().join("second"),
    ];
    for root in &roots {
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/a.rs"), source_a)?;
        fs::write(root.join("src/b.rs"), source_b)?;
    }

    let mut first_options = program_options(&roots[0]);
    first_options.program_artifacts = vec![artifact_a.clone(), artifact_b.clone()];
    let first = build_local_graph(&first_options)?;
    let first_bytes = fs::read(first.output_dir.join("program.json"))?;
    assert!(
        !String::from_utf8_lossy(&first_bytes)
            .contains(directory.path().to_string_lossy().as_ref())
    );

    let mut second_options = program_options(&roots[1]);
    second_options.program_artifacts = vec![artifact_b, artifact_a];
    let second = build_local_graph(&second_options)?;
    assert_eq!(
        fs::read(second.output_dir.join("program.json"))?,
        first_bytes
    );
    Ok(())
}

#[test]
fn malformed_discovered_scip_and_obstructed_output_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn stable() {}\n")?;
    let options = program_options(directory.path());
    let first = build_local_graph(&options)?;
    let program_path = first.output_dir.join("program.json");
    let before = fs::read(&program_path)?;

    fs::write(directory.path().join("index.scip"), [0x12, 0x05, 0x01])?;
    assert!(build_local_graph(&options).is_err());
    assert_eq!(fs::read(&program_path)?, before);
    assert!(first.output_dir.join(".compass-build-incomplete").is_file());

    fs::remove_file(directory.path().join("index.scip"))?;
    fs::remove_file(&program_path)?;
    fs::create_dir(&program_path)?;
    assert!(build_local_graph(&options).is_err());
    assert!(program_path.is_dir());
    assert!(first.output_dir.join(".compass-build-incomplete").is_file());
    Ok(())
}
