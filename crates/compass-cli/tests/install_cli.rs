use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const PROJECT_PLATFORMS: &[&str] = &[
    "claude",
    "cline",
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
    "cline",
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

    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
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
        let output = fixture.run(&["install", "--user", "--platform", platform])?;
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
    let mut generic_tree = directory_tree(&generic.project)?;
    let mut direct_tree = directory_tree(&direct.project)?;
    generic_tree.remove(Path::new(".agents/skills/compass/.compass-install.json"));
    direct_tree.remove(Path::new(".agents/skills/compass/.compass-install.json"));
    assert_eq!(generic_tree, direct_tree);
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
    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
    fs::create_dir_all(skill.parent().ok_or("skill parent")?)?;
    fs::write(&skill, "user-owned")?;

    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("not managed by Compass")
            || String::from_utf8_lossy(&output.stderr).contains("not managed by Compass")
    );
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

#[test]
fn purge_rejects_paths_outside_the_selected_scope() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let outside = fixture
        .project
        .parent()
        .ok_or("fixture parent")?
        .join("outside");
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("sentinel"), "keep")?;

    for value in [
        outside.to_string_lossy().into_owned(),
        "../outside".to_owned(),
        ".".to_owned(),
    ] {
        let output = fixture.run_with_env(
            &["uninstall", "--project", "--purge"],
            &[("COMPASS_OUT", &value)],
        )?;
        assert!(!output.status.success(), "unsafe purge value was accepted");
        assert!(outside.join("sentinel").is_file());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, fixture.project.join("linked-output"))?;
        let output = fixture.run_with_env(
            &["uninstall", "--project", "--purge"],
            &[("COMPASS_OUT", "linked-output")],
        )?;
        assert!(!output.status.success(), "symlink escape was accepted");
        assert!(outside.join("sentinel").is_file());
    }
    Ok(())
}

#[test]
fn non_utf8_instructions_are_preserved() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let instructions = fixture.project.join("AGENTS.md");
    let original = vec![0xff, 0xfe, b'x'];
    fs::write(&instructions, &original)?;

    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert_eq!(fs::read(instructions)?, original);
    assert!(!fixture.project.join(".agents/skills/compass").exists());
    Ok(())
}

#[test]
fn modified_adapter_is_preserved_during_uninstall() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "cursor install",
        &fixture.run(&["install", "--platform", "cursor", "--project"])?,
    );
    let rule = fixture.project.join(".cursor/rules/compass.mdc");
    let modified = format!("{}\nuser change\n", fs::read_to_string(&rule)?);
    fs::write(&rule, &modified)?;

    assert_success(
        "cursor uninstall",
        &fixture.run(&["uninstall", "--platform", "cursor", "--project"])?,
    );
    assert_eq!(fs::read_to_string(rule)?, modified);
    Ok(())
}

#[test]
fn copilot_uninstall_removes_managed_instructions() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let instructions = fixture.project.join(".github/copilot-instructions.md");
    fs::create_dir_all(instructions.parent().ok_or("instructions parent")?)?;
    fs::write(&instructions, "user instructions\n")?;
    assert_success(
        "copilot install",
        &fixture.run(&["install", "--platform", "copilot", "--project"])?,
    );
    assert_success(
        "copilot uninstall",
        &fixture.run(&["uninstall", "--platform", "copilot", "--project"])?,
    );
    assert_eq!(fs::read_to_string(instructions)?, "user instructions\n");
    Ok(())
}

#[test]
fn user_destination_and_claude_config_overrides_round_trip() -> Result<(), Box<dyn Error>> {
    for platform in ["amp", "antigravity", "devin"] {
        let fixture = InstallFixture::new()?;
        assert_success(
            &format!("{platform} user install"),
            &fixture.run(&["install", "--user", "--platform", platform])?,
        );
        assert_success(
            &format!("{platform} user uninstall"),
            &fixture.run(&["uninstall", "--user", "--platform", platform])?,
        );
        assert!(
            !tree_contains_compass_skill(&fixture.home)?,
            "{platform} user skill remained after uninstall"
        );
    }

    let fixture = InstallFixture::new()?;
    let custom = fixture.home.join("custom-claude");
    let custom_value = custom.to_string_lossy().into_owned();
    assert_success(
        "custom Claude install",
        &fixture.run_with_env(
            &["install", "--user", "--platform", "claude"],
            &[("CLAUDE_CONFIG_DIR", &custom_value)],
        )?,
    );
    let registration = fs::read_to_string(custom.join("CLAUDE.md"))?;
    assert!(registration.contains(&custom.join("skills/compass/SKILL.md").display().to_string()));
    assert!(!fixture.home.join(".claude/CLAUDE.md").exists());
    Ok(())
}

#[test]
fn kilo_uninstall_removes_only_the_exact_plugin_entry() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "kilo install",
        &fixture.run(&["install", "--platform", "kilo", "--project"])?,
    );
    let config = fixture.project.join(".kilo/kilo.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
    document["plugin"]
        .as_array_mut()
        .ok_or("plugin array")?
        .push(serde_json::json!(
            "file:///unrelated/.kilo/plugins/compass.js.backup"
        ));
    fs::write(&config, serde_json::to_vec_pretty(&document)?)?;

    assert_success(
        "kilo uninstall",
        &fixture.run(&["uninstall", "--platform", "kilo", "--project"])?,
    );
    let after: serde_json::Value = serde_json::from_slice(&fs::read(config)?)?;
    assert_eq!(
        after["plugin"],
        serde_json::json!(["file:///unrelated/.kilo/plugins/compass.js.backup"])
    );
    Ok(())
}

