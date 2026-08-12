use std::collections::BTreeMap;

use compass_pr_intelligence::{
    ChangeHunk, ChangeRequest, Completeness, EvidenceManifest, EvidenceSource, FacetState,
    GateState, GraphSnapshot, LocalOwnership, MergeOutcome, PullRequestReadiness,
    PullRequestReport, ReadinessExtractionFingerprints, RepositoryIdentity, RevisionSet, RiskBand,
    RiskFactorKind, SourceRange, analyze, build_readiness, canonical_json_bytes,
};
use compass_semantic_diff::{
    AffectedConsumer, Comparison, Compatibility, Confidence, DependencyTopology, EvidenceRef,
    FindingOrigin, FindingType, GraphDelta, SemanticDiffReport, SemanticFinding, Verification,
    VerificationState, WitnessHop, WitnessPath,
};
use serde::Deserialize;
use serde_json::json;

const BASE: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";
const RESULT: &str = "3333333333333333333333333333333333333333";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PROFILE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn repository() -> RepositoryIdentity {
    RepositoryIdentity {
        forge: "git".to_owned(),
        host: "local".to_owned(),
        owner: "fixture".to_owned(),
        name: "project".to_owned(),
    }
}

fn extraction_fingerprints() -> ReadinessExtractionFingerprints {
    ReadinessExtractionFingerprints {
        base: PROFILE.to_owned(),
        comparison: PROFILE.to_owned(),
    }
}

fn request(merge_result: MergeOutcome) -> ChangeRequest {
    ChangeRequest {
        repository: repository(),
        pull_request_number: Some(7),
        revisions: RevisionSet {
            merge_base: BASE.to_owned(),
            pull_request_head: HEAD.to_owned(),
            target_head: BASE.to_owned(),
            merge_result,
        },
        hunks: Vec::new(),
    }
}

fn snapshot(revision: &str) -> GraphSnapshot {
    GraphSnapshot {
        revision: revision.to_owned(),
        realization: format!("realization-{revision}"),
        graph_schema: "networkx-node-link/v1".to_owned(),
        extractor_version: "extractor/1".to_owned(),
        configuration_digest: PROFILE.to_owned(),
    }
}

fn manifest(
    completeness: Completeness,
) -> Result<EvidenceManifest, compass_pr_intelligence::PrIntelligenceError> {
    EvidenceManifest {
        digest: String::new(),
        graph_schema: "networkx-node-link/v1".to_owned(),
        extractor_version: "extractor/1".to_owned(),
        configuration_digest: PROFILE.to_owned(),
        policy_pack_digest: DIGEST.to_owned(),
        completeness,
        repositories: Vec::new(),
        sources: vec![EvidenceSource {
            kind: "semantic_diff".to_owned(),
            identity: "fixture".to_owned(),
            digest: DIGEST.to_owned(),
            completeness,
        }],
    }
    .seal()
}

fn semantic_finding(confidence: Confidence) -> SemanticFinding {
    SemanticFinding {
        id: "semantic-1".to_owned(),
        finding_type: FindingType::ContractChange,
        subject: "symbol:api".to_owned(),
        origin: FindingOrigin::Direct,
        headline: "Public API changed".to_owned(),
        explanation: "The public signature changed".to_owned(),
        compatibility: Compatibility::ProvenBreak,
        confidence,
        review_priority: 100,
        public_surface: true,
        routine: false,
        before: Some(json!({"signature": "fn old()", "start_line": 4})),
        after: Some(json!({"signature": "fn new()", "start_line": 9})),
        affected_consumers: vec![AffectedConsumer {
            symbol_id: "symbol:caller".to_owned(),
            display_name: "caller".to_owned(),
            source_file: "src/caller.rs".to_owned(),
            distance: 1,
        }],
        witness_paths: vec![WitnessPath {
            consumer: "symbol:caller".to_owned(),
            confidence,
            hops: vec![WitnessHop {
                source: "symbol:caller".to_owned(),
                relation: "calls".to_owned(),
                target: "symbol:api".to_owned(),
                confidence,
            }],
        }],
        verification: Verification {
            state: VerificationState::Gap,
            exact_tests: Vec::new(),
            recommended_tests: vec!["test_api".to_owned()],
            reason: "No exact test evidence".to_owned(),
        },
        reviewer_action: "Update consumers and add an exact test".to_owned(),
        evidence: vec![EvidenceRef {
            source_file: "src/api.rs".to_owned(),
            start_byte: Some(10),
            end_byte: Some(20),
            record_key: None,
            capability: "signature".to_owned(),
        }],
        dependency_topology: None,
        completeness: BTreeMap::new(),
    }
}

