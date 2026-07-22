use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;

use compass_core::default_graph_path;
use compass_prs::{
    PrInfo, RenderOptions, SystemRunner, attach_graph_impact, detect_default_branch, fetch_prs,
    fetch_worktrees, render_conflicts, render_dashboard, render_pr_detail, render_worktrees,
    triage_prompt,
};
use compass_semantic::{
    PlainTextOptions, backend_api_key, builtin_backend, execute_plain_text_backend,
    load_custom_providers, resolve_builtin_backend,
};
use time::OffsetDateTime;

use crate::{Frontend, Outcome};

const HELP: &str = "graphify prs — graph-aware PR dashboard.\n\nFast terminal overview of open PRs with CI/review state, worktree mapping,\nand optional graph-impact analysis (which communities a PR touches) and\nOpus-powered triage ranking.\n\nUsage:\n  graphify prs                   # dashboard of all open PRs\n  graphify prs <number>          # deep dive on one PR\n  graphify prs --triage          # Opus ranks your review queue\n  graphify prs --worktrees       # show worktree → branch → PR mapping\n  graphify prs --conflicts       # PRs sharing graph communities (merge-order risk)\n  graphify prs --base <branch>   # filter to PRs targeting this base (default: v8)\n";

#[derive(Default)]
struct Arguments {
    base: Option<String>,
    repo: Option<String>,
    triage: bool,
    worktrees: bool,
    conflicts: bool,
    wrong_base: bool,
    number: Option<u64>,
    graph: PathBuf,
    help: bool,
}

#[derive(Clone, Copy)]
struct Colors {
    enabled: bool,
}

impl Colors {
    fn paint(self, code: &str, text: impl AsRef<str>) -> String {
        if self.enabled {
            format!("\u{1b}[{code}m{}\u{1b}[0m", text.as_ref())
        } else {
            text.as_ref().to_owned()
        }
    }

    fn red(self, text: impl AsRef<str>) -> String {
        self.paint("31", text)
    }

    fn bold(self, text: impl AsRef<str>) -> String {
        self.paint("1", text)
    }

    fn dim(self, text: impl AsRef<str>) -> String {
        self.paint("2", text)
    }
}

