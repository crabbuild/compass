use std::collections::BTreeMap;
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
    scip_fixture_with_tool(
        path,
        language,
        source,
        definition_range,
        reference_range,
        symbol,
        "fixture-indexer",
        "1.0",
    )
}

#[allow(clippy::too_many_arguments)]
fn scip_fixture_with_tool(
    path: &str,
    language: &str,
    source: &str,
    definition_range: Vec<i32>,
    reference_range: Vec<i32>,
    symbol: &str,
    tool_name: &str,
    tool_version: &str,
) -> Result<Vec<u8>, protobuf::Error> {
    let mut tool = ToolInfo::new();
    tool.name = tool_name.to_owned();
    tool.version = tool_version.to_owned();
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

fn write_managed_python_scip(
    artifact: &Path,
    source: &str,
    stubs_digest: char,
) -> Result<String, Box<dyn Error>> {
    let source_path = "src/app.py";
    let bytes = scip_fixture_with_tool(
        source_path,
        "python",
        source,
        vec![0, 4, 10],
        vec![1, 11, 17],
        "python pypi fixture 1.0 target().",
        "scip-python",
        "0.6.2",
    )?;
    fs::write(artifact, &bytes)?;
    let source_digest = hex_sha256(source.as_bytes());
    let source_inventory_digest = compass_program::source_inventory_digest(&BTreeMap::from([(
        source_path.to_owned(),
        source_digest.clone(),
    )]))?;
    let profile = serde_json::json!({
        "schema": "compass.managed-analyzer-profile/1",
        "language": "python",
        "provider": "scip-python",
        "provider_version": "0.6.2",
        "protocol_version": "scip/1",
        "state": "complete",
        "source_inventory_digest": source_inventory_digest,
        "environment": {
            "implementation": "cpython",
            "python_version": "3.12.5",
            "platform": "manylinux_2_28_x86_64",
            "source_roots": ["src"],
            "import_roots": ["stubs"],
            "editable_packages": [],
            "environment_digest": "c".repeat(64),
            "project_configuration_digest": "d".repeat(64),
            "typeshed_digest": "e".repeat(64),
            "stubs_digest": stubs_digest.to_string().repeat(64),
            "namespace_policy": "pep420",
            "use_library_code_for_types": false
        },
        "permissions": {
            "allow_dependency_network": false,
            "allow_package_install": false,
            "allow_project_execution": false
        }
    });
    let profile: compass_program::ManagedAnalyzerProfile = serde_json::from_value(profile.clone())?;
    let profile_digest = compass_program::managed_analyzer_profile_digest(&profile)?;
    let companion = artifact.with_file_name("index.scip.compass-manifest.json");
    fs::write(
        companion,
        serde_json::to_vec(&serde_json::json!({
            "schema": "compass.scip-manifest/1",
            "index_sha256": hex_sha256(&bytes),
            "documents": {source_path: source_digest},
            "managed_analyzer": profile,
        }))?,
    )?;
    Ok(profile_digest)
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

fn write_java_overload_scip(
    artifact: &Path,
    source_path: &str,
    source: &str,
) -> Result<(), Box<dyn Error>> {
    let int_symbol = "java maven fixture 1.0 Demo#pick(int).";
    let string_symbol = "java maven fixture 1.0 Demo#pick(java.lang.String).";
    let use_symbol = "java maven fixture 1.0 Demo#use().";
    let mut occurrences = Vec::new();
    for (range, symbol, role) in [
        (vec![1, 7, 11], int_symbol, SymbolRole::Definition),
        (vec![2, 7, 11], string_symbol, SymbolRole::Definition),
        (vec![3, 7, 10], use_symbol, SymbolRole::Definition),
        (vec![3, 15, 19], string_symbol, SymbolRole::ReadAccess),
    ] {
        let mut occurrence = Occurrence::new();
        occurrence.range = range;
        occurrence.symbol = symbol.to_owned();
        occurrence.symbol_roles = role as i32;
        occurrences.push(occurrence);
    }
    let symbols = [int_symbol, string_symbol, use_symbol]
        .into_iter()
        .map(|symbol| {
            let mut information = SymbolInformation::new();
            information.symbol = symbol.to_owned();
            information
        })
        .collect();
    let mut document = Document::new();
    document.language = "java".to_owned();
    document.relative_path = source_path.to_owned();
    document.occurrences = occurrences;
    document.symbols = symbols;
    document.text = source.to_owned();
    document.position_encoding =
        EnumOrUnknown::new(PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
    let mut tool = ToolInfo::new();
    tool.name = "fixture-indexer".to_owned();
    tool.version = "1.0".to_owned();
    let mut metadata = Metadata::new();
    metadata.tool_info = MessageField::some(tool);
    metadata.text_document_encoding = EnumOrUnknown::new(TextEncoding::UTF8);
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = vec![document];
    let bytes = index.write_to_bytes()?;
    fs::write(artifact, &bytes)?;
    let companion = artifact.with_file_name("index.scip.compass-manifest.json");
    fs::write(
        companion,
        serde_json::to_vec(&serde_json::json!({
            "schema": "compass.scip-manifest/1",
            "index_sha256": hex_sha256(&bytes),
            "documents": {source_path: hex_sha256(source.as_bytes())},
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
    let cold_output = cold.output_dir.join("program.json");
    assert!(cold_output.is_file());
    assert_eq!(cold.program_modules, 1);
    assert!(cold.program_summaries >= 2);
    assert_eq!(cold.program_syntax_analyzed, 1);
    assert_eq!(cold.program_syntax_reused, 0);
    let cold_bytes = fs::read(&cold_output)?;
    let document: serde_json::Value = serde_json::from_slice(&cold_bytes)?;
    assert_eq!(
        document["program"]["schema"],
        "http://crab.build/compass/v1"
    );
    assert_eq!(
        document["analysis_schema_version"],
        compass_analysis::ANALYSIS_SCHEMA_VERSION
    );

    let warm = build_local_graph(&options)?;
    assert_eq!(warm.program_syntax_analyzed, 0);
    assert_eq!(warm.program_syntax_reused, 1);
    let warm_output = warm.output_dir.join("program.json");
    assert_eq!(fs::read(&warm_output)?, cold_bytes);

    let mut same_size_program_damage = cold_bytes.clone();
    same_size_program_damage[0] = b'[';
    fs::write(&warm_output, same_size_program_damage)?;
    let repaired_same_size_program = build_local_graph(&options)?;
    assert_eq!(repaired_same_size_program.program_syntax_reused, 1);
    let repaired_program_output = repaired_same_size_program.output_dir.join("program.json");
    assert_eq!(fs::read(&repaired_program_output)?, cold_bytes);

    let graph_output = repaired_same_size_program.output_dir.join("graph.json");
    let graph_bytes = fs::read(&graph_output)?;
    let mut same_size_graph_damage = graph_bytes.clone();
    same_size_graph_damage[0] = b'[';
    fs::write(&graph_output, same_size_graph_damage)?;
    let repaired_same_size_graph = build_local_graph(&options)?;
    assert_eq!(repaired_same_size_graph.program_syntax_reused, 1);
    assert_eq!(
        fs::read(repaired_same_size_graph.output_dir.join("graph.json"))?,
        graph_bytes
    );

    let repaired_graph_program = repaired_same_size_graph.output_dir.join("program.json");
    fs::write(
        &repaired_graph_program,
        serde_json::to_vec_pretty(&document)?,
    )?;
    let repaired = build_local_graph(&options)?;
    assert_eq!(repaired.program_syntax_reused, 1);
    assert_eq!(
        fs::read(repaired.output_dir.join("program.json"))?,
        cold_bytes
    );

    fs::write(
        &source,
        "pub fn helper(value: i32) -> i32 { value + 2 }\npub fn run() { let answer = helper(40); println!(\"{answer}\"); }\n",
    )?;
    let changed = build_local_graph(&options)?;
    assert_eq!(changed.program_syntax_analyzed, 1);
    assert_eq!(changed.program_syntax_reused, 0);
    assert_ne!(
        fs::read(changed.output_dir.join("program.json"))?,
        cold_bytes
    );
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
fn disabling_program_analysis_removes_the_previous_snapshot_program() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn visible() {}\n")?;
    let enabled = program_options(directory.path());
    let first = build_local_graph(&enabled)?;
    assert!(first.output_dir.join("program.json").is_file());

    let mut disabled = enabled;
    disabled.program_analysis = false;
    disabled.force = true;
    let second = build_local_graph(&disabled)?;
    assert!(!second.output_dir.join("program.json").exists());
    assert_eq!(second.program_modules, 0);
    let resolved = compass_files::BuildGuard::resolve_artifact(
        &directory.path().join("compass-out"),
        "program.json",
    )?;
    assert!(
        !resolved.exists(),
        "stale Program must not remain addressable"
    );
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
    let pointer = directory.path().join("compass-out/current-snapshot");
    let active_before = fs::read_to_string(&pointer)?;

    options
        .program_artifacts
        .push(directory.path().join("missing.scip"));
    assert!(matches!(
        build_local_graph(&options),
        Err(CoreError::InvalidProgramInput(_))
    ));
    assert_eq!(fs::read_to_string(pointer)?, active_before);
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
    assert_eq!(fs::read(warm.output_dir.join("program.json"))?, first);

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
    let second = fs::read(artifact_changed.output_dir.join("program.json"))?;
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
    let stale_bytes = fs::read(stale.output_dir.join("program.json"))?;
    assert!(!String::from_utf8_lossy(&stale_bytes).contains("fixture 2.0"));

    fs::remove_file(source)?;
    let deleted = build_local_graph(&options)?;
    assert_eq!(deleted.program_modules, 1);
    assert!(
        !String::from_utf8_lossy(&fs::read(deleted.output_dir.join("program.json"))?)
            .contains("src/app.ts")
    );
    Ok(())
}

#[test]
fn fresh_scip_symbols_resolve_java_overloads_in_graph_json() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_text = "class Demo {\n  void pick(int x) {}\n  void pick(String x) {}\n  void use() { pick(\"x\"); }\n}\n";
    fs::create_dir(directory.path().join("src"))?;
    fs::write(directory.path().join("src/Demo.java"), source_text)?;
    write_java_overload_scip(
        &directory.path().join("index.scip"),
        "src/Demo.java",
        source_text,
    )?;

    let result = build_local_graph(&program_options(directory.path()))?;
    let graph =
        compass_model::code_graph::GraphDocument::load(&result.output_dir.join("graph.json"))?;
    let compiler_call = graph
        .links
        .iter()
        .find(|edge| {
            edge.kind == compass_model::code_graph::EdgeKind::Calls
                && edge
                    .evidence
                    .iter()
                    .any(|evidence| evidence.rule.as_deref() == Some("compiler-exact-anchor"))
        })
        .ok_or("missing compiler-resolved Java call")?;
    assert!(compiler_call.evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Artifact
            && evidence.confidence == compass_model::provenance::EvidenceConfidence::Exact
            && evidence.extractor == "compass.resolve.java.program"
    }));
    let target = graph
        .nodes
        .iter()
        .find(|node| node.id == compiler_call.target)
        .ok_or("missing compiler-resolved target")?;
    assert_eq!(
        target.source.as_ref().map(|source| source.start_line),
        Some(3)
    );
    assert_eq!(
        compiler_call
            .relationship_site
            .as_ref()
            .map(|source| (source.start_line, source.start_column)),
        Some((4, 15))
    );
    Ok(())
}

#[test]
fn managed_scip_python_enriches_exact_calls_and_native_output_is_unchanged_when_unavailable()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_text = "def target(): pass\ndef run(): target()\n";
    fs::create_dir(directory.path().join("src"))?;
    fs::write(directory.path().join("src/app.py"), source_text)?;
    let artifact = directory.path().join("index.scip");
    let first_profile = write_managed_python_scip(&artifact, source_text, 'f')?;

    let enabled = build_local_graph(&program_options(directory.path()))?;
    assert_eq!(enabled.program_artifacts_loaded, 1);
    assert_eq!(enabled.program_artifact_documents_analyzed, 1);
    let graph =
        compass_model::code_graph::GraphDocument::load(&enabled.output_dir.join("graph.json"))?;
    let call = graph
        .links
        .iter()
        .find(|edge| {
            edge.kind == compass_model::code_graph::EdgeKind::Calls
                && edge.evidence.iter().any(|evidence| {
                    evidence.extractor == "compass.resolve.python.scip-python"
                        && evidence.rule.as_deref() == Some("scip-python-exact-anchor")
                })
        })
        .ok_or("missing scip-python exact call")?;
    assert!(call.evidence.iter().any(|evidence| {
        evidence.origin == compass_model::provenance::EvidenceOrigin::Artifact
            && evidence.confidence == compass_model::provenance::EvidenceConfidence::Exact
    }));
    let first_program = fs::read(enabled.output_dir.join("program.json"))?;
    assert!(String::from_utf8_lossy(&first_program).contains(&first_profile));
    assert!(!String::from_utf8_lossy(&first_program).contains("manylinux_2_28_x86_64"));

    let second_profile = write_managed_python_scip(&artifact, source_text, '9')?;
    let profile_changed = build_local_graph(&program_options(directory.path()))?;
    assert_eq!(profile_changed.program_artifact_documents_analyzed, 1);
    let second_program = fs::read(profile_changed.output_dir.join("program.json"))?;
    assert_ne!(second_program, first_program);
    assert!(String::from_utf8_lossy(&second_program).contains(&second_profile));

    let mut disabled = program_options(directory.path());
    disabled.program_analysis = false;
    disabled.force = true;
    let disabled_result = build_local_graph(&disabled)?;
    let native_graph = compass_model::code_graph::GraphDocument::load(
        &disabled_result.output_dir.join("graph.json"),
    )?;
    fs::remove_file(&artifact)?;
    let mut unavailable = program_options(directory.path());
    unavailable.force = true;
    let unavailable_result = build_local_graph(&unavailable)?;
    assert_eq!(unavailable_result.program_artifacts_loaded, 0);
    let unavailable_graph = compass_model::code_graph::GraphDocument::load(
        &unavailable_result.output_dir.join("graph.json"),
    )?;
    assert_eq!(unavailable_graph.nodes, native_graph.nodes);
    assert_eq!(unavailable_graph.links, native_graph.links);
    Ok(())
}

#[test]
fn managed_scip_python_rejects_stale_and_conflicting_companions_without_publication()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_text = "def target(): pass\ndef run(): target()\n";
    fs::create_dir(directory.path().join("src"))?;
    fs::write(directory.path().join("src/app.py"), source_text)?;
    let artifact = directory.path().join("index.scip");
    write_managed_python_scip(&artifact, source_text, 'f')?;
    let options = program_options(directory.path());
    let first = build_local_graph(&options)?;
    let pointer = directory.path().join("compass-out/current-snapshot");
    let active_before = fs::read_to_string(&pointer)?;
    let graph_before = fs::read(first.output_dir.join("graph.json"))?;

    fs::write(
        directory.path().join("src/app.py"),
        "def target(): pass\ndef run(): target(); target()\n",
    )?;
    assert!(matches!(
        build_local_graph(&options),
        Err(CoreError::ProgramProvider(_))
    ));
    assert_eq!(fs::read_to_string(&pointer)?, active_before);
    assert_eq!(fs::read(first.output_dir.join("graph.json"))?, graph_before);

    fs::write(directory.path().join("src/app.py"), source_text)?;
    let duplicate = directory.path().join("duplicate.scip");
    fs::copy(&artifact, &duplicate)?;
    let duplicate_companion = directory
        .path()
        .join("duplicate.scip.compass-manifest.json");
    let mut duplicate_manifest: serde_json::Value = serde_json::from_slice(&fs::read(
        directory.path().join("index.scip.compass-manifest.json"),
    )?)?;
    duplicate_manifest["managed_analyzer"]["environment"]["stubs_digest"] =
        serde_json::Value::String("9".repeat(64));
    fs::write(
        duplicate_companion,
        serde_json::to_vec(&duplicate_manifest)?,
    )?;
    let mut conflicting = options;
    conflicting.program_artifacts.push(duplicate);
    assert!(matches!(
        build_local_graph(&conflicting),
        Err(CoreError::InvalidProgramInput(message))
            if message.contains("conflicting companion manifests")
    ));
    assert_eq!(fs::read_to_string(pointer)?, active_before);
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
    let pointer = directory.path().join("compass-out/current-snapshot");
    let active_before = fs::read_to_string(&pointer)?;

    fs::write(directory.path().join("index.scip"), [0x12, 0x05, 0x01])?;
    assert!(build_local_graph(&options).is_err());
    assert_eq!(fs::read(&program_path)?, before);
    assert_eq!(fs::read_to_string(&pointer)?, active_before);
    assert!(!first.output_dir.join("build-incomplete").exists());

    fs::remove_file(directory.path().join("index.scip"))?;
    fs::remove_file(&program_path)?;
    fs::create_dir(&program_path)?;
    assert!(build_local_graph(&options).is_err());
    assert!(program_path.is_dir());
    assert_eq!(fs::read_to_string(pointer)?, active_before);
    assert!(!first.output_dir.join("build-incomplete").exists());
    Ok(())
}
