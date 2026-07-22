use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use compass_files::write_text_atomic;
use compass_semantic::{builtin_backend, graphify_endpoint_warning, provider_base_url_check};
use serde_json::{Map, Value, json};

use crate::{Frontend, Outcome};

const MAX_PROVIDER_REGISTRY_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn command_provider(frontend: Frontend, args: &[String]) -> Outcome {
    let path = match global_provider_path() {
        Ok(path) => path,
        Err(error) => return Outcome::failure(error),
    };
    command_provider_at(frontend, args, &path)
}

fn command_provider_at(frontend: Frontend, args: &[String], path: &Path) -> Outcome {
    let startup_warnings = provider_registry_warnings(path);
    let mut outcome = match args.first().map(String::as_str).unwrap_or_default() {
        "list" => provider_list(path),
        "show" => provider_show(args, path),
        "add" => provider_add(frontend, args, path),
        "remove" => provider_remove(args, path),
        subcommand => Outcome {
            code: u8::from(!subcommand.is_empty()),
            stdout: String::new(),
            stderr: provider_help(frontend),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
        },
    };
    if !startup_warnings.is_empty() {
        if outcome.stderr.is_empty() {
            outcome.stderr = startup_warnings;
        } else {
            outcome.stderr = format!("{startup_warnings}\n{}", outcome.stderr);
        }
    }
    outcome
}

