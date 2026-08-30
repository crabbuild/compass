use std::collections::BTreeMap;
use std::error::Error;
use std::io::Cursor;

use compass_ir::{Capability, CoverageState, hex_sha256};
use compass_program::{
    ArtifactInput, ArtifactLimits, ArtifactProvider, OfficialScipProvider, ProviderError,
    parse_artifact_manifest, source_inventory_digest,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use scip::types::{
    Document, Index, Metadata, Occurrence, PositionEncoding, SymbolInformation, SymbolRole,
    TextEncoding, ToolInfo,
};

fn fixture(path: &str, source: &str, range: Vec<i32>) -> Result<Vec<u8>, protobuf::Error> {
    fixture_with_tool(path, source, range, "fixture-indexer", "1.0")
}

fn fixture_with_tool(
    path: &str,
    source: &str,
    range: Vec<i32>,
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
    let mut definition = Occurrence::new();
    definition.range = range;
    definition.symbol = "rust cargo fixture 0.1 work().".to_owned();
    definition.symbol_roles = SymbolRole::Definition as i32;
    let mut reference = Occurrence::new();
    reference.range = vec![1, 11, 15];
    reference.symbol = "rust cargo fixture 0.1 work().".to_owned();
    reference.symbol_roles = SymbolRole::ReadAccess as i32;
    let mut symbol = SymbolInformation::new();
    symbol.symbol = definition.symbol.clone();
    let mut document = Document::new();
    document.language = "rust".to_owned();
    document.relative_path = path.to_owned();
    document.occurrences = vec![definition, reference];
    document.symbols = vec![symbol];
    document.text = source.to_owned();
    document.position_encoding =
        EnumOrUnknown::new(PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = vec![document];
    index.write_to_bytes()
}

fn managed_fixture(path: &str, source: &str, range: Vec<i32>) -> Result<Vec<u8>, protobuf::Error> {
    managed_fixture_with_identity(path, source, range, "scip-python", "0.6.2", "python")
}

fn managed_fixture_with_identity(
    path: &str,
    source: &str,
    range: Vec<i32>,
    tool_name: &str,
    tool_version: &str,
    language: &str,
) -> Result<Vec<u8>, protobuf::Error> {
    let bytes = fixture_with_tool(path, source, range, tool_name, tool_version)?;
    let mut index = Index::parse_from_bytes(&bytes)?;
    if let Some(document) = index.documents.first_mut() {
        document.language = language.to_owned();
    }
    index.write_to_bytes()
}

fn managed_profile(project_digest: &str, state: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": "compass.managed-analyzer-profile/1",
        "language": "python",
        "provider": "scip-python",
        "provider_version": "0.6.2",
        "protocol_version": "scip/1",
        "state": state,
        "source_inventory_digest": project_digest,
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
            "stubs_digest": "f".repeat(64),
            "namespace_policy": "pep420",
            "use_library_code_for_types": false
        },
        "permissions": {
            "allow_dependency_network": false,
            "allow_package_install": false,
            "allow_project_execution": false
        }
    })
}

fn managed_manifest(
    index_digest: &str,
    source_digest: &str,
    profile: serde_json::Value,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "compass.scip-manifest/1",
        "index_sha256": index_digest,
        "documents": {"src/app.py": source_digest},
        "managed_analyzer": profile
    }))
}

#[test]
fn official_scip_normalizes_evidence_without_absolute_metadata() -> Result<(), Box<dyn Error>> {
    let source = "fn work() {}\nfn run() { work(); }\n";
    let bytes = fixture("src/lib.rs", source, vec![0, 3, 7])?;
    let digest = hex_sha256(&bytes);
    let source_digest = hex_sha256(source.as_bytes());
    let manifest_bytes = format!(
        r#"{{"schema":"compass.scip-manifest/1","index_sha256":"{digest}","documents":{{"src/lib.rs":"{source_digest}"}}}}"#
    );
    let manifest = parse_artifact_manifest(manifest_bytes.as_bytes(), &digest)?;
    let source_digests = BTreeMap::from([("src/lib.rs".to_owned(), source_digest)]);
    let project_digest = source_inventory_digest(&source_digests)?;
    let source_texts = BTreeMap::from([("src/lib.rs".to_owned(), source.as_bytes().to_vec())]);
    let mut reader = Cursor::new(bytes.clone());
    let batch = OfficialScipProvider.analyze_artifact(
        ArtifactInput {
            logical_name: "index.scip",
            input_digest: &digest,
            byte_len: bytes.len() as u64,
            manifest: Some(&manifest),
            project_digest: &project_digest,
            source_digests: &source_digests,
            source_texts: &source_texts,
            limits: ArtifactLimits::default(),
        },
        &mut reader,
    )?;
    assert!(batch.facts.iter().any(|fact| {
        fact.capability == Capability::Definitions && fact.anchor.source_file == "src/lib.rs"
    }));
    assert!(!serde_json::to_string(&batch)?.contains("/absolute/"));
    assert!(matches!(
        batch.coverage["src/lib.rs"].get(&Capability::References),
        Some(CoverageState::Partial { reasons })
            if !reasons.iter().any(|reason| reason == "artifact_revision_unverified")
    ));
    Ok(())
}

