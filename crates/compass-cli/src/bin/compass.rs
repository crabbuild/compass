use std::io;
use std::process::ExitCode;

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.first().and_then(|value| value.to_str()) == Some("diff") {
        return ExitCode::from(compass_cli::run_diff(
            compass_cli::Frontend::Compass,
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
        ));
    }
    if arguments.first().and_then(|value| value.to_str()) == Some("watch") {
        return ExitCode::from(compass_cli::run_watch(
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
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
    let outcome = compass_cli::run(compass_cli::Frontend::Compass, arguments);
    ExitCode::from(compass_cli::write_outcome(
        &outcome,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