#[test]
fn plain_install_detects_agents_and_deduplicates_the_shared_skill() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    fs::create_dir(fixture.project.join(".codex"))?;
    fs::write(
        fixture.project.join(".codex/config.toml"),
        "model = \"test\"\n",
    )?;
    fs::create_dir(fixture.project.join(".gemini"))?;
    fs::write(fixture.project.join(".gemini/settings.json"), "{}")?;

    let output = fixture.run(&["install", "--format", "json"])?;
    assert_success("automatic multi-agent install", &output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["scope"], "project");
    assert!(report["detected"]["codex"].is_array());
    assert!(report["detected"]["gemini"].is_array());

    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
    assert!(skill.is_file());
    assert!(!fixture.project.join(".codex/skills/compass").exists());
    assert!(!fixture.project.join(".gemini/skills/compass").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(skill.with_file_name(".compass-install.json"))?)?;
    let consumers = manifest["consumers"].as_array().ok_or("consumers")?;
    for expected in ["agents", "codex", "gemini"] {
        assert!(
            consumers.iter().any(|value| value == expected),
            "missing consumer {expected}"
        );
    }
    Ok(())
}

#[test]
fn repeated_platforms_share_one_package_and_dry_run_is_read_only() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let dry_run = fixture.run(&[
        "install",
        "--platform",
        "codex",
        "--platform",
        "claude",
        "--dry-run",
        "--format",
        "json",
    ])?;
    assert_success("dry run", &dry_run);
    assert!(!tree_contains_compass_skill(&fixture.project)?);

    let output = fixture.run(&[
        "install",
        "--platform",
        "codex",
        "--platform",
        "gemini",
        "--platform",
        "copilot",
        "--format",
        "json",
    ])?;
    assert_success("shared explicit install", &output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let results = report["results"].as_array().ok_or("results")?;
    assert_eq!(
        results
            .iter()
            .filter(|result| result["id"] == "shared-agent-skill")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn malformed_config_and_modified_managed_files_are_preserved() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let config = fixture.project.join(".codex/hooks.json");
    fs::create_dir_all(config.parent().ok_or("config parent")?)?;
    fs::write(&config, "{not-json")?;
    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&config)?, "{not-json");
    assert!(!fixture.project.join(".agents/skills/compass").exists());

    fs::write(&config, "{}")?;
    assert_success(
        "initial managed install",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
    fs::write(&skill, "user modification")?;
    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(skill)?, "user modification");
    Ok(())
}

#[test]
fn failed_shared_install_does_not_remove_legacy_codex_skill() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "seed managed skill",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    let shared = fixture.project.join(".agents/skills/compass");
    let legacy = fixture.project.join(".codex/skills/compass");
    fs::create_dir_all(legacy.parent().ok_or("legacy parent")?)?;
    fs::rename(&shared, &legacy)?;
    fs::create_dir_all(&shared)?;
    fs::write(shared.join("SKILL.md"), "user-owned conflict")?;

    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert!(legacy.join("SKILL.md").is_file());
    assert_eq!(
        fs::read_to_string(shared.join("SKILL.md"))?,
        "user-owned conflict"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn uninstall_does_not_follow_a_symlinked_skill_directory() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "seed managed skill",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    let local = fixture.project.join(".agents/skills/compass");
    let external = fixture
        .project
        .parent()
        .ok_or("fixture parent")?
        .join("external-compass");
    fs::rename(&local, &external)?;
    std::os::unix::fs::symlink(&external, &local)?;

    assert_success(
        "safe symlink uninstall",
        &fixture.run(&["uninstall", "--platform", "codex", "--project"])?,
    );
    assert!(external.join("SKILL.md").is_file());
    assert!(local.is_symlink());
    Ok(())
}

#[test]
fn uninstall_removes_one_shared_consumer_without_breaking_another() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "multi install",
        &fixture.run(&[
            "install",
            "--platform",
            "codex",
            "--platform",
            "gemini",
            "--project",
        ])?,
    );
    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
    assert!(skill.is_file());

    assert_success(
        "remove codex consumer",
        &fixture.run(&["uninstall", "--platform", "codex", "--project"])?,
    );
    assert!(skill.is_file());
    let codex_hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.project.join(".codex/hooks.json"))?)?;
    assert!(
        codex_hooks["hooks"]["PreToolUse"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    assert!(fixture.project.join("GEMINI.md").is_file());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(skill.with_file_name(".compass-install.json"))?)?;
    assert_eq!(manifest["consumers"], serde_json::json!(["gemini"]));

    assert_success(
        "remove gemini consumer",
        &fixture.run(&["uninstall", "--platform", "gemini", "--project"])?,
    );
    assert!(!skill.exists());
    Ok(())
}

#[test]
fn uninstall_failure_is_nonzero_and_rolls_back_prior_removals() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert_success(
        "codex install",
        &fixture.run(&["install", "--platform", "codex", "--project"])?,
    );
    let hooks = fixture.project.join(".codex/hooks.json");
    fs::write(&hooks, "{invalid-json")?;
    let before = directory_tree(&fixture.project)?;

    let output = fixture.run(&["uninstall", "--platform", "codex", "--project"])?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("rollback")
            || String::from_utf8_lossy(&output.stderr).contains("rollback")
    );
    assert_eq!(directory_tree(&fixture.project)?, before);
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
        fs::create_dir(project.join(".git"))?;
        Ok(Self {
            _directory: directory,
            project,
            home,
        })
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        self.run_with_env(arguments, &[])
    }

    fn run_with_env(
        &self,
        arguments: &[&str],
        variables: &[(&str, &str)],
    ) -> Result<Output, Box<dyn Error>> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
        command
            .args(arguments)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME");
        for (name, value) in variables {
            command.env(name, value);
        }
        Ok(command.output()?)
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