fn semantic_report(new_commit: &str, findings: Vec<SemanticFinding>) -> SemanticDiffReport {
    SemanticDiffReport {
        schema: compass_semantic_diff::REPORT_SCHEMA.to_owned(),
        comparison: Comparison {
            old_commit: BASE.to_owned(),
            new_commit: new_commit.to_owned(),
            fingerprint: DIGEST.to_owned(),
        },
        findings,
        feature_groups: Vec::new(),
        collapsed_groups: Vec::new(),
        source_changes: Vec::new(),
        graph_delta: GraphDelta::default(),
        entity_display_names: BTreeMap::new(),
        completeness: BTreeMap::new(),
        limitations: Vec::new(),
    }
}

fn clean_report(
    completeness: Completeness,
    finding: Option<SemanticFinding>,
) -> Result<PullRequestReport, Box<dyn std::error::Error>> {
    let request = request(MergeOutcome::Clean {
        object_id: RESULT.to_owned(),
    });
    let semantic = semantic_report(RESULT, finding.into_iter().collect());
    Ok(analyze(
        &request,
        &snapshot(BASE),
        Some(&snapshot(RESULT)),
        &manifest(completeness)?,
        &semantic,
    )?)
}

#[test]
fn identical_input_is_byte_identical_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let first = clean_report(
        Completeness::DownstreamComplete,
        Some(semantic_finding(Confidence::Exact)),
    )?;
    let second = clean_report(
        Completeness::DownstreamComplete,
        Some(semantic_finding(Confidence::Exact)),
    )?;
    let first_bytes = canonical_json_bytes(&first)?;
    assert_eq!(first_bytes, canonical_json_bytes(&second)?);
    assert_eq!(first, PullRequestReport::from_json(&first_bytes)?);
    assert_eq!(first.gates[0].state, GateState::Fail);
    Ok(())
}

#[test]
fn readiness_is_additive_deterministic_and_conservative_about_tests_and_docs()
-> Result<(), Box<dyn std::error::Error>> {
    let mut documented_change = semantic_finding(Confidence::Exact);
    documented_change.witness_paths[0].hops[0].relation = "documents".to_owned();
    let report = clean_report(Completeness::DownstreamComplete, Some(documented_change))?;
    let report_bytes = canonical_json_bytes(&report)?;
    let mut change = request(MergeOutcome::Clean {
        object_id: RESULT.to_owned(),
    });
    change.hunks.push(ChangeHunk {
        old_path: "src/api.rs".to_owned(),
        new_path: "src/api.rs".to_owned(),
        status: "modified".to_owned(),
        old: SourceRange {
            start_line: 1,
            line_count: 1,
        },
        new: SourceRange {
            start_line: 1,
            line_count: 1,
        },
        patch_digest: DIGEST.to_owned(),
    });
    let ownership = vec![LocalOwnership {
        path: "src/api.rs".to_owned(),
        contributor: "maintainer@example.test".to_owned(),
        commits: 3,
        evidence_revision: HEAD.to_owned(),
    }];
    let first = build_readiness(
        &report,
        &change,
        extraction_fingerprints(),
        ownership.clone(),
        None,
    )?;
    let second = build_readiness(&report, &change, extraction_fingerprints(), ownership, None)?;
    let first_bytes = canonical_json_bytes(&first)?;
    assert_eq!(first_bytes, canonical_json_bytes(&second)?);
    assert_eq!(first, PullRequestReadiness::from_json(&first_bytes)?);
    assert_eq!(first.report_digest, report.report_digest);
    assert_eq!(canonical_json_bytes(&report)?, report_bytes);
    assert_eq!(first.facets.documentation_drift.state, FacetState::Advisory);
    assert!(first.facets.documentation_drift.advisory_only);
    assert!(
        !first
            .facets
            .documentation_drift
            .linked_documentation_entities
            .is_empty()
    );
    assert_eq!(first.facets.tests.state, FacetState::Advisory);
    assert!(
        first
            .facets
            .tests
            .statement
            .contains("does not claim tests were run")
    );
    Ok(())
}

#[test]
fn absent_test_and_ownership_evidence_remain_unknown_not_untested()
-> Result<(), Box<dyn std::error::Error>> {
    let report = clean_report(Completeness::DownstreamComplete, None)?;
    let change = request(MergeOutcome::Clean {
        object_id: RESULT.to_owned(),
    });
    let readiness = build_readiness(
        &report,
        &change,
        extraction_fingerprints(),
        Vec::new(),
        Some("local history unavailable".to_owned()),
    )?;
    assert_eq!(readiness.facets.tests.state, FacetState::Unknown);
    assert!(
        readiness
            .facets
            .tests
            .statement
            .contains("unknown rather than untested")
    );
    assert_eq!(readiness.facets.local_ownership.state, FacetState::Unknown);
    Ok(())
}

