use std::collections::BTreeMap;
use std::error::Error;

use compass_analysis::{AnalysisBundle, FunctionSummary, analyze};
use compass_history::{SourceFileDelta, SourceFileStatus, SourceHunk};
use compass_ir::ModuleIr;
use compass_languages::TreeSitterSyntaxProvider;
use compass_model::NodeRecord;
use compass_program::{FileInput, SyntaxProvider, merge_evidence};
use compass_semantic_diff::{
    Compatibility, NoTestEvidence, REPORT_SCHEMA, SemanticDiffError, SemanticDiffInput,
    SnapshotIdentity, SnapshotReader, SnapshotSide, compare,
};

struct Fixtures {
    old: AnalysisBundle,
    new: AnalysisBundle,
}

impl SnapshotReader for Fixtures {
    fn node(
        &self,
        _side: SnapshotSide,
        _node_id: &str,
    ) -> Result<Option<NodeRecord>, SemanticDiffError> {
        Ok(None)
    }

    fn module(
        &self,
        side: SnapshotSide,
        source_file: &str,
    ) -> Result<Option<ModuleIr>, SemanticDiffError> {
        Ok(bundle(self, side)
            .program
            .modules
            .iter()
            .find(|module| module.source_file == source_file)
            .cloned())
    }

    fn summary(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Option<FunctionSummary>, SemanticDiffError> {
        Ok(bundle(self, side)
            .summaries
            .iter()
            .find(|summary| summary.symbol_id == symbol_id)
            .cloned())
    }

    fn reverse_callers(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Vec<String>, SemanticDiffError> {
        Ok(bundle(self, side)
            .reverse_calls
            .get(symbol_id)
            .cloned()
            .unwrap_or_default())
    }
}

fn bundle(fixtures: &Fixtures, side: SnapshotSide) -> &AnalysisBundle {
    match side {
        SnapshotSide::Old => &fixtures.old,
        SnapshotSide::New => &fixtures.new,
    }
}

fn analyze_typescript(source: &[u8]) -> Result<AnalysisBundle, Box<dyn Error>> {
    let batch = TreeSitterSyntaxProvider::default()
        .analyze_file(FileInput {
            source_file: "src/app.ts",
            language: "typescript",
            source,
        })?
        .ok_or("missing TypeScript batch")?;
    Ok(analyze(merge_evidence(vec![batch])?)?)
}

#[test]
fn required_parameter_and_behavior_changes_are_actionable() -> Result<(), Box<dyn Error>> {
    let fixtures = Fixtures {
        old: analyze_typescript(b"export function load(id: string) { return work(id); }")?,
        new: analyze_typescript(
            b"export async function load(id: string, mode: string) { await save(id); }",
        )?,
    };
    let deltas = [SourceFileDelta {
        old_path: Some("src/app.ts".to_owned()),
        new_path: Some("src/app.ts".to_owned()),
        status: SourceFileStatus::Modified,
        hunks: vec![SourceHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
        }],
    }];
    let report = compare(SemanticDiffInput {
        old: SnapshotIdentity {
            commit: "a".repeat(40),
            realization: "old".to_owned(),
            fingerprint: "f".repeat(64),
        },
        new: SnapshotIdentity {
            commit: "b".repeat(40),
            realization: "new".to_owned(),
            fingerprint: "f".repeat(64),
        },
        source_deltas: &deltas,
        changed_node_ids: &[],
        dependency_deltas: &[],
        snapshots: &fixtures,
        test_evidence: &NoTestEvidence,
    })?;
    assert_eq!(report.schema, REPORT_SCHEMA);
    assert!(report.findings.iter().any(|finding| {
        finding.compatibility == Compatibility::ProvenBreak
            && finding.explanation.contains("required parameter mode")
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.compatibility == Compatibility::Behavioral && finding.explanation.contains("calls")
    }));
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.id.starts_with("sd1-"))
    );
    assert_eq!(
        report.completeness,
        BTreeMap::from([
            (
                "identity".to_owned(),
                compass_semantic_diff::Completeness::Complete
            ),
            (
                "source_delta".to_owned(),
                compass_semantic_diff::Completeness::Complete,
            ),
            (
                "test_mapping".to_owned(),
                compass_semantic_diff::Completeness::Unavailable,
            ),
        ])
    );
    Ok(())
}
