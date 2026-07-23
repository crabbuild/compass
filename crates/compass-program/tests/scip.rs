use std::collections::BTreeMap;
use std::error::Error;
use std::io::Cursor;

use compass_ir::{Capability, CoverageState, hex_sha256};
use compass_program::{
    ArtifactInput, ArtifactLimits, ArtifactProvider, OfficialScipProvider,
    parse_artifact_manifest,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use scip::types::{
    Document, Index, Metadata, Occurrence, PositionEncoding, SymbolInformation, SymbolRole,
    TextEncoding, ToolInfo,
};

fn fixture(path: &str, source: &str, range: Vec<i32>) -> Result<Vec<u8>, protobuf::Error> {
    let mut tool = ToolInfo::new();
    tool.name = "fixture-indexer".to_owned();
    tool.version = "1.0".to_owned();
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

#[test]
fn official_scip_normalizes_evidence_without_absolute_metadata()
-> Result<(), Box<dyn Error>> {
    let source = "fn work() {}\nfn run() { work(); }\n";
    let bytes = fixture("src/lib.rs", source, vec![0, 3, 7])?;
    let digest = hex_sha256(&bytes);
    let source_digest = hex_sha256(source.as_bytes());
    let manifest_bytes = format!(
        r#"{{"schema":"compass.scip-manifest/1","index_sha256":"{digest}","documents":{{"src/lib.rs":"{source_digest}"}}}}"#
    );
    let manifest = parse_artifact_manifest(manifest_bytes.as_bytes(), &digest)?;
    let source_digests = BTreeMap::from([("src/lib.rs".to_owned(), source_digest)]);
    let source_texts =
        BTreeMap::from([("src/lib.rs".to_owned(), source.as_bytes().to_vec())]);
    let mut reader = Cursor::new(bytes.clone());
    let batch = OfficialScipProvider.analyze_artifact(
        ArtifactInput {
            logical_name: "index.scip",
            input_digest: &digest,
            byte_len: bytes.len() as u64,
            manifest: Some(&manifest),
            source_digests: &source_digests,
            source_texts: &source_texts,
            limits: ArtifactLimits::default(),
        },
        &mut reader,
    )?;
    assert!(batch.facts.iter().any(|fact| {
        fact.capability == Capability::Definitions
            && fact.anchor.source_file == "src/lib.rs"
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
    let source_digests = BTreeMap::from([(
        "src/lib.rs".to_owned(),
        hex_sha256(source.as_bytes()),
    )]);
    let source_texts =
        BTreeMap::from([("src/lib.rs".to_owned(), source.as_bytes().to_vec())]);
    let mut reader = Cursor::new(bytes.clone());
    let raw = OfficialScipProvider.analyze_artifact(
        ArtifactInput {
            logical_name: "index.scip",
            input_digest: &digest,
            byte_len: bytes.len() as u64,
            manifest: None,
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
                    source_digests: &empty_digests,
                    source_texts: &empty_texts,
                    limits,
                },
                &mut reader,
            )
            .is_err()
    );
}