pub(super) fn command_prs(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Graphify
        && args
            .iter()
            .any(|argument| matches!(argument.as_str(), "-h" | "--help" | "-?"))
    {
        return Outcome::success("Run 'graphify --help' for full usage.".to_owned());
    }
    let parsed = parse_arguments(args);
    if parsed.help {
        return match frontend {
            Frontend::Graphify => {
                Outcome::success("Run 'graphify --help' for full usage.".to_owned())
            }
            Frontend::Compass => Outcome::success(prs_help(frontend)),
        };
    }

    let colors = Colors {
        enabled: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    let render = RenderOptions {
        color: colors.enabled,
        command_name: match frontend {
            Frontend::Compass => "compass prs",
            Frontend::Graphify => "graphify prs",
        },
    };
    let environment = std::env::vars().collect::<HashMap<_, _>>();
    let runner = SystemRunner;
    let base = parsed
        .base
        .clone()
        .unwrap_or_else(|| detect_default_branch(&runner, parsed.repo.as_deref()));
    let now = OffsetDateTime::now_utc();
    let mut prs = match fetch_prs(&runner, parsed.repo.as_deref(), Some(&base), None) {
        Ok(prs) => prs,
        Err(_) => {
            return Outcome::failure(
                colors.red("  Error: gh CLI not found or not authenticated. Run: gh auth login"),
            );
        }
    };

    let worktrees = fetch_worktrees(&runner);
    for pr in &mut prs {
        pr.worktree_path = worktrees.get(&pr.branch).cloned();
    }
    let needs_impact =
        parsed.graph.exists() && (parsed.number.is_some() || parsed.triage || parsed.conflicts);
    let community_labels = if needs_impact {
        attach_graph_impact(&runner, &mut prs, &parsed.graph, parsed.repo.as_deref())
    } else {
        Default::default()
    };

    if let Some(number) = parsed.number {
        let Some(pr) = prs.iter().find(|pr| pr.number == number) else {
            return Outcome::failure(colors.red(format!("  PR #{number} not found in open PRs.")));
        };
        return success_exact(render_pr_detail(pr, now, render));
    }

    if parsed.triage {
        let stdout = render_dashboard(&prs, &base, parsed.wrong_base, now, render);
        return run_triage(stdout, &prs, &base, now, colors, &environment);
    }
    if parsed.worktrees {
        return success_exact(render_worktrees(&prs, &worktrees, now, render));
    }
    if parsed.conflicts {
        return success_exact(format!(
            "{}{}",
            render_dashboard(&prs, &base, parsed.wrong_base, now, render),
            render_conflicts(&prs, &base, &community_labels, now, render)
        ));
    }
    success_exact(render_dashboard(
        &prs,
        &base,
        parsed.wrong_base,
        now,
        render,
    ))
}

fn success_exact(stdout: String) -> Outcome {
    Outcome {
        code: 0,
        stdout,
        stderr: String::new(),
        stdout_trailing_newline: false,
        stderr_trailing_newline: true,
    }
}

fn parse_arguments(args: &[String]) -> Arguments {
    let mut parsed = Arguments {
        graph: default_graph_path(),
        ..Arguments::default()
    };
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--triage" => parsed.triage = true,
            "--worktrees" => parsed.worktrees = true,
            "--conflicts" => parsed.conflicts = true,
            "--wrong-base" => parsed.wrong_base = true,
            "-h" | "--help" | "-?" => parsed.help = true,
            "--base" | "-b" | "--repo" | "-R" | "--graph" if index + 1 < args.len() => {
                index += 1;
                match argument.as_str() {
                    "--base" | "-b" => parsed.base = Some(args[index].clone()),
                    "--repo" | "-R" => parsed.repo = Some(args[index].clone()),
                    "--graph" => parsed.graph = PathBuf::from(&args[index]),
                    _ => {}
                }
            }
            value if value.starts_with("--base=") => parsed.base = Some(value[7..].to_owned()),
            value if value.starts_with("--graph=") => parsed.graph = PathBuf::from(&value[8..]),
            value => {
                let numeric = value.strip_prefix('#').unwrap_or(value);
                if !numeric.is_empty() && numeric.bytes().all(|byte| byte.is_ascii_digit()) {
                    parsed.number = numeric.parse().ok();
                }
            }
        }
        index += 1;
    }
    parsed
}

fn run_triage(
    mut stdout: String,
    prs: &[PrInfo],
    base: &str,
    now: OffsetDateTime,
    colors: Colors,
    environment: &HashMap<String, String>,
) -> Outcome {
    let Some(prompt) = triage_prompt(prs, base, now) else {
        stdout.push_str(&format!(
            "{}\n",
            colors.dim("  No actionable PRs to triage.")
        ));
        return success_exact(stdout);
    };
    let (backend, model) = resolve_triage_backend(environment);
    stdout.push_str(&format!(
        "\n{}{}\n\n",
        colors.bold("  Triage"),
        colors.dim(format!(" ({backend} / {model})"))
    ));

    // Python accepts every configured backend name, but only these branches
    // currently issue a triage request. Preserve that external behavior.
    if !matches!(
        backend.as_str(),
        "claude" | "kimi" | "openai" | "gemini" | "ollama" | "claude-cli"
    ) {
        return success_exact(stdout);
    }

    let resolved = match resolve_builtin_backend(&backend, environment, Some(&model)) {
        Ok(resolved) => resolved,
        Err(error) => {
            return Outcome {
                code: 0,
                stdout,
                stderr: format!("\n\n  {}", colors.red(format!("Triage failed: {error}"))),
                stdout_trailing_newline: false,
                stderr_trailing_newline: true,
            };
        }
    };
    match execute_plain_text_backend(
        &resolved,
        &prompt,
        &PlainTextOptions {
            max_tokens: 1_024,
            claude_cli_model_argument: None,
        },
        environment,
    ) {
        Ok(response) => {
            for line in response.text.lines() {
                stdout.push_str(&format!("  {line}\n"));
            }
            stdout.push('\n');
            success_exact(stdout)
        }
        Err(error) => Outcome {
            code: 0,
            stdout,
            stderr: format!("\n\n  {}", colors.red(format!("Triage failed: {error}"))),
            stdout_trailing_newline: false,
            stderr_trailing_newline: true,
        },
    }
}

