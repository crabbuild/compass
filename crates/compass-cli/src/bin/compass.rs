use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let events = match compass_cli::ide_contract::take_jsonl_events(&mut arguments) {
        Ok(enabled) => enabled,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
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
    if arguments.first().and_then(|value| value.to_str()) == Some("init") {
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
                compass_cli::Outcome::from_command_output(
                    code,
                    String::from_utf8_lossy(&human_stdout).into_owned(),
                    String::from_utf8_lossy(&human_stderr).into_owned(),
                ),
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
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
        ));
    }
    if events {
        arguments.push("--events=jsonl".into());
    }
    let outcome = compass_cli::run(compass_cli::Frontend::Compass, arguments);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let input_is_terminal = stdin.is_terminal();
    let prompt_is_terminal = stderr.is_terminal();
    let code = compass_cli::write_outcome(&outcome, &mut stdout, &mut stderr);
    if code == 0 && !events {
        let mut locked = stdin.lock();
        if let Err(error) = compass_cli::prompt_to_open_html(
            &outcome,
            &mut locked,
            &mut stderr,
            input_is_terminal,
            prompt_is_terminal,
        ) {
            let _ = writeln!(stderr, "warning: {error}");
        }
    }
    ExitCode::from(code)
}
