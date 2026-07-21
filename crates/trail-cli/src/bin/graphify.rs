use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.first().and_then(|value| value.to_str()) == Some("watch") {
        return ExitCode::from(trail_cli::run_graphify_watch(
            &arguments[1..],
            &mut io::stdout(),
            &mut io::stderr(),
        ));
    }
    let outcome = trail_cli::run(trail_cli::Frontend::Graphify, arguments);
    emit(
        &outcome.stdout,
        &mut io::stdout(),
        outcome.stdout_trailing_newline,
    );
    emit(
        &outcome.stderr,
        &mut io::stderr(),
        outcome.stderr_trailing_newline,
    );
    ExitCode::from(outcome.code)
}

fn emit(output: &str, stream: &mut impl Write, trailing_newline: bool) {
    if output.is_empty() {
        return;
    }
    if trailing_newline {
        let _result = writeln!(stream, "{output}");
    } else {
        let _result = write!(stream, "{output}");
    }
}
