//! Developer-only real-corpus differential qualification for the
//! TypeScript/JavaScript universal candidate.
//!
//! This test intentionally remains ignored.  It runs the independent pinned
//! TypeScript 5.9.3 compiler oracle over a read-only corpus supplied through
//! `COMPASS_TS_QUALIFICATION_ROOT`, then compares source-byte coverage against
//! the tree-sitter candidate.  It measures coverage by relation/capability;
//! target identity and precision still require the adjudication workflow in
//! Plan 013.

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_languages::{CandidateRelation, Engine, SemanticEvidenceBatch};
use serde::Deserialize;

const ORACLE_SCHEMA: &str = "compass.typescript-source-oracle/1";
const ORACLE_PROVIDER: &str = "typescript_compiler_api_5_9_3";
const MAX_ORACLE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OraclePayload {
    schema: String,
    provider: String,
    scanned_files: usize,
    parsed_files: usize,
    rejected_files: Vec<serde_json::Value>,
    constructs: Vec<OracleConstruct>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleConstruct {
    source_file: String,
    relation: String,
    capability: String,
    target_spelling: String,
    start_byte: u64,
    end_byte: u64,
}

type SourceKey = (String, String, String, u64, u64);

#[test]
#[ignore = "developer-only real TypeScript/JavaScript corpus differential"]
fn pinned_compiler_corpus_has_candidate_source_coverage() {
    let root = env::var_os("COMPASS_TS_QUALIFICATION_ROOT")
        .map(PathBuf::from)
        .expect("set COMPASS_TS_QUALIFICATION_ROOT to a read-only pinned corpus");
    let root = root.canonicalize().expect("qualification corpus root");
    assert!(
        root.is_dir(),
        "qualification corpus is not a directory: {root:?}"
    );

    let oracle = run_oracle(&root);
    assert_eq!(oracle.schema, ORACLE_SCHEMA);
    assert_eq!(oracle.provider, ORACLE_PROVIDER);
    assert_eq!(oracle.scanned_files, oracle.parsed_files);
    assert!(oracle.rejected_files.is_empty(), "oracle rejected files");

    let expected = oracle
        .constructs
        .iter()
        .filter(|construct| supported_construct(construct))
        .map(source_key)
        .collect::<BTreeSet<_>>();
    assert!(
        !expected.is_empty(),
        "oracle emitted no supported constructs"
    );

    let files = oracle
        .constructs
        .iter()
        .map(|construct| construct.source_file.clone())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    let mut extraction_errors = Vec::new();
    let mut engine = Engine::default();
    for source_file in files {
        let path = safe_source_path(&root, &source_file);
        let source = match fs::read(&path) {
            Ok(source) => source,
            Err(error) => {
                extraction_errors.push(format!("{source_file}: read failed: {error}"));
                continue;
            }
        };
        match engine.extract_source_universal_candidate_evidence(&path, &source_file, &source) {
            Ok(batch) => observed.extend(candidate_source_keys(&batch)),
            Err(error) => extraction_errors.push(format!("{source_file}: {error}")),
        }
    }
    assert!(
        extraction_errors.is_empty(),
        "candidate extraction failed for {} files: {}",
        extraction_errors.len(),
        extraction_errors
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );

    let covered = expected.intersection(&observed).count();
    let missing = expected.difference(&observed).cloned().collect::<Vec<_>>();
    let mut expected_by_stratum = BTreeMap::<(String, String), usize>::new();
    let mut missing_by_stratum = BTreeMap::<(String, String), usize>::new();
    for key in &expected {
        *expected_by_stratum
            .entry((key.1.clone(), key.2.clone()))
            .or_default() += 1;
    }
    for key in &missing {
        *missing_by_stratum
            .entry((key.1.clone(), key.2.clone()))
            .or_default() += 1;
    }
    eprintln!(
        "TypeScript/JavaScript candidate corpus coverage: {covered}/{} ({:.2}%), observed={} expected={} strata={expected_by_stratum:?} missing={missing_by_stratum:?}",
        expected.len(),
        (covered as f64) * 100.0 / (expected.len() as f64),
        observed.len(),
        expected.len(),
    );
    let mut examples_by_stratum = BTreeMap::<(String, String), Vec<String>>::new();
    for construct in &oracle.constructs {
        if !supported_construct(construct) || observed.contains(&source_key(construct)) {
            continue;
        }
        let bucket = (construct.relation.clone(), construct.capability.clone());
        let examples = examples_by_stratum.entry(bucket).or_default();
        if examples.len() < 5 {
            examples.push(format!(
                "{}:{}-{} {:?}",
                construct.source_file,
                construct.start_byte,
                construct.end_byte,
                construct.target_spelling,
            ));
        }
    }
    eprintln!("candidate coverage missing examples: {examples_by_stratum:?}");
    // This is a measurement fixture, not the release gate.  Keep the failure
    // useful for regressions without pretending source coverage proves target
    // precision or leadership.
    assert!(
        covered > 0,
        "candidate lost all supported source coverage; missing sample: {:?}",
        missing.iter().take(20).collect::<Vec<_>>()
    );
}

fn run_oracle(root: &Path) -> OraclePayload {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_owned();
    let script = repository.join("benchmarks/performance/oracles/typescript-source-oracle.mjs");
    let output = Command::new("node")
        .current_dir(&repository)
        .arg(&script)
        .arg("--root")
        .arg(root)
        .output()
        .expect("pinned TypeScript oracle requires node");
    assert!(
        output.stdout.len() <= MAX_ORACLE_OUTPUT_BYTES,
        "oracle output exceeds bounded test limit"
    );
    assert!(
        output.status.success(),
        "TypeScript oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid TypeScript oracle JSON")
}

fn safe_source_path(root: &Path, source_file: &str) -> PathBuf {
    let relative = Path::new(source_file);
    assert!(
        !relative.is_absolute(),
        "oracle source escaped root: {source_file:?}"
    );
    assert!(
        !relative
            .components()
            .any(|component| { matches!(component, std::path::Component::ParentDir) }),
        "oracle source escaped root: {source_file:?}"
    );
    let path = root.join(relative);
    let canonical = path.canonicalize().expect("oracle source path");
    assert!(
        canonical.starts_with(root),
        "oracle source escaped root: {source_file:?}"
    );
    canonical
}

fn supported_construct(construct: &OracleConstruct) -> bool {
    let jsx_component = construct
        .target_spelling
        .split('.')
        .next()
        .and_then(|name| name.chars().next())
        .is_some_and(|character| !character.is_ascii_lowercase());
    match (construct.relation.as_str(), construct.capability.as_str()) {
        ("declares", "declarations")
        | ("imports", "imports")
        | ("reexports", "reexports")
        | ("calls", "calls")
        | ("instantiates", "construction")
        | ("accesses", "members")
        | ("extends", "base_types")
        | ("implements", "base_types")
        | ("references", "type_references") => true,
        ("references", "jsx") => jsx_component,
        ("references", "jsx_values") => true,
        _ => false,
    }
}

fn source_key(construct: &OracleConstruct) -> SourceKey {
    (
        construct.source_file.clone(),
        construct.relation.clone(),
        construct.capability.clone(),
        construct.start_byte,
        construct.end_byte,
    )
}

fn candidate_source_keys(batch: &SemanticEvidenceBatch) -> BTreeSet<SourceKey> {
    let mut keys = BTreeSet::new();
    for declaration in &batch.declarations {
        keys.insert((
            declaration.range.source_file.clone(),
            "declares".to_owned(),
            "declarations".to_owned(),
            declaration.range.start_byte,
            declaration.range.end_byte,
        ));
    }
    for candidate in &batch.candidates {
        let Some(occurrence_id) = candidate.occurrence_id.as_deref() else {
            continue;
        };
        let Some(occurrence) = batch
            .occurrences
            .iter()
            .find(|occurrence| occurrence.id == occurrence_id)
        else {
            continue;
        };
        let Some((relation, capability)) = candidate_key_names(candidate, occurrence) else {
            continue;
        };
        keys.insert((
            occurrence.range.source_file.clone(),
            relation.to_owned(),
            capability.to_owned(),
            occurrence.range.start_byte,
            occurrence.range.end_byte,
        ));
    }
    keys
}

fn candidate_key_names(
    candidate: &compass_languages::RelationshipCandidate,
    occurrence: &compass_languages::OccurrenceFact,
) -> Option<(&'static str, &'static str)> {
    match candidate.relation {
        CandidateRelation::Imports => Some(("imports", "imports")),
        CandidateRelation::Reexports => Some(("reexports", "reexports")),
        CandidateRelation::Calls => Some(("calls", "calls")),
        CandidateRelation::Constructs => Some(("instantiates", "construction")),
        CandidateRelation::AccessesMember => Some(("accesses", "members")),
        CandidateRelation::Extends => Some(("extends", "base_types")),
        CandidateRelation::Implements => Some(("implements", "base_types")),
        CandidateRelation::References if occurrence.context.as_deref() == Some("jsx") => {
            Some(("references", "jsx"))
        }
        CandidateRelation::References
            if matches!(
                occurrence.context.as_deref(),
                Some("jsx_value" | "jsx_spread" | "jsx_child")
            ) =>
        {
            Some(("references", "jsx_values"))
        }
        CandidateRelation::References => Some(("references", "type_references")),
        _ => None,
    }
}
