use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_google_workspace::{
    GoogleWorkspaceError, GwsExporter, convert_google_workspace_file_with,
};

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

struct MarkdownExporter;

impl GwsExporter for MarkdownExporter {
    fn export(
        &self,
        file_id: &str,
        mime_type: &str,
        output: &Path,
        resource_key: Option<&str>,
    ) -> Result<(), GoogleWorkspaceError> {
        assert_eq!(file_id, "doc-123");
        assert_eq!(mime_type, "text/markdown");
        assert_eq!(resource_key, Some("rk-1"));
        std::fs::write(output, "# Planning\n\nExported doc text.\n").map_err(|source| {
            GoogleWorkspaceError::WriteSidecar {
                path: output.to_path_buf(),
                source,
            }
        })
    }
}

#[test]
fn converted_markdown_matches_python_oracle_bytes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let shortcut = directory.path().join("Planning.gdoc");
    std::fs::write(
        &shortcut,
        r#"{"url":"https://docs.google.com/document/d/doc-123/edit?resourcekey=rk-1","email":"me@example.com"}"#,
    )?;
    let rust_dir = directory.path().join("rust");
    let python_dir = directory.path().join("python");
    let rust_path = convert_google_workspace_file_with(&shortcut, &rust_dir, &MarkdownExporter)?
        .ok_or("Rust conversion returned no sidecar")?;

    let repository = repository_root();
    let output = Command::new(repository.join(".venv/bin/python"))
        .env("PYTHONPATH", &repository)
        .args([
            "-c",
            "import sys; from pathlib import Path; import graphify.google_workspace as g; g._run_gws_export=lambda file_id,mime_type,output,resource_key=None: output.write_text('# Planning\\n\\nExported doc text.\\n',encoding='utf-8'); p=g.convert_google_workspace_file(Path(sys.argv[1]),Path(sys.argv[2])); print(p.name)",
        ])
        .arg(&shortcut)
        .arg(&python_dir)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let python_name = String::from_utf8(output.stdout)?.trim().to_owned();
    assert_eq!(
        rust_path.file_name().and_then(|value| value.to_str()),
        Some(python_name.as_str())
    );
    assert_eq!(
        std::fs::read(&rust_path)?,
        std::fs::read(python_dir.join(python_name))?
    );
    Ok(())
}
