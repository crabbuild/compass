//! Developer-only compiler-checker target adjudication for the
//! TypeScript/JavaScript universal candidate.
//!
//! The test is intentionally ignored. It compares exact local declaration
//! anchors from an independent TypeScript 5.9.3 checker oracle with candidate
//! target constraints. Cross-file, external, unresolved, and ambiguous
//! outcomes are reported separately; source occurrence recall is measured by
//! typescript_corpus_differential.rs.

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_languages::{CandidateRelation, Engine, SemanticEvidenceBatch};
use serde::Deserialize;

const ORACLE_SCHEMA: &str = "compass.typescript-target-oracle/1";
const ORACLE_PROVIDER: &str = "typescript_checker_api_5_9_3";
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
    resolution_kind: String,
    target_file: Option<String>,
    target_start_byte: Option<u64>,
    target_end_byte: Option<u64>,
}

type SourceKey = (String, String, String, u64, u64);

#[derive(Debug, Clone)]
enum CandidateTarget {
    Local {
        source_file: String,
        start: u64,
        end: u64,
    },
    External {
        qualified_name: String,
    },
    Unresolved,
}

#[test]
#[ignore = "developer-only real TypeScript/JavaScript target adjudication"]
fn checker_oracle_adjudicates_local_candidate_targets() {
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

    let supported = oracle
        .constructs
        .iter()
        .filter(|construct| is_supported_construct(construct))
        .collect::<Vec<_>>();
    let expected = supported
        .iter()
        .filter(|construct| {
            construct.resolution_kind == "source"
                && construct.target_file.as_deref() == Some(construct.source_file.as_str())
                && construct.target_start_byte.is_some()
                && construct.target_end_byte.is_some()
        })
        .map(|construct| source_key(construct))
        .collect::<BTreeSet<_>>();
    assert!(
        !expected.is_empty(),
        "checker oracle emitted no local targets"
    );

    let files = oracle
        .constructs
        .iter()
        .map(|construct| construct.source_file.clone())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeMap::<SourceKey, Vec<CandidateTarget>>::new();
    let mut extraction_errors = Vec::new();
    for source_file in files {
        let path = safe_source_path(&root, &source_file);
        let source = match fs::read(&path) {
            Ok(source) => source,
            Err(error) => {
                extraction_errors.push(format!("{source_file}: read failed: {error}"));
                continue;
            }
        };
        // Keep a parser isolated per large qualification input. The vendored
        // TypeScript grammar can retain a deep parser stack across unrelated
        // files; a fresh engine preserves bounded default-stack runs without
        // changing production parser/cache behavior.
        let mut engine = Engine::default();
        match engine.extract_source_universal_candidate_evidence(&path, &source_file, &source) {
            Ok(batch) => merge_candidate_targets(&mut observed, &batch),
            Err(error) => extraction_errors.push(format!("{source_file}: {error}")),
        }
    }
    assert!(
        extraction_errors.is_empty(),
        "candidate extraction failed: {}",
        extraction_errors
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );

    let mut correct = 0usize;
    let mut missing = 0usize;
    let mut wrong = 0usize;
    let mut positive = 0usize;
    let mut positive_correct = 0usize;
    let mut false_positive = 0usize;
    let mut external_positive = 0usize;
    let mut by_stratum = BTreeMap::<(String, String), [usize; 4]>::new();
    let expected_by_key = supported
        .iter()
        .map(|construct| (source_key(construct), *construct))
        .collect::<HashMap<_, _>>();
    let mut wrong_examples = Vec::new();
    let mut false_positive_examples = Vec::new();
    let mut missing_examples = BTreeMap::<(String, String), Vec<String>>::new();
    for key in &expected {
        let Some(oracle_construct) = expected_by_key.get(key).copied() else {
            continue;
        };
        let expected_target = (
            oracle_construct.target_start_byte.expect("local start"),
            oracle_construct.target_end_byte.expect("local end"),
        );
        let bucket = by_stratum
            .entry((key.1.clone(), key.2.clone()))
            .or_default();
        bucket[0] = bucket[0].saturating_add(1);
        match observed.get(key) {
            Some(targets)
                if targets.iter().any(|target| {
                    matches!(
                        target,
                        CandidateTarget::Local { source_file, start, end }
                            if source_file == &key.0 && (*start, *end) == expected_target
                    )
                }) =>
            {
                correct = correct.saturating_add(1);
                bucket[1] = bucket[1].saturating_add(1);
            }
            Some(targets)
                if targets
                    .iter()
                    .any(|target| matches!(target, CandidateTarget::Local { .. })) =>
            {
                wrong = wrong.saturating_add(1);
                bucket[3] = bucket[3].saturating_add(1);
                if wrong_examples.len() < 20 {
                    wrong_examples.push(format!(
                        "{}:{}:{} {}-{} {} expected={expected_target:?} observed={targets:?}",
                        key.0, key.1, key.2, key.3, key.4, oracle_construct.target_spelling,
                    ));
                }
            }
            None => {
                missing = missing.saturating_add(1);
                bucket[2] = bucket[2].saturating_add(1);
                let examples = missing_examples
                    .entry((key.1.clone(), key.2.clone()))
                    .or_default();
                if examples.len() < 5 {
                    examples.push(format!(
                        "{}:{}:{} {}-{} {} expected={expected_target:?} observed=none",
                        key.0, key.1, key.2, key.3, key.4, oracle_construct.target_spelling,
                    ));
                }
            }
            Some(_) => {
                missing = missing.saturating_add(1);
                bucket[2] = bucket[2].saturating_add(1);
                let examples = missing_examples
                    .entry((key.1.clone(), key.2.clone()))
                    .or_default();
                if examples.len() < 5 {
                    examples.push(format!(
                        "{}:{}:{} {}-{} {} expected={expected_target:?} observed=unresolved-or-external",
                        key.0,
                        key.1,
                        key.2,
                        key.3,
                        key.4,
                        oracle_construct.target_spelling,
                    ));
                }
            }
        }
    }
    for (key, targets) in &observed {
        let Some(oracle_construct) = expected_by_key.get(key).copied() else {
            continue;
        };
        for target in targets {
            if let CandidateTarget::Local {
                source_file,
                start,
                end,
            } = target
            {
                positive = positive.saturating_add(1);
                if oracle_construct.resolution_kind == "source"
                    && oracle_construct.target_file.as_deref()
                        == Some(oracle_construct.source_file.as_str())
                    && source_file == &key.0
                    && Some((*start, *end))
                        == oracle_construct
                            .target_start_byte
                            .zip(oracle_construct.target_end_byte)
                {
                    positive_correct = positive_correct.saturating_add(1);
                } else {
                    false_positive = false_positive.saturating_add(1);
                    if false_positive_examples.len() < 20 {
                        false_positive_examples.push(format!(
                            "{}:{}:{} {}-{} local={source_file}:{start}-{end} oracle_kind={} oracle_target={:?}",
                            key.0,
                            key.1,
                            key.2,
                            key.3,
                            key.4,
                            oracle_construct.resolution_kind,
                            oracle_construct
                                .target_file
                                .as_ref()
                                .zip(oracle_construct.target_start_byte)
                                .zip(oracle_construct.target_end_byte),
                        ));
                    }
                }
            } else if let CandidateTarget::External { qualified_name } = target
                && !qualified_name.is_empty()
            {
                external_positive = external_positive.saturating_add(1);
            }
        }
    }
    eprintln!(
        "TypeScript/JavaScript checker target adjudication: correct={correct} expected_local={} missing={missing} wrong={wrong} positive={positive} positive_correct={positive_correct} false_positive={false_positive} external_positive={external_positive} supported={} strata={by_stratum:?} wrong_examples={wrong_examples:?} false_positive_examples={false_positive_examples:?} missing_examples={missing_examples:?}",
        expected.len(),
        supported.len(),
    );
    // This is an adjudication report, not the release gate. Keep the test
    // useful for regressions while Plan 013's accepted-sample labels and
    // Wilson interval policy are still being assembled.
    assert!(correct.saturating_add(missing).saturating_add(wrong) == expected.len());
}

