use std::collections::BTreeMap;
use std::error::Error;

use compass_analysis::{AnalysisBundle, FunctionSummary, analyze};
use compass_history::{SourceFileDelta, SourceFileStatus, SourceHunk};
use compass_ir::ModuleIr;
use compass_languages::TreeSitterSyntaxProvider;
use compass_model::NodeRecord;
use compass_program::{FileInput, SyntaxProvider, merge_evidence};
use compass_semantic_diff::{
    ChangeDirection, Compatibility, Confidence, DependencyDelta, FindingType, GraphDelta,
    NoTestEvidence, REPORT_SCHEMA, SemanticDiffError, SemanticDiffInput, SnapshotIdentity,
    SnapshotReader, SnapshotSide, compare,
};

struct Fixtures {
    old: AnalysisBundle,
    new: AnalysisBundle,
    old_nodes: BTreeMap<String, NodeRecord>,
    new_nodes: BTreeMap<String, NodeRecord>,
}

impl SnapshotReader for Fixtures {
    fn node(
        &self,
        side: SnapshotSide,
        node_id: &str,
    ) -> Result<Option<NodeRecord>, SemanticDiffError> {
        let nodes = match side {
            SnapshotSide::Old => &self.old_nodes,
            SnapshotSide::New => &self.new_nodes,
        };
        Ok(nodes.get(node_id).cloned())
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

    fn dependency_participates_in_cycle(
        &self,
        _side: SnapshotSide,
        _source: &str,
        _target: &str,
    ) -> Result<Option<bool>, SemanticDiffError> {
        Ok(Some(true))
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
        old_nodes: BTreeMap::new(),
        new_nodes: BTreeMap::new(),
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
        patch: String::new(),
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
        graph_delta: &GraphDelta::default(),
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
                "call_resolution".to_owned(),
                compass_semantic_diff::Completeness::Partial,
            ),
            (
                "test_mapping".to_owned(),
                compass_semantic_diff::Completeness::Unavailable,
            ),
        ])
    );
    assert!(!report.feature_groups.is_empty());
    Ok(())
}

#[test]
fn comparisons_complete_in_both_directions_when_one_side_has_no_functions()
-> Result<(), Box<dyn Error>> {
    let old = analyze_typescript(b"export function load() { return 1; }")?;
    let new = analyze_typescript(b"export const value = 1;")?;
    let delta = [SourceFileDelta {
        old_path: Some("src/app.ts".to_owned()),
        new_path: Some("src/app.ts".to_owned()),
        status: SourceFileStatus::Modified,
        hunks: Vec::new(),
        patch: String::new(),
    }];
    let run = |old: AnalysisBundle, new: AnalysisBundle| {
        let fixtures = Fixtures {
            old,
            new,
            old_nodes: BTreeMap::new(),
            new_nodes: BTreeMap::new(),
        };
        compare(SemanticDiffInput {
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
            source_deltas: &delta,
            changed_node_ids: &[],
            dependency_deltas: &[],
            graph_delta: &GraphDelta::default(),
            snapshots: &fixtures,
            test_evidence: &NoTestEvidence,
        })
    };

    let forward = run(old.clone(), new.clone())?;
    let reverse = run(new, old)?;
    assert!(!forward.findings.is_empty());
    assert!(!reverse.findings.is_empty());
    Ok(())
}