fn resolve_triage_backend(environment: &HashMap<String, String>) -> (String, String) {
    let custom = load_triage_custom_providers(environment);
    let explicit = environment
        .get("GRAPHIFY_TRIAGE_BACKEND")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .filter(|name| builtin_backend(name).is_some() || custom.contains_key(*name));
    let backend = explicit.map(str::to_owned).unwrap_or_else(|| {
        ["claude", "kimi", "openai", "gemini"]
            .into_iter()
            .find(|name| {
                builtin_backend(name)
                    .and_then(|backend| backend_api_key(backend, environment))
                    .is_some()
            })
            .map(str::to_owned)
            .or_else(|| executable_on_path("claude").then(|| "claude-cli".to_owned()))
            .unwrap_or_else(|| "ollama".to_owned())
    });
    let default = match backend.as_str() {
        "claude" => "claude-opus-4-7".to_owned(),
        "kimi" => "kimi-k2.6".to_owned(),
        "openai" => "gpt-4.1-mini".to_owned(),
        "gemini" => "gemini-3-flash-preview".to_owned(),
        "claude-cli" => "claude-code-plan".to_owned(),
        name if custom.contains_key(name) => custom
            .get(name)
            .and_then(|config| config.get("default_model"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name => builtin_backend(name)
            .map(|spec| {
                spec.model_variable
                    .and_then(|key| environment.get(key))
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .unwrap_or_else(|| spec.default_model.to_owned())
            })
            .unwrap_or_default(),
    };
    let model = environment
        .get("GRAPHIFY_TRIAGE_MODEL")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or(default);
    (backend, model)
}

fn load_triage_custom_providers(
    environment: &HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    let global = home_directory()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".graphify")
        .join("providers.json");
    let local = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".graphify")
        .join("providers.json");
    let allow_local = environment
        .get("GRAPHIFY_ALLOW_LOCAL_PROVIDERS")
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        });
    load_custom_providers(&global, &local, allow_local).providers
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn executable_on_path(name: &str) -> bool {
    let names = if cfg!(windows) {
        vec![
            format!("{name}.cmd"),
            format!("{name}.exe"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|path| names.iter().any(|candidate| path.join(candidate).is_file()))
    })
}

pub(super) fn prs_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Compass => "Usage: compass prs [NUMBER] [--triage] [--worktrees] [--conflicts] [--wrong-base] [--base BRANCH] [--repo OWNER/REPO] [--graph PATH]".to_owned(),
        Frontend::Graphify => HELP.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_explicit_triage_backend_falls_back_like_python() {
        let environment = HashMap::from([
            ("GRAPHIFY_TRIAGE_BACKEND".to_owned(), "not-real".to_owned()),
            ("OPENAI_API_KEY".to_owned(), "configured".to_owned()),
        ]);
        assert_eq!(
            resolve_triage_backend(&environment),
            ("openai".to_owned(), "gpt-4.1-mini".to_owned())
        );
    }

    #[test]
    fn ollama_triage_respects_python_model_override_precedence() {
        let environment = HashMap::from([
            ("GRAPHIFY_TRIAGE_BACKEND".to_owned(), "ollama".to_owned()),
            ("OLLAMA_MODEL".to_owned(), "local-model".to_owned()),
        ]);
        assert_eq!(
            resolve_triage_backend(&environment),
            ("ollama".to_owned(), "local-model".to_owned())
        );
    }
}
