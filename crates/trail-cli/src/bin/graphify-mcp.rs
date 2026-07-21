use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ExitCode::from(trail_cli::run_mcp(
        trail_cli::McpFrontend::Graphify,
        &arguments,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
