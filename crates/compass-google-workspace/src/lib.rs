//! Optional, bounded Google Workspace shortcut export through `gws`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use url::Url;
use wait_timeout::ChildExt;

pub const GOOGLE_WORKSPACE_EXTENSIONS: [&str; 3] = ["gdoc", "gsheet", "gslides"];
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_PROCESS_OUTPUT: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoogleShortcut {
    pub file_id: String,
    pub url: Option<String>,
    pub resource_key: Option<String>,
    pub account: Option<String>,
}

pub trait GwsExporter {
    fn export(
        &self,
        file_id: &str,
        mime_type: &str,
        output: &Path,
        resource_key: Option<&str>,
    ) -> Result<(), GoogleWorkspaceError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemGwsExporter;

impl GwsExporter for SystemGwsExporter {
    fn export(
        &self,
        file_id: &str,
        mime_type: &str,
        output: &Path,
        resource_key: Option<&str>,
    ) -> Result<(), GoogleWorkspaceError> {
        run_gws_export(file_id, mime_type, output, resource_key)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GoogleWorkspaceError {
    #[error("could not read Google Workspace shortcut {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("Google Workspace shortcut {0} does not include a Drive file ID")]
    MissingFileId(PathBuf),
    #[error(
        "gws is required for Google Workspace export. Install it from https://github.com/googleworkspace/cli and run `gws auth login -s drive`."
    )]
    MissingGws,
    #[error("could not start gws export for {file_id}: {source}")]
    Start {
        file_id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not wait for gws export for {file_id}: {source}")]
    Wait {
        file_id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("gws export timed out for {file_id} after {seconds} seconds")]
    Timeout { file_id: String, seconds: u64 },
    #[error("invalid GRAPHIFY_GOOGLE_WORKSPACE_TIMEOUT value: {0}")]
    InvalidTimeout(String),
    #[error("gws export output exceeded the {limit}-byte safety limit for {file_id}")]
    OutputTooLarge { file_id: String, limit: usize },
    #[error("gws export failed for {file_id}: {message}")]
    Export { file_id: String, message: String },
    #[error("Google Sheets export failed: {0}")]
    Sheet(String),
    #[error("could not create Google Workspace conversion directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create a temporary Google Workspace export in {path}: {source}")]
    TemporaryFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read Google Workspace export {path}: {source}")]
    ReadExport {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write Google Workspace sidecar {path}: {source}")]
    WriteSidecar {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[must_use]
pub fn is_google_workspace_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            GOOGLE_WORKSPACE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

#[must_use]
pub fn google_workspace_enabled(value: Option<&str>) -> bool {
    let raw = value
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("GRAPHIFY_GOOGLE_WORKSPACE").unwrap_or_default());
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn read_google_shortcut(path: &Path) -> Result<GoogleShortcut, GoogleWorkspaceError> {
    let text = fs::read_to_string(path).map_err(|error| GoogleWorkspaceError::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| GoogleWorkspaceError::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let data = value
        .as_object()
        .ok_or_else(|| GoogleWorkspaceError::Read {
            path: path.to_path_buf(),
            message: "shortcut root is not an object".to_owned(),
        })?;
    let url = data
        .get("url")
        .and_then(python_optional_string)
        .unwrap_or_default();
    let file_id = ["doc_id", "file_id", "fileId", "id"]
        .into_iter()
        .find_map(|key| data.get(key).and_then(python_optional_string))
        .or_else(|| extract_file_id_from_url(&url))
        .or_else(|| {
            data.get("resource_id")
                .and_then(python_optional_string)
                .and_then(|resource| resource.split_once(':').map(|(_, id)| id.to_owned()))
        })
        .filter(|id| !id.is_empty())
        .ok_or_else(|| GoogleWorkspaceError::MissingFileId(path.to_path_buf()))?;
    let resource_key = ["resource_key", "resourceKey"]
        .into_iter()
        .find_map(|key| data.get(key).and_then(python_optional_string))
        .or_else(|| query_value(&url, "resourcekey"));
    let account = data.get("email").and_then(python_optional_string);
    Ok(GoogleShortcut {
        file_id,
        url: (!url.is_empty()).then_some(url),
        resource_key,
        account,
    })
}

pub fn convert_google_workspace_file(
    path: &Path,
    out_dir: &Path,
) -> Result<Option<PathBuf>, GoogleWorkspaceError> {
    convert_google_workspace_file_with(path, out_dir, &SystemGwsExporter)
}

pub fn convert_google_workspace_file_with(
    path: &Path,
    out_dir: &Path,
    exporter: &impl GwsExporter,
) -> Result<Option<PathBuf>, GoogleWorkspaceError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !GOOGLE_WORKSPACE_EXTENSIONS.contains(&extension.as_str()) {
        return Ok(None);
    }
    let shortcut = read_google_shortcut(path)?;
    fs::create_dir_all(out_dir).map_err(|source| GoogleWorkspaceError::CreateDirectory {
        path: out_dir.to_path_buf(),
        source,
    })?;
    let output = sidecar_path(path, out_dir);
    let (suffix, mime_type) = match extension.as_str() {
        "gdoc" => (".md", "text/markdown"),
        "gslides" => (".txt", "text/plain"),
        "gsheet" => (
            ".xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        _ => return Ok(None),
    };
    let temporary = Builder::new()
        .suffix(suffix)
        .tempfile_in(out_dir)
        .map_err(|source| GoogleWorkspaceError::TemporaryFile {
            path: out_dir.to_path_buf(),
            source,
        })?;
    exporter.export(
        &shortcut.file_id,
        mime_type,
        temporary.path(),
        shortcut.resource_key.as_deref(),
    )?;
    let body = if extension == "gsheet" {
        compass_media::xlsx_to_markdown(temporary.path())
            .map_err(|error| GoogleWorkspaceError::Sheet(error.to_string()))?
    } else {
        fs::read(temporary.path())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|source| GoogleWorkspaceError::ReadExport {
                path: temporary.path().to_path_buf(),
                source,
            })?
    };
    if body.trim().is_empty() {
        return Ok(None);
    }
    fs::write(&output, with_frontmatter(path, &shortcut, &body, mime_type)).map_err(|source| {
        GoogleWorkspaceError::WriteSidecar {
            path: output.clone(),
            source,
        }
    })?;
    Ok(Some(output))
}

fn run_gws_export(
    file_id: &str,
    mime_type: &str,
    output: &Path,
    resource_key: Option<&str>,
) -> Result<(), GoogleWorkspaceError> {
    let _ = resource_key;
    let output = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let params = json!({"fileId": file_id, "mimeType": mime_type}).to_string();
    let mut child = Command::new("gws")
        .args(["drive", "files", "export", "--params", &params, "-o", name])
        .current_dir(parent)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                GoogleWorkspaceError::MissingGws
            } else {
                GoogleWorkspaceError::Start {
                    file_id: file_id.to_owned(),
                    source,
                }
            }
        })?;
    let stdout = child
        .stdout
        .take()
        .map(|stream| std::thread::spawn(move || drain(stream)));
    let stderr = child
        .stderr
        .take()
        .map(|stream| std::thread::spawn(move || drain(stream)));
    let seconds = match std::env::var("GRAPHIFY_GOOGLE_WORKSPACE_TIMEOUT") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| GoogleWorkspaceError::InvalidTimeout(value))?,
        Err(_) => DEFAULT_TIMEOUT_SECONDS,
    };
    let status = match child.wait_timeout(Duration::from_secs(seconds)) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_output(stdout);
            let _ = join_output(stderr);
            return Err(GoogleWorkspaceError::Timeout {
                file_id: file_id.to_owned(),
                seconds,
            });
        }
        Err(source) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_output(stdout);
            let _ = join_output(stderr);
            return Err(GoogleWorkspaceError::Wait {
                file_id: file_id.to_owned(),
                source,
            });
        }
    };
    let (stdout, stdout_truncated) = join_output(stdout);
    let (stderr, stderr_truncated) = join_output(stderr);
    if stdout_truncated || stderr_truncated {
        return Err(GoogleWorkspaceError::OutputTooLarge {
            file_id: file_id.to_owned(),
            limit: MAX_PROCESS_OUTPUT,
        });
    }
    if !status.success() {
        let selected = if stderr.is_empty() { stdout } else { stderr };
        let mut message = String::from_utf8_lossy(&selected).trim().to_owned();
        if message.len() > 1_200 {
            message.truncate(floor_char_boundary(&message, 1_200));
            message.push_str("...");
        }
        return Err(GoogleWorkspaceError::Export {
            file_id: file_id.to_owned(),
            message,
        });
    }
    Ok(())
}