#[test]
fn source_location_moves_do_not_change_fingerprint() -> Result<(), Box<dyn std::error::Error>> {
    let first = clean_report(
        Completeness::DownstreamComplete,
        Some(semantic_finding(Confidence::Exact)),
    )?;
    let mut moved = semantic_finding(Confidence::Exact);
    moved.before = Some(json!({"signature": "fn old()", "start_line": 400}));
    moved.after = Some(json!({"signature": "fn new()", "start_line": 900}));
    moved.evidence[0].start_byte = Some(1_000);
    moved.evidence[0].end_byte = Some(2_000);
    let second = clean_report(Completeness::DownstreamComplete, Some(moved))?;
    assert_eq!(
        first.findings[0].fingerprint,
        second.findings[0].fingerprint
    );

    let mut changed = semantic_finding(Confidence::Exact);
    changed.subject = "symbol:different-api".to_owned();
    let third = clean_report(Completeness::DownstreamComplete, Some(changed))?;
    assert_ne!(first.findings[0].fingerprint, third.findings[0].fingerprint);

    let mut witness_changed = semantic_finding(Confidence::Exact);
    witness_changed.witness_paths[0].hops[0].relation = "references".to_owned();
    let fourth = clean_report(Completeness::DownstreamComplete, Some(witness_changed))?;
    assert_ne!(
        first.findings[0].fingerprint,
        fourth.findings[0].fingerprint
    );

    let mut volatile_only = semantic_finding(Confidence::Exact);
    volatile_only.before = Some(json!({
        "signature": "fn old()",
        "start_line": 400,
        "duration_ms": 19,
        "generated_at": "tomorrow"
    }));
    volatile_only.after = Some(json!({
        "signature": "fn new()",
        "start_line": 900,
        "duration_ms": 20,
        "generated_at": "tomorrow"
    }));
    let fifth = clean_report(Completeness::DownstreamComplete, Some(volatile_only))?;
    assert_eq!(first.findings[0].fingerprint, fifth.findings[0].fingerprint);
    Ok(())
}

#[test]
fn incomplete_evidence_never_reduces_advisory_risk() -> Result<(), Box<dyn std::error::Error>> {
    let complete = clean_report(
        Completeness::DownstreamComplete,
        Some(semantic_finding(Confidence::Inferred)),
    )?;
    let partial = clean_report(
        Completeness::DownstreamPartial,
        Some(semantic_finding(Confidence::Inferred)),
    )?;
    assert!(partial.advisory_risk.score >= complete.advisory_risk.score);
    assert!(partial.advisory_risk.band >= complete.advisory_risk.band);
    assert_eq!(partial.gates[0].state, GateState::Indeterminate);
    Ok(())
}

#[test]
fn dependency_factors_require_typed_topology_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let mut dependency = semantic_finding(Confidence::Exact);
    dependency.finding_type = FindingType::DependencyChange;
    dependency.public_surface = false;
    dependency.compatibility = Compatibility::Behavioral;
    dependency.affected_consumers.clear();
    dependency.witness_paths.clear();
    dependency.verification.state = VerificationState::Covered;
    dependency.verification.recommended_tests.clear();
    dependency.verification.reason = "Exact test coverage".to_owned();

    let unqualified = clean_report(Completeness::DownstreamComplete, Some(dependency.clone()))?;
    assert!(unqualified.risk_factors.is_empty());

    dependency.dependency_topology = Some(DependencyTopology {
        source_community: Some(7),
        target_community: Some(9),
        participates_in_cycle: Some(true),
    });
    let qualified = clean_report(Completeness::DownstreamComplete, Some(dependency))?;
    let kinds = qualified
        .risk_factors
        .iter()
        .map(|factor| factor.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [RiskFactorKind::CrossBoundaryImpact, RiskFactorKind::Cycle]
    );
    Ok(())
}

#[test]
fn conflicted_merge_is_unavailable_and_indeterminate() -> Result<(), Box<dyn std::error::Error>> {
    let request = request(MergeOutcome::Conflicted {
        evidence_digest: DIGEST.to_owned(),
    });
    let report = analyze(
        &request,
        &snapshot(BASE),
        None,
        &manifest(Completeness::LocalExact)?,
        &semantic_report(HEAD, vec![semantic_finding(Confidence::Exact)]),
    )?;
    assert_eq!(report.advisory_risk.band, RiskBand::Unavailable);
    assert_eq!(report.gates[0].state, GateState::Indeterminate);
    Ok(())
}

