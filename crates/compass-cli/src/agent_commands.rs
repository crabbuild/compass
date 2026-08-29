use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use compass_files::{DetectOptions, Manifest, ManifestKind, detect, write_bytes_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::distribution::{native_package, verified_harness_version};
use crate::install_commands::{
    agent_inventory, command_install, embedded_skill_files, managed_skill_collection_directories,
    managed_skill_directory, verify_managed_skill_collection,
};
use crate::{Frontend, Outcome};

const BUNDLE_SCHEMA: &str = "compass.agent-bundle/1";
const INVENTORY_SCHEMA: &str = "compass.agent-list/1";
const DOCTOR_SCHEMA: &str = "compass.agent-doctor/1";
const VALIDATION_SCHEMA: &str = "compass.agent-validation/1";
const GENERIC_MCP_SCHEMA: &str = "compass.agent-mcp-config/1";
const MCP_HTTP_URL: &str = "http://127.0.0.1:8080/mcp";
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const SKILL_NAMES: &[&str] = &[
    "compass",
    "compass-architecture",
    "compass-change-impact",
    "compass-debug",
    "compass-index-maintenance",
    "compass-mcp-setup",
    "compass-navigate",
];
const MCP_PLATFORMS: &[&str] = &["agents", "claude", "codex", "opencode"];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, Deserialize, Serialize)]
struct BundleManifest {
    schema: String,
    compass_version: String,
    platform: String,
    transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness_version: Option<String>,
    files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct ValidationFinding {
    path: String,
    rule: String,
}

#[derive(Debug, Serialize)]
struct ValidationReport {
    schema: &'static str,
    valid: bool,
    findings: Vec<ValidationFinding>,
}

#[derive(Clone, Debug, Serialize)]
struct DoctorCheck {
    id: &'static str,
    status: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    schema: &'static str,
    platform: String,
    healthy: bool,
    checks: Vec<DoctorCheck>,
}

pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return usage_error("error: missing agent subcommand");
    };
    match subcommand {
        "list" => command_list(&args[1..]),
        "install" => command_install(frontend, &args[1..]),
        "doctor" => command_doctor(&args[1..]),
        "export" => command_export(&args[1..]),
        "validate" => command_validate(&args[1..]),
        "mcp-config" => command_mcp_config(&args[1..]),
        unknown => usage_error(&format!("error: unknown agent subcommand '{unknown}'")),
    }
}

fn command_list(args: &[String]) -> Outcome {
    let format = match parse_format_only(args) {
        Ok(format) => format,
        Err(error) => return usage_error(&error),
    };
    let inventory = match agent_inventory() {
        Ok(inventory) => inventory,
        Err(error) => return Outcome::failure(error),
    };
    match format {
        OutputFormat::Text => {
            let lines = inventory
                .iter()
                .map(|agent| {
                    format!(
                        "{}\t{}\t{}",
                        agent.id,
                        agent.support,
                        agent.config_paths.join(",")
                    )
                })
                .collect::<Vec<_>>();
            Outcome::success(lines.join("\n"))
        }
        OutputFormat::Json => {
            let agents = inventory
                .iter()
                .map(|agent| {
                    json!({
                        "id": agent.id,
                        "aliases": agent.aliases,
                        "support": agent.support,
                        "commands": agent.commands,
                        "config_paths": agent.config_paths,
                        "project_skill": agent.project_skill,
                        "user_skill": agent.user_skill,
                        "documentation_url": agent.documentation_url,
                        "verified_on": agent.verified_on,
                    })
                })
                .collect::<Vec<_>>();
            json_success(&json!({"schema": INVENTORY_SCHEMA, "agents": agents}))
        }
    }
}

fn command_mcp_config(args: &[String]) -> Outcome {
    let (platform, transport) = match parse_mcp_options(args) {
        Ok(options) => options,
        Err(error) => return usage_error(&error),
    };
    match render_mcp_config(&platform, transport) {
        Ok(config) => Outcome::success(config),
        Err(error) => usage_error(&error),
    }
}

fn command_export(args: &[String]) -> Outcome {
    let (platform, output, transport, format) = match parse_export_options(args) {
        Ok(options) => options,
        Err(error) => return usage_error(&error),
    };
    match export_bundle(&platform, transport, &output) {
        Ok(manifest) => match format {
            OutputFormat::Text => Outcome::success(format!(
                "exported {} files for {} to {}",
                manifest.files.len(),
                platform,
                output.display()
            )),
            OutputFormat::Json => match serde_json::to_value(&manifest) {
                Ok(value) => json_success(&value),
                Err(error) => {
                    Outcome::failure(format!("error: could not encode export report: {error}"))
                }
            },
        },
        Err(error) => Outcome::failure(error),
    }
}