fn drain(mut stream: impl Read) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 16 * 1024];
    while let Ok(read) = stream.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = MAX_PROCESS_OUTPUT.saturating_sub(kept.len());
        let copied = remaining.min(read);
        kept.extend_from_slice(&buffer[..copied]);
        truncated |= copied < read;
    }
    (kept, truncated)
}

fn join_output(handle: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_else(|| (Vec::new(), false))
}

fn sidecar_path(path: &Path, out_dir: &Path) -> PathBuf {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let digest = Sha256::digest(absolute.to_string_lossy().as_bytes());
    let hash = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    out_dir.join(format!("{stem}_{hash}.md"))
}

fn with_frontmatter(path: &Path, shortcut: &GoogleShortcut, body: &str, mime_type: &str) -> String {
    let account_line = shortcut
        .account
        .as_ref()
        .map_or_else(String::new, |account| {
            let digest = Sha256::digest(account.as_bytes());
            let hash = digest[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!("google_account_hash: \"{hash}\"\n")
        });
    format!(
        "---\nsource_file: \"{}\"\nsource_type: \"google_workspace\"\ngoogle_file_id: \"{}\"\ngoogle_export_mime_type: \"{}\"\nsource_url: \"{}\"\n{}---\n\n<!-- converted from Google Workspace shortcut: {} -->\n\n{}\n",
        safe_yaml(&path.to_string_lossy()),
        safe_yaml(&shortcut.file_id),
        safe_yaml(mime_type),
        safe_yaml(shortcut.url.as_deref().unwrap_or_default()),
        account_line,
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        body.trim()
    )
}

fn safe_yaml(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

fn extract_file_id_from_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if let Some(id) = url
        .query_pairs()
        .find_map(|(key, value)| (key == "id").then(|| value.into_owned()))
    {
        return Some(id);
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    segments.windows(3).find_map(|window| {
        (matches!(
            window[0],
            "document" | "spreadsheets" | "presentation" | "file"
        ) && window[1] == "d")
            .then(|| window[2].to_owned())
    })
}

fn query_value(value: &str, key: &str) -> Option<String> {
    Url::parse(value)
        .ok()?
        .query_pairs()
        .find_map(|(candidate, value)| (candidate == key).then(|| value.into_owned()))
}

fn python_optional_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) if value.is_empty() => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(false) => None,
        Value::Bool(true) => Some("True".to_owned()),
        Value::Number(value) if value.as_f64() == Some(0.0) => None,
        Value::Number(value) => Some(value.to_string()),
        Value::Array(values) if values.is_empty() => None,
        Value::Object(values) if values.is_empty() => None,
        Value::Array(_) | Value::Object(_) => Some(python_container_string(value)),
    }
}

