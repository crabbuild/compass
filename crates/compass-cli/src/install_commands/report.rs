use super::model::{InstallReport, InstallStatus, OutputFormat};

pub(super) fn render_report(
    report: &InstallReport,
    format: OutputFormat,
) -> Result<String, String> {
    if format == OutputFormat::Json {
        return serde_json::to_string_pretty(report)
            .map_err(|error| format!("error: could not encode install report: {error}"));
    }
    let mut lines = vec![
        format!(
            "Compass install ({:?} scope: {})",
            report.scope,
            report.root.display()
        ),
        String::new(),
        format!("Selected: {}", report.selected.join(", ")),
        String::new(),
    ];
    if !report.detected.is_empty() {
        lines.push("Detected:".to_owned());
        for (agent, evidence) in &report.detected {
            lines.push(format!("  {agent}: {}", evidence.join(", ")));
        }
        lines.push(String::new());
    }
    for result in &report.results {
        let status = match result.status {
            InstallStatus::Installed => "installed",
            InstallStatus::Updated => "updated",
            InstallStatus::Current => "current",
            InstallStatus::Skipped => "skipped",
            InstallStatus::Failed => "failed",
        };
        lines.push(format!(
            "{status:>9}  {}  [{}]",
            result.id,
            result
                .consumers
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for path in &result.paths {
            lines.push(format!("           {}", path.display()));
        }
        if let Some(reason) = &result.reason {
            lines.push(format!("           {reason}"));
        }
        if let Some(rollback) = &result.rollback {
            lines.push(format!("           rollback: {rollback}"));
        }
    }
    if !report.next_actions.is_empty() {
        lines.push(String::new());
        lines.push("Next:".to_owned());
        lines.extend(
            report
                .next_actions
                .iter()
                .map(|action| format!("  {action}")),
        );
    }
    Ok(lines.join("\n"))
}
