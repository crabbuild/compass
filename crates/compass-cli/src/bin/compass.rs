use std::io::{self, IsTerminal};
use std::process::ExitCode;

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let compatibility = std::env::var_os("COMPASS_INTERNAL_GRAPHIFY_COMPAT").is_some();
    let events = if compatibility {
        false
    } else {
        match compass_cli::ide_contract::take_jsonl_events(&mut arguments) {
            Ok(enabled) => enabled,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
        }
    };
    if !compatibility {
        let style = compass_cli::HelpStyle::detect(
            io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").as_deref(),
            std::env::var_os("TERM").as_deref(),
        );
        if let Some(outcome) = compass_cli::compass_help_request(&arguments, style) {
            return ExitCode::from(compass_cli::write_outcome(
                &outcome,
                &mut io::stdout(),
                &mut io::stderr(),
            ));
        }
    }
    if !compatibility && arguments.first().and_then(|value| value.to_str()) == Some("init") {
        let stdin = io::stdin();
        let input_is_terminal = stdin.is_terminal();
        let mut locked = stdin.lock();
        if events {
            let mut human_stdout = Vec::new();
            let mut human_stderr = Vec::new();
            let code = compass_cli::run_init(
                &arguments[1..],
                &mut locked,
                &mut human_stdout,
                &mut human_stderr,
                input_is_terminal,
            );
            let outcome = compass_cli::ide_contract::progress_outcome(
                "init",
                compass_cli::Outcome {
                    code,
                    stdout: String::from_utf8_lossy(&human_stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&human_stderr).into_owned(),
                    stdout_trailing_newline: true,
                    stderr_trailing_newline: true,
                },
            );
            return ExitCode::from(compass_cli::write_outcome(
                &outcome,
                &mut io::stdout(),
                &mut io::stderr(),
            ));
        }
        return ExitCode::from(compass_cli::run_init(
            &arguments[1..],
            &mut locked,
            &mut io::stdout(),
            &mut io::stderr(),
            input_is_terminal,
        ));
    }
    if arguments.first().and_then(|value| value.to_str()) == Some("watch") {
        if compatibility {
            return ExitCode::from(compass_cli::run_graphify_watch(
                &arguments[1..],
                &mut io::stdout(),
                &mut io::stderr(),
            ));
        }
        if events {
            return ExitCode::from(compass_cli::run_watch_jsonl(
                &arguments[1..],
                &mut io::stdout(),
                &mut io::stderr(),
            ));
        }
        let output_is_terminal = io::stdout().is_terminal();
        return ExitCode::from(compass_cli::run_watch_with_terminal(
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
            output_is_terminal,
        ));
    }
    if arguments.first().and_then(|value| value.to_str()) == Some("serve") {
        return ExitCode::from(compass_cli::run_mcp(
            compass_cli::McpFrontend::Compass,
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
        ));
    }
    if events {
        arguments.push("--events=jsonl".into());
    }
    let outcome = compass_cli::run(
        if compatibility {
            compass_cli::Frontend::Graphify
        } else {
            compass_cli::Frontend::Compass
        },
        arguments,
    );
    ExitCode::from(compass_cli::write_outcome(
        &outcome,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
