use std::collections::BTreeSet;
use std::fmt::Write;

use compass_semantic_diff::{
    Compatibility, FindingType, SemanticDiffError, SemanticDiffReport, SemanticFinding,
    VerificationState,
};

pub(crate) struct RenderOptions<'a> {
    pub include_routine: bool,
    pub max_findings_per_section: Option<usize>,
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
    let public_changes = visible
        .iter()
        .filter(|finding| finding.public_surface)
        .count();
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
    let test_mapping = report
        .completeness
        .get("test_mapping")
        .copied()
        .unwrap_or(compass_semantic_diff::Completeness::Unavailable);
    let call_resolution = report
        .completeness
        .get("call_resolution")
        .copied()
        .unwrap_or(compass_semantic_diff::Completeness::Unavailable);
    let consumer_summary = if call_resolution == compass_semantic_diff::Completeness::Complete {
        format!("{consumers} affected consumers")
    } else {
        format!(
            "{consumers} resolved affected consumers · call mapping {}",
            completeness_name(call_resolution)
        )
    };
    let gap_summary = if test_mapping == compass_semantic_diff::Completeness::Complete {
        format!("{gaps} test gaps")
    } else {
        format!(
            "{gaps} proven test gaps · test mapping {}",
            completeness_name(test_mapping)
        )
    };
    let _ = writeln!(
        output,
        "{breaks} likely breaks · {public_changes} public-surface changes · {behaviors} behavior changes · {consumer_summary} · {gap_summary}"
    );
    if !report.feature_groups.is_empty() {
        output.push_str("\nFeature-level changes\n");
        let group_limit = if options.include_routine {
            report.feature_groups.len()
        } else {
            5
        };
        for group in report.feature_groups.iter().take(group_limit) {
            let _ = writeln!(output, "  {} ({})", group.headline, group.id);
            let _ = writeln!(output, "    {}", group.summary);
        }
        if report.feature_groups.len() > group_limit {
            let _ = writeln!(
                output,
                "  … {} more feature groups",
                report.feature_groups.len() - group_limit
            );
        }
    }
    render_section(
        &mut output,
        "Public API changes",
        visible
            .iter()
            .copied()
            .filter(|finding| finding.public_surface),
        options.max_findings_per_section,
    );
    render_section(
        &mut output,
        "Likely breaks",
        visible.iter().copied().filter(|finding| {
            matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            ) && !finding.public_surface
        }),
        options.max_findings_per_section,
    );
    render_section(
        &mut output,
        "Behavior and dependency changes",
        visible.iter().copied().filter(|finding| {
            matches!(
                finding.finding_type,
                FindingType::BehaviorChange | FindingType::DependencyChange
            ) && !finding.public_surface
                && !matches!(
                    finding.compatibility,
                    Compatibility::ProvenBreak | Compatibility::PossibleBreak
                )
        }),
        options.max_findings_per_section,
    );
    render_section(
        &mut output,
        "Other semantic changes",
        visible.iter().copied().filter(|finding| {
            !matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            ) && !finding.public_surface
                && !matches!(
                    finding.finding_type,
                    FindingType::BehaviorChange
                        | FindingType::DependencyChange
                        | FindingType::VerificationGap
                )
        }),
        options.max_findings_per_section,
    );
    if !options.include_routine {
        let count = report
            .collapsed_groups
            .iter()
            .map(|group| group.count)
            .sum::<usize>();
        if count > 0 {
            let detail = report
                .collapsed_groups
                .iter()
                .map(|group| format!("{} {}", group.count, group.label))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                output,
                "\nRoutine changes collapsed: {count} ({detail}; use --all to expand)"
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
    limit: Option<usize>,
) {
    let findings = findings.collect::<Vec<_>>();
    if findings.is_empty() {
        return;
    }
    let _ = write!(output, "\n{title}\n");
    let mut shown = 0_usize;
    let total = findings.len();
    for finding in findings {
        if limit.is_some_and(|limit| shown >= limit)
            && finding.compatibility != Compatibility::ProvenBreak
        {
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
    let hidden = total.saturating_sub(shown);
    if hidden > 0 {
        let _ = writeln!(
            output,
            "  … {hidden} more findings (use --limit {} or --all)",
            shown.saturating_add(hidden)
        );
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

fn completeness_name(completeness: compass_semantic_diff::Completeness) -> &'static str {
    match completeness {
        compass_semantic_diff::Completeness::Complete => "complete",
        compass_semantic_diff::Completeness::Partial => "partial",
        compass_semantic_diff::Completeness::Unavailable => "unavailable",
    }
}

fn short_revision(revision: &str) -> &str {
    revision.get(..revision.len().min(12)).unwrap_or(revision)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use compass_semantic_diff::{
        Compatibility, Confidence, FindingOrigin, FindingType, SemanticFinding, Verification,
        VerificationState,
    };

    use super::render_section;

    fn finding(index: usize, compatibility: Compatibility) -> SemanticFinding {
        SemanticFinding {
            id: format!("sd2-{index:024x}"),
            finding_type: FindingType::BehaviorChange,
            subject: format!("subject-{index}"),
            origin: FindingOrigin::Direct,
            headline: format!("finding {index}"),
            explanation: "changed".to_owned(),
            compatibility,
            confidence: Confidence::Exact,
            review_priority: 1,
            public_surface: false,
            routine: false,
            before: None,
            after: None,
            affected_consumers: Vec::new(),
            witness_paths: Vec::new(),
            verification: Verification {
                state: VerificationState::Unknown,
                exact_tests: Vec::new(),
                recommended_tests: Vec::new(),
                reason: "unavailable".to_owned(),
            },
            reviewer_action: "review".to_owned(),
            evidence: Vec::new(),
            completeness: BTreeMap::new(),
        }
    }

    #[test]
    fn section_limits_are_explicit_and_unlimited_is_exhaustive() {
        let findings = (0..23)
            .map(|index| finding(index, Compatibility::Behavioral))
            .collect::<Vec<_>>();
        let mut limited = String::new();
        render_section(&mut limited, "Changes", findings.iter(), Some(20));
        assert!(limited.contains("… 3 more findings (use --limit 23 or --all)"));
        assert!(!limited.contains("finding 22"));

        let mut exhaustive = String::new();
        render_section(&mut exhaustive, "Changes", findings.iter(), None);
        assert!(exhaustive.contains("finding 22"));
        assert!(!exhaustive.contains("more findings"));
    }

    #[test]
    fn limits_never_hide_proven_breaks() {
        let findings = [
            finding(0, Compatibility::Behavioral),
            finding(1, Compatibility::Behavioral),
            finding(2, Compatibility::ProvenBreak),
        ];
        let mut output = String::new();
        render_section(&mut output, "Changes", findings.iter(), Some(1));
        assert!(output.contains("finding 0"));
        assert!(output.contains("finding 2"));
        assert!(output.contains("… 1 more finding"));
    }
}