fn command_validate(args: &[String]) -> Outcome {
    let (path, platform, format) = match parse_validate_options(args) {
        Ok(options) => options,
        Err(error) => return usage_error(&error),
    };
    let report = validate_path(&path, platform.as_deref());
    let code = u8::from(!report.valid);
    match format {
        OutputFormat::Text => {
            let stdout = if report.valid {
                format!("valid agent bundle: {}", path.display())
            } else {
                report
                    .findings
                    .iter()
                    .map(|finding| format!("{}: {}", finding.path, finding.rule))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            outcome(code, stdout, String::new())
        }
        OutputFormat::Json => match serde_json::to_string_pretty(&report) {
            Ok(stdout) => outcome(code, stdout, String::new()),
            Err(error) => Outcome::failure(format!("error: could not encode validation: {error}")),
        },
    }
}

fn command_doctor(args: &[String]) -> Outcome {
    let (platform, root, user, format) = match parse_doctor_options(args) {
        Ok(options) => options,
        Err(error) => return usage_error(&error),
    };
    let report = doctor(&platform, &root, user);
    let code = u8::from(!report.healthy);
    match format {
        OutputFormat::Text => {
            let stdout = report
                .checks
                .iter()
                .map(|check| format!("{}\t{}\t{}", check.status, check.id, check.detail))
                .collect::<Vec<_>>()
                .join("\n");
            outcome(code, stdout, String::new())
        }
        OutputFormat::Json => match serde_json::to_string_pretty(&report) {
            Ok(stdout) => outcome(code, stdout, String::new()),
            Err(error) => {
                Outcome::failure(format!("error: could not encode doctor report: {error}"))
            }
        },
    }
}

fn parse_format_only(args: &[String]) -> Result<OutputFormat, String> {
    let mut format = OutputFormat::Text;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                index += 1;
                format = parse_format(required_value(args, index, "--format")?)?;
            }
            value if value.starts_with("--format=") => format = parse_format(&value[9..])?,
            value => return Err(format!("error: unknown agent list option '{value}'")),
        }
        index += 1;
    }
    Ok(format)
}

fn parse_mcp_options(args: &[String]) -> Result<(String, Transport), String> {
    let mut platform = None;
    let mut transport = Transport::Stdio;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--platform" => {
                index += 1;
                platform = Some(required_value(args, index, "--platform")?.to_owned());
            }
            "--transport" => {
                index += 1;
                transport = parse_transport(required_value(args, index, "--transport")?)?;
            }
            value if value.starts_with("--platform=") => platform = Some(value[11..].to_owned()),
            value if value.starts_with("--transport=") => {
                transport = parse_transport(&value[12..])?;
            }
            value => return Err(format!("error: unknown agent mcp-config option '{value}'")),
        }
        index += 1;
    }
    let platform = platform.ok_or_else(|| "error: --platform is required".to_owned())?;
    validate_mcp_platform(&platform)?;
    Ok((platform, transport))
}

fn parse_export_options(
    args: &[String],
) -> Result<(String, PathBuf, Transport, OutputFormat), String> {
    let mut platform = None;
    let mut output = None;
    let mut transport = Transport::Stdio;
    let mut format = OutputFormat::Text;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--platform" => {
                index += 1;
                platform = Some(required_value(args, index, "--platform")?.to_owned());
            }
            "--out" => {
                index += 1;
                output = Some(PathBuf::from(required_value(args, index, "--out")?));
            }
            "--transport" => {
                index += 1;
                transport = parse_transport(required_value(args, index, "--transport")?)?;
            }
            "--format" => {
                index += 1;
                format = parse_format(required_value(args, index, "--format")?)?;
            }
            value if value.starts_with("--platform=") => platform = Some(value[11..].to_owned()),
            value if value.starts_with("--out=") => output = Some(PathBuf::from(&value[6..])),
            value if value.starts_with("--transport=") => {
                transport = parse_transport(&value[12..])?;
            }
            value if value.starts_with("--format=") => format = parse_format(&value[9..])?,
            value => return Err(format!("error: unknown agent export option '{value}'")),
        }
        index += 1;
    }
    let platform = platform.ok_or_else(|| "error: --platform is required".to_owned())?;
    validate_mcp_platform(&platform)?;
    let output = output.ok_or_else(|| "error: --out is required".to_owned())?;
    Ok((platform, output, transport, format))
}

fn parse_validate_options(
    args: &[String],
) -> Result<(PathBuf, Option<String>, OutputFormat), String> {
    let mut path = None;
    let mut platform = None;
    let mut format = OutputFormat::Text;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                index += 1;
                path = Some(PathBuf::from(required_value(args, index, "--path")?));
            }
            "--platform" => {
                index += 1;
                platform = Some(required_value(args, index, "--platform")?.to_owned());
            }
            "--format" => {
                index += 1;
                format = parse_format(required_value(args, index, "--format")?)?;
            }
            value if value.starts_with("--path=") => path = Some(PathBuf::from(&value[7..])),
            value if value.starts_with("--platform=") => platform = Some(value[11..].to_owned()),
            value if value.starts_with("--format=") => format = parse_format(&value[9..])?,
            value => return Err(format!("error: unknown agent validate option '{value}'")),
        }
        index += 1;
    }
    if let Some(platform) = platform.as_deref() {
        validate_mcp_platform(platform)?;
    }
    Ok((
        path.ok_or_else(|| "error: --path is required".to_owned())?,
        platform,
        format,
    ))
}

fn parse_doctor_options(args: &[String]) -> Result<(String, PathBuf, bool, OutputFormat), String> {
    let mut platform = None;
    let mut root = PathBuf::from(".");
    let mut user = false;
    let mut selected_scope = None;
    let mut format = OutputFormat::Text;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--platform" => {
                index += 1;
                platform = Some(required_value(args, index, "--platform")?.to_owned());
            }
            "--project" | "--project-root" => {
                if let Some(selected) = selected_scope {
                    return Err(if selected == "project" {
                        "error: duplicate project root option".to_owned()
                    } else {
                        "error: project and user roots cannot be used together".to_owned()
                    });
                }
                index += 1;
                root = PathBuf::from(required_value(args, index, args[index - 1].as_str())?);
                selected_scope = Some("project");
            }
            "--user" | "--user-root" => {
                if let Some(selected) = selected_scope {
                    return Err(if selected == "user" {
                        "error: duplicate user root option".to_owned()
                    } else {
                        "error: project and user roots cannot be used together".to_owned()
                    });
                }
                index += 1;
                root = PathBuf::from(required_value(args, index, args[index - 1].as_str())?);
                user = true;
                selected_scope = Some("user");
            }
            "--format" => {
                index += 1;
                format = parse_format(required_value(args, index, "--format")?)?;
            }
            value if value.starts_with("--platform=") => platform = Some(value[11..].to_owned()),
            value if value.starts_with("--project=") || value.starts_with("--project-root=") => {
                if let Some(selected) = selected_scope {
                    return Err(if selected == "project" {
                        "error: duplicate project root option".to_owned()
                    } else {
                        "error: project and user roots cannot be used together".to_owned()
                    });
                }
                root = PathBuf::from(inline_option_value(value)?);
                selected_scope = Some("project");
            }
            value if value.starts_with("--user=") || value.starts_with("--user-root=") => {
                if let Some(selected) = selected_scope {
                    return Err(if selected == "user" {
                        "error: duplicate user root option".to_owned()
                    } else {
                        "error: project and user roots cannot be used together".to_owned()
                    });
                }
                root = PathBuf::from(inline_option_value(value)?);
                user = true;
                selected_scope = Some("user");
            }
            value if value.starts_with("--format=") => format = parse_format(&value[9..])?,
            value => return Err(format!("error: unknown agent doctor option '{value}'")),
        }
        index += 1;
    }
    let platform = platform.ok_or_else(|| "error: --platform is required".to_owned())?;
    validate_mcp_platform(&platform)?;
    Ok((platform, root, user, format))
}

