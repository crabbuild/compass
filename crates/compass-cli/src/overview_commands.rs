use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use compass_core::ExportInputs;
use compass_query::{ConnectionLayers, OverviewScope, ScopeSelection, hotspots, scoped_document};

use crate::{Outcome, SharedOutputFormat, default_graph_path, parse_shared_output_format};

pub(super) fn apply_scope(
    inputs: &mut ExportInputs,
    filters: Vec<String>,
) -> Result<ScopeSelection, String> {
    let scope = OverviewScope::new(filters).map_err(|error| error.to_string())?;
    let (document, selection) =
        scoped_document(&inputs.document, &scope).map_err(|error| error.to_string())?;
    let ids = document
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    for members in inputs.communities.values_mut() {
        members.retain(|id| ids.contains(id.as_str()));
        members.sort();
        members.dedup();
    }
    inputs.communities.retain(|_, members| !members.is_empty());
    inputs
        .labels
        .retain(|id, _| inputs.communities.contains_key(id));
    inputs.document = document;
    Ok(selection)
}

pub(super) fn scope_text(selection: &ScopeSelection) -> String {
    format!(
        "Scope {}: {}/{} nodes; {} outside relationships omitted ({} cross the boundary).",
        if selection.scope.filters.is_empty() {
            "all".into()
        } else {
            selection.scope.filters.join(", ")
        },
        selection.selected_nodes,
        selection.input_nodes,
        selection.omitted_relationships,
        selection.boundary_relationships
    )
}

pub(super) fn scoped_output(
    mut output: Outcome,
    selection: &ScopeSelection,
    format: SharedOutputFormat,
) -> Outcome {
    if output.code != 0 {
        return output;
    }
    if matches!(format, SharedOutputFormat::Text) {
        output.stdout = format!("{}\n{}", scope_text(selection), output.stdout);
    } else {
        let result = match serde_json::from_str::<serde_json::Value>(&output.stdout) {
            Ok(value) => value,
            Err(error) => {
                return Outcome::failure(format!(
                    "error: scoped architecture encoding failed: {error}"
                ));
            }
        };
        match serde_json::to_string_pretty(
            &serde_json::json!({"schema":"compass.architecture.scoped-view/1", "selection":selection, "result":result}),
        ) {
            Ok(text) => output.stdout = text,
            Err(error) => {
                return Outcome::failure(format!(
                    "error: scoped architecture encoding failed: {error}"
                ));
            }
        }
    }
    output
}

