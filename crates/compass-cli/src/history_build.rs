use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use compass_core::{CompleteGraphBuilder, MaterializeError};
use compass_files::{DetectOptions, IgnorePolicy, Manifest, detect};
use compass_history::{
    BuildProfile, CompletedGraphArtifacts, CompletionEvidence, GraphArtifacts, HistoryError,
    MAX_DIAGNOSTIC_BYTES,
};

#[derive(Clone, Debug)]
pub(crate) struct HistoryBuildOptions {
    profile: BuildProfile,
    forwarded: Vec<String>,
    gitignore: bool,
    excludes: Vec<String>,
    semantic_environment: Vec<(String, String)>,
    semantic_environment_remove: Vec<String>,
}

pub(crate) struct ParsedBuildCommand {
    pub(crate) revision: String,
    pub(crate) format: String,
    pub(crate) replace_corrupt: bool,
    pub(crate) profile_from: Option<String>,
    pub(crate) options: HistoryBuildOptions,
}

impl HistoryBuildOptions {
    pub(crate) fn defaults() -> Result<Self, HistoryError> {
        Self::from_values(HistoryBuildValues::default())
    }

    pub(crate) fn profile(&self) -> BuildProfile {
        self.profile.clone()
    }

    pub(crate) fn from_profile(profile: BuildProfile) -> Result<Self, HistoryError> {
        validate_persisted_profile(&profile)?;
        let gitignore = profile.value("gitignore") != Some("false");
        let excludes = profile
            .entries()
            .filter(|(key, _)| key.starts_with("exclude."))
            .map(|(_, value)| value.to_owned())
            .collect::<Vec<_>>();
        let mut forwarded = Vec::new();
        push_profile_option(&profile, &mut forwarded, "provider", "--backend", "none");
        push_profile_option(&profile, &mut forwarded, "model", "--model", "none");
        if profile.value("semantic_mode") == Some("deep") {
            forwarded.extend(["--mode".to_owned(), "deep".to_owned()]);
        }
        for (key, flag) in [("cargo", "--cargo"), ("dedup_llm", "--dedup-llm")] {
            if profile.value(key) == Some("true") {
                forwarded.push(flag.to_owned());
            }
        }
        push_profile_option(
            &profile,
            &mut forwarded,
            "token_budget",
            "--token-budget",
            "default",
        );
        push_profile_option(
            &profile,
            &mut forwarded,
            "resolution",
            "--resolution",
            "none",
        );
        push_profile_option(
            &profile,
            &mut forwarded,
            "exclude_hubs",
            "--exclude-hubs",
            "none",
        );
        if !gitignore {
            forwarded.push("--no-gitignore".to_owned());
        }
        for exclude in &excludes {
            forwarded.extend(["--exclude".to_owned(), exclude.clone()]);
        }
        let (semantic_environment, semantic_environment_remove) =
            pinned_provider_environment(&profile);
        Ok(Self {
            semantic_environment,
            semantic_environment_remove,
            profile,
            forwarded,
            gitignore,
            excludes,
        })
    }

    pub(crate) fn builder(&self, executable: PathBuf) -> NativeCompleteGraphBuilder {
        NativeCompleteGraphBuilder {
            executable,
            forwarded: self.forwarded.clone(),
            gitignore: self.gitignore,
            excludes: self.excludes.clone(),
            semantic_environment: self.semantic_environment.clone(),
            semantic_environment_remove: self.semantic_environment_remove.clone(),
        }
    }

