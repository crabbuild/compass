//! Developer-only differential qualification for the TypeScript/JavaScript
//! universal candidate.
//!
//! The test is deliberately ignored in the normal Rust suite: the production
//! boundary must not require Node.js or the TypeScript compiler. Run it with
//! `cargo test -p compass-languages --test typescript_oracle_differential
//! -- --ignored` when the pinned benchmark oracle is available.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_languages::{CandidateRelation, Engine, SemanticEvidenceBatch};
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OraclePayload {
    schema: String,
    provider: String,
    constructs: Vec<OracleConstruct>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleConstruct {
    source_file: String,
    relation: String,
    capability: String,
    start_byte: u64,
    end_byte: u64,
}

#[test]
#[ignore = "developer-only TypeScript compiler differential"]
fn pinned_compiler_constructs_have_candidate_source_coverage() {
    let root = tempdir().expect("temporary differential root");
    let main = root.path().join("src/main.tsx");
    let ui = root.path().join("src/ui.ts");
    fs::create_dir_all(main.parent().expect("main parent")).expect("source directory");
    fs::write(
        &ui,
        "export class Button {}\nexport function parse(value: string) { return value; }\n",
    )
    .expect("ui fixture");
    fs::write(
        &main,
        "import * as UI from './ui';\n\
         interface Shape { value: string }\n\
         class Base { run() {} }\n\
         class Child extends Base {\n\
             render(value: string) {\n\
                 super.run();\n\
                 UI.Button;\n\
                 return <UI.Button />;\n\
             }\n\
         }\n\
         function parse(value: string) { return value; }\n\
         function parse(value: number) { return value; }\n\
         export { Child, parse };\n\
         new Child().render('ok');\n\
         const instance = new Child();\n\
         instance[\"render\"]('ok');\n",
    )
    .expect("main fixture");

    let oracle = run_oracle(root.path());
    assert_eq!(oracle.schema, "compass.typescript-source-oracle/1");
    assert_eq!(oracle.provider, "typescript_compiler_api_5_9_3");

    let mut engine = Engine::default();
    let source = fs::read(&main).expect("main source");
    let batch = engine
        .extract_source_universal_candidate_evidence(&main, "src/main.tsx", &source)
        .expect("candidate evidence");
    let observed = candidate_source_keys(&batch);
    let expected = oracle
        .constructs
        .iter()
        .filter(|construct| construct.source_file == "src/main.tsx")
        .filter(|construct| supported_construct(construct))
        .map(|construct| {
            (
                construct.relation.clone(),
                construct.capability.clone(),
                construct.start_byte,
                construct.end_byte,
            )
        })
        .collect::<BTreeSet<_>>();
    assert!(
        expected.len() >= 20,
        "fixture did not exercise enough constructs"
    );
    let missing = expected.difference(&observed).cloned().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "candidate lost compiler-backed source coverage: {missing:?}"
    );
}

fn run_oracle(root: &Path) -> OraclePayload {
    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest_directory
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let script = repository.join("benchmarks/performance/oracles/typescript-source-oracle.mjs");
    let output = Command::new("node")
        .current_dir(repository)
        .arg(&script)
        .arg("--root")
        .arg(root)
        .output()
        .expect("pinned TypeScript oracle requires node");
    assert!(
        output.status.success(),
        "TypeScript oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid TypeScript oracle JSON")
}

fn supported_construct(construct: &OracleConstruct) -> bool {
    matches!(
        (construct.relation.as_str(), construct.capability.as_str()),
        ("declares", "declarations")
            | ("imports", "imports")
            | ("reexports", "reexports")
            | ("calls", "calls")
            | ("instantiates", "construction")
            | ("accesses", "members")
            | ("extends", "base_types")
            | ("implements", "base_types")
            | ("references", "jsx")
            | ("references", "jsx_values")
            | ("references", "type_references")
    )
}

fn candidate_source_keys(batch: &SemanticEvidenceBatch) -> BTreeSet<(String, String, u64, u64)> {
    let mut keys = BTreeSet::new();
    for declaration in &batch.declarations {
        keys.insert((
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
        let (relation, capability) = match candidate.relation {
            CandidateRelation::Imports => ("imports", "imports"),
            CandidateRelation::Reexports => ("reexports", "reexports"),
            CandidateRelation::Calls => ("calls", "calls"),
            CandidateRelation::Constructs => ("instantiates", "construction"),
            CandidateRelation::AccessesMember => ("accesses", "members"),
            CandidateRelation::Extends => ("extends", "base_types"),
            CandidateRelation::Implements => ("implements", "base_types"),
            CandidateRelation::References if occurrence.context.as_deref() == Some("jsx") => {
                ("references", "jsx")
            }
            CandidateRelation::References
                if matches!(
                    occurrence.context.as_deref(),
                    Some("jsx_value" | "jsx_spread" | "jsx_child")
                ) =>
            {
                ("references", "jsx_values")
            }
            CandidateRelation::References => ("references", "type_references"),
            _ => continue,
        };
        keys.insert((
            relation.to_owned(),
            capability.to_owned(),
            occurrence.range.start_byte,
            occurrence.range.end_byte,
        ));
    }
    keys
}
