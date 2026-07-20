//! Development-only differential verification against the pinned Python baseline.

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::json;
    use tempfile::TempDir;
    use trail_cli::Frontend;

    #[test]
    fn read_commands_match_python_cli() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        for arguments in [
            vec![
                "query",
                "who calls extract",
                "--context",
                "call",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "path",
                "createPatchHandler",
                "validateSanitySession",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "path",
                "validateSanitySession",
                "createPatchHandler",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "explain",
                "validateSanitySession",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "affected",
                "validateSanitySession",
                "--relation",
                "calls",
                "--graph",
                fixture.graph_string(),
            ],
        ] {
            compare(&arguments)?;
        }
        Ok(())
    }

    fn compare(arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        let rust = trail_cli::run(
            Frontend::Graphify,
            arguments.iter().map(|argument| OsString::from(*argument)),
        );
        let repo = repository_root();
        let python = python_executable(&repo);
        let output = Command::new(&python)
            .arg("-m")
            .arg("graphify")
            .args(arguments)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .env("PYTHONHASHSEED", "0")
            .env("GRAPHIFY_QUERY_LOG_DISABLE", "1")
            .output()?;
        assert_eq!(
            rust.code,
            output.status.code().unwrap_or(1) as u8,
            "{arguments:?}"
        );
        assert_eq!(
            with_newline(&rust.stdout),
            String::from_utf8(output.stdout)?,
            "stdout mismatch for {arguments:?}"
        );
        assert_eq!(
            with_newline(&rust.stderr),
            String::from_utf8(output.stderr)?,
            "stderr mismatch for {arguments:?}"
        );
        Ok(())
    }

    fn with_newline(value: &str) -> String {
        if value.is_empty() {
            String::new()
        } else {
            format!("{value}\n")
        }
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
    }

    fn python_executable(repo: &Path) -> PathBuf {
        if let Ok(value) = std::env::var("GRAPHIFY_PYTHON") {
            return PathBuf::from(value);
        }
        if cfg!(windows) {
            repo.join(".venv/Scripts/python.exe")
        } else {
            repo.join(".venv/bin/python")
        }
    }

    struct Fixture {
        _directory: TempDir,
        graph: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = tempfile::tempdir()?;
            let graph = directory.path().join("graph.json");
            let document = json!({
                "directed": false,
                "multigraph": false,
                "graph": {},
                "nodes": [
                    {"id": "extract", "label": "extract", "source_file": "extract.py", "source_location": "L10", "community": 0},
                    {"id": "cluster", "label": "cluster", "source_file": "cluster.py", "source_location": "L5", "community": 0},
                    {"id": "build", "label": "build", "source_file": "build.py", "source_location": "L1", "community": 1},
                    {"id": "create", "label": "createPatchHandler()", "source_file": "create.ts", "source_location": "L2", "community": 2},
                    {"id": "validate", "label": "validateSanitySession()", "source_file": "validate.ts", "source_location": "L4", "community": 2}
                ],
                "links": [
                    {"source": "extract", "target": "cluster", "relation": "calls", "confidence": "EXTRACTED", "context": "call"},
                    {"source": "cluster", "target": "build", "relation": "imports", "confidence": "EXTRACTED", "context": "import"},
                    {"source": "create", "target": "validate", "relation": "calls", "confidence": "EXTRACTED", "context": "call"}
                ]
            });
            fs::write(&graph, serde_json::to_vec(&document)?)?;
            Ok(Self {
                _directory: directory,
                graph,
            })
        }

        fn graph_string(&self) -> &str {
            self.graph.to_str().unwrap_or_default()
        }
    }
}