#[test]
fn raw_stale_and_unsafe_scip_are_explicit() -> Result<(), Box<dyn Error>> {
    let source = "fn work() {}\nfn run() { work(); }\n";
    let bytes = fixture("src/lib.rs", source, vec![0, 3, 7])?;
    let digest = hex_sha256(&bytes);
    let source_digests = BTreeMap::from([("src/lib.rs".to_owned(), hex_sha256(source.as_bytes()))]);
    let project_digest = source_inventory_digest(&source_digests)?;
    let source_texts = BTreeMap::from([("src/lib.rs".to_owned(), source.as_bytes().to_vec())]);
    let mut reader = Cursor::new(bytes.clone());
    let raw = OfficialScipProvider.analyze_artifact(
        ArtifactInput {
            logical_name: "index.scip",
            input_digest: &digest,
            byte_len: bytes.len() as u64,
            manifest: None,
            project_digest: &project_digest,
            source_digests: &source_digests,
            source_texts: &source_texts,
            limits: ArtifactLimits::default(),
        },
        &mut reader,
    )?;
    assert!(matches!(
        raw.coverage["src/lib.rs"].get(&Capability::References),
        Some(CoverageState::Partial { reasons })
            if reasons.iter().any(|reason| reason == "artifact_revision_unverified")
    ));

    let unsafe_bytes = fixture("../escape.rs", source, vec![0, 3, 7])?;
    let unsafe_digest = hex_sha256(&unsafe_bytes);
    let mut unsafe_reader = Cursor::new(unsafe_bytes.clone());
    assert!(
        OfficialScipProvider
            .analyze_artifact(
                ArtifactInput {
                    logical_name: "index.scip",
                    input_digest: &unsafe_digest,
                    byte_len: unsafe_bytes.len() as u64,
                    manifest: None,
                    project_digest: &project_digest,
                    source_digests: &source_digests,
                    source_texts: &source_texts,
                    limits: ArtifactLimits::default(),
                },
                &mut unsafe_reader,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn malformed_and_resource_limited_indexes_fail_closed() {
    let bytes = vec![0x12, 0x05, 0x01];
    let digest = hex_sha256(&bytes);
    let mut reader = Cursor::new(bytes.clone());
    let empty_digests = BTreeMap::new();
    let project_digest = source_inventory_digest(&empty_digests).unwrap_or_default();
    let empty_texts = BTreeMap::new();
    let limits = ArtifactLimits {
        max_artifact_bytes: 2,
        ..ArtifactLimits::default()
    };
    assert!(
        OfficialScipProvider
            .analyze_artifact(
                ArtifactInput {
                    logical_name: "index.scip",
                    input_digest: &digest,
                    byte_len: bytes.len() as u64,
                    manifest: None,
                    project_digest: &project_digest,
                    source_digests: &empty_digests,
                    source_texts: &empty_texts,
                    limits,
                },
                &mut reader,
            )
            .is_err()
    );
}

#[test]
fn managed_scip_python_profile_is_frozen_offline_and_part_of_identity() -> Result<(), Box<dyn Error>>
{
    let source = "def work(): pass\ndef run(): work()\n";
    let bytes = managed_fixture("src/app.py", source, vec![0, 4, 8])?;
    let index_digest = hex_sha256(&bytes);
    let source_digests = BTreeMap::from([("src/app.py".to_owned(), hex_sha256(source.as_bytes()))]);
    let project_digest = source_inventory_digest(&source_digests)?;
    let manifest_bytes = managed_manifest(
        &index_digest,
        &source_digests["src/app.py"],
        managed_profile(&project_digest, "complete"),
    )?;
    let manifest = parse_artifact_manifest(&manifest_bytes, &index_digest)?;
    let source_texts = BTreeMap::from([("src/app.py".to_owned(), source.as_bytes().to_vec())]);
    let input = ArtifactInput {
        logical_name: "index.scip",
        input_digest: &index_digest,
        byte_len: bytes.len() as u64,
        manifest: Some(&manifest),
        project_digest: &project_digest,
        source_digests: &source_digests,
        source_texts: &source_texts,
        limits: ArtifactLimits::default(),
    };
    let descriptor = OfficialScipProvider.descriptor(&input);
    assert!(descriptor.id.starts_with("scip-python:"));
    assert!(descriptor.version.starts_with("scip-python/1/0.6.2"));
    assert_eq!(descriptor.scope, project_digest);

    let mut reader = Cursor::new(bytes);
    let batch = OfficialScipProvider.analyze_artifact(input, &mut reader)?;
    assert_eq!(batch.descriptor, descriptor);
    let serialized = serde_json::to_string(&batch)?;
    assert!(!serialized.contains("manylinux_2_28_x86_64"));
    assert!(!serialized.contains("stubs"));

    let limited = ArtifactInput {
        limits: ArtifactLimits {
            max_records: input.limits.max_records - 1,
            ..input.limits
        },
        ..input
    };
    assert_ne!(
        OfficialScipProvider
            .descriptor(&limited)
            .configuration_digest,
        descriptor.configuration_digest
    );
    Ok(())
}

#[test]
fn managed_profile_timeout_cancel_permission_staleness_and_unknown_major_fail_closed()
-> Result<(), Box<dyn Error>> {
    let source = "def work(): pass\ndef run(): work()\n";
    let bytes = managed_fixture("src/app.py", source, vec![0, 4, 8])?;
    let index_digest = hex_sha256(&bytes);
    let source_digests = BTreeMap::from([("src/app.py".to_owned(), hex_sha256(source.as_bytes()))]);
    let project_digest = source_inventory_digest(&source_digests)?;
    for (state, expected) in [
        ("timed_out", "timed out"),
        ("cancelled", "cancelled"),
        ("permission_denied", "permission denied"),
        ("partial", "incomplete"),
        ("failed", "incomplete"),
    ] {
        let bytes = managed_manifest(
            &index_digest,
            &source_digests["src/app.py"],
            managed_profile(&project_digest, state),
        )?;
        let error = parse_artifact_manifest(&bytes, &index_digest)
            .err()
            .ok_or("managed profile state unexpectedly accepted")?;
        assert!(error.to_string().contains(expected), "{state}: {error}");
    }

    let mut permission = managed_profile(&project_digest, "complete");
    permission["permissions"]["allow_dependency_network"] = serde_json::Value::Bool(true);
    let manifest_bytes =
        managed_manifest(&index_digest, &source_digests["src/app.py"], permission)?;
    assert!(matches!(
        parse_artifact_manifest(&manifest_bytes, &index_digest),
        Err(ProviderError::AnalyzerPermissionDenied(_))
    ));

    let mut unknown = managed_profile(&project_digest, "complete");
    unknown["schema"] = serde_json::Value::String("compass.managed-analyzer-profile/2".to_owned());
    let manifest_bytes = managed_manifest(&index_digest, &source_digests["src/app.py"], unknown)?;
    assert!(matches!(
        parse_artifact_manifest(&manifest_bytes, &index_digest),
        Err(ProviderError::UnsupportedArtifact(_))
    ));

    let stale_digest = "a".repeat(64);
    let manifest_bytes = managed_manifest(
        &index_digest,
        &source_digests["src/app.py"],
        managed_profile(&stale_digest, "complete"),
    )?;
    let manifest = parse_artifact_manifest(&manifest_bytes, &index_digest)?;
    let source_texts = BTreeMap::from([("src/app.py".to_owned(), source.as_bytes().to_vec())]);
    let mut reader = Cursor::new(bytes);
    let result = OfficialScipProvider.analyze_artifact(
        ArtifactInput {
            logical_name: "index.scip",
            input_digest: &index_digest,
            byte_len: reader.get_ref().len() as u64,
            manifest: Some(&manifest),
            project_digest: &project_digest,
            source_digests: &source_digests,
            source_texts: &source_texts,
            limits: ArtifactLimits::default(),
        },
        &mut reader,
    );
    assert!(matches!(
        result,
        Err(ProviderError::StaleAnalyzerProfile(_))
    ));

    for (tool_name, language, expected) in [
        ("other-indexer", "python", "producer"),
        ("scip-python", "rust", "declared Python source"),
    ] {
        let invalid = managed_fixture_with_identity(
            "src/app.py",
            source,
            vec![0, 4, 8],
            tool_name,
            "0.6.2",
            language,
        )?;
        let invalid_digest = hex_sha256(&invalid);
        let manifest_bytes = managed_manifest(
            &invalid_digest,
            &source_digests["src/app.py"],
            managed_profile(&project_digest, "complete"),
        )?;
        let manifest = parse_artifact_manifest(&manifest_bytes, &invalid_digest)?;
        let mut reader = Cursor::new(invalid);
        let error = OfficialScipProvider
            .analyze_artifact(
                ArtifactInput {
                    logical_name: "index.scip",
                    input_digest: &invalid_digest,
                    byte_len: reader.get_ref().len() as u64,
                    manifest: Some(&manifest),
                    project_digest: &project_digest,
                    source_digests: &source_digests,
                    source_texts: &source_texts,
                    limits: ArtifactLimits::default(),
                },
                &mut reader,
            )
            .err()
            .ok_or("invalid managed artifact unexpectedly accepted")?;
        assert!(error.to_string().contains(expected), "{error}");
    }
    Ok(())
}