fn required_value<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("error: {option} requires a value"))
}

fn inline_option_value(value: &str) -> Result<&str, String> {
    let (option, value) = value
        .split_once('=')
        .ok_or_else(|| "error: invalid inline option".to_owned())?;
    if value.is_empty() {
        Err(format!("error: {option} requires a value"))
    } else {
        Ok(value)
    }
}

fn parse_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        _ => Err(format!(
            "error: unknown output format '{value}'; use text or json"
        )),
    }
}

fn parse_transport(value: &str) -> Result<Transport, String> {
    match value {
        "stdio" => Ok(Transport::Stdio),
        "http" => Ok(Transport::Http),
        _ => Err(format!(
            "error: unsupported transport '{value}'; use stdio or http"
        )),
    }
}

fn validate_mcp_platform(platform: &str) -> Result<(), String> {
    if MCP_PLATFORMS.contains(&platform) {
        Ok(())
    } else {
        Err(format!(
            "error: unsupported MCP platform '{platform}'; choose from {}",
            MCP_PLATFORMS.join(", ")
        ))
    }
}

fn render_mcp_config(platform: &str, transport: Transport) -> Result<String, String> {
    validate_mcp_platform(platform)?;
    let value = match (platform, transport) {
        ("codex", Transport::Stdio) => {
            return Ok("[mcp_servers.compass]\ncommand = \"compass\"\nargs = [\"serve\", \"--transport\", \"stdio\"]".to_owned());
        }
        ("codex", Transport::Http) => {
            return Ok(format!("[mcp_servers.compass]\nurl = \"{MCP_HTTP_URL}\""));
        }
        ("claude", Transport::Stdio) => json!({
            "mcpServers": {"compass": {"command": "compass", "args": ["serve", "--transport", "stdio"]}}
        }),
        ("claude", Transport::Http) => json!({
            "mcpServers": {"compass": {"type": "http", "url": MCP_HTTP_URL}}
        }),
        ("opencode", Transport::Stdio) => json!({
            "mcp": {"compass": {"type": "local", "command": ["compass", "serve", "--transport", "stdio"], "enabled": true}}
        }),
        ("opencode", Transport::Http) => json!({
            "mcp": {"compass": {"type": "remote", "url": MCP_HTTP_URL, "enabled": true}}
        }),
        ("agents", Transport::Stdio) => json!({
            "schema": GENERIC_MCP_SCHEMA,
            "mcpServers": {"compass": {"command": "compass", "args": ["serve", "--transport", "stdio"]}}
        }),
        ("agents", Transport::Http) => json!({
            "schema": GENERIC_MCP_SCHEMA,
            "mcpServers": {"compass": {"transport": "http", "url": MCP_HTTP_URL}}
        }),
        _ => return Err(format!("error: unsupported MCP platform '{platform}'")),
    };
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("error: could not encode MCP configuration: {error}"))
}

