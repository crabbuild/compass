use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use compass_core::{
    ClusterExistingOptions, ClusterLabelContext, ClusterLabelSelection,
    cluster_existing_graph_with_labeler,
};
use compass_output::TokenCost;
use compass_semantic::{
    CommunityLabelCallError, CommunityLabelOptions, CommunityLabelResult, PlainTextOptions,
    builtin_backend, detect_backend_with_custom, execute_plain_text_backend,
    execute_plain_text_custom_backend, label_communities_with, label_communities_with_errors,
    load_custom_providers, placeholder_labels, resolve_builtin_backend, resolve_custom_backend,
};

use crate::{Frontend, Outcome, environment_truthy_from, home_directory};

#[derive(Debug)]
struct LabelArguments {
    root: PathBuf,
    graph_override: Option<PathBuf>,
    no_viz: bool,
    missing_only: bool,
    timing: bool,
    backend: Option<String>,
    model: Option<String>,
    resolution: f64,
    exclude_hubs: Option<f64>,
    max_concurrency: usize,
    batch_size: usize,
    min_community_size: usize,
}

pub(super) fn command_label(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Graphify
        && args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        return Outcome::success("Run 'graphify --help' for full usage.".to_owned());
    }
    let parsed = match parse_arguments(args) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure(error),
    };
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let graph_path = parsed
        .graph_override
        .clone()
        .unwrap_or_else(|| parsed.root.join(&output_name).join("graph.json"));
    if !graph_path.exists() {
        let message = match frontend {
            Frontend::Graphify => {
                format!(
                    "error: no graph found at {} — run /graphify first",
                    graph_path.display()
                )
            }
            Frontend::Compass => format!(
                "error: no graph found at {} — run `compass extract {}` first",
                graph_path.display(),
                parsed.root.display()
            ),
        };
        return Outcome::failure(message);
    }
    let output_dir = if parsed.graph_override.is_some()
        && graph_path.parent().and_then(Path::file_name) == Path::new(&output_name).file_name()
    {
        graph_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        parsed.root.join(&output_name)
    };

    let environment = std::env::vars().collect::<HashMap<_, _>>();
    let global_providers = home_directory()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".graphify/providers.json");
    let local_providers = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".graphify/providers.json");
    let allow_local = environment_truthy_from(&environment, "GRAPHIFY_ALLOW_LOCAL_PROVIDERS");
    let custom = load_custom_providers(&global_providers, &local_providers, allow_local);
    let selected = parsed
        .backend
        .clone()
        .or_else(|| detect_backend_with_custom(&custom.providers, &environment).map(str::to_owned));
    let mut warnings = (!allow_local && local_providers.is_file())
        .then(|| {
            "[graphify] WARNING: ignoring project-local .graphify/providers.json (custom providers control where your corpus and API key are sent). Set GRAPHIFY_ALLOW_LOCAL_PROVIDERS=1 to load it."
                .to_owned()
        })
        .into_iter()
        .collect::<Vec<_>>();
    warnings.extend(
        custom
            .warnings
            .iter()
            .filter(|warning| !warning.starts_with("ignoring project-local "))
            .map(|warning| {
                if warning.starts_with("[graphify]") {
                    warning.clone()
                } else {
                    format!("[graphify label] warning: {warning}")
                }
            }),
    );
    let options = ClusterExistingOptions {
        graph_path,
        output_dir: output_dir.clone(),
        root: parsed.root.clone(),
        no_viz: parsed.no_viz,
        no_label: false,
        resolution: parsed.resolution,
        exclude_hubs: parsed.exclude_hubs,
        min_community_size: parsed.min_community_size,
    };
    let result = cluster_existing_graph_with_labeler(&options, |context| {
        select_labels(
            context,
            &parsed,
            selected.as_deref(),
            &custom.providers,
            &environment,
            &mut warnings,
        )
    });
    let result = match result {
        Ok(result) => result,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if parsed.timing {
        let mut ordered = result.load_warning.clone().into_iter().collect::<Vec<_>>();
        ordered.extend(
            [
                ("load", result.timings.load),
                ("cluster", result.timings.cluster),
                ("analyze", result.timings.analyze),
            ]
            .into_iter()
            .map(|(stage, duration)| {
                format!("[graphify timing] {stage}: {:.1}s", duration.as_secs_f64())
            }),
        );
        ordered.append(&mut warnings);
        ordered.push(format!(
            "[graphify timing] label: {:.1}s",
            result.timings.label.as_secs_f64()
        ));
        ordered.push(format!(
            "[graphify timing] report: {:.1}s",
            result.timings.report.as_secs_f64()
        ));
        if let Some(warning) = result.backup_warning.clone() {
            ordered.push(warning);
        }
        ordered.push(format!(
            "[graphify timing] export: {:.1}s",
            result.timings.export.as_secs_f64()
        ));
        ordered.push(format!(
            "[graphify timing] total: {:.1}s",
            result.timings.total.as_secs_f64()
        ));
        warnings = ordered;
    } else {
        if let Some(warning) = result.load_warning.clone() {
            warnings.insert(0, warning);
        }
        if let Some(warning) = result.backup_warning.clone() {
            warnings.push(warning);
        }
    }
    let done = if parsed.no_viz {
        format!(
            "Done - {} communities. GRAPH_REPORT.md and graph.json updated (--no-viz; graph.html removed).",
            result.communities
        )
    } else if result.html_written {
        format!(
            "Done - {} communities. GRAPH_REPORT.md, graph.json and graph.html updated.",
            result.communities
        )
    } else {
        format!(
            "Done - {} communities. GRAPH_REPORT.md and graph.json updated.",
            result.communities
        )
    };
    let backup = result
        .backup_message
        .as_deref()
        .map(|message| format!("{message}\n"))
        .unwrap_or_default();
    Outcome {
        code: 0,
        stdout: format!(
            "Loading existing graph...\nGraph: {} nodes, {} edges\nRe-clustering...\nLabeling communities...\n{backup}{done}",
            result.nodes, result.edges
        ),
        stderr: warnings.join("\n"),
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
    }
}