    fn from_values(mut values: HistoryBuildValues) -> Result<Self, HistoryError> {
        resolve_provider(&mut values)?;
        let mut profile = BuildProfile::default();
        for (key, value) in [
            ("compass_version", env!("CARGO_PKG_VERSION").to_owned()),
            ("graph_schema", "networkx-node-link/v1".to_owned()),
            ("extractor_version", "compass-languages/v1".to_owned()),
            ("resolver_version", "compass-resolve/v1".to_owned()),
            ("pipeline_version", "compass-core/v1".to_owned()),
            ("enabled_features", "workspace-default".to_owned()),
            ("direction", "native-source-semantics".to_owned()),
            ("cluster_algorithm", "seeded-louvain/v1".to_owned()),
            ("cluster_seed", "42".to_owned()),
            ("gitignore", values.gitignore.to_string()),
            ("cargo", values.cargo.to_string()),
            ("dedup_llm", values.dedup_llm.to_string()),
            (
                "semantic_mode",
                if values.deep { "deep" } else { "standard" }.to_owned(),
            ),
            (
                "semantic_prompt_sha256",
                compass_semantic::extraction_prompt_sha256(values.deep),
            ),
            (
                "provider",
                values.backend.clone().unwrap_or_else(|| "none".to_owned()),
            ),
            (
                "model",
                values.model.clone().unwrap_or_else(|| "none".to_owned()),
            ),
            ("resolution", normalized_float(values.resolution)),
            (
                "exclude_hubs",
                values
                    .exclude_hubs
                    .map_or_else(|| "none".to_owned(), normalized_float),
            ),
            (
                "token_budget",
                values
                    .token_budget
                    .map_or_else(|| "default".to_owned(), |value| value.to_string()),
            ),
            (
                "provider_endpoint",
                values
                    .provider_endpoint
                    .clone()
                    .unwrap_or_else(|| "none".to_owned()),
            ),
            (
                "provider_temperature",
                values
                    .provider_temperature
                    .map_or_else(|| "none".to_owned(), normalized_float),
            ),
            (
                "provider_max_output_tokens",
                values
                    .provider_max_output_tokens
                    .map_or_else(|| "none".to_owned(), |value| value.to_string()),
            ),
            (
                "provider_region",
                values
                    .provider_region
                    .clone()
                    .unwrap_or_else(|| "none".to_owned()),
            ),
        ] {
            profile.insert(key, &value)?;
        }
        for (index, exclude) in values.excludes.iter().enumerate() {
            profile.insert(&format!("exclude.{index:06}"), exclude)?;
        }

        let mut forwarded = Vec::new();
        if let Some(backend) = &values.backend {
            forwarded.extend(["--backend".to_owned(), backend.clone()]);
        }
        if let Some(model) = &values.model {
            forwarded.extend(["--model".to_owned(), model.clone()]);
        }
        if values.deep {
            forwarded.extend(["--mode".to_owned(), "deep".to_owned()]);
        }
        if values.cargo {
            forwarded.push("--cargo".to_owned());
        }
        if values.dedup_llm {
            forwarded.push("--dedup-llm".to_owned());
        }
        if let Some(token_budget) = values.token_budget {
            forwarded.extend(["--token-budget".to_owned(), token_budget.to_string()]);
        }
        if !values.gitignore {
            forwarded.push("--no-gitignore".to_owned());
        }
        for exclude in &values.excludes {
            forwarded.extend(["--exclude".to_owned(), exclude.clone()]);
        }
        forwarded.extend([
            "--resolution".to_owned(),
            normalized_float(values.resolution),
        ]);
        if let Some(exclude_hubs) = values.exclude_hubs {
            forwarded.extend(["--exclude-hubs".to_owned(), normalized_float(exclude_hubs)]);
        }

        let (semantic_environment, semantic_environment_remove) =
            pinned_provider_environment(&profile);
        Ok(Self {
            semantic_environment,
            semantic_environment_remove,
            profile,
            forwarded,
            gitignore: values.gitignore,
            excludes: values.excludes,
        })
    }
}

