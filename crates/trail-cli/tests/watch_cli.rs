use std::error::Error;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn run_python(arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let repository = repository_root();
    Ok(Command::new(repository.join(".venv/bin/python"))
        .args(["-m", "graphify", "watch"])
        .args(arguments)
        .current_dir(&repository)
        .env("PYTHONPATH", &repository)
        .output()?)
}

fn run_rust(arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_graphify"))
        .arg("watch")
        .args(arguments)
        .current_dir(repository_root())
        .output()?)
}

fn assert_same(expected: &Output, actual: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout, "stdout mismatch");
    assert_eq!(actual.stderr, expected.stderr, "stderr mismatch");
}

#[test]
fn watch_missing_path_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing");
    let missing = missing.to_string_lossy();
    assert_same(
        &run_python(&[missing.as_ref()])?,
        &run_rust(&[missing.as_ref()])?,
    );
    Ok(())
}

#[cfg(unix)]
fn run_until_interrupted(mut command: Command) -> Result<Output, Box<dyn Error>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("missing stdout pipe")?;
    let mut stdout = BufReader::new(stdout);
    let mut bytes = Vec::new();
    for _ in 0..3 {
        stdout.read_until(b'\n', &mut bytes)?;
    }
    let pid = child.id().to_string();
    let status = Command::new("kill").args(["-INT", pid.as_str()]).status()?;
    if !status.success() {
        return Err("could not interrupt watch child".into());
    }
    stdout.read_to_end(&mut bytes)?;
    let status = child.wait()?;
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .ok_or("missing stderr pipe")?
        .read_to_end(&mut stderr)?;
    Ok(Output {
        status,
        stdout: bytes,
        stderr,
    })
}

#[cfg(unix)]
#[test]
fn watch_startup_and_interrupt_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().to_string_lossy();
    let repository = repository_root();
    let shim = directory.path().join("oracle-shim");
    std::fs::create_dir_all(shim.join("watchdog/observers"))?;
    std::fs::write(shim.join("watchdog/__init__.py"), "")?;
    std::fs::write(
        shim.join("watchdog/events.py"),
        "class FileSystemEventHandler:\n    pass\n",
    )?;
    std::fs::write(
        shim.join("watchdog/observers/__init__.py"),
        concat!(
            "class Observer:\n",
            "    def schedule(self, *args, **kwargs): pass\n",
            "    def start(self): pass\n",
            "    def stop(self): pass\n",
            "    def join(self): pass\n",
        ),
    )?;
    std::fs::write(
        shim.join("watchdog/observers/polling.py"),
        "from . import Observer\nPollingObserver = Observer\n",
    )?;
    let python_path = format!("{}:{}", shim.display(), repository.display());

    let mut python = Command::new(repository.join(".venv/bin/python"));
    python
        .args(["-m", "graphify", "watch", root.as_ref(), "--ignored"])
        .current_dir(&repository)
        .env("PYTHONPATH", python_path)
        .env("PYTHONUNBUFFERED", "1");
    let expected = run_until_interrupted(python)?;

    let mut rust = Command::new(env!("CARGO_BIN_EXE_graphify"));
    rust.args(["watch", root.as_ref(), "--ignored"])
        .current_dir(&repository);
    let actual = run_until_interrupted(rust)?;
    assert_same(&expected, &actual);
    Ok(())
}