fn provider_list(path: &Path) -> Outcome {
    let providers = read_registry(path);
    if providers.is_empty() {
        return Outcome::success("No custom providers registered.".to_owned());
    }
    Outcome::success(
        providers
            .iter()
            .map(|(name, config)| {
                let base_url = config
                    .get("base_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                format!("  {name}  ({base_url})")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn provider_show(args: &[String], path: &Path) -> Outcome {
    let name = args.get(1).map(String::as_str).unwrap_or_default();
    if name.is_empty() {
        return Outcome::failure("Usage: graphify provider show <name>".to_owned());
    }
    let providers = read_registry(path);
    let Some(provider) = providers.get(name) else {
        return Outcome::failure(format!("Provider '{name}' not found."));
    };
    let mut selected = Map::new();
    selected.insert(name.to_owned(), provider.clone());
    match python_pretty_json(&Value::Object(selected)) {
        Ok(encoded) => Outcome::success(encoded),
        Err(error) => Outcome::failure(format!("error: could not encode provider: {error}")),
    }
}

fn provider_add(frontend: Frontend, args: &[String], path: &Path) -> Outcome {
    let name = args
        .get(1)
        .filter(|name| !name.starts_with('-'))
        .map(String::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return Outcome::failure(
            "Usage: graphify provider add <name> --base-url URL --default-model MODEL --env-key KEY"
                .to_owned(),
        );
    }
    if builtin_backend(name).is_some() {
        return Outcome::failure(format!(
            "Error: '{name}' is a built-in provider and cannot be overridden."
        ));
    }
    let mut base_url = String::new();
    let mut default_model = String::new();
    let mut env_key = String::new();
    let mut pricing_input = 0.0_f64;
    let mut pricing_output = 0.0_f64;
    let mut index = 2;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--base-url" if index + 1 < args.len() => {
                base_url.clone_from(&args[index + 1]);
                index += 1;
            }
            "--default-model" if index + 1 < args.len() => {
                default_model.clone_from(&args[index + 1]);
                index += 1;
            }
            "--env-key" if index + 1 < args.len() => {
                env_key.clone_from(&args[index + 1]);
                index += 1;
            }
            "--pricing-input" if index + 1 < args.len() => {
                pricing_input = match parse_price(&args[index + 1], "--pricing-input") {
                    Ok(value) => value,
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            "--pricing-output" if index + 1 < args.len() => {
                pricing_output = match parse_price(&args[index + 1], "--pricing-output") {
                    Ok(value) => value,
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            _ if argument.starts_with("--base-url=") => {
                base_url = argument[11..].to_owned();
            }
            _ if argument.starts_with("--default-model=") => {
                default_model = argument[16..].to_owned();
            }
            _ if argument.starts_with("--env-key=") => env_key = argument[10..].to_owned(),
            _ => {}
        }
        index += 1;
    }
    if base_url.is_empty() || default_model.is_empty() || env_key.is_empty() {
        return Outcome::failure(
            "Error: --base-url, --default-model, and --env-key are required.".to_owned(),
        );
    }
    let endpoint = provider_base_url_check(&base_url, name);
    let endpoint_warning = graphify_endpoint_warning(&base_url, name, endpoint.allowed);
    if !endpoint.allowed {
        return Outcome::failure(format!(
            "{}\nError: refusing to add provider with unsafe base_url '{}'.",
            endpoint_warning.unwrap_or_else(|| format!(
                "[graphify] WARNING: provider '{name}' has an unsafe base_url."
            )),
            base_url.replace('\\', "\\\\").replace('\'', "\\'")
        ));
    }
    let mut providers = read_registry(path);
    providers.insert(
        name.to_owned(),
        json!({
            "base_url":base_url,
            "default_model":default_model,
            "env_key":env_key,
            "pricing":{"input":pricing_input,"output":pricing_output},
            "temperature":0,
        }),
    );
    if let Err(error) = write_registry(path, &providers) {
        return Outcome::failure(error);
    }
    let invocation = match frontend {
        Frontend::Compass => "compass extract",
        Frontend::Graphify => "graphify extract",
    };
    Outcome {
        code: 0,
        stdout: format!("Provider '{name}' added. Use with: {invocation} . --backend {name}"),
        stderr: endpoint_warning.unwrap_or_default(),
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
    }
}

fn provider_remove(args: &[String], path: &Path) -> Outcome {
    let name = args.get(1).map(String::as_str).unwrap_or_default();
    if name.is_empty() {
        return Outcome::failure("Usage: graphify provider remove <name>".to_owned());
    }
    let mut providers = read_registry(path);
    if providers.remove(name).is_none() {
        return Outcome::failure(format!("Provider '{name}' not found."));
    }
    if let Err(error) = write_registry(path, &providers) {
        return Outcome::failure(error);
    }
    Outcome::success(format!("Provider '{name}' removed."))
}

fn read_registry(path: &Path) -> Map<String, Value> {
    let Ok(metadata) = path.metadata() else {
        return Map::new();
    };
    if !metadata.is_file() || metadata.len() > MAX_PROVIDER_REGISTRY_BYTES {
        return Map::new();
    }
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn provider_registry_warnings(path: &Path) -> String {
    read_registry(path)
        .into_iter()
        .filter(|(name, config)| builtin_backend(name).is_none() && config.is_object())
        .filter_map(|(name, config)| {
            let base_url = config
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let check = provider_base_url_check(base_url, &name);
            graphify_endpoint_warning(base_url, &name, check.allowed)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_registry(path: &Path, providers: &Map<String, Value>) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "error: provider registry has no parent directory: {}",
            path.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("error: could not create {}: {error}", parent.display()))?;
    let encoded = python_pretty_json(&Value::Object(providers.clone()))
        .map_err(|error| format!("error: could not encode provider registry: {error}"))?;
    write_text_atomic(path, &format!("{encoded}\n"))
        .map_err(|error| format!("error: could not write {}: {error}", path.display()))
}

fn parse_price(value: &str, option: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("error: {option} must be a finite number, got {value:?}"))
}

fn python_pretty_json(value: &Value) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string_pretty(value)?;
    let mut ascii = String::with_capacity(json.len());
    for character in json.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            let point = u32::from(character);
            if point <= 0xffff {
                use std::fmt::Write as _;
                let _ = write!(ascii, "\\u{point:04x}");
            } else {
                use std::fmt::Write as _;
                let adjusted = point - 0x1_0000;
                let high = 0xd800 + (adjusted >> 10);
                let low = 0xdc00 + (adjusted & 0x3ff);
                let _ = write!(ascii, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    Ok(ascii)
}

fn global_provider_path() -> Result<PathBuf, String> {
    let home = home_directory().ok_or_else(|| {
        "error: could not determine the home directory for ~/.graphify/providers.json".to_owned()
    })?;
    Ok(home.join(".graphify/providers.json"))
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(non_empty_os_string)
        .or_else(|| env::var_os("USERPROFILE").filter(non_empty_os_string))
        .or_else(windows_home_directory)
        .map(PathBuf::from)
}

fn non_empty_os_string(value: &OsString) -> bool {
    !value.is_empty()
}

fn windows_home_directory() -> Option<OsString> {
    let drive = env::var_os("HOMEDRIVE")?;
    let path = env::var_os("HOMEPATH")?;
    if drive.is_empty() || path.is_empty() {
        return None;
    }
    let mut combined = drive;
    combined.push(path);
    Some(combined)
}

pub(super) fn provider_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Compass => "Usage: compass provider [add|list|show|remove]",
        Frontend::Graphify => "Usage: graphify provider [add|list|show|remove]",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn provider_registry_round_trip_is_ordered_and_ascii() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let registry = directory.path().join("providers.json");
        let add = command_provider_at(
            Frontend::Graphify,
            &[
                "add".to_owned(),
                "本地".to_owned(),
                "--base-url".to_owned(),
                "https://example.test/v1".to_owned(),
                "--default-model".to_owned(),
                "模型".to_owned(),
                "--env-key".to_owned(),
                "MODEL_KEY".to_owned(),
                "--pricing-input".to_owned(),
                "0.25".to_owned(),
            ],
            &registry,
        );
        assert_eq!(add.code, 0, "{}", add.stderr);
        assert!(fs::read_to_string(&registry)?.contains("\\u672c\\u5730"));
        let list = command_provider_at(Frontend::Graphify, &["list".to_owned()], &registry);
        assert_eq!(list.stdout, "  本地  (https://example.test/v1)");
        let show = command_provider_at(
            Frontend::Graphify,
            &["show".to_owned(), "本地".to_owned()],
            &registry,
        );
        assert!(show.stdout.contains("\\u6a21\\u578b"));
        let remove = command_provider_at(
            Frontend::Graphify,
            &["remove".to_owned(), "本地".to_owned()],
            &registry,
        );
        assert_eq!(remove.stdout, "Provider '本地' removed.");
        assert_eq!(fs::read_to_string(registry)?, "{}\n");
        Ok(())
    }

    #[test]
    fn provider_registry_protects_builtins_and_unsafe_urls() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let registry = directory.path().join("providers.json");
        let builtin = command_provider_at(
            Frontend::Graphify,
            &[
                "add".to_owned(),
                "openai".to_owned(),
                "--base-url=https://example.test/v1".to_owned(),
                "--default-model=m".to_owned(),
                "--env-key=K".to_owned(),
            ],
            &registry,
        );
        assert_eq!(builtin.code, 1);
        assert!(builtin.stderr.contains("built-in provider"));
        let unsafe_url = command_provider_at(
            Frontend::Graphify,
            &[
                "add".to_owned(),
                "unsafe".to_owned(),
                "--base-url=file:///etc/passwd".to_owned(),
                "--default-model=m".to_owned(),
                "--env-key=K".to_owned(),
            ],
            &registry,
        );
        assert_eq!(unsafe_url.code, 1);
        assert!(!registry.exists());
        Ok(())
    }
}