fn export_bundle(
    platform: &str,
    transport: Transport,
    destination: &Path,
) -> Result<BundleManifest, String> {
    let destination_permissions = if destination.exists() {
        let metadata = fs::symlink_metadata(destination).map_err(|error| {
            format!(
                "error: could not inspect export destination {}: {error}",
                destination.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "error: export destination {} must be a directory and not a symbolic link",
                destination.display()
            ));
        }
        let mut entries = fs::read_dir(destination).map_err(|error| {
            format!(
                "error: could not inspect export destination {}: {error}",
                destination.display()
            )
        })?;
        if entries
            .next()
            .transpose()
            .map_err(|error| {
                format!(
                    "error: could not inspect export destination {}: {error}",
                    destination.display()
                )
            })?
            .is_some()
        {
            return Err(format!(
                "error: export destination {} is not empty",
                destination.display()
            ));
        }
        Some(metadata.permissions())
    } else {
        None
    };
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "error: could not create export parent {}: {error}",
            parent.display()
        )
    })?;
    let stage_directory = tempfile::Builder::new()
        .prefix(".compass-agent-stage-")
        .tempdir_in(parent)
        .map_err(|error| format!("error: could not create export staging directory: {error}"))?;
    let stage = stage_directory.path();
    let result = (|| {
        let mut files = BTreeMap::new();
        for asset in embedded_skill_files() {
            write_bundle_file(stage, &asset.path, asset.bytes)?;
            files.insert(asset.path, digest_bytes(asset.bytes));
        }
        let harness_version = if matches!(platform, "codex" | "claude" | "opencode") {
            let package = native_package(platform, transport_name(transport))?;
            for (path, bytes) in package.files {
                write_bundle_file(stage, &path, &bytes)?;
                files.insert(path, digest_bytes(&bytes));
            }
            Some(verified_harness_version(platform)?)
        } else {
            let config_path = bundle_config_path(platform).to_owned();
            let config = render_mcp_config(platform, transport)?;
            write_bundle_file(stage, &config_path, config.as_bytes())?;
            files.insert(config_path.clone(), digest_bytes(config.as_bytes()));
            None
        };
        let manifest = BundleManifest {
            schema: BUNDLE_SCHEMA.to_owned(),
            compass_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: platform.to_owned(),
            transport: transport_name(transport).to_owned(),
            harness_version,
            files,
        };
        let mut encoded = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("error: could not encode export manifest: {error}"))?;
        encoded.push(b'\n');
        write_bundle_file(stage, "manifest.json", &encoded)?;
        let validation = validate_export_bundle(stage, Some(platform));
        if !validation.valid {
            let rules = validation
                .findings
                .iter()
                .map(|finding| format!("{}: {}", finding.path, finding.rule))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "error: staged agent bundle failed validation: {rules}"
            ));
        }
        Ok(manifest)
    })();
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            return Err(error);
        }
    };
    if let Some(permissions) = &destination_permissions {
        fs::set_permissions(stage, permissions.clone()).map_err(|error| {
            format!(
                "error: could not preserve export destination permissions for {}: {error}",
                destination.display()
            )
        })?;
    }
    let destination_was_empty = destination_permissions.is_some();
    if destination_was_empty {
        fs::remove_dir(destination).map_err(|error| {
            format!(
                "error: could not prepare empty export destination {}: {error}",
                destination.display()
            )
        })?;
    }
    let stage = stage_directory.keep();
    if let Err(error) = fs::rename(&stage, destination) {
        let _cleanup = fs::remove_dir_all(&stage);
        let restore_error = destination_permissions.and_then(|permissions| {
            fs::create_dir(destination)
                .and_then(|()| fs::set_permissions(destination, permissions))
                .err()
        });
        let restore_detail = restore_error.map_or_else(String::new, |restore| {
            format!("; could not restore the original empty destination: {restore}")
        });
        return Err(format!(
            "error: could not publish export {}: {error}{restore_detail}",
            destination.display()
        ));
    }
    Ok(manifest)
}

fn write_bundle_file(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let path = safe_relative_path(root, relative)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("error: could not create {}: {error}", parent.display()))?;
    }
    write_bytes_atomic(&path, bytes)
        .map_err(|error| format!("error: could not write {}: {error}", path.display()))
}

fn validate_path(root: &Path, expected_platform: Option<&str>) -> ValidationReport {
    let mut report = if root.join("manifest.json").is_file() {
        validate_export_bundle(root, expected_platform)
    } else if root.join(".compass-install.json").is_file() {
        if expected_platform.is_some() {
            validation_report(vec![ValidationFinding {
                path: display_relative(root, root),
                rule: "--platform applies only to exported bundles".to_owned(),
            }])
        } else {
            validate_managed_bundle(root)
        }
    } else {
        ValidationReport {
            schema: VALIDATION_SCHEMA,
            valid: false,
            findings: vec![ValidationFinding {
                path: display_relative(root, root),
                rule: "missing bundle or managed install manifest".to_owned(),
            }],
        }
    };
    report.findings.sort();
    report.findings.dedup();
    report.valid = report.findings.is_empty();
    report
}

fn validate_export_bundle(root: &Path, expected_platform: Option<&str>) -> ValidationReport {
    let mut findings = Vec::new();
    let files = match collect_files_bounded(root) {
        Ok(files) => files,
        Err(rule) => {
            findings.push(ValidationFinding {
                path: display_relative(root, root),
                rule,
            });
            return validation_report(findings);
        }
    };
    let manifest_bytes = match files.get("manifest.json") {
        Some(bytes) => bytes,
        None => {
            findings.push(ValidationFinding {
                path: "manifest.json".to_owned(),
                rule: "missing bundle manifest".to_owned(),
            });
            return validation_report(findings);
        }
    };
    let manifest = match serde_json::from_slice::<BundleManifest>(manifest_bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            findings.push(ValidationFinding {
                path: "manifest.json".to_owned(),
                rule: format!("invalid bundle manifest: {error}"),
            });
            return validation_report(findings);
        }
    };
    if manifest.schema != BUNDLE_SCHEMA {
        findings.push(ValidationFinding {
            path: "manifest.json".to_owned(),
            rule: format!("unsupported bundle schema '{}'", manifest.schema),
        });
    }
    let declared_transport = match parse_transport(&manifest.transport) {
        Ok(transport) => Some(transport),
        Err(_) => {
            findings.push(ValidationFinding {
                path: "manifest.json".to_owned(),
                rule: format!("unsupported bundle transport '{}'", manifest.transport),
            });
            None
        }
    };
    if validate_mcp_platform(&manifest.platform).is_err() {
        findings.push(ValidationFinding {
            path: "manifest.json".to_owned(),
            rule: format!("unsupported bundle platform '{}'", manifest.platform),
        });
    }
    if expected_platform.is_some_and(|expected| expected != manifest.platform) {
        findings.push(ValidationFinding {
            path: "manifest.json".to_owned(),
            rule: format!(
                "bundle platform '{}' does not match requested platform",
                manifest.platform
            ),
        });
    }
    for name in SKILL_NAMES {
        let path = format!("skills/{name}/SKILL.md");
        if !files.contains_key(&path) {
            findings.push(ValidationFinding {
                path,
                rule: "missing required skill entry point".to_owned(),
            });
        }
    }
    let actual_paths = files
        .keys()
        .filter(|path| path.as_str() != "manifest.json")
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_paths = manifest.files.keys().cloned().collect::<BTreeSet<_>>();
    if actual_paths != expected_paths {
        findings.push(ValidationFinding {
            path: "manifest.json".to_owned(),
            rule: "manifest file inventory does not match bundle contents".to_owned(),
        });
    }
    for (path, expected) in &manifest.files {
        if safe_relative_path(root, path).is_err() {
            findings.push(ValidationFinding {
                path: "manifest.json".to_owned(),
                rule: format!("unsafe manifest path '{path}'"),
            });
            continue;
        }
        match files.get(path) {
            Some(bytes) if digest_bytes(bytes) == *expected => {}
            Some(_) => findings.push(ValidationFinding {
                path: path.clone(),
                rule: "checksum mismatch".to_owned(),
            }),
            None => findings.push(ValidationFinding {
                path: path.clone(),
                rule: "manifest file is missing".to_owned(),
            }),
        }
    }
    let config_path = bundle_config_path(&manifest.platform);
    match files.get(config_path) {
        Some(bytes) => {
            let validation =
                if matches!(manifest.platform.as_str(), "codex" | "claude" | "opencode") {
                    validate_native_mcp_config_bytes(&manifest.platform, bytes, declared_transport)
                } else {
                    validate_mcp_config_bytes(&manifest.platform, bytes, declared_transport)
                };
            if let Err(rule) = validation {
                findings.push(ValidationFinding {
                    path: config_path.to_owned(),
                    rule,
                });
            }
        }
        None => findings.push(ValidationFinding {
            path: config_path.to_owned(),
            rule: "missing platform MCP configuration".to_owned(),
        }),
    }
    validate_native_package_files(&manifest, &files, &mut findings);
    scan_portability(&files, &mut findings);
    validation_report(findings)
}

