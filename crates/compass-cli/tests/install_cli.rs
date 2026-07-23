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

const GLOBAL_PLATFORMS: &[&str] = &[
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
];

#[test]
fn project_codex_install_creates_native_compass_skill() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert_success("codex project install", &output);

    let skill = fixture.project.join(".codex/skills/compass/SKILL.md");
    let body = fs::read_to_string(&skill)?;
    assert!(body.starts_with("---\nname: compass\n"));
    assert!(body.contains("compass query"));
    assert!(body.contains("compass-out/"));
    assert!(body.contains("references/history.md"));
    assert!(body.contains("references/semantic-extraction.md"));
    assert!(body.contains("references/operations.md"));
    assert!(body.contains("references/command-reference.md"));
    assert!(body.contains("references/labeling.md"));
    assert!(body.contains("references/security-and-boundaries.md"));
    assert_native(&body);
    assert!(skill.with_file_name(".compass_version").is_file());
    assert!(
        skill
            .with_file_name("references")
            .join("query.md")
            .is_file()
    );
    let references = skill.with_file_name("references");
    assert_eq!(
        fs::read_dir(&references)?
            .collect::<Result<Vec<_>, _>>()?
            .len(),
        15
    );
    Ok(())
}

#[test]
fn every_project_platform_installs_native_content() -> Result<(), Box<dyn Error>> {
    for platform in PROJECT_PLATFORMS {
        let fixture = InstallFixture::new()?;
        let output = fixture.run(&["install", "--platform", platform, "--project"])?;
        assert_success(&format!("{platform} project install"), &output);
        assert_native_tree(&fixture.project)?;
        assert_native_tree(&fixture.home)?;

        let output = fixture.run(&["uninstall", "--platform", platform, "--project"])?;
        assert_success(&format!("{platform} project uninstall"), &output);
        assert!(
            !tree_contains_compass_skill(&fixture.project)?,
            "{platform} left a project Compass skill after uninstall"
        );
    }
    Ok(())
}

#[test]
fn every_global_platform_installs_native_content() -> Result<(), Box<dyn Error>> {
    for platform in GLOBAL_PLATFORMS {
        let fixture = InstallFixture::new()?;
        let output = fixture.run(&["install", "--platform", platform])?;
        assert_success(&format!("{platform} global install"), &output);
        assert_native_tree(&fixture.project)?;
        assert_native_tree(&fixture.home)?;
    }
    Ok(())
}

#[test]
fn direct_and_generic_codex_installs_match() -> Result<(), Box<dyn Error>> {
    let generic = InstallFixture::new()?;
    let direct = InstallFixture::new()?;
    assert_success(
        "generic codex install",
        &generic.run(&["install", "--platform", "codex", "--project"])?,
    );
    assert_success(
        "direct codex install",
        &direct.run(&["codex", "install", "--project"])?,
    );
    assert_eq!(
        directory_tree(&generic.project)?,
        directory_tree(&direct.project)?
    );
    Ok(())
}

#[test]
fn compass_lifecycle_preserves_adjacent_graphify_install() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let graphify = fixture.project.join(".codex/skills/graphify/SKILL.md");
    fs::create_dir_all(graphify.parent().ok_or("graphify parent")?)?;
    fs::write(&graphify, "---\nname: graphify\n---\n")?;
    fs::create_dir_all(fixture.project.join("graphify-out"))?;

    assert_success(
        "install beside graphify",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    assert_success(
        "uninstall beside graphify",
        &fixture.run(&["uninstall", "--platform", "codex", "--project"])?,
    );

    assert_eq!(fs::read_to_string(graphify)?, "---\nname: graphify\n---\n");
    assert!(fixture.project.join("graphify-out").is_dir());
    Ok(())
}

#[test]
fn reinstall_is_idempotent_and_parser_errors_do_not_mutate() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "first install",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    let first = directory_tree(&fixture.project)?;
    assert_success(
        "second install",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    assert_eq!(directory_tree(&fixture.project)?, first);

    let rejected = fixture.run(&["install", "--unknown"])?;
    assert!(!rejected.status.success());
    assert_eq!(directory_tree(&fixture.project)?, first);
    Ok(())
}

#[test]
fn install_does_not_overwrite_an_unowned_compass_skill() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let skill = fixture.project.join(".codex/skills/compass/SKILL.md");
    fs::create_dir_all(skill.parent().ok_or("skill parent")?)?;
    fs::write(&skill, "user-owned")?;

    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not managed by Compass"));
    assert_eq!(fs::read_to_string(skill)?, "user-owned");
    Ok(())
}

#[test]
fn purge_removes_only_compass_output() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    fs::create_dir_all(fixture.project.join("compass-out"))?;
    fs::create_dir_all(fixture.project.join("graphify-out"))?;
    fs::write(fixture.project.join("compass-out/graph.json"), "{}")?;
    fs::write(fixture.project.join("graphify-out/graph.json"), "{}")?;

    let output = fixture.run(&["uninstall", "--project", "--purge"])?;
    assert_success("purge", &output);
    assert!(!fixture.project.join("compass-out").exists());
    assert!(fixture.project.join("graphify-out/graph.json").is_file());
    Ok(())
}

struct InstallFixture {
    _directory: TempDir,
    project: PathBuf,
    home: PathBuf,
}

impl InstallFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let home = directory.path().join("home");
        fs::create_dir_all(&project)?;
        fs::create_dir_all(&home)?;
        Ok(Self {
            _directory: directory,
            project,
            home,
        })
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
            .args(arguments)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .output()?)
    }
}

fn assert_success(context: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{context}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_native_tree(root: &Path) -> Result<(), Box<dyn Error>> {
    for (path, bytes) in directory_tree(root)? {
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        assert_native(&text);
        if path.ends_with("SKILL.md") {
            assert!(
                text.starts_with("---\nname: compass\n"),
                "{} is not a Compass skill",
                path.display()
            );
        }
    }
    Ok(())
}

fn assert_native(value: &str) {
    let normalized = value.replace(env!("CARGO_BIN_EXE_compass"), "compass");
    let lowercase = normalized.to_ascii_lowercase();
    assert!(
        !lowercase.contains("graphify"),
        "installed content contains Graphify: {normalized}"
    );
    assert!(
        !lowercase.contains("python -m"),
        "installed content contains a Python module command: {normalized}"
    );
}

fn tree_contains_compass_skill(root: &Path) -> Result<bool, Box<dyn Error>> {
    Ok(directory_tree(root)?.into_iter().any(|(path, bytes)| {
        path.ends_with("SKILL.md")
            && String::from_utf8(bytes).is_ok_and(|text| text.starts_with("---\nname: compass\n"))
    }))
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