pub(super) fn command(command: &str, args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        let id = if command == "community" { " [ID]" } else { "" };
        let layers = if command == "hotspots" {
            " [--include-inferred] [--include-documents]"
        } else {
            ""
        };
        return Outcome::success(format!(
            "Usage: compass {command}{id} [--graph PATH] [--scope PATH|module:NAME] [--limit N]{layers} [--format text|json] [--budget N]"
        ));
    }
    let (format, args) = match parse_shared_output_format(args, command) {
        Ok(value) => value,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 2),
    };
    if matches!(format, SharedOutputFormat::AgentJson) {
        return Outcome::failure_with_code(
            "error: --format must be text or json; run compass help community or compass help hotspots".into(),
            2,
        );
    }
    let mut graph = default_graph_path();
    let mut filters = Vec::new();
    let mut layers = ConnectionLayers::default();
    let mut limit = if command == "hotspots" { 10 } else { 20 };
    let mut community = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, b)| (a, Some(b)));
        match flag {
            "--include-inferred" if inline.is_none() && command == "hotspots" => layers.inferred = true,
            "--include-documents" if inline.is_none() && command == "hotspots" => layers.documents = true,
            "--graph" | "--scope" | "--limit" => {
                let value = match inline.or_else(|| { index += 1; args.get(index).map(String::as_str) }) {
                    Some(value) if !value.is_empty() && !value.starts_with("--") => value,
                    _ => return Outcome::failure_with_code(format!("error: {flag} requires a value; run compass help {command}"), 2),
                };
                match flag {
                    "--graph" => graph = PathBuf::from(value),
                    "--scope" => filters.push(value.to_owned()),
                    _ => match value.parse::<usize>() {
                        Ok(value) if (1..=100).contains(&value) => limit = value,
                        _ => return Outcome::failure_with_code("error: --limit must be between 1 and 100".into(), 2),
                    },
                }
            }
            value if command == "community" && !value.starts_with('-') && community.is_none() => {
                match value.parse::<usize>() {
                    Ok(value) => community = Some(value),
                    Err(_) => return Outcome::failure_with_code("error: community ID must be a nonnegative integer; run compass community to list IDs".into(), 2),
                }
            }
            _ => return Outcome::failure_with_code(format!("error: unknown {command} argument {arg}; run compass help {command}"), 2),
        }
        index += 1;
    }
    let scope = match OverviewScope::new(filters) {
        Ok(value) => value,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 2),
    };
    let graph = match compass_files::BuildGuard::resolve_requested_artifact(&graph) {
        Ok(path) => path,
        Err(error) => {
            return Outcome::failure(format!(
                "error: cannot select graph: {error}; run compass ensure"
            ));
        }
    };
    if command == "hotspots" {
        let document = match compass_model::GraphDocument::load(&graph) {
            Ok(value) => value,
            Err(error) => return crate::graph_load_outcome(error),
        };
        crate::freshness::record(&graph, crate::graph_source_commit(&document));
        let report = match hotspots(&document, &scope, layers, limit) {
            Ok(report) => report,
            Err(error) => return Outcome::failure(format!("error: {error}")),
        };
        if !matches!(format, SharedOutputFormat::Text) {
            return encode(&report);
        }
        return Outcome::success(compass_output::render_hotspots_text(&report));
    }
    let document = match compass_model::GraphDocument::load(&graph) {
        Ok(value) => value,
        Err(error) => return crate::graph_load_outcome(error),
    };
    crate::freshness::record(&graph, crate::graph_source_commit(&document));
    let mut inputs = ExportInputs {
        document,
        communities: BTreeMap::new(),
        labels: BTreeMap::new(),
        cohesion: BTreeMap::new(),
        gods: Vec::new(),
        report: String::new(),
    };
    // Use membership from the pinned graph, matching MCP, rather than mixing
    // it with a possibly unrelated analysis sidecar.
    for node in &inputs.document.nodes {
        if let Some(id) = node
            .unsigned("community")
            .and_then(|id| usize::try_from(id).ok())
        {
            inputs
                .communities
                .entry(id)
                .or_default()
                .push(node.id.clone());
            let label = node.string("community_name");
            if !label.is_empty() {
                inputs
                    .labels
                    .entry(id)
                    .and_modify(|existing| {
                        if label < *existing {
                            *existing = label.clone();
                        }
                    })
                    .or_insert(label);
            }
        }
    }
    let selection = match apply_scope(&mut inputs, scope.filters.clone()) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let all_count = inputs.communities.len();
    let groups = inputs.communities.iter().filter(|(id,_)| community.is_none_or(|wanted| wanted == **id))
        .take(limit).map(|(id,members)| serde_json::json!({"id":id,"label":inputs.labels.get(id),"memberCount":members.len(),
            "members":members.iter().take(limit).collect::<Vec<_>>(), "omittedMembers":members.len().saturating_sub(limit)})).collect::<Vec<_>>();
    let omitted = if community.is_some() {
        0
    } else {
        all_count.saturating_sub(groups.len())
    };
    let result = serde_json::json!({"schema":"compass.communities/1", "selection":selection, "communities":groups, "omittedCommunities":omitted});
    if !matches!(format, SharedOutputFormat::Text) {
        return encode(&result);
    }
    let nodes = inputs
        .document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut lines = vec![scope_text(&selection)];
    if groups.is_empty() {
        lines.push("No communities matched. Run compass community without --scope to list IDs, or compass cluster-only to publish community membership.".into());
    }
    for group in groups {
        lines.push(format!(
            "Community {}: {} ({} nodes; {} members omitted)",
            group["id"],
            compass_query::sanitize_label(group["label"].as_str().unwrap_or("unlabelled")),
            group["memberCount"],
            group["omittedMembers"]
        ));
        if let Some(members) = group["members"].as_array() {
            for id in members {
                let id = id.as_str().unwrap_or_default();
                if let Some(node) = nodes.get(id) {
                    lines.push(format!(
                        "  {} {}:{}",
                        compass_query::sanitize_label(&node.display_label()),
                        compass_query::sanitize_label(node.source_file().unwrap_or("?")),
                        compass_query::sanitize_label(&node.string("source_location"))
                    ));
                } else {
                    lines.push(format!("  {}", compass_query::sanitize_label(id)));
                }
            }
        }
    }
    lines.push(format!(
        "{omitted} communities omitted; use --limit 100 or compass community ID."
    ));
    Outcome::success(lines.join("\n"))
}

fn encode(value: &impl serde::Serialize) -> Outcome {
    match serde_json::to_string_pretty(value) {
        Ok(text) => Outcome::success(text),
        Err(error) => Outcome::failure(format!("error: overview encoding failed: {error}")),
    }
}