#[test]
fn unknown_fields_and_unknown_major_schema_fail() -> Result<(), Box<dyn std::error::Error>> {
    let report = clean_report(Completeness::LocalExact, None)?;
    let mut value = serde_json::to_value(report)?;
    value["unexpected"] = json!(true);
    assert!(PullRequestReport::from_json(&serde_json::to_vec(&value)?).is_err());
    value.as_object_mut().ok_or("object")?.remove("unexpected");
    value["schema"] = json!("compass.pr_intelligence.report/2");
    assert!(PullRequestReport::from_json(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}

#[test]
fn nested_contract_tampering_and_unknown_references_fail() -> Result<(), Box<dyn std::error::Error>>
{
    let report = clean_report(
        Completeness::DownstreamComplete,
        Some(semantic_finding(Confidence::Exact)),
    )?;
    let mut unknown_nested = serde_json::to_value(&report)?;
    unknown_nested["findings"][0]["unexpected"] = json!(true);
    assert!(PullRequestReport::from_json(&serde_json::to_vec(&unknown_nested)?).is_err());

    let mut unknown_reference = report.clone();
    unknown_reference.gates[0].finding_fingerprints = vec![format!("cmpprv1:{}", "f".repeat(64))];
    unknown_reference.report_digest = compass_pr_intelligence::report_digest(&unknown_reference)?;
    assert!(unknown_reference.validate().is_err());

    let mut malformed_digest = report;
    malformed_digest.identity.evidence_manifest_digest = "sha256:not-hex".to_owned();
    malformed_digest.report_digest = compass_pr_intelligence::report_digest(&malformed_digest)?;
    assert!(malformed_digest.validate().is_err());
    Ok(())
}

#[test]
fn manifest_sealing_normalizes_order_and_rejects_duplicates()
-> Result<(), Box<dyn std::error::Error>> {
    let mut first = manifest(Completeness::LocalExact)?;
    first.sources.push(EvidenceSource {
        kind: "change_request".to_owned(),
        identity: "fixture".to_owned(),
        digest: DIGEST.to_owned(),
        completeness: Completeness::LocalExact,
    });
    first.digest.clear();
    let first = first.seal()?;
    let mut reversed = first.clone();
    reversed.sources.reverse();
    reversed.digest.clear();
    let reversed = reversed.seal()?;
    assert_eq!(first, reversed);

    let mut duplicate = first;
    duplicate.sources.push(duplicate.sources[0].clone());
    duplicate.digest.clear();
    assert!(duplicate.seal().is_err());
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCase {
    name: String,
    confidence: String,
    completeness: Completeness,
    merge: String,
    expected_band: RiskBand,
    expected_gate: GateState,
}

#[test]
fn golden_scenario_matrix_covers_required_evidence_states() -> Result<(), Box<dyn std::error::Error>>
{
    let cases: Vec<GoldenCase> = serde_json::from_str(include_str!("fixtures/cases.json"))?;
    let names = cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "exact",
            "inferred",
            "ambiguous",
            "partial",
            "identical",
            "conflicted",
            "corrupt_incompatible"
        ]
    );
    for case in cases {
        if case.name == "corrupt_incompatible" {
            let mut bad = semantic_report(RESULT, Vec::new());
            bad.schema = "compass.semantic_diff.report/2".to_owned();
            assert!(
                analyze(
                    &request(MergeOutcome::Clean {
                        object_id: RESULT.to_owned(),
                    }),
                    &snapshot(BASE),
                    Some(&snapshot(RESULT)),
                    &manifest(case.completeness)?,
                    &bad,
                )
                .is_err()
            );
            continue;
        }
        let confidence = match case.confidence.as_str() {
            "exact" => Confidence::Exact,
            "inferred" => Confidence::Inferred,
            "unknown" => Confidence::Unknown,
            "none" => {
                let report = clean_report(case.completeness, None)?;
                assert_eq!(
                    report.advisory_risk.band, case.expected_band,
                    "{}",
                    case.name
                );
                assert_eq!(report.gates[0].state, case.expected_gate, "{}", case.name);
                continue;
            }
            value => return Err(format!("unsupported confidence {value}").into()),
        };
        if case.merge == "conflicted" {
            let report = analyze(
                &request(MergeOutcome::Conflicted {
                    evidence_digest: DIGEST.to_owned(),
                }),
                &snapshot(BASE),
                None,
                &manifest(case.completeness)?,
                &semantic_report(HEAD, vec![semantic_finding(confidence)]),
            )?;
            assert_eq!(
                report.advisory_risk.band, case.expected_band,
                "{}",
                case.name
            );
            assert_eq!(report.gates[0].state, case.expected_gate, "{}", case.name);
        } else {
            let report = clean_report(case.completeness, Some(semantic_finding(confidence)))?;
            assert_eq!(
                report.advisory_risk.band, case.expected_band,
                "{}",
                case.name
            );
            assert_eq!(report.gates[0].state, case.expected_gate, "{}", case.name);
        }
    }
    Ok(())
}