#[test]
fn comparisons_complete_in_both_directions_when_impact_exceeds_depth() -> Result<(), Box<dyn Error>>
{
    let mut old = analyze_typescript(b"export function load(id: string) { return id; }")?;
    let mut new =
        analyze_typescript(b"export function load(id: string, mode: string) { return mode; }")?;
    for bundle in [&mut old, &mut new] {
        let mut callee = bundle
            .program
            .modules
            .iter()
            .flat_map(|module| &module.functions)
            .find(|function| function.name == "load")
            .ok_or("missing load function")?
            .symbol_id
            .clone();
        for depth in 0..=usize::from(compass_semantic_diff::MAX_IMPACT_DEPTH) {
            let caller = format!("caller-{depth}");
            bundle
                .reverse_calls
                .insert(callee.clone(), vec![caller.clone()]);
            callee = caller;
        }
    }
    let delta = [SourceFileDelta {
        old_path: Some("src/app.ts".to_owned()),
        new_path: Some("src/app.ts".to_owned()),
        status: SourceFileStatus::Modified,
        hunks: Vec::new(),
        patch: String::new(),
    }];
    let run = |old: AnalysisBundle, new: AnalysisBundle| {
        let fixtures = Fixtures {
            old,
            new,
            old_nodes: BTreeMap::new(),
            new_nodes: BTreeMap::new(),
        };
        compare(SemanticDiffInput {
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
            source_deltas: &delta,
            changed_node_ids: &[],
            dependency_deltas: &[],
            graph_delta: &GraphDelta::default(),
            snapshots: &fixtures,
            test_evidence: &NoTestEvidence,
        })
    };

    for report in [run(old.clone(), new.clone())?, run(new, old)?] {
        assert!(report.limitations.iter().any(|limitation| {
            limitation.contains("affected-consumer mapping")
                && limitation.contains("truncated at depth")
        }));
    }
    Ok(())
}

#[test]
fn graph_only_additions_and_removals_are_classified_before_digest_changes()
-> Result<(), Box<dyn Error>> {
    let old = analyze_typescript(b"")?;
    let new = analyze_typescript(b"")?;
    let added_id = "src_api_new_handler".to_owned();
    let removed_id = "src_api_old_handler".to_owned();
    let internal_exported_id = "src_internal_exported".to_owned();
    let internal_hidden_id = "src_internal_hidden".to_owned();
    let node = |id: &str, label: &str, source_file: &str| NodeRecord {
        id: id.to_owned(),
        attributes: serde_json::Map::from_iter([
            ("label".to_owned(), serde_json::json!(label)),
            ("symbol_kind".to_owned(), serde_json::json!("function")),
            ("source_file".to_owned(), serde_json::json!(source_file)),
            (
                "signature_hash".to_owned(),
                serde_json::json!("digest-that-must-not-be-treated-as-a-modification"),
            ),
        ]),
    };
    let community_node = |id: &str, label: &str, source_file: &str, community: u64| {
        let mut node = node(id, label, source_file);
        node.attributes
            .insert("community".to_owned(), serde_json::json!({"id": community}));
        node
    };
    let fixtures = Fixtures {
        old,
        new,
        old_nodes: BTreeMap::from([(
            removed_id.clone(),
            node(&removed_id, "old_handler", "src/api.ts"),
        )]),
        new_nodes: BTreeMap::from([
            (
                "src_package_api".to_owned(),
                community_node("src_package_api", "api", "src/package/api.py", 7),
            ),
            (
                added_id.clone(),
                node(&added_id, "new_handler", "src/api.ts"),
            ),
            (
                internal_exported_id.clone(),
                community_node(
                    &internal_exported_id,
                    "Exported",
                    "src/package/_internal.py",
                    9,
                ),
            ),
            (
                internal_hidden_id.clone(),
                node(&internal_hidden_id, "Hidden", "src/package/_internal.py"),
            ),
        ]),
    };
    let changed = [
        added_id.clone(),
        removed_id.clone(),
        internal_exported_id.clone(),
        internal_hidden_id.clone(),
    ];
    let dependencies = [DependencyDelta {
        source: "src_package_api".to_owned(),
        relation: "imports".to_owned(),
        target: internal_exported_id.clone(),
        change: ChangeDirection::Added,
        evidence: Vec::new(),
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
        source_deltas: &[],
        changed_node_ids: &changed,
        dependency_deltas: &dependencies,
        graph_delta: &GraphDelta::default(),
        snapshots: &fixtures,
        test_evidence: &NoTestEvidence,
    })?;
    let added = report
        .findings
        .iter()
        .find(|finding| finding.subject == added_id)
        .ok_or("missing added finding")?;
    let removed = report
        .findings
        .iter()
        .find(|finding| finding.subject == removed_id)
        .ok_or("missing removed finding")?;
    assert!(added.before.is_none() && added.after.is_some());
    assert!(removed.before.is_some() && removed.after.is_none());
    assert!(added.public_surface && removed.public_surface);
    assert!(added.headline.contains("was added"));
    assert!(removed.headline.contains("was removed"));
    assert!(
        report
            .findings
            .iter()
            .find(|finding| finding.subject == internal_exported_id)
            .is_some_and(|finding| finding.public_surface)
    );
    assert!(
        report
            .findings
            .iter()
            .find(|finding| finding.subject == internal_hidden_id)
            .is_some_and(|finding| !finding.public_surface)
    );
    let dependency = report
        .findings
        .iter()
        .find(|finding| finding.finding_type == FindingType::DependencyChange)
        .and_then(|finding| finding.dependency_topology.as_ref())
        .ok_or("missing dependency topology")?;
    assert_eq!(dependency.source_community, Some(7));
    assert_eq!(dependency.target_community, Some(9));
    assert_eq!(dependency.participates_in_cycle, Some(true));
    Ok(())
}

