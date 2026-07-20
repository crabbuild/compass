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
    use trail_files::{
        Cache, CacheKind, DetectOptions, Manifest, ManifestKind, detect, file_hash,
        prompt_fingerprint,
    };
    use trail_languages::Engine;

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

    #[test]
    fn deterministic_files_match_python() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join("src"))?;
        fs::create_dir_all(root.join("vendor/nested"))?;
        fs::create_dir(root.join("secrets"))?;
        fs::write(root.join("src/main.py"), "def main():\n    return 42\n")?;
        fs::write(
            root.join("src/tool"),
            "#!/usr/bin/env python3\nprint('ok')\n",
        )?;
        fs::write(root.join("README.md"), "# Project\n\nA small project.\n")?;
        fs::write(
            root.join("paper.md"),
            "# Abstract\nWe propose a method in this arXiv preprint. See [1].\\cite{x}\n",
        )?;
        fs::write(root.join("diagram.svg"), "<svg></svg>")?;
        fs::write(root.join("secrets/db.json"), "{\"token\": \"redacted\"}")?;
        fs::write(root.join("vendor/ignored.py"), "x = 1\n")?;
        fs::write(root.join("vendor/nested/keep.py"), "x = 2\n")?;
        fs::write(root.join("vendor/nested/debug.log"), "noise\n")?;
        fs::write(root.join("unknown.bin"), [0_u8, 1, 2])?;
        fs::write(root.join(".graphifyignore"), "vendor/ignored.py\n")?;
        fs::write(root.join("vendor/nested/.gitignore"), "*.log\n")?;

        let rust = serde_json::to_value(detect(root, &DetectOptions::default())?)?;
        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.detect import detect; print(json.dumps(detect(Path(sys.argv[1]))))",
            ])
            .arg(root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);

        let source = root.join("README.md");
        let rust_hash = file_hash(&source, root)?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import sys; from pathlib import Path; from graphify.cache import file_hash; print(file_hash(Path(sys.argv[1]), Path(sys.argv[2])))",
            ])
            .arg(&source)
            .arg(root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(rust_hash, String::from_utf8(output.stdout)?.trim());
        assert_eq!(prompt_fingerprint("hello  \r\nworld\r\n"), "26c60a61d01d");
        Ok(())
    }

    #[test]
    fn persisted_file_artifacts_cross_read_with_python() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = fs::canonicalize(directory.path())?;
        let source = root.join("src.py");
        fs::write(&source, "def trail():\n    return 1\n")?;
        let manifest_path = root.join("graphify-out/manifest.json");
        let source_string = source.to_string_lossy().into_owned();
        let mut buckets = std::collections::BTreeMap::new();
        buckets.insert("code".to_owned(), vec![source_string.clone()]);

        let mut manifest = Manifest::default();
        manifest.save(
            &buckets,
            &manifest_path,
            ManifestKind::Both,
            Some(&root),
            None,
            None,
        )?;

        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.detect import load_manifest; print(json.dumps(load_manifest(sys.argv[1], root=Path(sys.argv[2])), sort_keys=True))",
            ])
            .arg(&manifest_path)
            .arg(&root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python_manifest: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let python_entry = python_manifest
            .get(&source_string)
            .ok_or("Python did not load the Rust manifest entry")?;
        let rust_entry = manifest
            .entries()
            .get(&source_string)
            .ok_or("Rust manifest entry missing")?;
        assert_eq!(
            python_entry.get("ast_hash"),
            Some(&json!(rust_entry.ast_hash))
        );
        assert_eq!(
            python_entry.get("semantic_hash"),
            Some(&json!(rust_entry.semantic_hash))
        );

        let cached = json!({
            "nodes": [{"id": "trail", "source_file": source_string}],
            "edges": []
        });
        let mut cache = Cache::new(&root, None)?;
        cache.save(&source, &cached, &CacheKind::Ast, None)?;
        cache.flush()?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.cache import load_cached; print(json.dumps(load_cached(Path(sys.argv[1]), root=Path(sys.argv[2]), kind='ast'), sort_keys=True))",
            ])
            .arg(&source)
            .arg(&root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python_cache: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(python_cache, cached);
        Ok(())
    }

    #[test]
    fn python_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.py", "extract_python")
    }

    #[test]
    fn typescript_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.ts", "extract_js")
    }

    #[test]
    fn java_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.java", "extract_java")
    }

    #[test]
    fn go_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.go", "extract_go")
    }

    #[test]
    fn rust_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.rs", "extract_rust")
    }

    #[test]
    fn c_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.c", "extract_c")
    }

    #[test]
    fn ruby_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.rb", "extract_ruby")
    }

    #[test]
    fn kotlin_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.kt", "extract_kotlin")
    }
    fn compare_extraction(fixture: &str, extractor: &str) -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        let source = repo.join("tests/fixtures").join(fixture);
        let rust = serde_json::to_value(Engine::default().extract(&source)?)?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; import graphify.extract as e; print(json.dumps(getattr(e, sys.argv[1])(Path(sys.argv[2])), ensure_ascii=False))",
                extractor,
            ])
            .arg(&source)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python, "fixture: {fixture}");
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
