use std::collections::BTreeSet;
use std::fmt::Write;

use compass_history::{SourceFileDelta, SourceFileStatus};
use compass_semantic_diff::{
    Compatibility, FindingType, GraphDelta, GraphEdgeDelta, GraphNodeDelta, SemanticDiffError,
    SemanticDiffReport, SemanticFinding, VerificationState,
};

const PIERRE_DIFFS_JS: &str = include_str!("../assets/vendor/pierre-diffs-v1.2.12.js");
const SEMANTIC_DIFF_GRAPH_CSS: &str = include_str!("../assets/semantic-diff-graph.css");
const SEMANTIC_DIFF_GRAPH_JS: &str = include_str!("../assets/semantic-diff-graph.js");

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

pub(crate) fn render_html(
    report: &SemanticDiffReport,
    options: &RenderOptions<'_>,
) -> Result<String, SemanticDiffError> {
    let findings = if let Some(id) = options.explain {
        vec![
            report
                .findings
                .iter()
                .find(|finding| finding.id == id)
                .ok_or_else(|| SemanticDiffError::FindingNotFound(id.to_owned()))?,
        ]
    } else {
        report.findings.iter().collect::<Vec<_>>()
    };
    let collapsed = report
        .collapsed_groups
        .iter()
        .flat_map(|group| &group.finding_ids)
        .collect::<BTreeSet<_>>();
    let breaks = findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.compatibility,
                Compatibility::ProvenBreak | Compatibility::PossibleBreak
            )
        })
        .count();
    let public_changes = findings
        .iter()
        .filter(|finding| finding.public_surface)
        .count();
    let behavior_changes = findings
        .iter()
        .filter(|finding| finding.finding_type == FindingType::BehaviorChange)
        .count();
    let affected_consumers = findings
        .iter()
        .map(|finding| finding.affected_consumers.len())
        .sum::<usize>();
    let test_gaps = findings
        .iter()
        .filter(|finding| finding.verification.state == VerificationState::Gap)
        .count();
    let embedded_report = serde_json::to_string(report)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");

    let mut output = String::new();
    output.push_str(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Compass semantic diff</title>
<style>
:root{color-scheme:dark;--canvas:#0e1116;--surface:#141820;--surface-raised:#191e27;--surface-inset:#0b0e13;--border:#2b313b;--border-strong:#3a424e;--text:#e7eaf0;--text-soft:#c5cad3;--muted:#8d96a5;--accent:#8ab4f8;--red:#ff7b86;--amber:#d9a441;--green:#65bd84}
*{box-sizing:border-box}
body{margin:0;background:var(--canvas);color:var(--text);font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Helvetica,Arial,sans-serif;-webkit-font-smoothing:antialiased}
button,input,select{font:inherit}
main{width:min(1120px,calc(100% - 48px));margin:0 auto;padding:48px 0 72px}
.report-header{padding-bottom:24px;border-bottom:1px solid var(--border)}
.title-row{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}
h1{margin:0;font-size:30px;line-height:1.2;letter-spacing:-.025em;font-weight:650}
.dek{margin:7px 0 0;color:var(--muted);max-width:680px}
.schema{color:var(--muted);font:11px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;white-space:nowrap;margin-top:7px}
.comparison{display:flex;align-items:center;gap:9px;margin-top:20px;color:var(--muted);flex-wrap:wrap;font-size:12px}
.comparison code,.evidence code{background:var(--surface-inset);border:1px solid var(--border);border-radius:4px;padding:3px 6px;color:var(--text-soft);font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.metrics{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));margin:0 0 32px;border-bottom:1px solid var(--border)}
.metric{padding:18px 18px 18px 0}
.metric+.metric{border-left:1px solid var(--border);padding-left:18px}
.metric strong{display:block;font-size:20px;line-height:1.2;font-weight:620;font-variant-numeric:tabular-nums}
.metric span{display:block;margin-top:4px;color:var(--muted);font-size:11px;line-height:1.35}
.panel{margin:28px 0;padding:0}
.panel h2,.section-heading h2{margin:0;font-size:15px;line-height:1.4;font-weight:620;letter-spacing:-.01em}
.panel>h2{margin-bottom:12px}
.completeness{display:flex;gap:6px;flex-wrap:wrap}
.pill,.badge{display:inline-flex;align-items:center;border:1px solid var(--border);border-radius:4px;padding:2px 6px;font-size:10px;line-height:1.5;font-weight:600;color:var(--muted)}
.pill span{color:var(--muted);margin-right:5px;font-weight:500}
.complete{border-color:#315b40;color:var(--green)}
.partial{border-color:#5d4b27;color:var(--amber)}
.unavailable{border-color:#63343a;color:var(--red)}
.feature-grid{border-top:1px solid var(--border)}
.feature{display:grid;grid-template-columns:minmax(260px,.8fr) minmax(320px,1.2fr);gap:24px;padding:16px 0;border-bottom:1px solid var(--border)}
.feature h3{font-size:13px;line-height:1.45;font-weight:600;margin:0}
.feature p{color:var(--text-soft);margin:0}
.files{grid-column:2;color:var(--muted);font:11px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;margin-top:-16px}
.report-nav{display:flex;gap:24px;border-bottom:1px solid var(--border);margin:0 0 30px}
.report-nav a{display:inline-flex;gap:7px;align-items:baseline;color:var(--muted);text-decoration:none;padding:0 0 10px;border-bottom:2px solid transparent;font-size:12px}
.report-nav a:hover,.report-nav a:focus-visible{color:var(--text);border-bottom-color:var(--border-strong);outline:none}
.report-nav strong{color:var(--text-soft);font-weight:600}
.report-nav span{font-variant-numeric:tabular-nums}
.section-anchor{scroll-margin-top:58px}
.section-heading{display:flex;align-items:baseline;justify-content:space-between;gap:16px;margin:36px 0 10px}
.toolbar{position:sticky;top:0;z-index:4;display:grid;grid-template-columns:minmax(240px,1fr) 180px auto auto auto auto;gap:8px;align-items:center;background:rgba(14,17,22,.96);backdrop-filter:blur(10px);border-top:1px solid var(--border);border-bottom:1px solid var(--border);padding:10px 0;margin-bottom:8px}
input,select,button{min-height:34px;border:1px solid var(--border);border-radius:5px;background:var(--surface-inset);color:var(--text);padding:6px 9px}
input::placeholder{color:#747d8b}
input:focus,select:focus,button:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
button{cursor:pointer;background:var(--surface)}
button:hover{background:var(--surface-raised);border-color:var(--border-strong)}
label.toggle{display:flex;align-items:center;gap:7px;white-space:nowrap;color:var(--text-soft);font-size:12px}
input[type=checkbox]{min-height:auto;accent-color:var(--accent)}
.shown{color:var(--muted);font-size:11px;white-space:nowrap;text-align:right}
.finding{background:var(--surface);border:1px solid var(--border);border-radius:6px;margin:7px 0;overflow:hidden}
.finding summary{position:relative;cursor:pointer;list-style:none;padding:13px 42px 13px 15px}
.finding summary::-webkit-details-marker{display:none}
.finding summary:after{content:"›";position:absolute;top:13px;right:15px;color:var(--muted);font-size:20px;line-height:1;transform:rotate(90deg);transition:transform .14s ease}
.finding[open] summary:after{transform:rotate(-90deg)}
.finding[open] summary{background:var(--surface-raised)}
.headline{font-weight:600;font-size:13px;line-height:1.45;margin-right:30px}
.meta{display:flex;gap:5px;flex-wrap:wrap;margin-top:7px}
.badge.break{color:var(--red);border-color:#63343a}
.badge.behavioral{color:var(--accent);border-color:#344d70}
.badge.compatible{color:var(--green);border-color:#315b40}
.badge.unknown{color:var(--muted)}
.detail{display:grid;grid-template-columns:132px minmax(0,1fr);column-gap:24px;row-gap:13px;border-top:1px solid var(--border);padding:18px 16px 20px}
.detail h4{grid-column:1;margin:0;color:var(--muted);font-size:10px;line-height:1.5;font-weight:650;letter-spacing:.07em;text-transform:uppercase}
.detail>p,.detail>ul,.detail>pre{grid-column:2;margin:0}
.detail p{color:var(--text-soft)}
.action{padding-left:10px;border-left:2px solid var(--border-strong)}
.detail ul{padding-left:18px;color:var(--text-soft)}
.detail li+li{margin-top:5px}
.evidence code{font-size:11px}
pre{white-space:pre-wrap;overflow-wrap:anywhere;background:var(--surface-inset);border:1px solid var(--border);border-radius:4px;padding:10px 12px;color:var(--text-soft);font:11px/1.55 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.source-file{border-top:1px solid var(--border)}
.source-file:last-child{border-bottom:1px solid var(--border)}
.source-file summary{position:relative;cursor:pointer;list-style:none;padding:13px 36px 13px 0}
.source-file summary::-webkit-details-marker{display:none}
.source-file summary:after{content:"›";position:absolute;top:12px;right:4px;color:var(--muted);font-size:20px;line-height:1;transform:rotate(90deg)}
.source-file[open] summary:after{transform:rotate(-90deg)}
.source-summary{display:flex;justify-content:space-between;gap:20px;align-items:baseline}
.source-path{color:var(--text);font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;overflow-wrap:anywhere}
.source-meta{color:var(--muted);font-size:11px;white-space:nowrap}
.code-heading{align-items:center}
.code-heading-copy{display:flex;align-items:baseline;gap:10px}
.code-controls{display:flex;align-items:center;gap:8px}
.segmented{display:inline-flex;border:1px solid var(--border);border-radius:5px;overflow:hidden;background:var(--surface-inset)}
.segmented button{min-height:30px;border:0;border-radius:0;background:transparent;color:var(--muted);padding:4px 10px;font-size:11px}
.segmented button+button{border-left:1px solid var(--border)}
.segmented button[aria-pressed=true]{background:var(--surface-raised);color:var(--text)}
.code-controls .toggle{min-height:30px;padding:0 8px;border:1px solid var(--border);border-radius:5px;background:var(--surface-inset)}
.diff-renderer{margin:0 0 18px;border:1px solid var(--border);border-radius:4px;overflow:hidden;background:var(--surface-inset)}
.diff-renderer:empty{display:none}
.source-patch{margin:0 0 18px;padding:0;overflow:auto;white-space:pre;background:var(--surface-inset);border:1px solid var(--border);border-radius:4px;color:var(--text-soft);font:11px/1.55 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.source-patch[hidden]{display:none}
.diff-fallback-note{margin:0 0 8px;color:var(--amber);font-size:11px}
.diff-line{display:block;min-height:1.55em;padding:0 12px}
.diff-line.add{background:rgba(48,112,72,.18);color:#a9d9b7}
.diff-line.remove{background:rgba(139,57,65,.18);color:#e4a7ad}
.diff-line.hunk{background:rgba(69,96,137,.18);color:#a9c4ed}
.diff-line.meta{color:var(--muted)}
.graph-summary{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));border-top:1px solid var(--border);border-bottom:1px solid var(--border);margin-bottom:16px}
.graph-stat{padding:13px 12px 13px 0}
.graph-stat+.graph-stat{border-left:1px solid var(--border);padding-left:12px}
.graph-stat strong{display:block;font-size:17px;font-weight:620;font-variant-numeric:tabular-nums}
.graph-stat span{display:block;color:var(--muted);font-size:10px;margin-top:2px}
.graph-canvas{min-height:340px;border:1px solid var(--border);border-radius:4px;background:var(--surface-inset);overflow:hidden}
.graph-canvas svg{display:block;width:100%;height:420px}
.graph-edge{stroke:var(--border-strong);stroke-width:1.2;opacity:.78}
.graph-edge.added{stroke:var(--green)}.graph-edge.removed{stroke:var(--red);stroke-dasharray:5 4}.graph-edge.changed{stroke:var(--amber);stroke-dasharray:2 3}
.graph-arrow.context{fill:var(--border-strong)}.graph-arrow.added{fill:var(--green)}.graph-arrow.removed{fill:var(--red)}.graph-arrow.changed{fill:var(--amber)}
.graph-node circle{fill:var(--surface-raised);stroke:var(--muted);stroke-width:1.5}
.graph-node.added circle{stroke:var(--green)}.graph-node.removed circle{stroke:var(--red);stroke-dasharray:3 2}.graph-node.changed circle{stroke:var(--amber)}
.graph-node text{fill:var(--text-soft);font:10px ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;paint-order:stroke;stroke:var(--surface-inset);stroke-width:3px;stroke-linejoin:round}
.graph-empty{display:flex;align-items:center;justify-content:center;min-height:340px;color:var(--muted);font-size:12px}
.graph-note{color:var(--muted);font-size:11px;margin:8px 0 0}
.graph-legend{display:flex;gap:14px;flex-wrap:wrap;margin:10px 0;color:var(--muted);font-size:10px}
.graph-legend span:before{display:inline-block;width:7px;height:7px;border-radius:50%;margin-right:5px;content:""}
.graph-legend .added:before{background:var(--green)}.graph-legend .removed:before{background:var(--red)}.graph-legend .changed:before{background:var(--amber)}.graph-legend .context:before{background:var(--muted)}
.delta-grid{display:grid;grid-template-columns:1fr 1fr;gap:24px;margin-top:20px}
.delta-column>h3{font-size:12px;margin:0 0 8px}
.delta-group{border-top:1px solid var(--border)}
.delta-group:last-child{border-bottom:1px solid var(--border)}
.delta-group summary{cursor:pointer;list-style:none;padding:9px 0;color:var(--text-soft);font-size:11px}
.delta-group summary::-webkit-details-marker{display:none}
.delta-count{color:var(--muted);font-variant-numeric:tabular-nums}
.delta-list{list-style:none!important;padding:0!important;margin:0 0 10px!important}
.delta-row{display:grid;grid-template-columns:18px minmax(0,1fr);gap:5px;padding:6px 0;border-top:1px solid rgba(43,49,59,.55);font-size:11px}
.delta-mark{font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.delta-mark.added{color:var(--green)}.delta-mark.removed{color:var(--red)}.delta-mark.changed{color:var(--amber)}
.delta-primary{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;overflow-wrap:anywhere}
.delta-secondary{grid-column:2;color:var(--muted);font-size:10px;overflow-wrap:anywhere}
.churn{margin-top:14px;color:var(--muted);font-size:11px}
.empty{display:none;text-align:center;color:var(--muted);padding:32px;border-bottom:1px solid var(--border)}
.limitations{padding:16px;border-left:2px solid var(--amber);background:var(--surface)}
.limitations li{margin:5px 0}
footer{color:var(--muted);border-top:1px solid var(--border);margin-top:40px;padding-top:16px;font-size:11px}
@media(max-width:900px){main{width:min(100% - 32px,1120px)}.metrics{grid-template-columns:repeat(3,1fr)}.metric:nth-child(4){border-left:0;padding-left:0}.feature{grid-template-columns:1fr}.files{grid-column:1;margin-top:-16px}.toolbar{position:static;grid-template-columns:1fr 180px auto auto}.shown{grid-column:1/-1;text-align:left}.detail{grid-template-columns:112px minmax(0,1fr)}.graph-summary{grid-template-columns:repeat(3,1fr)}.graph-stat:nth-child(4){border-left:0;padding-left:0}}
@media(max-width:620px){main{width:min(100% - 24px,1120px);padding-top:28px}.title-row{display:block}.schema{margin-top:12px}.metrics{grid-template-columns:1fr 1fr}.metric:nth-child(odd){border-left:0;padding-left:0}.metric:nth-child(even){border-left:1px solid var(--border);padding-left:14px}.report-nav{gap:16px}.toolbar{grid-template-columns:1fr 1fr}.toolbar input{grid-column:1/-1}.toolbar label{grid-column:1/-1}.detail{display:block;padding:16px}.detail h4{margin:18px 0 5px}.detail h4:first-child{margin-top:0}.detail>p,.detail>ul,.detail>pre{margin:0}.feature{gap:7px}.files{margin-top:0}.code-heading{display:block}.code-heading-copy{margin-bottom:9px}.code-controls{justify-content:space-between}.source-summary{display:block}.source-meta{display:block;margin-top:4px}.graph-summary{grid-template-columns:1fr 1fr}.graph-stat:nth-child(odd){border-left:0;padding-left:0}.graph-stat:nth-child(even){border-left:1px solid var(--border);padding-left:12px}.delta-grid{grid-template-columns:1fr}.graph-canvas svg{height:360px}}
@media(prefers-reduced-motion:reduce){.finding summary:after{transition:none}}
"#,
    );
    output.push_str(SEMANTIC_DIFF_GRAPH_CSS);
    output.push_str(
        r#"</style>
</head>
<body>
<main>
<header class="report-header">
<div class="title-row"><div><h1>Semantic diff</h1><p class="dek">Review behavior, compatibility, affected code, and verification evidence.</p></div><div class="schema">"#,
    );
    output.push_str(&html_escape(&report.schema));
    output.push_str("</div></div><div class=\"comparison\"><span>Base</span><code>");
    output.push_str(&html_escape(short_revision(&report.comparison.old_commit)));
    output.push_str("</code><span>→</span><span>Target</span><code>");
    output.push_str(&html_escape(short_revision(&report.comparison.new_commit)));
    output.push_str("</code><span>·</span><span>Profile ");
    output.push_str(&html_escape(short_revision(&report.comparison.fingerprint)));
    output.push_str("</span></div></header><section class=\"metrics\">");
    for (value, label) in [
        (breaks, "likely breaks"),
        (public_changes, "public-surface changes"),
        (behavior_changes, "behavior changes"),
        (affected_consumers, "resolved affected consumers"),
        (test_gaps, "proven test gaps"),
    ] {
        let _ = write!(
            output,
            "<div class=\"metric\"><strong>{value}</strong><span>{}</span></div>",
            html_escape(label)
        );
    }
    output.push_str("</section>");
    let graph_change_count = graph_change_count(&report.graph_delta);
    let _ = write!(
        output,
        "<nav class=\"report-nav\" aria-label=\"Report sections\"><a href=\"#review\"><strong>Review</strong><span>{}</span></a><a href=\"#code\"><strong>Code</strong><span>{}</span></a><a href=\"#graph\"><strong>Graph</strong><span>{graph_change_count}</span></a></nav>",
        findings.len(),
        report.source_changes.len()
    );

    if !report.completeness.is_empty() {
        output.push_str(
            "<section class=\"panel\"><h2>Analysis completeness</h2><div class=\"completeness\">",
        );
        for (capability, completeness) in &report.completeness {
            let name = completeness_name(*completeness);
            let _ = write!(
                output,
                "<span class=\"pill {}\"><span>{}</span>{}</span>",
                html_attr(name),
                html_escape(&capability.replace('_', " ")),
                html_escape(name)
            );
        }
        output.push_str("</div></section>");
    }

    if options.explain.is_none() && !report.feature_groups.is_empty() {
        output.push_str(
            "<section class=\"panel\"><h2>Feature-level changes</h2><div class=\"feature-grid\">",
        );
        for group in &report.feature_groups {
            let _ = write!(
                output,
                "<article class=\"feature\" id=\"{}\"><h3>{}</h3><p>{}</p>",
                html_attr(&group.id),
                html_escape(&group.headline),
                html_escape(&group.summary)
            );
            if !group.source_files.is_empty() {
                let _ = write!(
                    output,
                    "<div class=\"files\">{}</div>",
                    html_escape(&group.source_files.join(" · "))
                );
            }
            output.push_str("</article>");
        }
        output.push_str("</div></section>");
    }

    let checked = if options.include_routine {
        " checked"
    } else {
        ""
    };
    let _ = write!(
        output,
        "<section id=\"review\" class=\"section-anchor\" aria-labelledby=\"findings-heading\"><div class=\"section-heading\"><h2 id=\"findings-heading\">Findings</h2></div><div class=\"toolbar\"><input id=\"search\" type=\"search\" placeholder=\"Search findings, files, or symbols\" aria-label=\"Search findings\"><select id=\"type-filter\" aria-label=\"Filter finding type\"><option value=\"\">All change types</option><option value=\"contract_change\">Contract</option><option value=\"behavior_change\">Behavior</option><option value=\"dependency_change\">Dependency</option><option value=\"impact_change\">Impact</option><option value=\"verification_gap\">Verification gap</option><option value=\"structural_change\">Structural</option></select><label class=\"toggle\"><input id=\"routine\" type=\"checkbox\"{checked}> Show routine churn</label><button id=\"expand\" type=\"button\">Expand all</button><button id=\"collapse\" type=\"button\">Collapse all</button><span class=\"shown\" id=\"shown\"></span></div><div id=\"finding-list\">"
    );

    for finding in findings {
        render_html_finding(&mut output, finding, collapsed.contains(&finding.id));
    }
    output.push_str(
        "</div><div class=\"empty\" id=\"empty\">No findings match these filters.</div></section>",
    );
    render_source_changes(&mut output, &report.source_changes);
    render_graph_delta(&mut output, &report.graph_delta);

    if !report.limitations.is_empty() {
        output.push_str("<section class=\"panel limitations\"><h2>Limitations</h2><ul>");
        for limitation in &report.limitations {
            let _ = write!(output, "<li>{}</li>", html_escape(limitation));
        }
        output.push_str("</ul></section>");
    }
    if !report.collapsed_groups.is_empty() {
        output.push_str("<section class=\"panel\"><h2>Routine-change groups</h2><ul>");
        for group in &report.collapsed_groups {
            let _ = write!(
                output,
                "<li><strong>{}</strong> {}</li>",
                group.count,
                html_escape(&group.label)
            );
        }
        output.push_str("</ul></section>");
    }

    let _ = write!(
        output,
        "<script type=\"application/json\" id=\"semantic-diff-data\">{embedded_report}</script>"
    );
    output.push_str("<script>");
    output.push_str(&PIERRE_DIFFS_JS.replace("</script", "<\\/script"));
    output.push_str("</script>");
    output.push_str("<script>");
    output.push_str(&SEMANTIC_DIFF_GRAPH_JS.replace("</script", "<\\/script"));
    output.push_str("</script>");
    output.push_str(
        r#"<script>
const cards=[...document.querySelectorAll(".finding")];
const search=document.getElementById("search");
const typeFilter=document.getElementById("type-filter");
const routine=document.getElementById("routine");
const shown=document.getElementById("shown");
const empty=document.getElementById("empty");
function applyFilters(){
  const query=search.value.trim().toLowerCase();
  const type=typeFilter.value;
  let count=0;
  for(const card of cards){
    const matchesText=!query||card.textContent.toLowerCase().includes(query);
    const matchesType=!type||card.dataset.type===type;
    const matchesRoutine=routine.checked||card.dataset.routine!=="true";
    card.hidden=!(matchesText&&matchesType&&matchesRoutine);
    if(!card.hidden)count++;
  }
  shown.textContent=`${count} of ${cards.length} findings`;
  empty.style.display=count===0?"block":"none";
}
search.addEventListener("input",applyFilters);
typeFilter.addEventListener("change",applyFilters);
routine.addEventListener("change",applyFilters);
document.getElementById("expand").addEventListener("click",()=>cards.filter(card=>!card.hidden).forEach(card=>card.open=true));
document.getElementById("collapse").addEventListener("click",()=>cards.forEach(card=>card.open=false));
applyFilters();

const reportData=JSON.parse(document.getElementById("semantic-diff-data").textContent);
const sourceDiffs=new Map();
const diffState={style:"unified",overflow:"scroll"};
const diffThemeCSS=`:host{
  --diffs-dark-bg:#0b0e13;
  --diffs-dark:#c5cad3;
  --diffs-font-size:11px;
  --diffs-line-height:19px;
  --diffs-added-dark:#65bd84;
  --diffs-deleted-dark:#ff7b86;
  --diffs-modified-dark:#8ab4f8;
  --diffs-bg-context-override:#10151d;
  --diffs-bg-context-gutter-override:#0d1118;
  --diffs-bg-separator-override:#171c24;
  --diffs-fg-number-override:#747d8b;
  --diffs-font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
}`;
function mountSourceDiff(index){
  if(sourceDiffs.has(index))return;
  const host=document.querySelector(`[data-diff-index="${index}"]`);
  const fallback=document.querySelector(`[data-diff-fallback="${index}"]`);
  const change=reportData.source_changes?.[index];
  if(!host||!fallback||!change?.patch||!globalThis.CompassDiffs)return;
  try{
    const files=CompassDiffs.parsePatchFiles(change.patch,`compass-source-${index}`,true)
      .flatMap(patch=>patch.files||[]);
    const fileDiff=files[0];
    if(!fileDiff)throw new Error("No text diff was parsed");
    fileDiff.lang="text";
    const instance=new CompassDiffs.FileDiff({
      theme:"compass-dark",
      themeType:"dark",
      diffStyle:diffState.style,
      diffIndicators:"classic",
      hunkSeparators:"metadata",
      lineDiffType:"word-alt",
      overflow:diffState.overflow,
      disableFileHeader:true,
      disableVirtualizationBuffers:true,
      unsafeCSS:diffThemeCSS
    });
    instance.render({fileDiff,containerWrapper:host});
    if(!host.firstElementChild)throw new Error("Diff renderer produced no content");
    fallback.hidden=true;
    sourceDiffs.set(index,instance);
  }catch(error){
    const note=document.createElement("p");
    note.className="diff-fallback-note";
    note.textContent="Enhanced diff unavailable; showing the exact Git patch.";
    host.replaceChildren(note);
    console.warn("Compass could not render an enhanced source diff",error);
  }
}
for(const details of document.querySelectorAll(".source-file[data-source-index]")){
  const index=Number(details.dataset.sourceIndex);
  if(details.open)mountSourceDiff(index);
  details.addEventListener("toggle",()=>{if(details.open)mountSourceDiff(index)});
}
for(const button of document.querySelectorAll("[data-diff-style]")){
  button.addEventListener("click",()=>{
    diffState.style=button.dataset.diffStyle;
    for(const candidate of document.querySelectorAll("[data-diff-style]")){
      candidate.setAttribute("aria-pressed",String(candidate===button));
    }
    for(const instance of sourceDiffs.values()){
      instance.setOptions({...instance.options,diffStyle:diffState.style});
      instance.rerender();
    }
  });
}
document.getElementById("diff-wrap")?.addEventListener("change",event=>{
  diffState.overflow=event.currentTarget.checked?"wrap":"scroll";
  for(const instance of sourceDiffs.values()){
    instance.setOptions({...instance.options,overflow:diffState.overflow});
    instance.rerender();
  }
});
globalThis.CompassSemanticDiffGraph.mount({
  report:reportData,
  host:document.getElementById("graph-canvas"),
  inspector:document.getElementById("graph-inspector"),
  liveRegion:document.getElementById("graph-live"),
  note:document.getElementById("graph-note")
});
</script>
<footer>Generated by Compass · Source diffs rendered with @pierre/diffs 1.2.12 · Embedded report data is available in <code>#semantic-diff-data</code>.</footer>
</main>
</body>
</html>
"#,
    );
    Ok(output)
}

fn render_source_changes(output: &mut String, changes: &[SourceFileDelta]) {
    output.push_str(
        "<section id=\"code\" class=\"section-anchor\" aria-labelledby=\"code-heading\"><div class=\"section-heading code-heading\"><div class=\"code-heading-copy\"><h2 id=\"code-heading\">Code changes</h2><span class=\"shown\">Exact Git patch</span></div>",
    );
    if changes.is_empty() {
        output.push_str(
            "</div><p class=\"graph-note\">No committed source-file changes.</p></section>",
        );
        return;
    }
    output.push_str(
        "<div class=\"code-controls\"><div class=\"segmented\" role=\"group\" aria-label=\"Diff layout\"><button type=\"button\" data-diff-style=\"unified\" aria-pressed=\"true\">Unified</button><button type=\"button\" data-diff-style=\"split\" aria-pressed=\"false\">Split</button></div><label class=\"toggle\"><input id=\"diff-wrap\" type=\"checkbox\"> Wrap lines</label></div></div>",
    );
    for (index, change) in changes.iter().enumerate() {
        let open = if index == 0 { " open" } else { "" };
        let path = source_change_path(change);
        let _ = write!(
            output,
            "<details id=\"source-change-{index}\" class=\"source-file\" data-source-index=\"{index}\"{open}><summary><div class=\"source-summary\"><span class=\"source-path\">{}</span><span class=\"source-meta\">{} · {} hunks</span></div></summary>",
            html_escape(&path),
            source_status_name(change.status),
            change.hunks.len()
        );
        if change.patch.is_empty() {
            output.push_str(
                "<p class=\"graph-note\">Git reported no textual patch for this file.</p>",
            );
        } else {
            let _ = write!(
                output,
                "<div class=\"diff-renderer\" data-diff-index=\"{index}\" aria-label=\"Rendered source diff\"></div><pre class=\"source-patch\" data-diff-fallback=\"{index}\" aria-label=\"Unified source patch\">"
            );
            for line in change.patch.lines() {
                let class = source_line_class(line);
                let _ = writeln!(
                    output,
                    "<span class=\"diff-line {class}\">{}</span>",
                    html_escape(line)
                );
            }
            output.push_str("</pre>");
        }
        output.push_str("</details>");
    }
    output.push_str("</section>");
}

fn render_graph_delta(output: &mut String, delta: &GraphDelta) {
    output.push_str(
        "<section id=\"graph\" class=\"section-anchor\" aria-labelledby=\"graph-heading\"><div class=\"section-heading\"><h2 id=\"graph-heading\">Graph changes</h2><span class=\"shown\">Meaningful topology and attributes</span></div>",
    );
    output.push_str("<div class=\"graph-summary\">");
    for (value, label) in [
        (delta.added_nodes.len(), "nodes added"),
        (delta.removed_nodes.len(), "nodes removed"),
        (delta.changed_nodes.len(), "nodes changed"),
        (delta.added_edges.len(), "edges added"),
        (delta.removed_edges.len(), "edges removed"),
        (delta.changed_edges.len(), "edges changed"),
    ] {
        let _ = write!(
            output,
            "<div class=\"graph-stat\"><strong>{value}</strong><span>{label}</span></div>"
        );
    }
    output.push_str("</div>");
    if graph_change_count(delta) == 0 {
        output
            .push_str("<div class=\"graph-canvas graph-empty\">No meaningful graph changes.</div>");
    } else {
        output.push_str(
            "<div class=\"graph-explorer\"><div id=\"graph-canvas\" class=\"graph-canvas\" aria-label=\"Changed code graph\"></div><aside id=\"graph-inspector\" class=\"graph-inspector\" aria-labelledby=\"graph-inspector-heading\"><h3 id=\"graph-inspector-heading\" class=\"sr-only\">Node inspector</h3><p class=\"graph-inspector-empty\">Select a node to inspect its change.</p></aside></div><p id=\"graph-live\" class=\"sr-only\" aria-live=\"polite\"></p><div class=\"graph-legend\"><span class=\"added\">Added (+)</span><span class=\"removed\">Removed (−)</span><span class=\"changed\">Changed (~)</span><span class=\"context\">Context (·)</span></div><p class=\"graph-note\" id=\"graph-note\">The visualization focuses on the changed subgraph. The lists below and embedded JSON are exhaustive.</p>",
        );
    }
    output.push_str("<div class=\"delta-grid\"><div class=\"delta-column\"><h3>Nodes</h3>");
    render_node_delta_group(output, "Added", "added", "+", &delta.added_nodes);
    render_node_delta_group(output, "Removed", "removed", "−", &delta.removed_nodes);
    render_node_delta_group(output, "Changed", "changed", "~", &delta.changed_nodes);
    output.push_str("</div><div class=\"delta-column\"><h3>Edges</h3>");
    render_edge_delta_group(output, "Added", "added", "+", &delta.added_edges);
    render_edge_delta_group(output, "Removed", "removed", "−", &delta.removed_edges);
    render_edge_delta_group(output, "Changed", "changed", "~", &delta.changed_edges);
    output.push_str("</div></div>");
    if !delta.collapsed_attribute_changes.is_empty() {
        let collapsed = delta
            .collapsed_attribute_changes
            .iter()
            .map(|(field, count)| format!("{field} × {count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let _ = write!(
            output,
            "<p class=\"churn\">Collapsed non-semantic graph metadata churn: {}.</p>",
            html_escape(&collapsed)
        );
    }
    output.push_str("</section>");
}

fn render_node_delta_group(
    output: &mut String,
    label: &str,
    class: &str,
    mark: &str,
    nodes: &[GraphNodeDelta],
) {
    let open = if nodes.is_empty() { "" } else { " open" };
    let _ = write!(
        output,
        "<details class=\"delta-group\"{open}><summary>{label} <span class=\"delta-count\">{}</span></summary><ul class=\"delta-list\">",
        nodes.len()
    );
    for node in nodes {
        let display = if node.label.is_empty() {
            &node.id
        } else {
            &node.label
        };
        let mut metadata = Vec::new();
        if display != &node.id {
            metadata.push(node.id.clone());
        }
        if !node.kind.is_empty() {
            metadata.push(node.kind.clone());
        }
        if !node.changed_fields.is_empty() {
            metadata.push(format!("changed: {}", node.changed_fields.join(", ")));
        }
        if !node.source_file.is_empty() {
            metadata.push(node.source_file.clone());
        }
        let _ = write!(
            output,
            "<li class=\"delta-row\" data-graph-node-id=\"{}\"><span class=\"delta-mark {class}\">{mark}</span><span class=\"delta-primary\">{}</span>",
            html_attr(&node.id),
            html_escape(display)
        );
        if !metadata.is_empty() {
            let _ = write!(
                output,
                "<span class=\"delta-secondary\">{}</span>",
                html_escape(&metadata.join(" · "))
            );
        }
        output.push_str("</li>");
    }
    output.push_str("</ul></details>");
}

fn render_edge_delta_group(
    output: &mut String,
    label: &str,
    class: &str,
    mark: &str,
    edges: &[GraphEdgeDelta],
) {
    let open = if edges.is_empty() { "" } else { " open" };
    let _ = write!(
        output,
        "<details class=\"delta-group\"{open}><summary>{label} <span class=\"delta-count\">{}</span></summary><ul class=\"delta-list\">",
        edges.len()
    );
    for edge in edges {
        let mut metadata = Vec::new();
        if !edge.key.is_empty() {
            metadata.push(format!("edge key {}", edge.key));
        }
        if !edge.changed_fields.is_empty() {
            metadata.push(format!("changed: {}", edge.changed_fields.join(", ")));
        }
        if !edge.source_file.is_empty() {
            metadata.push(edge.source_file.clone());
        }
        let _ = write!(
            output,
            "<li class=\"delta-row\" data-graph-edge-source=\"{}\" data-graph-edge-target=\"{}\" data-graph-edge-relation=\"{}\"><span class=\"delta-mark {class}\">{mark}</span><span class=\"delta-primary\">{} —{}→ {}</span>",
            html_attr(&edge.source),
            html_attr(&edge.target),
            html_attr(&edge.relation),
            html_escape(&edge.source),
            html_escape(&edge.relation),
            html_escape(&edge.target)
        );
        if !metadata.is_empty() {
            let _ = write!(
                output,
                "<span class=\"delta-secondary\">{}</span>",
                html_escape(&metadata.join(" · "))
            );
        }
        output.push_str("</li>");
    }
    output.push_str("</ul></details>");
}

fn graph_change_count(delta: &GraphDelta) -> usize {
    delta.added_nodes.len()
        + delta.removed_nodes.len()
        + delta.changed_nodes.len()
        + delta.added_edges.len()
        + delta.removed_edges.len()
        + delta.changed_edges.len()
}

fn source_change_path(change: &SourceFileDelta) -> String {
    match (&change.old_path, &change.new_path) {
        (Some(old), Some(new)) if old != new => format!("{old} → {new}"),
        (_, Some(new)) => new.clone(),
        (Some(old), _) => old.clone(),
        _ => "unknown path".to_owned(),
    }
}

fn source_status_name(status: SourceFileStatus) -> &'static str {
    match status {
        SourceFileStatus::Added => "added",
        SourceFileStatus::Modified => "modified",
        SourceFileStatus::Deleted => "deleted",
        SourceFileStatus::Renamed => "renamed",
    }
}

fn source_line_class(line: &str) -> &'static str {
    if line.starts_with("@@") {
        "hunk"
    } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
        "meta"
    } else if line.starts_with('+') {
        "add"
    } else if line.starts_with('-') {
        "remove"
    } else if line.starts_with("index ")
        || line.starts_with("new file ")
        || line.starts_with("deleted file ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
    {
        "meta"
    } else {
        "context"
    }
}

fn render_html_finding(output: &mut String, finding: &SemanticFinding, collapsed_routine: bool) {
    let break_class = matches!(
        finding.compatibility,
        Compatibility::ProvenBreak | Compatibility::PossibleBreak
    );
    let open = if break_class { " open" } else { "" };
    let _ = write!(
        output,
        "<details class=\"finding{}{}\" id=\"{}\" data-type=\"{}\" data-routine=\"{}\"{open}><summary><div class=\"headline\">{}</div><div class=\"meta\"><span class=\"badge {}\">{}</span><span class=\"badge\">{}</span><span class=\"badge\">{}</span>{}</div></summary><div class=\"detail\">",
        if break_class { " break" } else { "" },
        if finding.public_surface {
            " public"
        } else {
            ""
        },
        html_attr(&finding.id),
        finding_type_name(finding.finding_type),
        collapsed_routine || finding.routine,
        html_escape(&finding.headline),
        compatibility_css(finding.compatibility),
        html_escape(compatibility_name(finding.compatibility)),
        html_escape(confidence_name(finding.confidence)),
        html_escape(finding_type_label(finding.finding_type)),
        if finding.public_surface {
            "<span class=\"badge\">public surface</span>"
        } else {
            ""
        }
    );
    let _ = write!(
        output,
        "<h4>What changed</h4><p>{}</p><h4>Reviewer action</h4><p class=\"action\">{}</p><h4>Verification</h4><p><strong>{}</strong> — {}</p>",
        html_escape(&finding.explanation),
        html_escape(&finding.reviewer_action),
        html_escape(verification_name(finding.verification.state)),
        html_escape(&finding.verification.reason)
    );
    if !finding.verification.exact_tests.is_empty()
        || !finding.verification.recommended_tests.is_empty()
    {
        output.push_str("<ul>");
        for test in &finding.verification.exact_tests {
            let _ = write!(
                output,
                "<li>Mapped test: <code>{}</code></li>",
                html_escape(test)
            );
        }
        for test in &finding.verification.recommended_tests {
            let _ = write!(
                output,
                "<li>Recommended test: <code>{}</code></li>",
                html_escape(test)
            );
        }
        output.push_str("</ul>");
    }
    if !finding.affected_consumers.is_empty() {
        output.push_str("<h4>Affected consumers</h4><ul>");
        for consumer in &finding.affected_consumers {
            let _ = write!(
                output,
                "<li><code>{}</code> · distance {}{}</li>",
                html_escape(&consumer.display_name),
                consumer.distance,
                if consumer.source_file.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", html_escape(&consumer.source_file))
                }
            );
        }
        output.push_str("</ul>");
    }
    if !finding.witness_paths.is_empty() {
        output.push_str("<h4>Witness paths</h4><ul>");
        for path in &finding.witness_paths {
            let hops = path
                .hops
                .iter()
                .map(|hop| format!("{} {} {}", hop.source, hop.relation, hop.target))
                .collect::<Vec<_>>()
                .join(" → ");
            let _ = write!(
                output,
                "<li><code>{}</code>: {}</li>",
                html_escape(&path.consumer),
                html_escape(&hops)
            );
        }
        output.push_str("</ul>");
    }
    if !finding.evidence.is_empty() {
        output.push_str("<h4>Evidence</h4><ul class=\"evidence\">");
        for evidence in &finding.evidence {
            let location = match (evidence.start_byte, evidence.end_byte) {
                (Some(start), Some(end)) => format!(" {start}..{end}"),
                (Some(start), None) => format!(" at {start}"),
                _ => String::new(),
            };
            let record = evidence
                .record_key
                .as_deref()
                .map(|key| format!(" · {}", html_escape(key)))
                .unwrap_or_default();
            let _ = write!(
                output,
                "<li><code>{}</code>{} · {}{record}</li>",
                html_escape(&evidence.source_file),
                html_escape(&location),
                html_escape(&evidence.capability)
            );
        }
        output.push_str("</ul>");
    }
    for (label, value) in [("Before", &finding.before), ("After", &finding.after)] {
        if let Some(value) = value {
            let rendered =
                serde_json::to_string_pretty(value).unwrap_or_else(|_| "unavailable".to_owned());
            let _ = write!(
                output,
                "<h4>{label}</h4><pre>{}</pre>",
                html_escape(&rendered)
            );
        }
    }
    output.push_str("</div></details>");
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn html_attr(value: &str) -> String {
    html_escape(value)
}

fn finding_type_name(finding_type: FindingType) -> &'static str {
    match finding_type {
        FindingType::ContractChange => "contract_change",
        FindingType::BehaviorChange => "behavior_change",
        FindingType::DependencyChange => "dependency_change",
        FindingType::ImpactChange => "impact_change",
        FindingType::VerificationGap => "verification_gap",
        FindingType::StructuralChange => "structural_change",
    }
}

fn finding_type_label(finding_type: FindingType) -> &'static str {
    match finding_type {
        FindingType::ContractChange => "contract",
        FindingType::BehaviorChange => "behavior",
        FindingType::DependencyChange => "dependency",
        FindingType::ImpactChange => "impact",
        FindingType::VerificationGap => "verification gap",
        FindingType::StructuralChange => "structural",
    }
}

fn compatibility_css(compatibility: Compatibility) -> &'static str {
    match compatibility {
        Compatibility::ProvenBreak | Compatibility::PossibleBreak => "break",
        Compatibility::Compatible => "compatible",
        Compatibility::Behavioral => "behavioral",
        Compatibility::NotApplicable | Compatibility::Indeterminate => "unknown",
    }
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

    use compass_history::{SourceFileDelta, SourceFileStatus};
    use compass_semantic_diff::{
        CollapsedGroup, Comparison, Compatibility, Completeness, Confidence, FindingOrigin,
        FindingType, GraphDelta, GraphEdgeDelta, GraphNodeDelta, SemanticDiffReport,
        SemanticFinding, Verification, VerificationState,
    };

    use super::{RenderOptions, render_html, render_section};

    fn finding(index: usize, compatibility: Compatibility) -> SemanticFinding {
        SemanticFinding {
            id: format!("sd1-{index:024x}"),
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

    #[test]
    fn html_report_is_standalone_exhaustive_and_escapes_report_data() {
        let mut routine = finding(1, Compatibility::Behavioral);
        routine.headline = "changed </script><b>unsafe</b>".to_owned();
        routine.routine = true;
        let mut completeness = BTreeMap::new();
        completeness.insert("call_resolution".to_owned(), Completeness::Partial);
        let report = SemanticDiffReport {
            schema: "compass.semantic_diff.report/1".to_owned(),
            comparison: Comparison {
                old_commit: "a".repeat(40),
                new_commit: "b".repeat(40),
                fingerprint: "c".repeat(64),
            },
            findings: vec![routine],
            feature_groups: Vec::new(),
            collapsed_groups: vec![CollapsedGroup {
                label: "routine churn".to_owned(),
                count: 1,
                finding_ids: vec!["sd1-000000000000000000000001".to_owned()],
            }],
            source_changes: vec![SourceFileDelta {
                old_path: Some("old.rs".to_owned()),
                new_path: Some("new.rs".to_owned()),
                status: SourceFileStatus::Renamed,
                hunks: Vec::new(),
                patch: "diff --git a/old.rs b/new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old()\n+new()\n".to_owned(),
            }],
            graph_delta: GraphDelta {
                added_nodes: vec![GraphNodeDelta {
                    id: "new".to_owned(),
                    label: "new()".to_owned(),
                    kind: "function".to_owned(),
                    source_file: "new.rs".to_owned(),
                    changed_fields: Vec::new(),
                }],
                added_edges: vec![GraphEdgeDelta {
                    source: "caller".to_owned(),
                    target: "new".to_owned(),
                    relation: "calls".to_owned(),
                    key: "call-site-1".to_owned(),
                    source_file: "new.rs".to_owned(),
                    changed_fields: Vec::new(),
                }],
                ..GraphDelta::default()
            },
            completeness,
            limitations: vec!["test mapping partial".to_owned()],
        };
        let html = render_html(
            &report,
            &RenderOptions {
                include_routine: false,
                max_findings_per_section: Some(20),
                explain: None,
            },
        );
        let Ok(html) = html else {
            assert!(html.is_ok());
            return;
        };
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("id=\"semantic-diff-data\""));
        assert!(html.contains("compass.semantic_diff.report/1"));
        assert!(html.contains("data-routine=\"true\""));
        assert!(html.contains("changed &lt;/script&gt;&lt;b&gt;unsafe&lt;/b&gt;"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("id=\"code\""));
        assert!(html.contains("old.rs → new.rs"));
        assert!(html.contains("data-diff-style=\"unified\""));
        assert!(html.contains("data-diff-style=\"split\""));
        assert!(html.contains("data-diff-index=\"0\""));
        assert!(html.contains("globalThis.CompassDiffs"));
        assert!(html.contains("@pierre/diffs 1.2.12"));
        assert!(html.contains("<span class=\"diff-line remove\">-old()</span>"));
        assert!(html.contains("id=\"graph\""));
        assert!(html.contains("id=\"graph-canvas\""));
        assert!(html.contains("globalThis.CompassSemanticDiffGraph"));
        assert!(html.contains("class=\"graph-explorer\""));
        assert!(html.contains("id=\"graph-inspector\""));
        assert!(html.contains("id=\"graph-live\""));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains("id=\"source-change-0\""));
        assert!(html.contains("data-graph-node-id=\"new\""));
        assert!(html.contains("data-graph-edge-source=\"caller\""));
        assert!(html.contains("Select a node to inspect its change."));
        assert!(html.contains("Interactive graph unavailable."));
        assert!(html.contains("@media (max-width: 760px)"));
        assert!(html.contains("@media (prefers-reduced-motion: reduce)"));
        assert!(!html.contains("href=\"#source-change-undefined\""));
        assert!(html.contains("caller —calls→ new"));
        assert!(!html.contains("<script src="));
        assert!(!html.contains("<link rel="));
    }
}
