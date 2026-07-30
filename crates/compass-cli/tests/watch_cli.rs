use std::error::Error;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use compass_files::BuildGuard;

#[cfg(unix)]
#[test]
fn native_watch_synchronizes_before_reporting_ready() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("main.rs"),
        "fn synchronized_on_start() {}\n",
    )?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["watch", ".", "--poll", "--no-cluster", "--no-viz"])
        .current_dir(directory.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("missing stdout pipe")?;
    let mut stdout = BufReader::new(stdout);
    let mut output = String::new();
    for _ in 0..20 {
        let mut line = String::new();
        if stdout.read_line(&mut line)? == 0 {
            break;
        }
        output.push_str(&line);
        if line.contains("Watching for changes") || line.contains("Watching ") {
            break;
        }
    }
    let pid = child.id().to_string();
    let status = Command::new("kill").args(["-INT", pid.as_str()]).status()?;
    if !status.success() {
        return Err("could not interrupt native watch child".into());
    }
    stdout.read_to_string(&mut output)?;
    let status = child.wait()?;
    assert!(status.success(), "watch output: {output}");
    assert!(output.contains("Starting"));
    assert!(output.contains("Synchronizing current graph"));
    assert!(output.contains("Watching"));
    assert!(
        BuildGuard::resolve_artifact(&directory.path().join("compass-out"), "graph.json")?
            .is_file()
    );
    Ok(())
}