fn validate_managed_bundle(primary: &Path) -> ValidationReport {
    let mut findings = Vec::new();
    let directories = match managed_skill_collection_directories(primary) {
        Ok(directories) => directories,
        Err(error) => {
            findings.push(ValidationFinding {
                path: display_relative(primary, primary),
                rule: strip_error_prefix(&error),
            });
            return validation_report(findings);
        }
    };
    let container = primary.parent().unwrap_or(primary);
    let mut files = BTreeMap::new();
    let mut total_files = 0_usize;
    let mut total_bytes = 0_u64;
    for directory in directories {
        let remaining_files = MAX_FILES.saturating_sub(total_files);
        let remaining_bytes = MAX_TOTAL_BYTES.saturating_sub(total_bytes);
        match collect_files_bounded_with_limits(&directory, remaining_files, remaining_bytes) {
            Ok(collected) => {
                total_files = total_files.saturating_add(collected.len());
                for (relative, bytes) in collected {
                    total_bytes = total_bytes.saturating_add(bytes.len() as u64);
                    let prefix = directory
                        .strip_prefix(container)
                        .unwrap_or(&directory)
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.insert(format!("{prefix}/{relative}"), bytes);
                }
            }
            Err(rule) => {
                findings.push(ValidationFinding {
                    path: display_relative(container, &directory),
                    rule,
                });
                return validation_report(findings);
            }
        }
    }
    if let Err(error) = verify_managed_skill_collection(primary) {
        findings.push(ValidationFinding {
            path: display_relative(primary, primary),
            rule: strip_error_prefix(&error),
        });
        return validation_report(findings);
    }
    files.retain(|path, _| {
        !path.ends_with("/.compass-install.json")
            && !path.ends_with("/.compass_version")
            && path != ".compass-install.json"
            && path != ".compass_version"
    });
    scan_portability(&files, &mut findings);
    validation_report(findings)
}

fn validation_report(mut findings: Vec<ValidationFinding>) -> ValidationReport {
    findings.sort();
    findings.dedup();
    ValidationReport {
        schema: VALIDATION_SCHEMA,
        valid: findings.is_empty(),
        findings,
    }
}

fn collect_files_bounded(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    collect_files_bounded_with_limits(root, MAX_FILES, MAX_TOTAL_BYTES)
}

fn collect_files_bounded_with_limits(
    root: &Path,
    max_files: usize,
    max_total_bytes: u64,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let metadata =
        fs::symlink_metadata(root).map_err(|error| format!("could not inspect bundle: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("bundle root must be a directory and not a symbolic link".to_owned());
    }
    let canonical = fs::canonicalize(root)
        .map_err(|error| format!("could not resolve bundle root: {error}"))?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(|error| format!("could not inspect bundle directory: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not inspect bundle entry: {error}"))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("could not inspect bundle entry: {error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "symbolic links are not allowed: {}",
                    display_relative(root, &path)
                ));
            }
            if file_type.is_dir() {
                let resolved = fs::canonicalize(&path)
                    .map_err(|error| format!("could not resolve bundle directory: {error}"))?;
                if !resolved.starts_with(&canonical) {
                    return Err(format!(
                        "path escapes bundle root: {}",
                        display_relative(root, &path)
                    ));
                }
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(format!(
                    "unsupported file type: {}",
                    display_relative(root, &path)
                ));
            }
            if files.len() >= max_files {
                return Err(format!("bundle exceeds file limit of {max_files}"));
            }
            let remaining = max_total_bytes.saturating_sub(total);
            let limit = remaining.min(MAX_FILE_BYTES);
            let bytes = read_file_with_limit(&path, limit).map_err(|error| {
                if remaining < MAX_FILE_BYTES {
                    format!("bundle exceeds byte limit of {max_total_bytes}: {error}")
                } else {
                    format!(
                        "file exceeds byte limit: {}: {error}",
                        display_relative(root, &path)
                    )
                }
            })?;
            total = total.saturating_add(bytes.len() as u64);
            files.insert(display_relative(root, &path), bytes);
        }
    }
    Ok(files)
}

