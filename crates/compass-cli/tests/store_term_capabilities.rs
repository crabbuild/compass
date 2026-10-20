use std::error::Error;
use std::process::Command;

use compass_files::BuildGuard;
use compass_model::code_graph::GraphDocument;
use compass_store::STORE_REF_FILE_NAME;
use serde_json::Value;

#[test]
fn source_terms_cannot_collide_with_store_capability_records() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    std::fs::write(
        root.path().join("lib.rs"),
        r#"pub const EXPLANATION: &str = "__compass_cap_identifier_subwords_v1__";
pub mod __compass_cap_operation_role_terms_v1__ {
    pub struct Builder;
}
pub struct __compass_cap_declaration_terms_v1__;
pub fn __compass_cap_relationship_terms_v1__() {}
pub fn __compass_cap_relationship_search_terms_v1__() {}
pub fn sha256_hex() {}
pub fn caller() {
    __compass_cap_relationship_terms_v1__();
    __compass_cap_relationship_search_terms_v1__();
}
"#,
    )?;
    let build = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--code-only",
            "--no-cluster",
            "--no-viz",
            "--store",
            "sqlite",
            "--max-workers",
            "1",
        ])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let graph_path = BuildGuard::resolve_artifact(&root.path().join("compass-out"), "graph.json")?;
    assert!(
        graph_path
            .parent()
            .ok_or("published graph parent missing")?
            .join(STORE_REF_FILE_NAME)
            .is_file()
    );
    let graph = GraphDocument::load(&graph_path)?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "EXPLANATION")
        .ok_or("source constant missing from published graph")?;
    let answer = Command::new(env!("CARGO_BIN_EXE_compass"))
        .arg("query")
        .arg(&target.id)
        .arg("--graph")
        .arg(&graph_path)
        .args(["--view", "ranked", "--format", "json"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        answer.status.success(),
        "{}",
        String::from_utf8_lossy(&answer.stderr)
    );
    let value: Value = serde_json::from_slice(&answer.stdout)?;
    assert_eq!(value["schema"], "compass.query.ranked-view/4");
    assert_eq!(value["candidates"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["candidates"][0]["id"], target.id);
    Ok(())
}

#[test]
fn ranked_cli_quotes_published_code_for_both_storage_modes() -> Result<(), Box<dyn Error>> {
    for storage in ["json", "sqlite"] {
        let root = tempfile::tempdir()?;
        let source = "def opaque(items):\n    return normalize_payload(items)\n";
        std::fs::write(root.path().join("sample.py"), source)?;
        let build = Command::new(env!("CARGO_BIN_EXE_compass"))
            .args([
                "update",
                ".",
                "--code-only",
                "--no-cluster",
                "--no-viz",
                "--store",
                storage,
                "--max-workers",
                "1",
            ])
            .current_dir(root.path())
            .env_remove("COMPASS_OUT")
            .output()?;
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let graph = BuildGuard::resolve_artifact(&root.path().join("compass-out"), "graph.json")?;
        let execute = |extra: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_compass"))
                .args(["query", "where is payload normalized", "--view", "ranked"])
                .arg("--graph")
                .arg(&graph)
                .args(extra)
                .current_dir(root.path())
                .env_remove("COMPASS_OUT")
                .output()
        };
        let answer = execute(&["--format", "json"])?;
        assert!(
            answer.status.success(),
            "{}",
            String::from_utf8_lossy(&answer.stderr)
        );
        let value: Value = serde_json::from_slice(&answer.stdout)?;
        assert_eq!(value["schema"], "compass.query.ranked-view/4");
        let candidate = &value["candidates"][0];
        assert!(
            candidate["name"]
                .as_str()
                .is_some_and(|name| name.contains("opaque"))
        );
        let quotes = candidate["excerpts"]
            .as_array()
            .ok_or("missing code excerpts")?;
        assert!(quotes.iter().any(|quote| {
            quote["kind"] == "callable_code"
                && quote["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("normalize_payload(items)"))
        }));
        for quote in quotes {
            let anchor = &quote["source"];
            let start = anchor["startByte"].as_u64().ok_or("start byte")? as usize;
            let end = anchor["endByte"].as_u64().ok_or("end byte")? as usize;
            assert_eq!(source.get(start..end), quote["text"].as_str());
        }
        std::fs::write(
            root.path().join("sample.py"),
            "# source changed after graph publication\n",
        )?;
        let pinned = execute(&["--format", "json"])?;
        assert!(pinned.status.success());
        let pinned: Value = serde_json::from_slice(&pinned.stdout)?;
        assert_eq!(pinned["candidates"], value["candidates"]);
        let text = execute(&["--text-budget", "1600"])?;
        assert!(
            text.status.success(),
            "{}",
            String::from_utf8_lossy(&text.stderr)
        );
        let rendered = String::from_utf8(text.stdout)?;
        assert!(rendered.contains("Code @"));
        assert!(rendered.contains("normalize_payload(items)"));
        assert!(rendered.len() < 1600 * 4);
    }
    Ok(())
}
