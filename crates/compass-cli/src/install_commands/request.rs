use std::env;
use std::path::{Path, PathBuf};

use super::model::{InstallRequest, InstallScope, OutputFormat};

pub(super) fn parse_install_request(args: &[String]) -> Result<InstallRequest, String> {
    let mut request = InstallRequest::default();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--project" => request.project = true,
            "--user" => request.user = true,
            "--all" => request.all = true,
            "--strict" => request.strict = true,
            "--dry-run" => request.dry_run = true,
            "--require-all" => request.require_all = true,
            "--platform" | "-p" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("error: {argument} requires a value"))?;
                request.platforms.push(value.clone());
            }
            "--format" => {
                index += 1;
                request.format = parse_format(
                    args.get(index)
                        .ok_or_else(|| "error: --format requires a value".to_owned())?,
                )?;
            }
            value if value.starts_with("--platform=") => {
                request.platforms.push(value[11..].to_owned());
            }
            value if value.starts_with("--format=") => {
                request.format = parse_format(&value[9..])?;
            }
            value if value.starts_with('-') => {
                return Err(format!("error: unknown install option '{value}'"));
            }
            value => request.platforms.push(value.to_owned()),
        }
        index += 1;
    }
    if request.project && request.user {
        return Err("error: --project and --user cannot be used together".to_owned());
    }
    if request.all && !request.platforms.is_empty() {
        return Err("error: --all and --platform cannot be used together".to_owned());
    }
    Ok(request)
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

pub(super) fn resolve_scope(request: &InstallRequest) -> Result<InstallScope, String> {
    let current = env::current_dir()
        .map_err(|error| format!("error: could not determine current directory: {error}"))?;
    resolve_scope_from(request, &current, home_directory())
}

fn resolve_scope_from(
    request: &InstallRequest,
    current: &Path,
    home: Option<PathBuf>,
) -> Result<InstallScope, String> {
    if request.user {
        return home
            .map(InstallScope::User)
            .ok_or_else(|| "error: could not determine user home directory".to_owned());
    }
    if let Some(root) = find_git_root(current) {
        return Ok(InstallScope::Project(root));
    }
    if request.project {
        return Err("error: --project requires a Git repository".to_owned());
    }
    home.map(InstallScope::User)
        .ok_or_else(|| "error: could not determine user home directory".to_owned())
}

fn find_git_root(current: &Path) -> Option<PathBuf> {
    current
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_platforms_and_conflicts_are_parsed() -> Result<(), String> {
        let args = ["--platform", "codex", "-p", "claude", "--dry-run"].map(str::to_owned);
        let request = parse_install_request(&args)?;
        assert_eq!(request.platforms, ["codex", "claude"]);
        assert!(request.dry_run);
        assert!(parse_install_request(&["--project".into(), "--user".into()]).is_err());
        assert!(parse_install_request(&["--all".into(), "codex".into()]).is_err());
        Ok(())
    }

    #[test]
    fn automatic_scope_uses_git_root_then_home() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let project = directory.path().join("project");
        let nested = project.join("one/two");
        std::fs::create_dir_all(project.join(".git")).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&nested).map_err(|error| error.to_string())?;
        let request = InstallRequest::default();
        assert_eq!(
            resolve_scope_from(&request, &nested, Some(directory.path().join("home")))?,
            InstallScope::Project(project)
        );

        let outside = directory.path().join("outside");
        let home = directory.path().join("home");
        std::fs::create_dir(&outside).map_err(|error| error.to_string())?;
        assert_eq!(
            resolve_scope_from(&request, &outside, Some(home.clone()))?,
            InstallScope::User(home)
        );
        Ok(())
    }
}
