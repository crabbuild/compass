use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.first().and_then(|value| value.to_str()) == Some("graph")
        && arguments.get(1).and_then(|value| value.to_str()) == Some("watch")
    {
        return ExitCode::from(trail_cli::run_watch(
            &arguments[2..],
            &mut io::stdout(),
            &mut io::stderr(),
        ));
    }
    let outcome = trail_cli::run(trail_cli::Frontend::Trail, arguments);
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