fn scan_portability(files: &BTreeMap<String, Vec<u8>>, findings: &mut Vec<ValidationFinding>) {
    for (path, bytes) in files {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        if contains_absolute_path(text) {
            findings.push(ValidationFinding {
                path: path.clone(),
                rule: "contains an absolute path or file URL".to_owned(),
            });
        }
        if contains_literal_credential(text) {
            findings.push(ValidationFinding {
                path: path.clone(),
                rule: "contains a likely literal credential".to_owned(),
            });
        }
    }
}

fn contains_absolute_path(text: &str) -> bool {
    if text.to_ascii_lowercase().contains("file://") {
        return true;
    }
    let bytes = text.as_bytes();
    for index in 0..bytes.len().saturating_sub(2) {
        let boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if !boundary {
            continue;
        }
        if bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'/' | b'\\')
        {
            return true;
        }
        if bytes[index] == b'\\' && bytes[index + 1] == b'\\' {
            return true;
        }
    }
    const UNIX_ROOTS: &[&str] = &[
        "/Applications",
        "/Library",
        "/Users",
        "/Volumes",
        "/bin",
        "/etc",
        "/home",
        "/mnt",
        "/nix",
        "/opt",
        "/private",
        "/root",
        "/sbin",
        "/snap",
        "/srv",
        "/tmp",
        "/usr",
        "/var",
    ];
    UNIX_ROOTS.iter().any(|root| {
        text.match_indices(root).any(|(index, _)| {
            let before = index == 0
                || !text.as_bytes()[index - 1].is_ascii_alphanumeric()
                    && text.as_bytes()[index - 1] != b':';
            let end = index.saturating_add(root.len());
            let after = text.as_bytes().get(end).is_none_or(|byte| {
                *byte == b'/'
                    || byte.is_ascii_whitespace()
                    || matches!(byte, b'"' | b'\'' | b',' | b')' | b']' | b'}')
            });
            before && after
        })
    })
}

fn contains_literal_credential(text: &str) -> bool {
    const KEYS: &[&str] = &[
        "api_key",
        "apikey",
        "access_token",
        "auth_token",
        "password",
        "client_secret",
        "authorization",
    ];
    fn sensitive_key(key: &str) -> bool {
        let key = key
            .trim()
            .trim_matches(|character: char| matches!(character, '"' | '\'' | '-' | ' '))
            .to_ascii_lowercase();
        KEYS.iter().any(|candidate| key.ends_with(candidate))
    }

    fn json_contains(value: &Value) -> bool {
        match value {
            Value::Object(values) => values.iter().any(|(key, value)| {
                if sensitive_key(key) {
                    return match value {
                        Value::Null => false,
                        Value::String(value) => !is_credential_placeholder(value),
                        _ => true,
                    };
                }
                json_contains(value)
            }),
            Value::Array(values) => values.iter().any(json_contains),
            _ => false,
        }
    }

    fn toml_contains(value: &toml::Value) -> bool {
        match value {
            toml::Value::Table(values) => values.iter().any(|(key, value)| {
                if sensitive_key(key) {
                    return value
                        .as_str()
                        .is_none_or(|value| !is_credential_placeholder(value));
                }
                toml_contains(value)
            }),
            toml::Value::Array(values) => values.iter().any(toml_contains),
            _ => false,
        }
    }

    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return json_contains(&value);
    }
    if let Ok(value) = toml::from_str::<toml::Value>(text) {
        return toml_contains(&value);
    }
    text.lines().any(|line| {
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return false;
        };
        if !sensitive_key(key) {
            return false;
        }
        let value = value
            .trim()
            .trim_end_matches(',')
            .trim()
            .trim_matches(|character| matches!(character, '"' | '\''));
        if value.is_empty() {
            return false;
        }
        !is_credential_placeholder(value)
    })
}

