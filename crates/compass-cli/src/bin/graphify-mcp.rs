use std::io;
use std::process::ExitCode;

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ExitCode::from(compass_cli::run_mcp(
        compass_cli::McpFrontend::Graphify,
        &arguments,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