fn validate_persisted_profile(profile: &BuildProfile) -> Result<(), HistoryError> {
    for (key, _) in profile.entries() {
        if !matches!(
            key,
            "compass_version"
                | "graph_schema"
                | "extractor_version"
                | "resolver_version"
                | "pipeline_version"
                | "enabled_features"
                | "direction"
                | "cluster_algorithm"
                | "cluster_seed"
                | "gitignore"
                | "cargo"
                | "dedup_llm"
                | "semantic_mode"
                | "semantic_prompt_sha256"
                | "provider"
                | "model"
                | "resolution"
                | "exclude_hubs"
                | "token_budget"
                | "provider_endpoint"
                | "provider_temperature"
                | "provider_max_output_tokens"
                | "provider_region"
        ) && !key.starts_with("exclude.")
        {
            return Err(HistoryError::InvalidFingerprint(format!(
                "unsupported persisted build-profile field {key:?}"
            )));
        }
    }
    for (key, expected) in [
        ("compass_version", env!("CARGO_PKG_VERSION")),
        ("graph_schema", "networkx-node-link/v1"),
        ("extractor_version", "compass-languages/v1"),
        ("resolver_version", "compass-resolve/v1"),
        ("pipeline_version", "compass-core/v1"),
        ("enabled_features", "workspace-default"),
        ("direction", "native-source-semantics"),
        ("cluster_algorithm", "seeded-louvain/v1"),
        ("cluster_seed", "42"),
    ] {
        if profile.value(key) != Some(expected) {
            return Err(HistoryError::InvalidFingerprint(format!(
                "persisted {key} is incompatible with {expected}"
            )));
        }
    }
    for key in ["gitignore", "cargo", "dedup_llm"] {
        if !matches!(profile.value(key), Some("true" | "false")) {
            return Err(HistoryError::InvalidFingerprint(format!(
                "persisted {key} is not boolean"
            )));
        }
    }
    let resolution = profile
        .value("resolution")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    if resolution.is_none() {
        return Err(HistoryError::InvalidFingerprint(
            "persisted resolution is invalid".to_owned(),
        ));
    }
    if profile.value("exclude_hubs") != Some("none")
        && profile
            .value("exclude_hubs")
            .and_then(|value| value.parse::<f64>().ok())
            .is_none_or(|value| !value.is_finite())
    {
        return Err(HistoryError::InvalidFingerprint(
            "persisted hub exclusion is invalid".to_owned(),
        ));
    }
    if profile.value("token_budget") != Some("default")
        && profile
            .value("token_budget")
            .and_then(|value| value.parse::<usize>().ok())
            .is_none_or(|value| value == 0)
    {
        return Err(HistoryError::InvalidFingerprint(
            "persisted token budget is invalid".to_owned(),
        ));
    }
    let deep = match profile.value("semantic_mode") {
        Some("standard") => false,
        Some("deep") => true,
        _ => {
            return Err(HistoryError::InvalidFingerprint(
                "persisted semantic mode is invalid".to_owned(),
            ));
        }
    };
    let prompt_digest = compass_semantic::extraction_prompt_sha256(deep);
    if profile.value("semantic_prompt_sha256") != Some(prompt_digest.as_str()) {
        return Err(HistoryError::InvalidFingerprint(
            "persisted semantic prompt does not match this binary".to_owned(),
        ));
    }
    if let Some(provider) = profile.value("provider").filter(|value| *value != "none")
        && compass_semantic::builtin_backend(provider).is_none()
    {
        return Err(HistoryError::InvalidFingerprint(format!(
            "unsupported persisted backend {provider:?}"
        )));
    }
    match (profile.value("provider"), profile.value("model")) {
        (Some("none"), Some("none")) => {}
        (Some(provider), Some(model))
            if compass_semantic::builtin_backend(provider).is_some() && model != "none" => {}
        _ => {
            return Err(HistoryError::InvalidFingerprint(
                "persisted provider and model are inconsistent".to_owned(),
            ));
        }
    }
    let provider_config = [
        "provider_endpoint",
        "provider_temperature",
        "provider_max_output_tokens",
        "provider_region",
    ];
    if profile.value("provider") == Some("none")
        && provider_config
            .iter()
            .any(|key| profile.value(key) != Some("none"))
    {
        return Err(HistoryError::InvalidFingerprint(
            "provider-free profile contains provider configuration".to_owned(),
        ));
    }
    if profile.value("provider_temperature") != Some("none")
        && profile
            .value("provider_temperature")
            .and_then(|value| value.parse::<f64>().ok())
            .is_none_or(|value| !value.is_finite())
    {
        return Err(HistoryError::InvalidFingerprint(
            "persisted provider temperature is invalid".to_owned(),
        ));
    }
    if profile.value("provider_max_output_tokens") != Some("none")
        && profile
            .value("provider_max_output_tokens")
            .and_then(|value| value.parse::<usize>().ok())
            .is_none_or(|value| value == 0)
    {
        return Err(HistoryError::InvalidFingerprint(
            "persisted provider output limit is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn push_profile_option(
    profile: &BuildProfile,
    forwarded: &mut Vec<String>,
    key: &str,
    flag: &str,
    absent: &str,
) {
    if let Some(value) = profile.value(key).filter(|value| *value != absent) {
        forwarded.extend([flag.to_owned(), value.to_owned()]);
    }
}

fn pinned_provider_environment(profile: &BuildProfile) -> (Vec<(String, String)>, Vec<String>) {
    let mut set = Vec::new();
    let mut remove = vec![
        "GRAPHIFY_LLM_TEMPERATURE".to_owned(),
        "GRAPHIFY_MAX_OUTPUT_TOKENS".to_owned(),
    ];
    if let Some(value) = profile
        .value("provider_temperature")
        .filter(|value| *value != "none")
    {
        set.push(("GRAPHIFY_LLM_TEMPERATURE".to_owned(), value.to_owned()));
    }
    if let Some(value) = profile
        .value("provider_max_output_tokens")
        .filter(|value| *value != "none")
    {
        set.push(("GRAPHIFY_MAX_OUTPUT_TOKENS".to_owned(), value.to_owned()));
    }
    let provider = profile.value("provider").unwrap_or("none");
    if let Some(variable) = match provider {
        "claude" => Some("ANTHROPIC_BASE_URL"),
        "kimi" => Some("KIMI_BASE_URL"),
        "gemini" => Some("GEMINI_BASE_URL"),
        "openai" => Some("OPENAI_BASE_URL"),
        "deepseek" => Some("DEEPSEEK_BASE_URL"),
        "ollama" => Some("OLLAMA_BASE_URL"),
        "azure" => Some("AZURE_OPENAI_ENDPOINT"),
        _ => None,
    } {
        remove.push(variable.to_owned());
        if let Some(endpoint) = profile
            .value("provider_endpoint")
            .filter(|value| *value != "none")
        {
            set.push((variable.to_owned(), endpoint.to_owned()));
        }
    }
    if provider == "bedrock" {
        remove.extend(["AWS_REGION".to_owned(), "AWS_DEFAULT_REGION".to_owned()]);
        if let Some(region) = profile
            .value("provider_region")
            .filter(|value| *value != "none")
        {
            set.push(("AWS_REGION".to_owned(), region.to_owned()));
        }
    }
    if provider == "none" {
        for backend in compass_semantic::BUILTIN_BACKENDS {
            remove.extend(
                backend
                    .api_key_variables
                    .iter()
                    .map(|variable| (*variable).to_owned()),
            );
        }
        remove.extend([
            "AWS_PROFILE".to_owned(),
            "AWS_REGION".to_owned(),
            "AWS_DEFAULT_REGION".to_owned(),
            "OLLAMA_BASE_URL".to_owned(),
        ]);
    }
    remove.sort();
    remove.dedup();
    (set, remove)
}

#[derive(Debug)]
struct HistoryBuildValues {
    backend: Option<String>,
    model: Option<String>,
    deep: bool,
    cargo: bool,
    dedup_llm: bool,
    token_budget: Option<usize>,
    resolution: f64,
    exclude_hubs: Option<f64>,
    gitignore: bool,
    excludes: Vec<String>,
    provider_endpoint: Option<String>,
    provider_temperature: Option<f64>,
    provider_max_output_tokens: Option<usize>,
    provider_region: Option<String>,
}

impl Default for HistoryBuildValues {
    fn default() -> Self {
        Self {
            backend: None,
            model: None,
            deep: false,
            cargo: false,
            dedup_llm: false,
            token_budget: None,
            resolution: 1.0,
            exclude_hubs: None,
            gitignore: true,
            excludes: Vec::new(),
            provider_endpoint: None,
            provider_temperature: None,
            provider_max_output_tokens: None,
            provider_region: None,
        }
    }
}

pub(crate) fn parse_build_command(
    command: &str,
    args: &[String],
) -> Result<ParsedBuildCommand, String> {
    let mut values = HistoryBuildValues::default();
    let mut revision = None;
    let mut format = None;
    let mut replace_corrupt = false;
    let mut profile_from = None;
    let mut seen = std::collections::BTreeSet::new();
    let mut options = true;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if options && argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if !options || !argument.starts_with('-') {
            if revision.replace(argument.to_owned()).is_some() {
                return Err(format!("history {command} requires exactly one revision"));
            }
            index += 1;
            continue;
        }
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        match name {
            "--cargo" | "--dedup-llm" | "--no-gitignore" | "--replace-corrupt" => {
                if inline.is_some() {
                    return Err(format!("{name} does not accept a value"));
                }
                if !seen.insert(name.to_owned()) {
                    return Err(format!("duplicate {name}"));
                }
                match name {
                    "--cargo" => values.cargo = true,
                    "--dedup-llm" => values.dedup_llm = true,
                    "--no-gitignore" => values.gitignore = false,
                    "--replace-corrupt" => replace_corrupt = true,
                    _ => unreachable!(),
                }
            }
            "--backend" | "--model" | "--mode" | "--token-budget" | "--resolution"
            | "--exclude-hubs" | "--format" | "--profile-from" => {
                if !seen.insert(name.to_owned()) {
                    return Err(format!("duplicate {name}"));
                }
                let value = option_value(args, &mut index, name, inline)?;
                match name {
                    "--backend" => values.backend = Some(nonempty(name, value)?.to_owned()),
                    "--model" => values.model = Some(nonempty(name, value)?.to_owned()),
                    "--mode" if value == "deep" => values.deep = true,
                    "--mode" => return Err("--mode must be deep".to_owned()),
                    "--token-budget" => values.token_budget = Some(positive_usize(name, value)?),
                    "--resolution" => values.resolution = positive_float(name, value)?,
                    "--exclude-hubs" => values.exclude_hubs = Some(finite_float(name, value)?),
                    "--format" => format = Some(value.to_owned()),
                    "--profile-from" => profile_from = Some(nonempty(name, value)?.to_owned()),
                    _ => unreachable!(),
                }
            }
            "--exclude" => {
                let value = option_value(args, &mut index, name, inline)?;
                values.excludes.push(nonempty(name, value)?.to_owned());
            }
            "--allow-partial" | "--code-only" | "--no-cluster" => {
                return Err(format!(
                    "{name} is incompatible with complete graph history"
                ));
            }
            "--google-workspace" | "--postgres" | "--global" | "--as" => {
                return Err(format!(
                    "{name} is not supported for immutable Git-commit history"
                ));
            }
            _ => return Err(format!("unknown history build option {argument}")),
        }
        index += 1;
    }
    let revision = revision.ok_or_else(|| format!("history {command} requires REV"))?;
    let format = format.unwrap_or_else(|| "text".to_owned());
    if !matches!(format.as_str(), "text" | "json") {
        return Err("--format must be text or json".to_owned());
    }
    if replace_corrupt && command != "rebuild" {
        return Err("--replace-corrupt is only valid for history rebuild".to_owned());
    }
    if profile_from.is_some() && command != "build" {
        return Err("--profile-from is only valid for history build".to_owned());
    }
    let direct_profile_option = seen.iter().any(|option| {
        !matches!(
            option.as_str(),
            "--format" | "--profile-from" | "--replace-corrupt"
        )
    }) || !values.excludes.is_empty();
    if profile_from.is_some() && direct_profile_option {
        return Err("--profile-from cannot be combined with build-profile options".to_owned());
    }
    let options = HistoryBuildOptions::from_values(values).map_err(|error| error.to_string())?;
    Ok(ParsedBuildCommand {
        revision,
        format,
        replace_corrupt,
        profile_from,
        options,
    })
}

pub(crate) fn parse_enable_options(args: &[String]) -> Result<HistoryBuildOptions, String> {
    if args.iter().any(|argument| {
        argument == "--format"
            || argument.starts_with("--format=")
            || argument == "--replace-corrupt"
    }) {
        return Err("history enable accepts build-profile options only".to_owned());
    }
    let mut build = vec!["__enable_profile__".to_owned()];
    build.extend_from_slice(args);
    parse_build_command("enable", &build).map(|parsed| parsed.options)
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
    inline: Option<&'a str>,
) -> Result<&'a str, String> {
    if let Some(value) = inline {
        return nonempty(option, value);
    }
    *index += 1;
    let value = args
        .get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))?;
    if value.starts_with('-') {
        return Err(format!("{option} requires a value"));
    }
    nonempty(option, value)
}