fn is_credential_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if matches!(lower.as_str(), "null" | "placeholder" | "<secret>")
        || lower.starts_with("your_")
        || lower.starts_with("your-")
        || lower.starts_with("env:")
    {
        return true;
    }
    let variable = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'));
    if variable.is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return true;
    }
    let placeholder = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .unwrap_or(value)
        .trim();
    placeholder
        .strip_prefix('<')
        .and_then(|placeholder| placeholder.strip_suffix('>'))
        .is_some_and(|placeholder| {
            !placeholder.is_empty()
                && placeholder
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
        || matches!(lower.as_str(), "redacted" | "[redacted]" | "***")
        || lower.starts_with("see ")
}

fn doctor(platform: &str, root: &Path, user: bool) -> DoctorReport {
    let mut checks = Vec::new();
    checks.push(pass_check(
        "binary_version",
        format!("compass {}", env!("CARGO_PKG_VERSION")),
    ));
    checks.push(if compass_mcp::supports_protocol("2026-07-28") {
        pass_check(
            "mcp_protocol",
            compass_mcp::SUPPORTED_PROTOCOL_VERSION.to_owned(),
        )
    } else {
        fail_check(
            "mcp_protocol",
            format!(
                "compiled server accepts {} instead of required 2026-07-28",
                compass_mcp::SUPPORTED_PROTOCOL_VERSION
            ),
        )
    });
    if user {
        checks.push(skip_check(
            "graph_presence",
            "not applicable to a user-scoped integration".to_owned(),
        ));
        checks.push(skip_check(
            "graph_freshness",
            "not applicable to a user-scoped integration".to_owned(),
        ));
    } else {
        let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
        let output = root.join(&output_name);
        let graph =
            compass_files::BuildGuard::resolve_requested_artifact(&output.join("graph.json"));
        let manifest_path =
            compass_files::BuildGuard::resolve_requested_artifact(&output.join("manifest.json"));
        match (graph, manifest_path) {
            (Ok(graph), Ok(manifest_path)) if graph.is_file() => {
                checks.push(pass_check("graph_presence", graph.display().to_string()));
                let manifest_bytes = read_file_with_limit(&manifest_path, MAX_TOTAL_BYTES);
                let manifest_valid = manifest_bytes.and_then(|bytes| {
                    serde_json::from_slice::<Value>(&bytes)
                        .map_err(|error| error.to_string())
                        .and_then(|value| {
                            value
                                .as_object()
                                .map(|_| ())
                                .ok_or_else(|| "graph manifest root must be an object".to_owned())
                        })
                });
                if let Err(error) = manifest_valid {
                    checks.push(fail_check(
                        "graph_freshness",
                        format!("could not load graph manifest: {error}"),
                    ));
                } else {
                    let freshness = detect(
                        root,
                        &DetectOptions {
                            output_name,
                            ..DetectOptions::default()
                        },
                    )
                    .map(|detection| {
                        Manifest::load(&manifest_path, Some(root))
                            .is_unchanged(&detection.files, ManifestKind::Ast)
                    });
                    checks.push(match freshness {
                        Ok(true) => {
                            pass_check("graph_freshness", "manifest matches sources".to_owned())
                        }
                        Ok(false) => {
                            fail_check("graph_freshness", "graph manifest is stale".to_owned())
                        }
                        Err(error) => fail_check(
                            "graph_freshness",
                            format!("source detection failed: {error}"),
                        ),
                    });
                }
            }
            (Ok(graph), Ok(_)) => {
                checks.push(fail_check(
                    "graph_presence",
                    format!("missing {}", graph.display()),
                ));
                checks.push(fail_check(
                    "graph_freshness",
                    "cannot assess freshness without graph".to_owned(),
                ));
            }
            (Err(error), _) | (_, Err(error)) => {
                checks.push(fail_check(
                    "graph_presence",
                    format!("could not resolve immutable graph snapshot: {error}"),
                ));
                checks.push(fail_check(
                    "graph_freshness",
                    "cannot assess freshness with an invalid snapshot selector".to_owned(),
                ));
            }
        }
    }
    match managed_skill_directory(platform, root, user) {
        Ok(directory) => {
            let validation = validate_managed_bundle(&directory);
            checks.push(if validation.valid {
                pass_check("skill_checksums", directory.display().to_string())
            } else {
                fail_check(
                    "skill_checksums",
                    validation
                        .findings
                        .first()
                        .map(|finding| finding.rule.clone())
                        .unwrap_or_else(|| "managed skill validation failed".to_owned()),
                )
            });
        }
        Err(error) => checks.push(fail_check("skill_checksums", strip_error_prefix(&error))),
    }
    checks.push(match locate_mcp_config(platform, root) {
        Some(path) => match read_bounded_file(&path) {
            Ok(bytes) => match validate_mcp_config_bytes(platform, &bytes, None) {
                Ok(()) => pass_check("mcp_config", path.display().to_string()),
                Err(error) => fail_check("mcp_config", error),
            },
            Err(error) => fail_check("mcp_config", error),
        },
        None => fail_check(
            "mcp_config",
            format!("no {} MCP configuration found", platform),
        ),
    });
    let healthy = checks.iter().all(|check| check.status != "fail");
    DoctorReport {
        schema: DOCTOR_SCHEMA,
        platform: platform.to_owned(),
        healthy,
        checks,
    }
}

fn locate_mcp_config(platform: &str, root: &Path) -> Option<PathBuf> {
    let candidates: &[&str] = match platform {
        "codex" => &[".codex/config.toml"],
        "claude" => &[".mcp.json"],
        "opencode" => &["opencode.json", ".opencode/opencode.json"],
        "agents" => &[".agents/mcp.json"],
        _ => &[],
    };
    candidates
        .iter()
        .map(|relative| root.join(relative))
        .find(|path| path.is_file())
}

fn validate_mcp_config_bytes(
    platform: &str,
    bytes: &[u8],
    expected_transport: Option<Transport>,
) -> Result<(), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "MCP configuration must be UTF-8".to_owned())?;
    if contains_literal_credential(text) {
        return Err("MCP configuration contains a likely literal credential".to_owned());
    }
    if platform == "codex" {
        let value = toml::from_str::<toml::Value>(text)
            .map_err(|error| format!("invalid Codex TOML: {error}"))?;
        let compass = value
            .get("mcp_servers")
            .and_then(|servers| servers.get("compass"))
            .ok_or_else(|| "Codex configuration is missing mcp_servers.compass".to_owned())?;
        let command_is_compass = compass.get("command").and_then(toml::Value::as_str)
            == Some("compass")
            && compass.get("args").is_some_and(|args| {
                args.as_array().is_some_and(|args| {
                    args.iter().filter_map(toml::Value::as_str).eq([
                        "serve",
                        "--transport",
                        "stdio",
                    ]) && args.len() == 3
                })
            });
        let url_is_compass = compass.get("url").and_then(toml::Value::as_str) == Some(MCP_HTTP_URL);
        let valid = match expected_transport {
            Some(Transport::Stdio) => command_is_compass,
            Some(Transport::Http) => url_is_compass,
            None => command_is_compass || url_is_compass,
        };
        if !valid {
            return Err(
                "Codex MCP entry does not invoke Compass serve or its loopback endpoint".to_owned(),
            );
        }
        return Ok(());
    }
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|error| format!("invalid {platform} JSON: {error}"))?;
    let entry = match platform {
        "claude" | "agents" => value.pointer("/mcpServers/compass"),
        "opencode" => value.pointer("/mcp/compass"),
        _ => None,
    };
    let entry = entry
        .filter(|entry| entry.is_object())
        .ok_or_else(|| format!("{platform} configuration is missing the Compass MCP entry"))?;
    if platform == "agents"
        && value.get("schema").and_then(Value::as_str) != Some(GENERIC_MCP_SCHEMA)
    {
        return Err(format!(
            "agents configuration requires schema {GENERIC_MCP_SCHEMA}"
        ));
    }
    let url_is_compass = entry.get("url").and_then(Value::as_str) == Some(MCP_HTTP_URL);
    let command_is_compass = if platform == "opencode" {
        entry
            .get("command")
            .and_then(Value::as_array)
            .is_some_and(|command| {
                command.iter().filter_map(Value::as_str).eq([
                    "compass",
                    "serve",
                    "--transport",
                    "stdio",
                ]) && command.len() == 4
            })
    } else {
        entry.get("command").and_then(Value::as_str) == Some("compass")
            && entry
                .get("args")
                .and_then(Value::as_array)
                .is_some_and(|args| {
                    args.iter()
                        .filter_map(Value::as_str)
                        .eq(["serve", "--transport", "stdio"])
                        && args.len() == 3
                })
    };
    let valid = match expected_transport {
        Some(Transport::Stdio) => command_is_compass,
        Some(Transport::Http) => url_is_compass,
        None => command_is_compass || url_is_compass,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{platform} MCP entry does not invoke Compass serve or its loopback endpoint"
        ))
    }
}

