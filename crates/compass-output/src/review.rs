use std::collections::BTreeMap;
use std::fmt::Write as _;

use compass_pr_intelligence::{
    Finding, FindingType, GateState, MergeOutcome, PullRequestReport, canonical_json_bytes,
    report_digest,
};
use serde_json::{Value, json};

use crate::OutputError;

pub const MAX_REVIEW_RENDER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedReview {
    pub content: String,
    pub omitted_findings: usize,
}

pub fn render_review_json(report: &PullRequestReport) -> Result<String, OutputError> {
    verify(report)?;
    let bytes = canonical_json_bytes(report)?;
    if bytes.len() > MAX_REVIEW_RENDER_BYTES {
        return Err(OutputError::ReviewBudgetExceeded {
            rendered_bytes: bytes.len(),
            limit: MAX_REVIEW_RENDER_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|error| OutputError::InvalidReview(error.to_string()))
}

pub fn render_review_text(report: &PullRequestReport) -> Result<String, OutputError> {
    verify(report)?;
    let mut output = String::new();
    let identity = &report.identity;
    let _ = writeln!(
        output,
        "PR review: {}{}",
        identity.repository.canonical_name(),
        identity
            .pull_request_number
            .map(|number| format!(" #{number}"))
            .unwrap_or_default()
    );
    let _ = writeln!(
        output,
        "Revisions: merge-base={} head={} target={} result={}",
        identity.revisions.merge_base,
        identity.revisions.pull_request_head,
        identity.revisions.target_head,
        merge_identity(&identity.revisions.merge_result)
    );
    let _ = writeln!(output, "Completeness: {:?}", report.completeness);
    let _ = writeln!(
        output,
        "Advisory risk: {:?}{} (rubric {}; never a merge gate)",
        report.advisory_risk.band,
        report
            .advisory_risk
            .score
            .map(|score| format!(" {score}/100"))
            .unwrap_or_default(),
        report.advisory_risk.rubric_version
    );
    output.push_str("Risk factors:\n");
    if report.risk_factors.is_empty() {
        output.push_str("  No risk factors.\n");
    }
    for factor in &report.risk_factors {
        let _ = writeln!(
            output,
            "  - {:?}: {} points — {}",
            factor.kind, factor.points, factor.explanation
        );
    }
    output.push_str("Gates:\n");
    for gate in &report.gates {
        let _ = writeln!(
            output,
            "  - {}: {:?} — {}",
            gate.id, gate.state, gate.statement
        );
    }
    output.push_str("Findings:\n");
    if report.findings.is_empty() {
        output.push_str("  No findings.\n");
    }
    for finding in &report.findings {
        let _ = writeln!(
            output,
            "  - [{}] {} ({:?}, {:?})",
            finding.fingerprint, finding.statement, finding.finding_type, finding.confidence
        );
        if finding.verification.gap {
            let _ = writeln!(
                output,
                "    verification gap: {}",
                finding.verification.reason
            );
        }
        for location in &finding.locations {
            let _ = writeln!(output, "    location: {}", location.path);
        }
        if !finding.witness.is_empty() {
            let witness = finding
                .witness
                .iter()
                .map(|hop| format!("{} -{}-> {}", hop.source, hop.relation, hop.target))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = writeln!(output, "    witness: {witness}");
        }
    }
    for omission in &report.omissions {
        let _ = writeln!(
            output,
            "Omitted {} {}: {}",
            omission.count, omission.category, omission.reason
        );
    }
    let _ = writeln!(output, "Report digest: {}", report.report_digest);
    enforce_budget(output)
}

pub fn render_review_markdown(report: &PullRequestReport) -> Result<RenderedReview, OutputError> {
    render_review_markdown_bounded(report, report.findings.len(), MAX_REVIEW_RENDER_BYTES)
}

pub fn render_review_markdown_bounded(
    report: &PullRequestReport,
    max_findings: usize,
    max_bytes: usize,
) -> Result<RenderedReview, OutputError> {
    verify(report)?;
    if max_bytes == 0 || max_bytes > MAX_REVIEW_RENDER_BYTES {
        return Err(OutputError::InvalidReview(format!(
            "Markdown byte limit must be between 1 and {MAX_REVIEW_RENDER_BYTES}"
        )));
    }
    let mut included = report.findings.len().min(max_findings);
    loop {
        let omitted = report.findings.len().saturating_sub(included);
        let content = markdown(report, included, omitted);
        if content.len() <= max_bytes {
            return Ok(RenderedReview {
                content,
                omitted_findings: omitted,
            });
        }
        if included == 0 {
            return Err(OutputError::ReviewBudgetExceeded {
                rendered_bytes: content.len(),
                limit: max_bytes,
            });
        }
        included -= 1;
    }
}

fn markdown(report: &PullRequestReport, included: usize, omitted: usize) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "## Compass PR review\n");
    let _ = writeln!(
        output,
        "Repository: `{}`  ",
        escape_markdown(&report.identity.repository.canonical_name())
    );
    let _ = writeln!(
        output,
        "Revisions: merge-base `{}`, head `{}`, target `{}`, result `{}`  ",
        report.identity.revisions.merge_base,
        report.identity.revisions.pull_request_head,
        report.identity.revisions.target_head,
        escape_markdown(&merge_identity(&report.identity.revisions.merge_result))
    );
    let _ = writeln!(output, "Completeness: `{:?}`  ", report.completeness);
    let _ = writeln!(
        output,
        "Advisory risk: **{:?}{}** (advisory only)  ",
        report.advisory_risk.band,
        report
            .advisory_risk
            .score
            .map(|score| format!(" · {score}/100"))
            .unwrap_or_default()
    );
    let _ = writeln!(output, "Report: `{}`\n", report.report_digest);
    output.push_str("### Advisory risk factors\n\n");
    if report.risk_factors.is_empty() {
        output.push_str("No risk factors.\n");
    }
    for factor in &report.risk_factors {
        let _ = writeln!(
            output,
            "- `{:?}` · **{} points** — {}",
            factor.kind,
            factor.points,
            escape_markdown(&factor.explanation)
        );
    }
    output.push_str("\n### Deterministic gates\n\n");
    for gate in &report.gates {
        let icon = match gate.state {
            GateState::Pass => "✅",
            GateState::Fail => "❌",
            GateState::Indeterminate => "⚠️",
            GateState::Error => "⛔",
        };
        let _ = writeln!(
            output,
            "- {icon} `{}`: **{:?}** — {}",
            escape_markdown(&gate.id),
            gate.state,
            escape_markdown(&gate.statement)
        );
    }
    output.push_str("\n### Findings\n\n");
    if report.findings.is_empty() {
        output.push_str("No findings.\n");
    }
    for finding in report.findings.iter().take(included) {
        let _ = writeln!(
            output,
            "- **{}** · `{:?}` · `{:?}`  ",
            escape_markdown(&finding.statement),
            finding.finding_type,
            finding.confidence
        );
        let _ = writeln!(output, "  Fingerprint: `{}`  ", finding.fingerprint);
        for location in &finding.locations {
            let _ = writeln!(
                output,
                "  Location: `{}`  ",
                escape_markdown(&location.path)
            );
        }
        if finding.verification.gap {
            let _ = writeln!(
                output,
                "  Verification gap: {}  ",
                escape_markdown(&finding.verification.reason)
            );
        }
        if !finding.witness.is_empty() {
            let witness = finding
                .witness
                .iter()
                .map(|hop| {
                    format!(
                        "{} -{}-> {}",
                        escape_markdown(&hop.source),
                        escape_markdown(&hop.relation),
                        escape_markdown(&hop.target)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            let _ = writeln!(output, "  Witness: {witness}  ");
        }
        if !finding.remediation.is_empty() {
            let _ = writeln!(
                output,
                "  Action: {}",
                escape_markdown(&finding.remediation)
            );
        }
    }
    if !report.omissions.is_empty() {
        output.push_str("\n### Canonical omissions\n\n");
        for omission in &report.omissions {
            let _ = writeln!(
                output,
                "- Exactly {} `{}` item(s): {}",
                omission.count,
                escape_markdown(&omission.category),
                escape_markdown(&omission.reason)
            );
        }
    }
    if omitted > 0 {
        let _ = writeln!(
            output,
            "\n_Exactly {omitted} finding(s) were omitted from this projection; the canonical JSON report is unchanged._"
        );
    }
    output
}

pub fn render_review_sarif(report: &PullRequestReport) -> Result<String, OutputError> {
    verify(report)?;
    let mut rules = BTreeMap::new();
    for finding in &report.findings {
        rules
            .entry(rule_id(finding.finding_type))
            .or_insert_with(|| {
                json!({
                    "id": rule_id(finding.finding_type),
                    "name": format!("{:?}", finding.finding_type),
                    "shortDescription": {"text": format!("Compass {:?}", finding.finding_type)},
                })
            });
    }
    let results = report.findings.iter().map(sarif_result).collect::<Vec<_>>();
    let sarif = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "Compass PR Intelligence",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/crabbuild/compass",
                "rules": rules.into_values().collect::<Vec<_>>(),
            }},
            "results": results,
            "properties": {
                "reportSchema": report.schema,
                "reportDigest": report.report_digest,
                "identity": report.identity,
                "completeness": report.completeness,
                "advisoryRisk": report.advisory_risk,
                "riskFactors": report.risk_factors,
                "gates": report.gates,
                "omissions": report.omissions,
            }
        }]
    });
    let bytes = canonical_json_bytes(&sarif)?;
    if bytes.len() > MAX_REVIEW_RENDER_BYTES {
        return Err(OutputError::ReviewBudgetExceeded {
            rendered_bytes: bytes.len(),
            limit: MAX_REVIEW_RENDER_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|error| OutputError::InvalidReview(error.to_string()))
}

fn merge_identity(outcome: &MergeOutcome) -> String {
    match outcome {
        MergeOutcome::Clean { object_id } => format!("clean:{object_id}"),
        MergeOutcome::Conflicted { evidence_digest } => {
            format!("conflicted:{evidence_digest}")
        }
        MergeOutcome::Unavailable { reason } => format!("unavailable:{reason}"),
    }
}

fn sarif_result(finding: &Finding) -> Value {
    let locations = finding
        .locations
        .iter()
        .map(|location| {
            json!({
                "physicalLocation": {
                    "artifactLocation": {"uri": location.path},
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "ruleId": rule_id(finding.finding_type),
        "level": if finding.deterministic {"error"} else if finding.verification.gap {"warning"} else {"note"},
        "message": {"text": finding.statement},
        "partialFingerprints": {"compassFindingFingerprint": finding.fingerprint},
        "locations": locations,
        "properties": {
            "confidence": finding.confidence,
            "completeness": finding.completeness,
            "freshness": finding.freshness,
            "sourceRevision": finding.source_revision,
            "evidenceDigest": finding.evidence_digest,
            "witness": finding.witness,
            "verification": finding.verification,
            "remediation": finding.remediation,
            "deterministic": finding.deterministic,
        }
    })
}

const fn rule_id(finding_type: FindingType) -> &'static str {
    match finding_type {
        FindingType::ArchitectureDelta => "compass/architecture-delta",
        FindingType::ContractChange => "compass/contract-change",
        FindingType::Impact => "compass/impact",
        FindingType::VerificationGap => "compass/verification-gap",
        FindingType::DependencyChange => "compass/dependency-change",
        FindingType::StructuralChange => "compass/structural-change",
    }
}

fn escape_markdown(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('`', "&#96;")
}

fn verify(report: &PullRequestReport) -> Result<(), OutputError> {
    report.validate()?;
    if report.report_digest != report_digest(report)? {
        return Err(OutputError::InvalidReview(
            "canonical report digest does not match its content".to_owned(),
        ));
    }
    Ok(())
}

fn enforce_budget(output: String) -> Result<String, OutputError> {
    if output.len() > MAX_REVIEW_RENDER_BYTES {
        Err(OutputError::ReviewBudgetExceeded {
            rendered_bytes: output.len(),
            limit: MAX_REVIEW_RENDER_BYTES,
        })
    } else {
        Ok(output)
    }
}
