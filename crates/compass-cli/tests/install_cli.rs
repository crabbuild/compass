use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const PROJECT_PLATFORMS: &[&str] = &[
    "claude",
    "windows",
    "codebuddy",
    "codex",
    "opencode",
    "kilo",
    "aider",
    "copilot",
    "claw",
    "droid",
    "trae",
    "trae-cn",
    "hermes",
    "kiro",
    "pi",
    "amp",
    "agents",
    "skills",
    "devin",
    "antigravity",
    "gemini",
    "cursor",
];

const DIRECT_PLATFORMS: &[&str] = &[
    "claude",
    "codebuddy",
    "gemini",
    "cursor",
    "vscode",
    "copilot",
    "kilo",
    "kiro",
    "devin",
    "pi",
    "amp",
    "agents",
    "skills",
    "aider",
    "codex",
    "opencode",
    "claw",
    "droid",
    "trae",
    "trae-cn",
    "hermes",
    "antigravity",
];

#[test]
fn every_project_installer_lifecycle_matches_python() -> Result<(), Box<dyn Error>> {
    for platform in PROJECT_PLATFORMS {
        let fixture = InstallFixture::new()?;
        fixture.assert_command(&["install", "--platform", platform, "--project"])?;
        fixture.assert_trees(platform)?;
        fixture.assert_command(&["uninstall", "--platform", platform, "--project"])?;
        fixture.assert_trees(platform)?;
    }
    Ok(())
}

#[test]
fn every_direct_installer_lifecycle_matches_python() -> Result<(), Box<dyn Error>> {
    for platform in DIRECT_PLATFORMS {
        let fixture = InstallFixture::new()?;
        fixture.assert_command(&[platform, "install"])?;
        fixture.assert_trees(platform)?;
        fixture.assert_command(&[platform, "uninstall"])?;
        fixture.assert_trees(platform)?;
    }
    Ok(())
}

#[test]
fn global_skill_installers_match_python() -> Result<(), Box<dyn Error>> {
    for platform in [
        "claude",
        "codex",
        "opencode",
        "kilo",
        "aider",
        "copilot",
        "claw",
        "droid",
        "trae",
        "trae-cn",
        "hermes",
        "kiro",
        "pi",
        "codebuddy",
        "antigravity",
        "antigravity-windows",
        "windows",
        "kimi",
        "amp",
        "agents",
        "devin",
        "gemini",
        "cursor",
    ] {
        let fixture = InstallFixture::new()?;
        let arguments = ["install", "--platform", platform];
        let python = fixture.python(&arguments)?;
        let rust = fixture.rust(&arguments)?;
        fixture.assert_output(platform, &rust, &python)?;
        fixture.assert_trees(platform)?;
    }
    Ok(())
}

struct InstallFixture {
    _directory: TempDir,
    repo: PathBuf,
    python_project: PathBuf,
    rust_project: PathBuf,
    python_home: PathBuf,
    rust_home: PathBuf,
}

impl InstallFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let repo = std::env::var_os("GRAPHIFY_REPO_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."));
        let python_project = directory.path().join("python-project");
        let rust_project = directory.path().join("rust-project");
        let python_home = directory.path().join("python-home");
        let rust_home = directory.path().join("rust-home");
        for path in [&python_project, &rust_project, &python_home, &rust_home] {
            fs::create_dir(path)?;
        }
        Ok(Self {
            _directory: directory,
            repo,
            python_project,
            rust_project,
            python_home,
            rust_home,
        })
    }

    fn python(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        let python = std::env::var_os("GRAPHIFY_PYTHON")
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    self.repo.join("rust").join(path)
                }
            })
            .unwrap_or_else(|| self.repo.join(".venv/bin/python"));
        let display = python.display().to_string();
        Ok(Command::new(python)
            .args(["-m", "graphify"])
            .args(arguments)
            .current_dir(&self.python_project)
            .env("PYTHONPATH", &self.repo)
            .env("HOME", &self.python_home)
            .env("USERPROFILE", &self.python_home)
            .output()
            .map_err(|error| format!("could not run Python oracle {display}: {error}"))?)
    }

    fn rust(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(Command::new(env!("CARGO_BIN_EXE_graphify"))
            .args(arguments)
            .current_dir(&self.rust_project)
            .env("HOME", &self.rust_home)
            .env("USERPROFILE", &self.rust_home)
            .output()?)
    }

    fn assert_output(
        &self,
        context: &str,
        rust: &Output,
        python: &Output,
    ) -> Result<(), Box<dyn Error>> {
        assert_eq!(rust.status.code(), python.status.code(), "{context}");
        assert_eq!(
            self.normalize(&String::from_utf8(rust.stdout.clone())?),
            self.normalize(&String::from_utf8(python.stdout.clone())?),
            "stdout mismatch for {context}"
        );
        assert_eq!(
            self.normalize(&String::from_utf8(rust.stderr.clone())?),
            self.normalize(&String::from_utf8(python.stderr.clone())?),
            "stderr mismatch for {context}"
        );
        Ok(())
    }

    fn assert_command(&self, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        let python = self.python(arguments)?;
        let rust = self.rust(arguments)?;
        self.assert_output(&arguments.join(" "), &rust, &python)
    }

    fn assert_trees(&self, context: &str) -> Result<(), Box<dyn Error>> {
        assert_eq!(
            self.normalized_tree(&self.rust_home)?,
            self.normalized_tree(&self.python_home)?,
            "home artifact mismatch for {context}"
        );
        assert_eq!(
            self.normalized_tree(&self.rust_project)?,
            self.normalized_tree(&self.python_project)?,
            "project artifact mismatch for {context}"
        );
        Ok(())
    }

    fn normalized_tree(&self, root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, Box<dyn Error>> {
        Ok(directory_tree(root)?
            .into_iter()
            .map(|(path, contents)| {
                let contents = String::from_utf8(contents)
                    .map(|value| self.normalize(&value).into_bytes())
                    .unwrap_or_else(|error| error.into_bytes());
                (path, contents)
            })
            .collect())
    }

    fn normalize(&self, value: &str) -> String {
        let mut value = value.to_owned();
        for (path, replacement) in [
            (&self.rust_project, "$PROJECT"),
            (&self.python_project, "$PROJECT"),
            (&self.rust_home, "$HOME"),
            (&self.python_home, "$HOME"),
        ] {
            if let Ok(path) = fs::canonicalize(path) {
                value = value.replace(&path.display().to_string(), replacement);
            }
            value = value.replace(&path.display().to_string(), replacement);
        }
        value
    }
}

fn directory_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, Box<dyn Error>> {
    fn visit(
        root: &Path,
        directory: &Path,
        output: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, output)?;
            } else if path.is_file() {
                output.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}
