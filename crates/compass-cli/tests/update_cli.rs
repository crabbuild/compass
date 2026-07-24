mod support;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _directory: tempfile::TempDir,
    repository: PathBuf,
    python_root: PathBuf,
    native_root: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let repository = std::env::var_os("GRAPHIFY_REPO_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(3)
                    .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
            });
        let python_root = directory.path().join("python");
        let native_root = directory.path().join("native");
        fs::create_dir(&python_root)?;
        fs::create_dir(&native_root)?;
        let source = "def greet(name):\n    return f'hello {name}'\n";
        fs::write(python_root.join("sample.py"), source)?;
        fs::write(native_root.join("sample.py"), source)?;
        Ok(Self {
            _directory: directory,
            repository,
            python_root,
            native_root,
        })
    }

    fn python(&self, extra: &[&str]) -> Result<Output, Box<dyn Error>> {
        self.python_with_env(extra, &[])
    }

    fn python_with_env(
        &self,
        extra: &[&str],
        environment: &[(&str, &str)],
    ) -> Result<Output, Box<dyn Error>> {
        let executable = if cfg!(windows) {
            self.repository.join(".venv/Scripts/python.exe")
        } else {
            self.repository.join(".venv/bin/python")
        };
        Ok(Command::new(executable)
            .args(["-m", "graphify", "update"])
            .arg(&self.python_root)
            .args(extra)
            .env("PYTHONPATH", &self.repository)
            .env_remove("GRAPHIFY_NO_TIPS")
            .envs(environment.iter().copied())
            .output()?)
    }

    fn native(&self, extra: &[&str]) -> Result<Output, Box<dyn Error>> {
        self.native_with_env(extra, &[])
    }

    fn native_with_env(
        &self,
        extra: &[&str],
        environment: &[(&str, &str)],
    ) -> Result<Output, Box<dyn Error>> {
        Ok(support::compat_command()
            .arg("update")
            .arg(&self.native_root)
            .args(extra)
            .env_remove("GRAPHIFY_NO_TIPS")
            .envs(environment.iter().copied())
            .output()?)
    }

    fn normalize(&self, bytes: &[u8]) -> String {
        let mut value = String::from_utf8_lossy(bytes).into_owned();
        for (root, replacement) in [(&self.python_root, "$ROOT"), (&self.native_root, "$ROOT")] {
            let displayed = root.display().to_string();
            value = value.replace(&format!("/private{displayed}"), replacement);
            value = value.replace(&displayed, replacement);
        }
        value
    }

    fn assert_same(&self, python: &Output, native: &Output) -> Result<(), Box<dyn Error>> {
        assert_eq!(native.status.code(), python.status.code());
        assert_eq!(
            self.normalize(&native.stdout),
            self.normalize(&python.stdout)
        );
        assert_eq!(
            self.normalize(&native.stderr),
            self.normalize(&python.stderr)
        );
        Ok(())
    }

    fn assert_graphs_match(&self) -> Result<(), Box<dyn Error>> {
        let mut python: serde_json::Value =
            serde_json::from_slice(&fs::read(self.python_root.join("graphify-out/graph.json"))?)?;
        let mut native: serde_json::Value =
            serde_json::from_slice(&fs::read(self.native_root.join("graphify-out/graph.json"))?)?;
        normalize_json(&mut python, self);
        normalize_json(&mut native, self);
        assert_eq!(native, python);
        assert_eq!(
            output_files(&self.native_root)?,
            output_files(&self.python_root)?
        );
        assert!(!self.native_root.join("graphify-out/manifest.json").exists());
        Ok(())
    }
}

#[test]
fn update_visualization_limit_matches_python() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let environment = [("GRAPHIFY_VIZ_NODE_LIMIT", "0")];
    let python = fixture.python_with_env(&[], &environment)?;
    let native = fixture.native_with_env(&[], &environment)?;
    fixture.assert_same(&python, &native)?;
    fixture.assert_graphs_match()?;
    assert!(!fixture.native_root.join("graphify-out/graph.html").exists());
    Ok(())
}

#[test]
fn cold_warm_and_raw_updates_match_python() -> Result<(), Box<dyn Error>> {
    let clustered = Fixture::new()?;
    let python = clustered.python(&[])?;
    let native = clustered.native(&[])?;
    clustered.assert_same(&python, &native)?;
    clustered.assert_graphs_match()?;

    let python = clustered.python(&[])?;
    let native = clustered.native(&[])?;
    clustered.assert_same(&python, &native)?;
    clustered.assert_graphs_match()?;

    let raw = Fixture::new()?;
    let python = raw.python(&["--no-cluster"])?;
    let native = raw.native(&["--no-cluster"])?;
    raw.assert_same(&python, &native)?;
    raw.assert_graphs_match()?;
    Ok(())
}

#[test]
fn update_argument_errors_match_python() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    for arguments in [["--no-viz"].as_slice(), ["second"].as_slice()] {
        let python = fixture.python(arguments)?;
        let native = fixture.native(arguments)?;
        fixture.assert_same(&python, &native)?;
    }
    Ok(())
}

fn output_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let output = root.join("graphify-out");
    let mut files = fs::read_dir(output)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            PathBuf::from(
                entry
                    .file_name()
                    .to_string_lossy()
                    .replace(".compass_", ".graphify_"),
            )
        })
        .filter(|path| path != Path::new(".graphify_output_stats.json"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn normalize_json(value: &mut serde_json::Value, fixture: &Fixture) {
    match value {
        serde_json::Value::String(text) => *text = fixture.normalize(text.as_bytes()),
        serde_json::Value::Array(values) => {
            for value in values {
                normalize_json(value, fixture);
            }
        }
        serde_json::Value::Object(object) => {
            object.remove("built_at");
            object.remove("built_at_commit");
            object.remove("signature_hash");
            object.remove("implementation_hash");
            object.remove("source_hash");
            object.remove("symbol_kind");
            object.remove("language");
            object.remove("line_start");
            object.remove("line_end");
            object.remove("signature");
            for value in object.values_mut() {
                normalize_json(value, fixture);
            }
        }
        _ => {}
    }
}