fn select_labels(
    context: &ClusterLabelContext<'_>,
    arguments: &LabelArguments,
    selected: Option<&str>,
    custom_providers: &serde_json::Map<String, serde_json::Value>,
    environment: &HashMap<String, String>,
    warnings: &mut Vec<String>,
) -> ClusterLabelSelection {
    let mut labels = if arguments.missing_only {
        context
            .communities
            .keys()
            .map(|community| {
                (
                    *community,
                    context
                        .saved_labels
                        .get(community)
                        .cloned()
                        .unwrap_or_else(|| context.hub_labels[community].clone()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    } else {
        context.hub_labels.clone()
    };
    let label_communities = if arguments.missing_only {
        context
            .communities
            .iter()
            .filter(|(community, _)| {
                context
                    .saved_labels
                    .get(community)
                    .is_none_or(|label| label == &format!("Community {community}"))
            })
            .map(|(community, members)| (*community, members.clone()))
            .collect()
    } else {
        context.communities.clone()
    };
    let generated = generate_labels(
        context,
        &label_communities,
        arguments,
        selected,
        custom_providers,
        environment,
    );
    warnings.extend(generated.warnings);
    for (community, label) in generated.labels {
        if !label.is_empty() && label != format!("Community {community}") {
            labels.insert(community, label);
        }
    }
    let labels_reused = if arguments.missing_only {
        labels
            .keys()
            .filter(|community| context.saved_labels.contains_key(community))
            .count()
    } else {
        0
    };
    ClusterLabelSelection {
        labels,
        labels_reused,
        token_cost: TokenCost {
            input: generated.input_tokens,
            output: generated.output_tokens,
        },
    }
}

fn generate_labels(
    context: &ClusterLabelContext<'_>,
    communities: &compass_graph::Communities,
    arguments: &LabelArguments,
    selected: Option<&str>,
    custom_providers: &serde_json::Map<String, serde_json::Value>,
    environment: &HashMap<String, String>,
) -> CommunityLabelResult {
    let Some(selected) = selected else {
        return CommunityLabelResult {
            labels: placeholder_labels(communities),
            warnings: vec![
                "[graphify label] no LLM backend configured; keeping Community N placeholders. Set an API key (e.g. GOOGLE_API_KEY) or pass --backend."
                    .to_owned(),
            ],
            ..CommunityLabelResult::default()
        };
    };
    let mut options = CommunityLabelOptions::new(selected);
    options.batch_size = arguments.batch_size;
    options.max_concurrency = arguments.max_concurrency;
    let configured_max = environment
        .get("GRAPHIFY_MAX_OUTPUT_TOKENS")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0);
    let node_labels = context
        .document
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.label().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let god_ids = context
        .gods
        .iter()
        .map(|god| god.id.clone())
        .collect::<HashSet<_>>();
    if builtin_backend(selected).is_some() {
        let resolved =
            match resolve_builtin_backend(selected, environment, arguments.model.as_deref()) {
                Ok(resolved) => resolved,
                Err(error) => {
                    return provider_failure(
                        &node_labels,
                        communities,
                        &god_ids,
                        &options,
                        error.to_string(),
                        false,
                    );
                }
            };
        label_communities_with(
            &node_labels,
            communities,
            &god_ids,
            &options,
            &|prompt, max_tokens| {
                execute_plain_text_backend(
                    &resolved,
                    prompt,
                    &PlainTextOptions {
                        max_tokens: configured_max.unwrap_or(max_tokens),
                        claude_cli_model_argument: (selected == "claude-cli")
                            .then(|| arguments.model.clone())
                            .flatten(),
                    },
                    environment,
                )
                .map_err(|error| error.to_string())
            },
        )
    } else if let Some(config) = custom_providers.get(selected) {
        let resolved = match resolve_custom_backend(
            selected,
            config,
            environment,
            arguments.model.as_deref(),
            None,
        ) {
            Ok(resolved) => resolved,
            Err(error) => {
                return provider_failure(
                    &node_labels,
                    communities,
                    &god_ids,
                    &options,
                    error.to_string(),
                    false,
                );
            }
        };
        label_communities_with(
            &node_labels,
            communities,
            &god_ids,
            &options,
            &|prompt, max_tokens| {
                execute_plain_text_custom_backend(
                    &resolved,
                    prompt,
                    &PlainTextOptions {
                        max_tokens: configured_max.unwrap_or(max_tokens),
                        claude_cli_model_argument: None,
                    },
                    environment,
                )
                .map_err(|error| error.to_string())
            },
        )
    } else {
        provider_failure(
            &node_labels,
            communities,
            &god_ids,
            &options,
            format!("Unknown backend {}", python_repr(selected)),
            true,
        )
    }
}

fn provider_failure(
    node_labels: &BTreeMap<String, String>,
    communities: &compass_graph::Communities,
    god_ids: &HashSet<String>,
    options: &CommunityLabelOptions,
    error: String,
    retryable_parse_failure: bool,
) -> CommunityLabelResult {
    label_communities_with_errors(node_labels, communities, god_ids, options, &|_, _| {
        Err(if retryable_parse_failure {
            CommunityLabelCallError::retryable_parse_failure(error.clone())
        } else {
            CommunityLabelCallError::fatal(error.clone())
        })
    })
}

fn python_repr(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn parse_arguments(args: &[String]) -> Result<LabelArguments, String> {
    let mut parsed = LabelArguments {
        root: PathBuf::from("."),
        graph_override: None,
        no_viz: false,
        missing_only: false,
        timing: false,
        backend: None,
        model: None,
        resolution: 1.0,
        exclude_hubs: None,
        max_concurrency: 4,
        batch_size: 100,
        min_community_size: 3,
    };
    let mut root_set = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--graph" | "--backend" | "--model" | "--resolution" | "--exclude-hubs"
            | "--max-concurrency" | "--batch-size" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("error: {argument} requires a value"));
                };
                match argument.as_str() {
                    "--graph" => parsed.graph_override = Some(PathBuf::from(value)),
                    "--backend" => parsed.backend = Some(value.clone()),
                    "--model" => parsed.model = Some(value.clone()),
                    "--resolution" => parsed.resolution = parse_number(value, argument)?,
                    "--exclude-hubs" => parsed.exclude_hubs = Some(parse_number(value, argument)?),
                    "--max-concurrency" => {
                        parsed.max_concurrency = parse_positive(value, argument)?
                    }
                    "--batch-size" => parsed.batch_size = parse_positive(value, argument)?,
                    _ => {}
                }
                index += 1;
            }
            "--no-viz" => parsed.no_viz = true,
            "--missing-only" => parsed.missing_only = true,
            "--timing" => parsed.timing = true,
            "--no-label" => {}
            value if value.starts_with("--backend=") => {
                parsed.backend = Some(value[10..].to_owned())
            }
            value if value.starts_with("--model=") => parsed.model = Some(value[8..].to_owned()),
            value if value.starts_with("--resolution=") => {
                parsed.resolution = parse_number(&value[13..], "--resolution")?;
            }
            value if value.starts_with("--exclude-hubs=") => {
                parsed.exclude_hubs = Some(parse_number(&value[15..], "--exclude-hubs")?);
            }
            value if value.starts_with("--max-concurrency=") => {
                parsed.max_concurrency = parse_positive(&value[18..], "--max-concurrency")?;
            }
            value if value.starts_with("--batch-size=") => {
                parsed.batch_size = parse_positive(&value[13..], "--batch-size")?;
            }
            value if value.starts_with("--min-community-size=") => {
                parsed.min_community_size = parse_positive(&value[21..], "--min-community-size")?;
            }
            value if value.starts_with("--") => {}
            value if !root_set => {
                parsed.root = PathBuf::from(value);
                root_set = true;
            }
            _ => {}
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_number(value: &str, option: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("error: {option} requires a number"))
}

fn parse_positive(value: &str, option: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("error: {option} requires a positive integer"))
}

