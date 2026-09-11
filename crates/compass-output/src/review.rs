use std::collections::BTreeMap;
use std::fmt::{Debug, Write as _};

use compass_pr_intelligence::{
    Finding, FindingType, GateState, MergeOutcome, PullRequestReadiness, PullRequestReport,
    RepositoryIdentity, canonical_json_bytes, readiness_digest, report_digest,
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

pub fn render_readiness_json(readiness: &PullRequestReadiness) -> Result<String, OutputError> {
    verify_readiness(readiness)?;
    let bytes = canonical_json_bytes(readiness)?;
    if bytes.len() > MAX_REVIEW_RENDER_BYTES {
        return Err(OutputError::ReviewBudgetExceeded {
            rendered_bytes: bytes.len(),
            limit: MAX_REVIEW_RENDER_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|error| OutputError::InvalidReview(error.to_string()))
}

pub fn render_readiness_markdown(readiness: &PullRequestReadiness) -> Result<String, OutputError> {
    verify_readiness(readiness)?;
    let facets = &readiness.facets;
    let revisions = [
        readiness.revisions.merge_base.as_str(),
        readiness.revisions.pull_request_head.as_str(),
        readiness.revisions.target_head.as_str(),
    ];
    let references = [
        readiness.report_digest.as_str(),
        readiness.readiness_digest.as_str(),
    ];
    let ownership_revisions = facets
        .local_ownership
        .records
        .iter()
        .map(|record| record.evidence_revision.as_str())
        .collect::<Vec<_>>();
    let mut output = String::new();
    let _ = writeln!(output, "## Compass PR readiness\n");
    let _ = writeln!(
        output,
        "Comparison: base `{}` → PR `{}` · target `{}`  ",
        short_identifier_among(&readiness.revisions.merge_base, &revisions),
        short_identifier_among(&readiness.revisions.pull_request_head, &revisions),
        short_identifier_among(&readiness.revisions.target_head, &revisions)
    );
    let _ = writeln!(
        output,
        "References: report `{}` · readiness `{}`  ",
        short_identifier_among(&readiness.report_digest, &references),
        short_identifier_among(&readiness.readiness_digest, &references)
    );
    output.push_str("Full revision, profile, and evidence IDs are available in JSON.\n\n");
    output.push_str("### Review evidence\n\n");
    let _ = writeln!(
        output,
        "- Signature/body: **{}** · {} signature findings · {} body findings",
        debug_label(&facets.signature_body.state),
        facets.signature_body.signature_finding_fingerprints.len(),
        facets.signature_body.body_finding_fingerprints.len()
    );
    let _ = writeln!(
        output,
        "- Impact: **{}** · {} direct entities · {} transitive entities",
        debug_label(&facets.impact.state),
        facets.impact.direct_entities.len(),
        facets.impact.transitive_entities.len()
    );
    let _ = writeln!(
        output,
        "- Tests: **{}** · {} exact mappings · {} recommendations — {}",
        debug_label(&facets.tests.state),
        facets.tests.exact_tests.len(),
        facets.tests.recommended_tests.len(),
        escape_markdown(&facets.tests.statement)
    );
    let _ = writeln!(
        output,
        "- Documentation drift: **{}** (advisory only) — {}",
        debug_label(&facets.documentation_drift.state),
        escape_markdown(&facets.documentation_drift.statement)
    );
    if !facets
        .documentation_drift
        .linked_documentation_entities
        .is_empty()
    {
        let _ = writeln!(
            output,
            "  - Linked documentation: {} entities (exact IDs in JSON)",
            facets
                .documentation_drift
                .linked_documentation_entities
                .len()
        );
    }
    let _ = writeln!(
        output,
        "- Local ownership: **{}** · {} records — {}",
        debug_label(&facets.local_ownership.state),
        facets.local_ownership.records.len(),
        escape_markdown(&facets.local_ownership.statement)
    );
    if !facets.local_ownership.records.is_empty() {
        output.push_str("\n### Recent local ownership\n\n");
        for record in &facets.local_ownership.records {
            let _ = writeln!(
                output,
                "- `{}` · `{}` · {} commits at `{}`",
                escape_markdown(&record.path),
                escape_markdown(&record.contributor),
                record.commits,
                short_identifier_among(&record.evidence_revision, &ownership_revisions)
            );
        }
    }
    if !readiness.missing_evidence.is_empty() {
        output.push_str("\n### Missing evidence\n\n");
        for omission in &readiness.missing_evidence {
            let _ = writeln!(
                output,
                "- `{}` · {} — {}",
                escape_markdown(&omission.category),
                omission.count,
                escape_markdown(&omission.reason)
            );
        }
    }
    enforce_budget(output)
}

pub fn render_review_text(report: &PullRequestReport) -> Result<String, OutputError> {
    verify(report)?;
    let mut output = String::new();
    let identity = &report.identity;
    let revisions = [
        identity.revisions.merge_base.as_str(),
        identity.revisions.pull_request_head.as_str(),
        identity.revisions.target_head.as_str(),
    ];
    let finding_references = report
        .findings
        .iter()
        .map(|finding| finding.fingerprint.as_str())
        .collect::<Vec<_>>();
    let _ = writeln!(
        output,
        "Compass review: {}{}",
        repository_name(&identity.repository),
        identity
            .pull_request_number
            .map(|number| format!(" #{number}"))
            .unwrap_or_default()
    );
    let _ = writeln!(
        output,
        "Comparison: base {} -> PR {} (target {}; merge {})",
        short_identifier_among(&identity.revisions.merge_base, &revisions),
        short_identifier_among(&identity.revisions.pull_request_head, &revisions),
        short_identifier_among(&identity.revisions.target_head, &revisions),
        merge_summary(&identity.revisions.merge_result)
    );
    let _ = writeln!(
        output,
        "Evidence coverage: {}",
        debug_label(&report.completeness)
    );
    let _ = writeln!(
        output,
        "Risk: {}{} (advisory only; never a merge gate)",
        debug_label(&report.advisory_risk.band),
        report
            .advisory_risk
            .score
            .map(|score| format!(" · {score}/100"))
            .unwrap_or_default()
    );
    output.push_str("Risk factors:\n");
    if report.risk_factors.is_empty() {
        output.push_str("  No risk factors.\n");
    }
    for factor in &report.risk_factors {
        let _ = writeln!(
            output,
            "  - {}: {} points — {}",
            debug_label(&factor.kind),
            factor.points,
            factor.explanation
        );
    }
    output.push_str("Checks:\n");
    for gate in &report.gates {
        let _ = writeln!(
            output,
            "  - {}: {} — {}",
            readable_label(&gate.id),
            debug_label(&gate.state),
            gate.statement
        );
    }
    output.push_str("Findings:\n");
    if report.findings.is_empty() {
        output.push_str("  No findings.\n");
    }
    for finding in &report.findings {
        let _ = writeln!(
            output,
            "  - {} ({}, {} confidence)",
            finding.statement,
            debug_label(&finding.finding_type),
            debug_label(&finding.confidence).to_ascii_lowercase()
        );
        let _ = writeln!(
            output,
            "    reference: {}",
            short_identifier_among(&finding.fingerprint, &finding_references)
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
            let _ = writeln!(output, "    evidence path: {}", witness_summary(finding));
        }
    }
    for omission in &report.omissions {
        let _ = writeln!(
            output,
            "Omitted {} {}: {}",
            omission.count, omission.category, omission.reason
        );
    }
    let _ = writeln!(
        output,
        "Report reference: {}",
        short_identifier(&report.report_digest)
    );
    output.push_str("Full IDs and evidence are available with --format json.\n");
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
    let revisions = [
        report.identity.revisions.merge_base.as_str(),
        report.identity.revisions.pull_request_head.as_str(),
        report.identity.revisions.target_head.as_str(),
    ];
    let finding_references = report
        .findings
        .iter()
        .map(|finding| finding.fingerprint.as_str())
        .collect::<Vec<_>>();
    let _ = writeln!(output, "## Compass PR review\n");
    let _ = writeln!(
        output,
        "Repository: `{}`  ",
        escape_markdown(&repository_name(&report.identity.repository))
    );
    let _ = writeln!(
        output,
        "Comparison: base `{}` → PR `{}` · target `{}` · merge **{}**  ",
        short_identifier_among(&report.identity.revisions.merge_base, &revisions),
        short_identifier_among(&report.identity.revisions.pull_request_head, &revisions),
        short_identifier_among(&report.identity.revisions.target_head, &revisions),
        escape_markdown(&merge_summary(&report.identity.revisions.merge_result))
    );
    let _ = writeln!(
        output,
        "Evidence coverage: **{}**  ",
        debug_label(&report.completeness)
    );
    let _ = writeln!(
        output,
        "Risk: **{}{}** (advisory only)  ",
        debug_label(&report.advisory_risk.band),
        report
            .advisory_risk
            .score
            .map(|score| format!(" · {score}/100"))
            .unwrap_or_default()
    );
    let _ = writeln!(
        output,
        "Report reference: `{}`\n",
        short_identifier(&report.report_digest)
    );
    output.push_str("### Risk factors\n\n");
    if report.risk_factors.is_empty() {
        output.push_str("No risk factors.\n");
    }
    for factor in &report.risk_factors {
        let _ = writeln!(
            output,
            "- {} · **{} points** — {}",
            escape_markdown(&debug_label(&factor.kind)),
            factor.points,
            escape_markdown(&factor.explanation)
        );
    }
    output.push_str("\n### Merge checks\n\n");
    for gate in &report.gates {
        let icon = match gate.state {
            GateState::Pass => "✅",
            GateState::Fail => "❌",
            GateState::Indeterminate => "⚠️",
            GateState::Error => "⛔",
        };
        let _ = writeln!(
            output,
            "- {icon} **{}: {}** — {}",
            escape_markdown(&readable_label(&gate.id)),
            debug_label(&gate.state),
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
            "- **{}** · {} · {} confidence  ",
            escape_markdown(&finding.statement),
            escape_markdown(&debug_label(&finding.finding_type)),
            escape_markdown(&debug_label(&finding.confidence).to_ascii_lowercase())
        );
        let _ = writeln!(
            output,
            "  Reference: `{}`  ",
            short_identifier_among(&finding.fingerprint, &finding_references)
        );
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
            let _ = writeln!(
                output,
                "  Evidence path: {}  ",
                escape_markdown(&witness_summary(finding))
            );
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
        output.push_str("\n### Not included\n\n");
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
    output.push_str(
        "\n_Full revision IDs, finding fingerprints, and evidence are available in the JSON report._\n",
    );
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

fn merge_summary(outcome: &MergeOutcome) -> String {
    match outcome {
        MergeOutcome::Clean { .. } => "clean".to_owned(),
        MergeOutcome::Conflicted { .. } => "conflict detected".to_owned(),
        MergeOutcome::Unavailable { reason } => format!("unavailable — {reason}"),
    }
}

fn repository_name(repository: &RepositoryIdentity) -> String {
    if repository.host == "github.com" {
        format!("{}/{}", repository.owner, repository.name)
    } else {
        format!(
            "{}/{}/{}",
            repository.host, repository.owner, repository.name
        )
    }
}

fn short_identifier(value: &str) -> String {
    short_identifier_among(value, &[])
}

fn short_identifier_among(value: &str, peers: &[&str]) -> String {
    const VISIBLE: usize = 12;
    let (prefix, payload) = identifier_parts(value);
    let payload_len = payload.chars().count();
    if payload_len <= VISIBLE {
        return value.to_owned();
    }
    let visible_len = (VISIBLE..=payload_len)
        .find(|visible_len| {
            !peers.iter().copied().any(|peer| {
                if peer == value {
                    return false;
                }
                let (peer_prefix, peer_payload) = identifier_parts(peer);
                peer_prefix == prefix
                    && peer_payload
                        .chars()
                        .take(*visible_len)
                        .eq(payload.chars().take(*visible_len))
            })
        })
        .unwrap_or(payload_len);
    if visible_len == payload_len {
        return value.to_owned();
    }
    let visible = payload.chars().take(visible_len).collect::<String>();
    prefix.map_or_else(
        || format!("{visible}…"),
        |prefix| format!("{prefix}:{visible}…"),
    )
}

fn identifier_parts(value: &str) -> (Option<&str>, &str) {
    value
        .split_once(':')
        .map_or((None, value), |(prefix, payload)| (Some(prefix), payload))
}

fn debug_label(value: &impl Debug) -> String {
    readable_label(&format!("{value:?}"))
}

fn readable_label(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_was_lowercase_or_digit = false;
    for character in value.chars() {
        if matches!(character, '-' | '_') {
            if !output.ends_with(' ') {
                output.push(' ');
            }
            previous_was_lowercase_or_digit = false;
            continue;
        }
        if character.is_uppercase() && previous_was_lowercase_or_digit && !output.ends_with(' ') {
            output.push(' ');
        }
        output.extend(character.to_lowercase());
        previous_was_lowercase_or_digit = character.is_lowercase() || character.is_ascii_digit();
    }
    let output = output.trim();
    let mut characters = output.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_uppercase().chain(characters).collect()
}

fn witness_summary(finding: &Finding) -> String {
    let relations = finding
        .witness
        .iter()
        .map(|hop| readable_label(&hop.relation).to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" → ");
    let count = finding.witness.len();
    format!(
        "{count} relationship{} via {relations} (exact entities in JSON)",
        if count == 1 { "" } else { "s" }
    )
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

fn verify_readiness(readiness: &PullRequestReadiness) -> Result<(), OutputError> {
    readiness.validate()?;
    let expected = readiness_digest(readiness)?;
    if expected != readiness.readiness_digest {
        return Err(OutputError::InvalidReview(format!(
            "readiness digest mismatch: expected {expected}, found {}",
            readiness.readiness_digest
        )));
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