fn nonempty<'a>(option: &str, value: &'a str) -> Result<&'a str, String> {
    if value.trim().is_empty() {
        Err(format!("{option} requires a value"))
    } else {
        Ok(value)
    }
}

fn positive_usize(option: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{option} must be a positive integer"))
}

fn positive_float(option: &str, value: &str) -> Result<f64, String> {
    let value = finite_float(option, value)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(format!("{option} must be greater than zero"))
    }
}

fn finite_float(option: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{option} must be a finite number"))
}

fn normalized_float(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

fn resolve_provider(values: &mut HistoryBuildValues) -> Result<(), HistoryError> {
    let environment = std::env::vars().collect::<HashMap<_, _>>();
    let requested = values
        .backend
        .clone()
        .or_else(|| compass_semantic::detect_builtin_backend(&environment).map(str::to_owned));
    let Some(name) = requested else {
        return Ok(());
    };
    if compass_semantic::builtin_backend(&name).is_none() {
        return Err(HistoryError::InvalidFingerprint(format!(
            "custom backend {name:?} is not supported for immutable graph history"
        )));
    }
    if name == "claude-cli" {
        return Err(HistoryError::InvalidFingerprint(
            "claude-cli is not supported for immutable graph history".to_owned(),
        ));
    }
    let resolved =
        compass_semantic::resolve_builtin_backend(&name, &environment, values.model.as_deref())
            .map_err(|error| HistoryError::InvalidFingerprint(error.to_string()))?;
    if let Some(endpoint) = resolved.base_url.as_deref() {
        let parsed = url::Url::parse(endpoint)
            .map_err(|error| HistoryError::InvalidFingerprint(error.to_string()))?;
        if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some()
        {
            return Err(HistoryError::InvalidFingerprint(
                "provider endpoint credentials or query parameters cannot enter graph history"
                    .to_owned(),
            ));
        }
    }
    values.backend = Some(name);
    values.model = Some(resolved.model);
    values.provider_endpoint = resolved.base_url;
    values.provider_temperature = resolved.temperature;
    values.provider_max_output_tokens = Some(resolved.max_output_tokens);
    values.provider_region = environment
        .get("AWS_REGION")
        .or_else(|| environment.get("AWS_DEFAULT_REGION"))
        .filter(|value| !value.is_empty())
        .cloned();
    if values.backend.as_deref() == Some("bedrock") && values.provider_region.is_none() {
        return Err(HistoryError::InvalidFingerprint(
            "Bedrock graph history requires an explicit AWS_REGION or AWS_DEFAULT_REGION"
                .to_owned(),
        ));
    }
    Ok(())
}

pub(crate) struct NativeCompleteGraphBuilder {
    executable: PathBuf,
    forwarded: Vec<String>,
    gitignore: bool,
    excludes: Vec<String>,
    semantic_environment: Vec<(String, String)>,
    semantic_environment_remove: Vec<String>,
}

impl CompleteGraphBuilder for NativeCompleteGraphBuilder {
    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError> {
        if let Some(seed) = seed {
            seed.write_seed(&output_root.join("compass-out"), &seed_completion(seed))?;
        }
        let mut command = Command::new(&self.executable);
        command
            .arg("extract")
            .arg(checkout)
            .arg("--out")
            .arg(output_root)
            .arg("--no-viz")
            .args(&self.forwarded)
            .current_dir(checkout)
            .env("GRAPHIFY_SKIP_HOOK", "1")
            .env("COMPASS_HISTORY_BUILD", "1")
            .env("COMPASS_OUT", "compass-out")
            .envs(self.semantic_environment.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_PREFIX",
        ] {
            command.env_remove(variable);
        }
        for variable in &self.semantic_environment_remove {
            command.env_remove(variable);
        }
        let output = bounded_output(command)?;
        if !output.status.success() {
            return Err(MaterializeError::BuilderProcess {
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }

        let output_dir = output_root.join("compass-out");
        let detection = detect(
            checkout,
            &DetectOptions {
                gitignore: self.gitignore,
                ignore_policy: IgnorePolicy::HistoricalCommit,
                extra_excludes: self.excludes.clone(),
                cache_root: Some(output_root.to_path_buf()),
                output_name: "compass-out".to_owned(),
                ..DetectOptions::default()
            },
        )?;
        let semantic = ["document", "paper", "image", "video"]
            .into_iter()
            .flat_map(|kind| detection.files.get(kind).into_iter().flatten())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let manifest = Manifest::load(&output_dir.join("manifest.json"), Some(checkout));
        let completed = semantic
            .iter()
            .filter(|path| {
                manifest
                    .entries()
                    .get(&path.to_string_lossy().into_owned())
                    .is_some_and(|entry| !entry.semantic_hash.is_empty())
            })
            .count();
        let semantic_files_expected = u64::try_from(semantic.len())
            .map_err(|_| MaterializeError::Builder("semantic corpus exceeds u64".to_owned()))?;
        let semantic_files_completed = u64::try_from(completed)
            .map_err(|_| MaterializeError::Builder("semantic completion exceeds u64".to_owned()))?;
        CompletedGraphArtifacts::load(
            &output_dir,
            CompletionEvidence {
                extraction_succeeded: true,
                allow_partial: false,
                semantic_files_expected,
                semantic_files_completed,
                failed_chunks: 0,
            },
        )
        .map_err(Into::into)
    }
}

fn seed_completion(seed: &GraphArtifacts) -> CompletionEvidence {
    let completed = seed
        .manifest
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|manifest| manifest.values())
        .filter(|entry| {
            entry
                .get("semantic_hash")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|hash| !hash.is_empty())
        })
        .count() as u64;
    CompletionEvidence {
        extraction_succeeded: true,
        allow_partial: false,
        semantic_files_expected: completed,
        semantic_files_completed: completed,
        failed_chunks: 0,
    }
}

struct BoundedOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn bounded_output(mut command: Command) -> Result<BoundedOutput, MaterializeError> {
    let mut child = command
        .spawn()
        .map_err(|source| MaterializeError::BuilderIo {
            operation: "start",
            source,
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MaterializeError::Builder("builder stdout was not piped".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MaterializeError::Builder("builder stderr was not piped".to_owned()))?;
    let stdout = thread::spawn(move || read_bounded(stdout));
    let stderr = thread::spawn(move || read_bounded(stderr));
    let status = child.wait().map_err(|source| MaterializeError::BuilderIo {
        operation: "wait for",
        source,
    })?;
    let stdout = stdout
        .join()
        .map_err(|_| MaterializeError::Builder("builder stdout reader panicked".to_owned()))?
        .map_err(|source| MaterializeError::BuilderIo {
            operation: "read stdout from",
            source,
        })?;
    let stderr = stderr
        .join()
        .map_err(|_| MaterializeError::Builder("builder stderr reader panicked".to_owned()))?
        .map_err(|source| MaterializeError::BuilderIo {
            operation: "read stderr from",
            source,
        })?;
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded(mut reader: impl Read) -> Result<String, std::io::Error> {
    const TRUNCATED_SUFFIX: &[u8] = b"\n...[truncated]";
    let content_limit = MAX_DIAGNOSTIC_BYTES.saturating_sub(TRUNCATED_SUFFIX.len());
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let available = content_limit.saturating_sub(retained.len());
        let keep = available.min(count);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    if truncated {
        retained.extend_from_slice(TRUNCATED_SUFFIX);
    }
    Ok(String::from_utf8_lossy(&retained).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_parser_normalizes_profiles_and_rejects_partial_or_external_inputs()
    -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_build_command(
            "build",
            &[
                "HEAD".to_owned(),
                "--mode=deep".to_owned(),
                "--token-budget".to_owned(),
                "4096".to_owned(),
                "--exclude=a".to_owned(),
                "--exclude".to_owned(),
                "!a/keep.rs".to_owned(),
                "--format=json".to_owned(),
            ],
        )?;
        assert_eq!(parsed.revision, "HEAD");
        assert_eq!(parsed.format, "json");
        assert_eq!(parsed.options.profile().value("token_budget"), Some("4096"));
        let inherited = parse_build_command(
            "build",
            &[
                "HEAD".to_owned(),
                "--profile-from".to_owned(),
                "HEAD~1".to_owned(),
            ],
        )?;
        assert_eq!(inherited.profile_from.as_deref(), Some("HEAD~1"));
        assert!(
            parse_build_command(
                "build",
                &[
                    "HEAD".to_owned(),
                    "--profile-from=HEAD~1".to_owned(),
                    "--cargo".to_owned(),
                ],
            )
            .is_err()
        );
        assert!(
            parse_build_command("build", &["HEAD".to_owned(), "--code-only".to_owned()]).is_err()
        );
        assert!(
            parse_build_command(
                "build",
                &["HEAD".to_owned(), "--google-workspace".to_owned()]
            )
            .is_err()
        );
        assert!(
            parse_build_command(
                "build",
                &["HEAD".to_owned(), "--replace-corrupt".to_owned()]
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn diagnostics_never_exceed_the_documented_limit() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = vec![b'x'; MAX_DIAGNOSTIC_BYTES + 1024];
        let output = read_bounded(bytes.as_slice())?;
        assert_eq!(output.len(), MAX_DIAGNOSTIC_BYTES);
        assert!(output.ends_with("...[truncated]"));
        Ok(())
    }

    #[test]
    fn build_parser_covers_complete_profile_and_every_argument_boundary()
    -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_build_command(
            "rebuild",
            &[
                "HEAD".to_owned(),
                "--cargo".to_owned(),
                "--dedup-llm".to_owned(),
                "--no-gitignore".to_owned(),
                "--mode".to_owned(),
                "deep".to_owned(),
                "--token-budget=1".to_owned(),
                "--resolution".to_owned(),
                "0.5".to_owned(),
                "--exclude-hubs=-0.0".to_owned(),
                "--exclude".to_owned(),
                "target".to_owned(),
                "--replace-corrupt".to_owned(),
                "--format".to_owned(),
                "text".to_owned(),
            ],
        )?;
        assert!(parsed.replace_corrupt);
        assert_eq!(parsed.revision, "HEAD");
        assert_eq!(parsed.options.profile().value("cargo"), Some("true"));
        assert_eq!(parsed.options.profile().value("gitignore"), Some("false"));
        assert_eq!(parsed.options.profile().value("exclude_hubs"), Some("0"));

        let end_marker = parse_build_command("build", &["--".to_owned(), "-revision".to_owned()])?;
        assert_eq!(end_marker.revision, "-revision");

        for arguments in [
            vec![],
            vec!["one", "two"],
            vec!["HEAD", "--cargo=true"],
            vec!["HEAD", "--cargo", "--cargo"],
            vec!["HEAD", "--mode", "shallow"],
            vec!["HEAD", "--mode"],
            vec!["HEAD", "--token-budget", "0"],
            vec!["HEAD", "--token-budget", "word"],
            vec!["HEAD", "--resolution", "0"],
            vec!["HEAD", "--resolution", "NaN"],
            vec!["HEAD", "--exclude-hubs", "NaN"],
            vec!["HEAD", "--exclude", ""],
            vec!["HEAD", "--format", "yaml"],
            vec!["HEAD", "--unknown"],
            vec!["HEAD", "--allow-partial"],
            vec!["HEAD", "--code-only"],
            vec!["HEAD", "--no-cluster"],
            vec!["HEAD", "--google-workspace"],
            vec!["HEAD", "--postgres"],
            vec!["HEAD", "--global"],
            vec!["HEAD", "--as"],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(
                parse_build_command("build", &arguments).is_err(),
                "unexpectedly accepted {arguments:?}"
            );
        }
        assert!(
            parse_build_command(
                "build",
                &["HEAD".to_owned(), "--replace-corrupt".to_owned()]
            )
            .is_err()
        );
        assert!(parse_enable_options(&["--format=json".to_owned()]).is_err());
        assert!(parse_enable_options(&["--replace-corrupt".to_owned()]).is_err());
        assert!(parse_enable_options(&["--cargo".to_owned()]).is_ok());
        Ok(())
    }

    #[test]
    fn numeric_profile_helpers_reject_non_finite_and_non_positive_values() {
        assert_eq!(nonempty("--x", " value "), Ok(" value "));
        assert!(nonempty("--x", " \t").is_err());
        assert_eq!(positive_usize("--x", "2"), Ok(2));
        assert!(positive_usize("--x", "0").is_err());
        assert_eq!(positive_float("--x", "0.25"), Ok(0.25));
        assert!(positive_float("--x", "-1").is_err());
        assert!(finite_float("--x", "inf").is_err());
        assert_eq!(normalized_float(-0.0), "0");
        assert_eq!(normalized_float(1.25), "1.25");
    }
}
