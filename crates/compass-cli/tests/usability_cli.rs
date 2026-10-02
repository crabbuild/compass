mod support;

use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    path::Path,
    process::{Command, Output},
};

fn token_count(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn execute(root: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(support::compass_command()
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join(".unused-global-config"))
        .args(args)
        .output()?)
}
fn parsed(output: Output) -> Result<Value, Box<dyn Error>> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn fixture(root: &Path) -> Result<(), Box<dyn Error>> {
    fs::write(
        root.join("graph.json"),
        serde_json::to_vec(&json!({
            "directed":true,"multigraph":true,"nodes":[
                {"id":"a","label":"Service","kind":"class","source_file":"app/services/a.py","source_location":"L2","qualified_name":"app.services.Service","community":0},
                {"id":"b","label":"Client","kind":"function","source_file":"other/b.py","community":0},
                {"id":"c","label":"Repository","kind":"class","source_file":"app/services_extra/c.py","community":1},
                {"id":"d","label":"Design","kind":"document_section","source_file":"README.md","community":1}
            ], "links":[
                {"source":"b","target":"a","relation":"calls","confidence":"EXTRACTED"},
                {"source":"b","target":"a","relation":"calls","confidence":"EXTRACTED"},
                {"source":"c","target":"a","relation":"uses","confidence":"INFERRED"},
                {"source":"d","target":"a","relation":"mentions","confidence":"EXTRACTED"}
            ]
        }))?,
    )?;
    Ok(())
}

#[test]
fn scoped_views_use_path_and_module_boundaries() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fixture(directory.path())?;
    for scope in ["app/services", "module:app.services"] {
        let architecture = parsed(execute(
            directory.path(),
            &[
                "architecture",
                "--graph",
                "graph.json",
                "--scope",
                scope,
                "--format",
                "json",
            ],
        )?)?;
        assert_eq!(architecture["schema"], "compass.architecture.scoped-view/1");
        assert_eq!(architecture["selection"]["selectedNodes"], 1);
        assert_eq!(architecture["selection"]["boundaryRelationships"], 4);
        let community = parsed(execute(
            directory.path(),
            &[
                "community",
                "0",
                "--graph",
                "graph.json",
                "--scope",
                scope,
                "--format",
                "json",
            ],
        )?)?;
        assert_eq!(community["communities"][0]["members"], json!(["a"]));
        assert_eq!(community["selection"]["omittedNodes"], 3);
    }
    fs::write(
        directory.path().join("analysis.json"),
        serde_json::to_vec(&json!({"communities":{"0":["b"],"1":["a"]}}))?,
    )?;
    let pinned = parsed(execute(
        directory.path(),
        &[
            "community",
            "0",
            "--graph",
            "graph.json",
            "--scope",
            "app/services",
            "--format",
            "json",
        ],
    )?)?;
    assert_eq!(pinned["communities"][0]["members"], json!(["a"]));
    let empty = parsed(execute(
        directory.path(),
        &[
            "architecture",
            "--graph",
            "graph.json",
            "--scope",
            "absent",
            "--format",
            "json",
        ],
    )?)?;
    assert_eq!(empty["selection"]["selectedNodes"], 0);
    let invalid_architecture = execute(
        directory.path(),
        &[
            "architecture",
            "--graph",
            "missing.json",
            "--scope",
            "../outside",
        ],
    )?;
    assert_eq!(invalid_architecture.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid_architecture.stderr).contains("invalid scope"));
    let invalid = execute(directory.path(), &["hotspots", "--scope", "../outside"])?;
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("relative path"));
    Ok(())
}

