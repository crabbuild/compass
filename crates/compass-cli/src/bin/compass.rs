use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

const ALLOCATOR_CONFIGURED: &str = "COMPASS_INTERNAL_ALLOCATOR_CONFIGURED";

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let one_shot_build = matches!(
        arguments.first().and_then(|argument| argument.to_str()),
        Some("extract" | "update")
    );
    if one_shot_build
        && (std::env::var_os("MIMALLOC_PURGE_DELAY").is_none()
            || std::env::var_os("MIMALLOC_DISALLOW_ARENA_ALLOC").is_none())
        && std::env::var_os(ALLOCATOR_CONFIGURED).is_none()
        && let Ok(executable) = std::env::current_exe()
    {
        let mut command = Command::new(executable);
        command.args(&arguments).env(ALLOCATOR_CONFIGURED, "1");
        if std::env::var_os("MIMALLOC_PURGE_DELAY").is_none() {
            command.env("MIMALLOC_PURGE_DELAY", "100");
        }
        if std::env::var_os("MIMALLOC_DISALLOW_ARENA_ALLOC").is_none() {
            command.env("MIMALLOC_DISALLOW_ARENA_ALLOC", "1");
        }
        #[cfg(unix)]
        {
            let _error = command.exec();
        }
        #[cfg(not(unix))]
        if let Ok(status) = command.status() {
            return ExitCode::from(
                status
                    .code()
                    .and_then(|code| u8::try_from(code).ok())
                    .unwrap_or(1),
            );
        }
    }
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
            return ExitCode::from(compass_cli::run_init_jsonl(
                &arguments[1..],
                &mut locked,
                io::stdout(),
                &mut io::stderr(),
                input_is_terminal,
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
    let mut code = compass_cli::write_outcome(&outcome, &mut stdout, &mut stderr);
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
    if stdout.flush().is_err() || stderr.flush().is_err() {
        code = 1;
    }
    if one_shot_build {
        std::process::exit(i32::from(code));
    }
    ExitCode::from(code)
}
