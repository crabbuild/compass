use compass_history::{HistoryStore, RealizationId, Repository};

use crate::{Frontend, Outcome};

pub(crate) fn help(frontend: Frontend) -> String {
    let prefix = if frontend == Frontend::Compass {
        "compass"
    } else {
        "graphify"
    };
    format!(
        "Usage: {prefix} history <command>\n\nCommands:\n  enable [build-profile options]\n  disable\n  status [REV] [--format text|json]\n  build REV [build-profile options] [--format text|json]\n  rebuild REV [--replace-corrupt] [--format text|json]\n  list [REV] [--format text|json]\n  show REALIZATION [--format text|json]\n  prefer REV REALIZATION [--format text|json]\n  export REV --format graph-json|graphify-out --output PATH\n  gc [--prune-non-preferred] [--yes] [--format text|json]"
    )
}

pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if args.is_empty()
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return Outcome::success(help(frontend));
    }
    match execute(args) {
        Ok(text) => Outcome::success(text),
        Err((2, error)) => Outcome::failure_with_code(format!("error: {error}"), 2),
        Err((_, error)) => Outcome::failure(format!("error: {error}")),
    }
}

fn execute(args: &[String]) -> Result<String, (u8, String)> {
    let repository =
        Repository::discover(&std::env::current_dir().map_err(runtime)?).map_err(runtime)?;
    let (positionals, format, output) = parse(&args[1..]).map_err(usage)?;
    match args[0].as_str() {
        "status" => {
            one_or_zero(&positionals, "status")?;
            let commit = repository
                .resolve(positionals.first().map(String::as_str).unwrap_or("HEAD"))
                .map_err(runtime)?;
            let Some(history) = HistoryStore::open_existing(&repository).map_err(runtime)? else {
                return Ok(if format == "json" {
                    serde_json::json!({"enabled":false,"store":false,"commit":commit}).to_string()
                } else {
                    format!("history: disabled\nstore: no store\ncommit: {commit}")
                });
            };
            let preferred = history.preferred(&commit).map_err(runtime)?;
            if format == "json" {
                Ok(serde_json::json!({"enabled":false,"store":true,"commit":commit,"preferred":preferred.as_ref().map(|v|v.id.as_hex()),"version":preferred.as_ref().map(|v|&v.version)}).to_string())
            } else if let Some(value) = preferred {
                history.validate(&value.id).map_err(runtime)?;
                Ok(format!(
                    "history: disabled\nstore: present\ncommit: {commit}\npreferred: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nvalidation: valid",
                    value.id,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count
                ))
            } else {
                Ok(format!(
                    "history: disabled\nstore: present\ncommit: {commit}\npreferred: none"
                ))
            }
        }
        "list" => {
            one_or_zero(&positionals, "list")?;
            let commit = positionals
                .first()
                .map(|rev| repository.resolve(rev))
                .transpose()
                .map_err(runtime)?;
            let Some(history) = HistoryStore::open_existing(&repository).map_err(runtime)? else {
                return Ok(if format == "json" { "[]" } else { "" }.to_owned());
            };
            let values = history.list(commit.as_ref()).map_err(runtime)?;
            if format == "json" {
                serde_json::to_string(&values.iter().map(|v|serde_json::json!({"id":v.id,"preferred":v.preferred,"version":v.version})).collect::<Vec<_>>()).map_err(runtime)
            } else {
                Ok(values
                    .into_iter()
                    .map(|v| {
                        format!(
                            "{}\t{}\t{}\t{}",
                            v.version.git_commit,
                            v.id,
                            v.version.extraction_fingerprint,
                            if v.preferred {
                                "preferred"
                            } else {
                                "alternate"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        "show" => {
            exact(&positionals, 1, "show requires REALIZATION")?;
            let id: RealizationId = positionals[0].parse().map_err(runtime)?;
            let value = store(&repository)?.get(&id).map_err(runtime)?;
            if format == "json" {
                serde_json::to_string(&value.version).map_err(runtime)
            } else {
                Ok(format!(
                    "realization: {}\ncommit: {}\nfingerprint: {}\nnodes: {}\nedges: {}",
                    value.id,
                    value.version.git_commit,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count
                ))
            }
        }
        "prefer" => {
            exact(&positionals, 2, "prefer requires REV REALIZATION")?;
            let commit = repository.resolve(&positionals[0]).map_err(runtime)?;
            let id: RealizationId = positionals[1].parse().map_err(runtime)?;
            let history = store(&repository)?;
            history.validate(&id).map_err(runtime)?;
            let current = history.preferred(&commit).map_err(runtime)?;
            if !history
                .compare_and_set_preferred(&commit, current.as_ref().map(|v| &v.id), &id)
                .map_err(runtime)?
            {
                return Err(runtime("preferred realization changed concurrently"));
            }
            Ok(if format == "json" {
                serde_json::json!({"commit":commit,"preferred":id}).to_string()
            } else {
                format!("preferred {id} for {commit}")
            })
        }
        "export" => {
            exact(&positionals, 1, "export requires REV")?;
            let output = output.ok_or_else(|| usage("export requires --output PATH"))?;
            let commit = repository.resolve(&positionals[0]).map_err(runtime)?;
            let history = store(&repository)?;
            let preferred = history
                .preferred(&commit)
                .map_err(runtime)?
                .ok_or_else(|| runtime("no preferred realization"))?;
            let artifacts = history.artifacts(&preferred.id).map_err(runtime)?;
            if format == "graph-json" {
                if output.is_dir() {
                    return Err(runtime("graph-json output must be a file"));
                }
                compass_files::write_json_atomic(&output, &artifacts.artifacts.document, false)
                    .map_err(runtime)?;
            } else if format == "graphify-out" {
                if output.exists() {
                    return Err(runtime("bundle output already exists"));
                }
                artifacts.write_seed(&output).map_err(runtime)?;
            } else {
                return Err(usage("export --format must be graph-json or graphify-out"));
            }
            Ok(format!("exported {} to {}", preferred.id, output.display()))
        }
        "enable" | "disable" | "build" | "rebuild" | "gc" => {
            Err(runtime(format!("history {} is not available yet", args[0])))
        }
        other => Err(usage(format!("unknown history command {other}"))),
    }
}

fn parse(args: &[String]) -> Result<(Vec<String>, String, Option<std::path::PathBuf>), String> {
    let mut p = Vec::new();
    let mut f = None;
    let mut o = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                let v = args.get(i).ok_or("--format requires a value")?;
                if f.replace(v.clone()).is_some() {
                    return Err("duplicate --format".into());
                }
            }
            "--output" => {
                i += 1;
                let v = args.get(i).ok_or("--output requires a path")?;
                if o.replace(v.into()).is_some() {
                    return Err("duplicate --output".into());
                }
            }
            v if v.starts_with('-') => return Err(format!("unknown option {v}")),
            v => p.push(v.into()),
        }
        i += 1;
    }
    Ok((p, f.unwrap_or_else(|| "text".into()), o))
}
fn store(r: &Repository) -> Result<HistoryStore, (u8, String)> {
    HistoryStore::open_existing(r)
        .map_err(runtime)?
        .ok_or_else(|| runtime("graph history has no store"))
}
fn exact(p: &[String], n: usize, m: &str) -> Result<(), (u8, String)> {
    if p.len() == n { Ok(()) } else { Err(usage(m)) }
}
fn one_or_zero(p: &[String], m: &str) -> Result<(), (u8, String)> {
    if p.len() <= 1 {
        Ok(())
    } else {
        Err(usage(format!("{m} accepts at most one revision")))
    }
}
fn runtime(e: impl ToString) -> (u8, String) {
    (1, e.to_string())
}
fn usage(e: impl ToString) -> (u8, String) {
    (2, e.to_string())
}