#[test]
fn hotspot_counts_keep_optional_evidence_and_global_contacts_explicit() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    fixture(directory.path())?;
    let args = [
        "hotspots",
        "--graph",
        "graph.json",
        "--scope",
        "app/services",
        "--format",
        "json",
    ];
    let base = parsed(execute(directory.path(), &args)?)?;
    assert_eq!(base["schema"], "compass.hotspots/1");
    assert_eq!(base["mostConnected"][0]["connections"], 1);
    assert_eq!(base["mostConnected"][0]["relationshipRecords"], 2);
    assert_eq!(base["excludedRelationshipRecords"], 2);
    assert_eq!(base, parsed(execute(directory.path(), &args)?)?);
    let all = parsed(execute(
        directory.path(),
        &[
            "hotspots",
            "--graph",
            "graph.json",
            "--scope",
            "app/services",
            "--include-inferred",
            "--include-documents",
            "--format",
            "json",
        ],
    )?)?;
    assert_eq!(all["mostConnected"][0]["connections"], 3);
    assert_eq!(all["mostDependedOn"][0]["dependents"], 3);
    assert_eq!(all["mostConnected"][0]["layers"]["inferred"], 1);
    assert_eq!(all["mostConnected"][0]["layers"]["document"], 1);
    assert_eq!(all["mostConnected"][0]["sourceLocation"], "L2");
    let capped = execute(
        directory.path(),
        &["hotspots", "--graph", "graph.json", "--budget", "200"],
    )?;
    assert!(capped.status.success());
    assert!(
        token_count(&String::from_utf8(capped.stdout)?)
            .saturating_add(token_count(&String::from_utf8(capped.stderr)?))
            <= 200
    );
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join(".unused-global-config"))
        .args(["-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
        .arg("-c")
        .arg(format!(
            "core.hooksPath={}",
            root.join(".no-hooks").display()
        ))
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

#[test]
fn freshness_follows_the_recorded_root_and_is_rechecked_on_cached_queries()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let graph_path = support::write_typed_graph(root)?;
    fs::write(
        root.join(".gitignore"),
        "graph.json\nsource-root.txt\ncache/\n.compass/\n",
    )?;
    git(root, &["init"])?;
    git(root, &["config", "user.name", "Fixture"])?;
    git(root, &["config", "user.email", "fixture@example.invalid"])?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-m", "fixture"])?;
    let built = git(root, &["rev-parse", "HEAD"])?;
    let mut graph: Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    graph["graph"]["build"]["sourceCommit"] = built.clone().into();
    fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    fs::write(
        root.join("source-root.txt"),
        fs::canonicalize(root)?.to_string_lossy().as_bytes(),
    )?;
    let elsewhere = tempfile::tempdir()?;
    let path = graph_path.to_str().ok_or("path encoding")?;
    let args = ["callers", "Target", "--graph", path, "--format", "json"];
    let current = execute(elsewhere.path(), &args)?;
    assert!(
        current.status.success(),
        "{}",
        String::from_utf8_lossy(&current.stderr)
    );
    assert!(!String::from_utf8_lossy(&current.stderr).contains("Graph built"));
    fs::write(root.join("src/lib.rs"), "changed")?;
    fs::write(root.join("new.py"), "pass")?;
    for _ in 0..2 {
        let dirty = execute(elsewhere.path(), &args)?;
        assert!(
            dirty.status.success(),
            "{}",
            String::from_utf8_lossy(&dirty.stderr)
        );
        let stderr = String::from_utf8(dirty.stderr)?;
        assert!(
            stderr.contains("2 files differ; 2 uncommitted files"),
            "{stderr}"
        );
        assert!(stderr.contains(root.to_str().ok_or("root")?), "{stderr}");
        let _: Value = serde_json::from_slice(&dirty.stdout)?;
    }
    for _ in 0..2 {
        let path_query = execute(
            elsewhere.path(),
            &["path", "Caller", "Target", "--graph", path],
        )?;
        assert!(
            path_query.status.success(),
            "{}",
            String::from_utf8_lossy(&path_query.stderr)
        );
        assert!(
            String::from_utf8_lossy(&path_query.stderr)
                .contains("2 files differ; 2 uncommitted files")
        );
    }
    git(root, &["add", "."])?;
    git(root, &["commit", "-m", "change"])?;
    let stale = execute(elsewhere.path(), &args)?;
    assert!(String::from_utf8_lossy(&stale.stderr).contains(&built[..12]));
    let capped = execute(
        elsewhere.path(),
        &["callers", "Target", "--graph", path, "--budget", "200"],
    )?;
    assert!(capped.status.success());
    assert!(
        token_count(&String::from_utf8(capped.stdout)?)
            .saturating_add(token_count(&String::from_utf8(capped.stderr)?))
            <= 200
    );
    git(
        root,
        &[
            "config",
            "filter.spy.clean",
            "echo invoked > freshness-filter-ran",
        ],
    )?;
    fs::write(root.join(".gitattributes"), "src/lib.rs filter=spy\n")?;
    fs::write(root.join("src/lib.rs"), "filtered change")?;
    let filtered = execute(elsewhere.path(), &args)?;
    assert!(filtered.status.success());
    assert!(String::from_utf8_lossy(&filtered.stderr).contains("conversion filters"));
    assert!(!root.join("freshness-filter-ran").exists());
    let _: Value = serde_json::from_slice(&filtered.stdout)?;
    fs::remove_file(root.join("source-root.txt"))?;
    assert!(
        !String::from_utf8_lossy(&execute(elsewhere.path(), &args)?.stderr).contains("Graph built")
    );
    Ok(())
}

#[test]
fn unavailable_program_and_graph_return_recovery_commands() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let missing = execute(directory.path(), &["program", "show", "run"])?;
    assert_eq!(missing.status.code(), Some(3));
    let stderr = String::from_utf8(missing.stderr)?;
    assert!(stderr.contains("Program IR unavailable"));
    assert!(stderr.contains("compass update --program"));
    assert!(stderr.contains("--program PATH"));
    fs::write(
        directory.path().join("lib.rs"),
        "pub fn run(value: usize) -> usize { value }\n",
    )?;
    let repair = execute(
        directory.path(),
        &["update", "--program", "--no-cluster", "--no-viz"],
    )?;
    assert!(
        repair.status.success(),
        "{}",
        String::from_utf8_lossy(&repair.stderr)
    );
    let functions = execute(
        directory.path(),
        &["program", "functions", "--format", "json"],
    )?;
    assert!(
        functions.status.success(),
        "{}",
        String::from_utf8_lossy(&functions.stderr)
    );
    assert!(String::from_utf8_lossy(&functions.stdout).contains("run"));
    let signature = execute(
        directory.path(),
        &["program", "show", "run", "--format", "json"],
    )?;
    assert!(
        signature.status.success(),
        "{}",
        String::from_utf8_lossy(&signature.stderr)
    );
    let unknown = execute(directory.path(), &["program", "unknown"])?;
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("compass help program"));
    let missing_graph = execute(
        directory.path(),
        &["callers", "Target", "--graph", "missing.json"],
    )?;
    assert!(!missing_graph.status.success());
    assert!(String::from_utf8_lossy(&missing_graph.stderr).contains("compass ensure"));
    Ok(())
}