pub(super) fn label_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Compass => "Usage: compass label [PATH] [--graph PATH] [--backend NAME] [--model NAME] [--missing-only] [--no-viz] [--resolution N] [--exclude-hubs N] [--max-concurrency N] [--batch-size N] [--min-community-size=N] [--timing]",
        Frontend::Graphify => "Usage: graphify label [PATH] [--graph PATH] [--backend NAME] [--model NAME] [--missing-only] [--no-viz] [--resolution N] [--exclude-hubs N] [--max-concurrency N] [--batch-size N] [--min-community-size=N] [--timing]",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use compass_model::GraphDocument;

    use super::*;

    #[test]
    fn label_argument_parser_covers_split_equals_flags_and_validation()
    -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_arguments(&[
            "project".to_owned(),
            "ignored".to_owned(),
            "--graph".to_owned(),
            "custom.json".to_owned(),
            "--backend".to_owned(),
            "openai".to_owned(),
            "--model".to_owned(),
            "model".to_owned(),
            "--resolution".to_owned(),
            "1.5".to_owned(),
            "--exclude-hubs".to_owned(),
            "95".to_owned(),
            "--max-concurrency".to_owned(),
            "2".to_owned(),
            "--batch-size".to_owned(),
            "8".to_owned(),
            "--min-community-size=4".to_owned(),
            "--no-viz".to_owned(),
            "--missing-only".to_owned(),
            "--timing".to_owned(),
            "--unknown".to_owned(),
        ])?;
        assert_eq!(parsed.root, PathBuf::from("project"));
        assert_eq!(parsed.graph_override, Some(PathBuf::from("custom.json")));
        assert_eq!(parsed.backend.as_deref(), Some("openai"));
        assert_eq!(parsed.model.as_deref(), Some("model"));
        assert_eq!(parsed.resolution, 1.5);
        assert_eq!(parsed.exclude_hubs, Some(95.0));
        assert_eq!(parsed.max_concurrency, 2);
        assert_eq!(parsed.batch_size, 8);
        assert_eq!(parsed.min_community_size, 4);
        assert!(parsed.no_viz && parsed.missing_only && parsed.timing);

        let equals = parse_arguments(&[
            "--backend=gemini".to_owned(),
            "--model=flash".to_owned(),
            "--resolution=2".to_owned(),
            "--exclude-hubs=90".to_owned(),
            "--max-concurrency=3".to_owned(),
            "--batch-size=9".to_owned(),
        ])?;
        assert_eq!(equals.backend.as_deref(), Some("gemini"));
        assert_eq!(equals.max_concurrency, 3);
        for args in [
            vec!["--backend".to_owned()],
            vec!["--resolution=nan".to_owned()],
            vec!["--max-concurrency=0".to_owned()],
            vec!["--batch-size=bad".to_owned()],
        ] {
            assert!(parse_arguments(&args).is_err());
        }
        assert!(parse_number("inf", "--value").is_err());
        assert!(parse_positive("0", "--value").is_err());
        assert_eq!(python_repr("a'b\\c"), "'a\\'b\\\\c'");
        assert!(label_help(Frontend::Graphify).starts_with("Usage: graphify"));
        Ok(())
    }

    #[test]
    fn label_selection_reuses_curated_labels_and_fails_providers_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = serde_json::from_value::<GraphDocument>(serde_json::json!({
            "directed":true,"multigraph":false,"graph":{},
            "nodes":[{"id":"a","label":"A"},{"id":"b","label":"B"}],"links":[]
        }))?;
        let communities = BTreeMap::from([(0, vec!["a".to_owned()]), (1, vec!["b".to_owned()])]);
        let hubs = BTreeMap::from([(0, "A".to_owned()), (1, "B".to_owned())]);
        let saved = BTreeMap::from([(0, "Curated".to_owned()), (1, "Community 1".to_owned())]);
        let empty = BTreeMap::new();
        let context = ClusterLabelContext {
            document: &document,
            communities: &communities,
            hub_labels: &hubs,
            saved_labels: &saved,
            saved_signatures: &empty,
            signatures: &empty,
            gods: &[],
        };
        let mut arguments = parse_arguments(&[])?;
        arguments.missing_only = true;
        let mut warnings = Vec::new();
        let selection = select_labels(
            &context,
            &arguments,
            None,
            &serde_json::Map::new(),
            &HashMap::new(),
            &mut warnings,
        );
        assert_eq!(
            selection.labels.get(&0).map(String::as_str),
            Some("Curated")
        );
        assert_eq!(selection.labels_reused, 2);
        assert!(!warnings.is_empty());

        for selected in ["openai", "unknown"] {
            let generated = generate_labels(
                &context,
                &communities,
                &arguments,
                Some(selected),
                &serde_json::Map::new(),
                &HashMap::new(),
            );
            assert_eq!(generated.labels.len(), 2);
            assert!(!generated.warnings.is_empty());
        }
        let providers = serde_json::Map::from_iter([(
            "custom".to_owned(),
            serde_json::json!({"base_url":"not a URL","model":"m","api_key":"key"}),
        )]);
        let generated = generate_labels(
            &context,
            &communities,
            &arguments,
            Some("custom"),
            &providers,
            &HashMap::new(),
        );
        assert_eq!(generated.labels.len(), 2);
        assert!(!generated.warnings.is_empty());
        Ok(())
    }
}
