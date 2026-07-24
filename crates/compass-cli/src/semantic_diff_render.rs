use std::collections::BTreeSet;
use std::fmt::Write;

use compass_semantic_diff::{
    Compatibility, FindingType, SemanticDiffError, SemanticDiffReport, SemanticFinding,
    VerificationState,
};

pub(crate) struct RenderOptions<'a> {
    pub include_routine: bool,
    pub explain: Option<&'a str>,
}

pub(crate) fn render_text(
    report: &SemanticDiffReport,
    options: &RenderOptions<'_>,
) -> Result<String, SemanticDiffError> {
    if let Some(id) = options.explain {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.id == id)
            .ok_or_else(|| SemanticDiffError::FindingNotFound(id.to_owned()))?;
        return Ok(render_finding_detail(finding));
    }
    let collapsed = report
        .collapsed_groups
        .iter()
        .flat_map(|group| &group.finding_ids)
        .collect::<BTreeSet<_>>();
    let visible = report
        .findings
        .iter()
        .filter(|finding| options.include_routine || !collapsed.contains(&finding.id))
        .collect::<Vec<_>>();
    let breaks = visible
        .iter()
        .filter(|finding| {
            matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            )
        })
        .count();
    let behaviors = visible
        .iter()
        .filter(|finding| finding.finding_type == FindingType::BehaviorChange)
        .count();
    let consumers = visible
        .iter()
        .map(|finding| finding.affected_consumers.len())
        .sum::<usize>();
    let gaps = visible
        .iter()
        .filter(|finding| finding.verification.state == VerificationState::Gap)
        .count();
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Semantic review: {} -> {}",
        short_revision(&report.comparison.old_commit),
        short_revision(&report.comparison.new_commit)
    );
    let _ = writeln!(
        output,
        "{breaks} likely breaks · {behaviors} behavior changes · {consumers} affected consumers · {gaps} test gaps"
    );
    render_section(
        &mut output,
        "Likely breaks",
        visible.iter().copied().filter(|finding| {
            matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            )
        }),
    );
    render_section(
        &mut output,
        "Behavior and dependency changes",
        visible.iter().copied().filter(|finding| {
            matches!(
                finding.finding_type,
                FindingType::BehaviorChange | FindingType::DependencyChange
            ) && !matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            )
        }),
    );
    render_section(
        &mut output,
        "Other semantic changes",
        visible.iter().copied().filter(|finding| {
            !matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            ) && !matches!(
                finding.finding_type,
                FindingType::BehaviorChange
                    | FindingType::DependencyChange
                    | FindingType::VerificationGap
            )
        }),
    );
    if !options.include_routine {
        let count = report
            .collapsed_groups
            .iter()
            .map(|group| group.count)
            .sum::<usize>();
        if count > 0 {
            let _ = writeln!(
                output,
                "\nRoutine changes collapsed: {count} (use --all to expand)"
            );
        }
    }
    if !report.limitations.is_empty() {
        output.push_str("\nLimitations\n");
        for limitation in &report.limitations {
            let _ = writeln!(output, "  - {limitation}");
        }
    }
    Ok(output.trim_end().to_owned())
}

pub(crate) fn render_json(
    report: &SemanticDiffReport,
    options: &RenderOptions<'_>,
) -> Result<String, SemanticDiffError> {
    let value = if let Some(id) = options.explain {
        serde_json::to_value(
            report
                .findings
                .iter()
                .find(|finding| finding.id == id)
                .ok_or_else(|| SemanticDiffError::FindingNotFound(id.to_owned()))?,
        )?
    } else {
        serde_json::to_value(report)?
    };
    Ok(serde_json::to_string_pretty(&value)?)
}

fn render_section<'a>(
    output: &mut String,
    title: &str,
    findings: impl Iterator<Item = &'a SemanticFinding>,
) {
    let findings = findings.collect::<Vec<_>>();
    if findings.is_empty() {
        return;
    }
    let _ = write!(output, "\n{title}\n");
    let mut shown = 0_usize;
    for finding in findings {
        if shown >= 20 && finding.compatibility != Compatibility::ProvenBreak {
            continue;
        }
        shown += 1;
        let _ = writeln!(
            output,
            "  [{} / {}] {} ({})",
            compatibility_name(finding.compatibility),
            confidence_name(finding.confidence),
            finding.headline,
            finding.id
        );
        let _ = writeln!(output, "    {}", finding.explanation);
        if !finding.affected_consumers.is_empty() {
            let names = finding
                .affected_consumers
                .iter()
                .take(5)
                .map(|consumer| consumer.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                output,
                "    Affected: {names}{}",
                if finding.affected_consumers.len() > 5 {
                    " …"
                } else {
                    ""
                }
            );
        }
        let _ = writeln!(output, "    Review: {}", finding.reviewer_action);
    }
}

fn render_finding_detail(finding: &SemanticFinding) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "{} ({})", finding.headline, finding.id);
    let _ = writeln!(
        output,
        "Classification: {} / {}",
        compatibility_name(finding.compatibility),
        confidence_name(finding.confidence)
    );
    let _ = writeln!(output, "What changed: {}", finding.explanation);
    let _ = writeln!(output, "Reviewer action: {}", finding.reviewer_action);
    let _ = writeln!(
        output,
        "Verification: {} — {}",
        verification_name(finding.verification.state),
        finding.verification.reason
    );
    if !finding.affected_consumers.is_empty() {
        output.push_str("Affected consumers:\n");
        for consumer in &finding.affected_consumers {
            let _ = writeln!(
                output,
                "  - {} (distance {})",
                consumer.display_name, consumer.distance
            );
        }
    }
    if !finding.evidence.is_empty() {
        output.push_str("Evidence:\n");
        for evidence in &finding.evidence {
            let _ = writeln!(
                output,
                "  - {} {}..{} [{}]",
                evidence.source_file,
                evidence.start_byte.unwrap_or(0),
                evidence.end_byte.unwrap_or(0),
                evidence.capability
            );
        }
    }
    output.trim_end().to_owned()
}

fn compatibility_name(compatibility: Compatibility) -> &'static str {
    match compatibility {
        Compatibility::ProvenBreak => "proven break",
        Compatibility::PossibleBreak => "possible break",
        Compatibility::Compatible => "compatible",
        Compatibility::Behavioral => "behavioral",
        Compatibility::NotApplicable => "not applicable",
        Compatibility::Indeterminate => "indeterminate",
    }
}

fn confidence_name(confidence: compass_semantic_diff::Confidence) -> &'static str {
    match confidence {
        compass_semantic_diff::Confidence::Exact => "exact",
        compass_semantic_diff::Confidence::Probable => "probable",
        compass_semantic_diff::Confidence::Inferred => "inferred",
        compass_semantic_diff::Confidence::Unknown => "unknown",
    }
}

fn verification_name(state: VerificationState) -> &'static str {
    match state {
        VerificationState::Unknown => "unknown",
        VerificationState::Covered => "covered",
        VerificationState::Gap => "gap",
        VerificationState::Partial => "partial",
        VerificationState::Stale => "stale",
        VerificationState::Failing => "failing",
        VerificationState::NotRun => "not run",
    }
}

fn short_revision(revision: &str) -> &str {
    revision.get(..revision.len().min(12)).unwrap_or(revision)
}
