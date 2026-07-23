use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use compass_cli::{Frontend, run};

fn arguments<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

#[test]
fn native_update_emits_and_reports_program_analysis() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("lib.rs"),
        "pub fn helper() {}\npub fn run() { helper(); }\n",
    )?;
    let root = directory.path().to_string_lossy();
    let args = arguments(["update", root.as_ref(), "--no-cluster", "--no-viz"]);

    let cold = run(Frontend::Compass, args.clone());
    assert_eq!(cold.code, 0, "{}", cold.stderr);
    assert!(cold.stdout.contains(
        "Program analysis: 1 syntax analyzed, 0 syntax reused, 0 artifacts loaded, 0 artifacts reused, 1 modules, 2 summaries, 0 conflicts"
    ));
    assert!(directory.path().join("compass-out/program.json").is_file());
    assert!(
        !directory
            .path()
            .join("compass-out/.compass_program.json")
            .exists()
    );

    let warm = run(Frontend::Compass, args);
    assert_eq!(warm.code, 0, "{}", warm.stderr);
    assert!(warm.stdout.contains(
        "Program analysis: 0 syntax analyzed, 1 syntax reused, 0 artifacts loaded, 0 artifacts reused, 1 modules, 2 summaries, 0 conflicts"
    ));
    Ok(())
}

#[test]
fn graphify_rejects_program_artifacts_and_never_enables_program_output()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn visible() {}\n")?;
    let root = directory.path().to_string_lossy();

    let rejected = run(
        Frontend::Graphify,
        arguments([
            "extract",
            root.as_ref(),
            "--code-only",
            "--program-artifact=index.scip",
        ]),
    );
    assert_ne!(rejected.code, 0);
    assert!(
        rejected
            .stderr
            .contains("--program-artifact is unsupported in Graphify compatibility mode")
    );

    let built = run(
        Frontend::Graphify,
        arguments(["extract", root.as_ref(), "--code-only", "--no-cluster"]),
    );
    assert_eq!(built.code, 0, "{}", built.stderr);
    assert!(!built.stdout.contains("Program analysis:"));
    assert!(!contains_program_json(directory.path()));
    Ok(())
}

#[test]
fn native_program_artifact_requires_a_nonempty_path() {
    for arguments in [
        vec!["update", "--program-artifact"],
        vec!["update", "--program-artifact="],
    ] {
        let outcome = run(Frontend::Compass, arguments.into_iter().map(OsString::from));
        assert_ne!(outcome.code, 0);
        assert!(
            outcome
                .stderr
                .contains("--program-artifact requires a path")
        );
    }
}

fn contains_program_json(root: &Path) -> bool {
    ["compass-out", "graphify-out"]
        .into_iter()
        .any(|output| root.join(output).join("program.json").exists())
}
