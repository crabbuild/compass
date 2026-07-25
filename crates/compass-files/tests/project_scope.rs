use std::error::Error;
use std::fs;

use compass_files::{BuildScope, DetectOptions, ProjectConfig, WatchPathFilter, detect};

#[test]
fn project_config_round_trips_normalized_scope() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn run() {}\n")?;
    let config = ProjectConfig::new(BuildScope {
        include: vec!["./src\\".to_owned(), "src/".to_owned()],
        exclude: vec!["**\\generated.rs".to_owned()],
    })
    .normalize(root.path())?;
    config.write(root.path())?;
    let loaded = ProjectConfig::load(root.path())?.ok_or("missing config")?;
    assert_eq!(loaded.build.include, ["src/"]);
    assert_eq!(loaded.build.exclude, ["**/generated.rs"]);
    Ok(())
}

#[test]
fn detection_applies_files_folders_globs_and_excludes() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    for (path, contents) in [
        ("src/lib.rs", "pub fn lib() {}\n"),
        ("services/api/src/main.rs", "pub fn api() {}\n"),
        ("services/api/src/generated.rs", "pub fn generated() {}\n"),
        ("tools/task.rs", "pub fn task() {}\n"),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().ok_or("parent")?)?;
        fs::write(path, contents)?;
    }
    let scope = BuildScope {
        include: vec!["src/".to_owned(), "services/*/src".to_owned()],
        exclude: vec!["**/generated.rs".to_owned()],
    }
    .normalize(root.path())?;
    let detection = detect(
        root.path(),
        &DetectOptions {
            scope,
            ..DetectOptions::default()
        },
    )?;
    let files = detection.files["code"].join("\n");
    assert!(files.contains("src/lib.rs"));
    assert!(files.contains("services/api/src/main.rs"));
    assert!(!files.contains("generated.rs"));
    assert!(!files.contains("tools/task.rs"));
    Ok(())
}

#[test]
fn project_config_rejects_unknown_versions_and_root_escapes() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join(".compass"))?;
    fs::write(
        root.path().join(".compass/config.toml"),
        "version = 2\n[build]\n",
    )?;
    let error = match ProjectConfig::load(root.path()) {
        Err(error) => error,
        Ok(_) => return Err("version must fail".into()),
    };
    assert!(error.to_string().contains("version 2"));
    assert!(
        ProjectConfig::new(BuildScope {
            include: vec!["../outside".to_owned()],
            exclude: Vec::new(),
        })
        .normalize(root.path())
        .is_err()
    );
    assert!(
        ProjectConfig::new(BuildScope {
            include: vec!["C:\\outside\\source".to_owned()],
            exclude: Vec::new(),
        })
        .normalize(root.path())
        .is_err()
    );
    Ok(())
}

#[test]
fn project_root_is_a_valid_literal_directory_scope() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
    let scope = BuildScope {
        include: vec!["./".to_owned()],
        exclude: Vec::new(),
    }
    .normalize(root.path())?;
    assert_eq!(scope.include, ["."]);
    let detection = detect(
        root.path(),
        &DetectOptions {
            scope,
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(detection.total_files, 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn project_config_rejects_an_out_of_root_symlink() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::create_dir(root.path().join(".compass"))?;
    let outside_config = outside.path().join("config.toml");
    fs::write(&outside_config, "version = 1\n[build]\n")?;
    symlink(&outside_config, root.path().join(".compass/config.toml"))?;

    assert!(ProjectConfig::load(root.path()).is_err());
    assert!(
        ProjectConfig::new(BuildScope::default())
            .write(root.path())
            .is_err()
    );
    Ok(())
}

#[test]
fn watcher_uses_the_same_saved_scope_rules() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("src/generated"))?;
    fs::create_dir(root.path().join("tools"))?;
    let scope = BuildScope {
        include: vec!["src/".to_owned()],
        exclude: vec!["src/generated/**".to_owned()],
    }
    .normalize(root.path())?;
    let filter = WatchPathFilter::new(
        root.path(),
        &DetectOptions {
            scope,
            ..DetectOptions::default()
        },
    )?;
    assert!(filter.allows(&root.path().join("src/new.rs")));
    assert!(!filter.allows(&root.path().join("src/generated/new.rs")));
    assert!(!filter.allows(&root.path().join("tools/new.rs")));
    Ok(())
}