fn validate_native_mcp_config_bytes(
    platform: &str,
    bytes: &[u8],
    expected_transport: Option<Transport>,
) -> Result<(), String> {
    if platform == "opencode" {
        return validate_mcp_config_bytes(platform, bytes, expected_transport);
    }
    validate_mcp_config_bytes("claude", bytes, expected_transport)
}

fn validate_native_package_files(
    manifest: &BundleManifest,
    files: &BTreeMap<String, Vec<u8>>,
    findings: &mut Vec<ValidationFinding>,
) {
    let required: &[&str] = match manifest.platform.as_str() {
        "codex" => &[
            ".agents/plugins/marketplace.json",
            ".codex-plugin/plugin.json",
            ".mcp.json",
        ],
        "claude" => &[
            ".claude-plugin/plugin.json",
            ".claude-plugin/marketplace.json",
            ".mcp.json",
        ],
        "opencode" => &["package.json", "opencode.json", "src/index.js"],
        _ => return,
    };
    if manifest
        .harness_version
        .as_deref()
        .is_none_or(str::is_empty)
    {
        findings.push(ValidationFinding {
            path: "manifest.json".to_owned(),
            rule: "native package does not record its verified harness version".to_owned(),
        });
    }
    for path in required {
        if !files.contains_key(*path) {
            findings.push(ValidationFinding {
                path: (*path).to_owned(),
                rule: "missing native package artifact".to_owned(),
            });
        }
    }
    let plugin_manifest = match manifest.platform.as_str() {
        "codex" => Some(".codex-plugin/plugin.json"),
        "claude" => Some(".claude-plugin/plugin.json"),
        "opencode" => Some("package.json"),
        _ => None,
    };
    if let Some(path) = plugin_manifest
        && let Some(bytes) = files.get(path)
    {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.get("name").and_then(Value::as_str).is_some() => {}
            Ok(_) => findings.push(ValidationFinding {
                path: path.to_owned(),
                rule: "native package manifest is missing its name".to_owned(),
            }),
            Err(error) => findings.push(ValidationFinding {
                path: path.to_owned(),
                rule: format!("invalid native package manifest: {error}"),
            }),
        }
    }
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{} must be a regular file and not a symbolic link",
            path.display()
        ));
    }
    read_file_with_limit(path, MAX_FILE_BYTES)
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

fn read_file_with_limit(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    let current = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if current.file_type().is_symlink() || !opened_file_matches_path(&opened, &current) {
        return Err("path changed or became a symbolic link while it was opened".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("content exceeds byte limit of {limit}"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn opened_file_matches_path(opened: &fs::Metadata, current: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    opened.dev() == current.dev() && opened.ino() == current.ino()
}

#[cfg(not(unix))]
fn opened_file_matches_path(opened: &fs::Metadata, current: &fs::Metadata) -> bool {
    opened.file_type() == current.file_type()
        && opened.len() == current.len()
        && opened.modified().ok() == current.modified().ok()
}

fn pass_check(id: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        id,
        status: "pass",
        detail,
    }
}

fn fail_check(id: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        id,
        status: "fail",
        detail,
    }
}

fn skip_check(id: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        id,
        status: "skip",
        detail,
    }
}

fn bundle_config_path(platform: &str) -> &'static str {
    match platform {
        "codex" | "claude" => ".mcp.json",
        "opencode" => "opencode.json",
        _ => ".agents/mcp.json",
    }
}

fn transport_name(transport: Transport) -> &'static str {
    match transport {
        Transport::Stdio => "stdio",
        Transport::Http => "http",
    }
}

fn safe_relative_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("error: unsafe bundle path '{relative}'"));
    }
    Ok(root.join(path))
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .replace('\\', "/")
}

fn strip_error_prefix(error: &str) -> String {
    error.strip_prefix("error: ").unwrap_or(error).to_owned()
}

fn json_success(value: &Value) -> Outcome {
    match serde_json::to_string_pretty(value) {
        Ok(stdout) => Outcome::success(stdout),
        Err(error) => Outcome::failure(format!("error: could not encode agent output: {error}")),
    }
}

fn usage_error(message: &str) -> Outcome {
    Outcome::failure_with_code(message.to_owned(), 2)
}

fn outcome(code: u8, stdout: String, stderr: String) -> Outcome {
    Outcome {
        code,
        stdout,
        stderr,
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
        html_output: None,
    }
}