fn run_oracle(root: &Path) -> OraclePayload {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_owned();
    let script = repository.join("benchmarks/performance/oracles/typescript-target-oracle.mjs");
    let output = Command::new("node")
        .current_dir(&repository)
        .arg(&script)
        .arg("--root")
        .arg(root)
        .output()
        .expect("target oracle requires node");
    assert!(
        output.stdout.len() <= MAX_ORACLE_OUTPUT_BYTES,
        "target oracle output exceeds bounded limit"
    );
    assert!(
        output.status.success(),
        "target oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("target oracle JSON")
}

fn safe_source_path(root: &Path, source_file: &str) -> PathBuf {
    let relative = Path::new(source_file);
    assert!(!relative.is_absolute() && !relative.components().any(|part| part.as_os_str() == ".."));
    let path = root.join(relative);
    let canonical = path.canonicalize().expect("oracle source path");
    assert!(
        canonical.starts_with(root),
        "oracle source escaped root: {source_file}"
    );
    canonical
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

fn is_supported_construct(construct: &OracleConstruct) -> bool {
    matches!(
        (construct.relation.as_str(), construct.capability.as_str()),
        ("imports", "imports")
            | ("reexports", "reexports")
            | ("calls", "calls")
            | ("instantiates", "construction")
            | ("accesses", "members")
            | ("extends", "base_types")
            | ("implements", "base_types")
            | ("references", "jsx")
            | ("references", "type_references")
    )
}

fn merge_candidate_targets(
    observed: &mut BTreeMap<SourceKey, Vec<CandidateTarget>>,
    batch: &SemanticEvidenceBatch,
) {
    let declarations = batch
        .declarations
        .iter()
        .map(|declaration| (declaration.id.as_str(), declaration))
        .collect::<HashMap<_, _>>();
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
        let Some((relation, capability)) =
            relation_name(candidate.relation, occurrence.context.as_deref())
        else {
            continue;
        };
        let key = (
            occurrence.range.source_file.clone(),
            relation.to_owned(),
            capability.to_owned(),
            occurrence.range.start_byte,
            occurrence.range.end_byte,
        );
        let target = if let Some(id) = candidate.constraints.exact_target_declaration_id.as_deref()
        {
            declarations
                .get(id)
                .map(|declaration| CandidateTarget::Local {
                    source_file: declaration.range.source_file.clone(),
                    start: declaration.range.start_byte,
                    end: declaration.range.end_byte,
                })
                .unwrap_or(CandidateTarget::Unresolved)
        } else if candidate.constraints.allow_external {
            candidate
                .constraints
                .qualified_name
                .as_ref()
                .map(|qualified_name| CandidateTarget::External {
                    qualified_name: qualified_name.clone(),
                })
                .unwrap_or(CandidateTarget::Unresolved)
        } else {
            CandidateTarget::Unresolved
        };
        observed.entry(key).or_default().push(target);
    }
}

fn relation_name(
    relation: CandidateRelation,
    context: Option<&str>,
) -> Option<(&'static str, &'static str)> {
    match relation {
        CandidateRelation::Imports => Some(("imports", "imports")),
        CandidateRelation::Reexports => Some(("reexports", "reexports")),
        CandidateRelation::Calls => Some(("calls", "calls")),
        CandidateRelation::Constructs => Some(("instantiates", "construction")),
        CandidateRelation::AccessesMember => Some(("accesses", "members")),
        CandidateRelation::Extends => Some(("extends", "base_types")),
        CandidateRelation::Implements => Some(("implements", "base_types")),
        CandidateRelation::References if context == Some("jsx") => Some(("references", "jsx")),
        CandidateRelation::References => Some(("references", "type_references")),
        _ => None,
    }
}