fn python_container_string(value: &Value) -> String {
    match value {
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "'{}': {}",
                    key.replace('\'', "\\'"),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => python_repr(value),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(value) => format!("'{}'", value.replace('\'', "\\'")),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => python_container_string(value),
    }
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut boundary = index.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    boundary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_metadata_matches_supported_google_shapes() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let shortcut = directory.path().join("Budget.gsheet");
        fs::write(
            &shortcut,
            r#"{"url":"https://docs.google.com/spreadsheets/d/sheet-456/edit?resourcekey=key-1","email":"me@example.com"}"#,
        )?;
        assert_eq!(
            read_google_shortcut(&shortcut)?,
            GoogleShortcut {
                file_id: "sheet-456".to_owned(),
                url: Some(
                    "https://docs.google.com/spreadsheets/d/sheet-456/edit?resourcekey=key-1"
                        .to_owned()
                ),
                resource_key: Some("key-1".to_owned()),
                account: Some("me@example.com".to_owned()),
            }
        );
        Ok(())
    }

    #[test]
    fn enablement_values_match_python() {
        for enabled in ["1", "TRUE", " yes ", "On"] {
            assert!(google_workspace_enabled(Some(enabled)));
        }
        for disabled in ["", "0", "false", "enabled"] {
            assert!(!google_workspace_enabled(Some(disabled)));
        }
    }
}