#[test]
fn graph_fallback_explains_control_flow_patch_for_changed_function() -> Result<(), Box<dyn Error>> {
    let node_id = "libpod_container_getcontainerinspectdata".to_owned();
    let node = |implementation_hash: &str| NodeRecord {
        id: node_id.clone(),
        attributes: serde_json::Map::from_iter([
            (
                "label".to_owned(),
                serde_json::json!(".getContainerInspectData()"),
            ),
            ("symbol_kind".to_owned(), serde_json::json!("method")),
            (
                "source_file".to_owned(),
                serde_json::json!("libpod/container_inspect.go"),
            ),
            ("line_start".to_owned(), serde_json::json!(66)),
            ("line_end".to_owned(), serde_json::json!(160)),
            (
                "implementation_hash".to_owned(),
                serde_json::json!(implementation_hash),
            ),
        ]),
    };
    let fixtures = Fixtures {
        old: analyze_typescript(b"")?,
        new: analyze_typescript(b"")?,
        old_nodes: BTreeMap::from([(node_id.clone(), node("old-body"))]),
        new_nodes: BTreeMap::from([(node_id.clone(), node("new-body"))]),
    };
    let source_deltas = [SourceFileDelta {
        old_path: Some("libpod/container_inspect.go".to_owned()),
        new_path: Some("libpod/container_inspect.go".to_owned()),
        status: SourceFileStatus::Modified,
        hunks: vec![SourceHunk {
            old_start: 80,
            old_lines: 2,
            new_start: 79,
            new_lines: 0,
        }],
        patch: concat!(
            "diff --git a/libpod/container_inspect.go b/libpod/container_inspect.go\n",
            "--- a/libpod/container_inspect.go\n",
            "+++ b/libpod/container_inspect.go\n",
            "@@ -80,2 +79,0 @@ func (c *Container) getContainerInspectData(size bool, driverData *define.DriverData) (*define.InspectContainerData, error) {\n",
            "-\t}\n",
            "-\tif len(args) > 1 {\n",
        )
        .to_owned(),
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
        source_deltas: &source_deltas,
        changed_node_ids: std::slice::from_ref(&node_id),
        dependency_deltas: &[],
        graph_delta: &GraphDelta::default(),
        snapshots: &fixtures,
        test_evidence: &NoTestEvidence,
    })?;
    assert_eq!(
        report
            .entity_display_names
            .get(&node_id)
            .map(String::as_str),
        Some(".getContainerInspectData()")
    );
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.subject == node_id)
        .ok_or("missing control-flow finding")?;
    assert_eq!(finding.finding_type, FindingType::BehaviorChange);
    assert_eq!(finding.compatibility, Compatibility::Behavioral);
    assert_eq!(finding.confidence, Confidence::Exact);
    assert!(
        finding
            .explanation
            .contains("removed condition `if len(args) > 1`")
    );
    assert_eq!(
        finding
            .before
            .as_ref()
            .and_then(|value| value["conditions"].as_array())
            .and_then(|values| values.first())
            .and_then(serde_json::Value::as_str),
        Some("if len(args) > 1")
    );
    assert_eq!(
        finding.completeness.get("control_flow").copied(),
        Some(compass_semantic_diff::Completeness::Partial)
    );
    Ok(())
}
