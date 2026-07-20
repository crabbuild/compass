use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let outcome = trail_cli::run(trail_cli::Frontend::Trail, std::env::args_os().skip(1));
    emit(&outcome.stdout, &mut io::stdout());
    emit(&outcome.stderr, &mut io::stderr());
    ExitCode::from(outcome.code)
}

fn emit(output: &str, stream: &mut impl Write) {
    if output.is_empty() {
        return;
    }
    let _result = writeln!(stream, "{output}");
}
