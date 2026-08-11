use compass_output::{
    render_review_json, render_review_markdown, render_review_markdown_bounded,
    render_review_sarif, render_review_text,
};
use compass_pr_intelligence::{
    AdvisoryRisk, Completeness, Confidence, Finding, FindingType, Freshness, GateResult, GateState,
    Location, MergeOutcome, PullRequestReport, ReportIdentity, RepositoryIdentity, RevisionSet,
    RiskBand, RiskFactor, RiskFactorKind, VerificationPlan, VerificationState, WitnessHop,
    report_digest,
};

fn report() -> Result<PullRequestReport, Box<dyn std::error::Error>> {
    let fingerprint = "cmpprv1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut report = PullRequestReport {
        schema: compass_pr_intelligence::REPORT_SCHEMA.to_owned(),
        identity: ReportIdentity {
            repository: RepositoryIdentity {
                forge: "github".to_owned(),
                host: "github.com".to_owned(),
                owner: "crabbuild".to_owned(),
                name: "compass".to_owned(),
            },
            pull_request_number: Some(42),
            revisions: RevisionSet {
                merge_base: "1".repeat(40),
                pull_request_head: "2".repeat(40),
                target_head: "3".repeat(40),
                merge_result: MergeOutcome::Clean {
                    object_id: "4".repeat(40),
                },
            },
            graph_schema: "networkx-node-link/v1".to_owned(),
            extractor_version: "extractor/1".to_owned(),
            configuration_digest: "5".repeat(64),
            policy_pack_digest: format!("sha256:{}", "6".repeat(64)),
            evidence_manifest_digest: format!("sha256:{}", "7".repeat(64)),
        },
        completeness: Completeness::DownstreamComplete,
        findings: vec![Finding {
            fingerprint: fingerprint.to_owned(),
            finding_type: FindingType::ContractChange,
            classifier_version: 1,
            statement: "Changed <public> `API`".to_owned(),
            source_entities: vec!["symbol:api".to_owned()],
            target_entities: vec!["symbol:caller".to_owned()],
            witness: vec![WitnessHop {
                source: "symbol:caller".to_owned(),
                relation: "calls".to_owned(),
                target: "symbol:api".to_owned(),
                confidence: Confidence::Exact,
            }],
            locations: vec![Location {
                path: "src/api.rs".to_owned(),
                start_byte: Some(10),
                end_byte: Some(20),
            }],
            verification: VerificationPlan {
                state: VerificationState::Gap,
                exact_tests: Vec::new(),
                recommended_tests: vec!["test_api".to_owned()],
                gap: true,
                reason: "No exact test".to_owned(),
            },
            source_revision: "4".repeat(40),
            evidence_source: "compass-semantic-diff".to_owned(),
            evidence_digest: format!("sha256:{}", "7".repeat(64)),
            confidence: Confidence::Exact,
            completeness: Completeness::DownstreamComplete,
            freshness: Freshness::ExactHead,
            remediation: "Update the caller".to_owned(),
            deterministic: true,
        }],
        risk_factors: vec![RiskFactor {
            kind: RiskFactorKind::PublicContractChange,
            points: 20,
            explanation: "Public contracts changed".to_owned(),
            finding_fingerprints: vec![fingerprint.to_owned()],
        }],
        advisory_risk: AdvisoryRisk {
            rubric_version: 1,
            score: Some(20),
            band: RiskBand::Moderate,
            explanation: "Advisory only".to_owned(),
        },
        gates: vec![GateResult {
            id: "proven-contract-break".to_owned(),
            rule_version: 1,
            state: GateState::Fail,
            statement: "Exact break".to_owned(),
            finding_fingerprints: vec![fingerprint.to_owned()],
        }],
        omissions: Vec::new(),
        report_digest: format!("sha256:{}", "0".repeat(64)),
    };
    report.report_digest = report_digest(&report)?;
    Ok(report)
}

#[test]
fn all_projections_preserve_fingerprint_and_count() -> Result<(), Box<dyn std::error::Error>> {
    let report = report()?;
    let json = render_review_json(&report)?;
    let text = render_review_text(&report)?;
    let markdown = render_review_markdown(&report)?.content;
    let sarif = render_review_sarif(&report)?;
    for projection in [&json, &text, &markdown, &sarif] {
        assert!(projection.contains(&report.findings[0].fingerprint));
    }
    let round_trip = PullRequestReport::from_json(json.as_bytes())?;
    assert_eq!(round_trip.findings.len(), 1);
    let sarif: serde_json::Value = serde_json::from_str(&sarif)?;
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(
        sarif["runs"][0]["results"].as_array().map(Vec::len),
        Some(1)
    );
    assert!(markdown.contains("&lt;public&gt; &#96;API&#96;"));
    Ok(())
}

#[test]
fn bounded_markdown_reports_exact_omission_without_mutating_digest()
-> Result<(), Box<dyn std::error::Error>> {
    let mut report = report()?;
    let second = Finding {
        fingerprint: "cmpprv1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
        statement: "Second finding".to_owned(),
        ..report.findings[0].clone()
    };
    report.findings.push(second);
    report.report_digest = report_digest(&report)?;
    let digest = report.report_digest.clone();
    let rendered = render_review_markdown_bounded(&report, 1, 16 * 1024)?;
    assert_eq!(rendered.omitted_findings, 1);
    assert!(
        rendered
            .content
            .contains("Exactly 1 finding(s) were omitted")
    );
    assert_eq!(report.report_digest, digest);
    Ok(())
}
