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
}

pub(crate) struct ParsedBuildCommand {
    pub(crate) revision: String,
    pub(crate) format: String,
    pub(crate) replace_corrupt: bool,
    pub(crate) options: HistoryBuildOptions,
}

impl HistoryBuildOptions {
    pub(crate) fn defaults() -> Result<Self, HistoryError> {
        Self::from_values(HistoryBuildValues::default())
    }

    pub(crate) fn profile(&self) -> BuildProfile {
        self.profile.clone()
    }

    pub(crate) fn builder(&self, executable: PathBuf) -> NativeCompleteGraphBuilder {
        NativeCompleteGraphBuilder {
            executable,
            forwarded: self.forwarded.clone(),
            gitignore: self.gitignore,
            excludes: self.excludes.clone(),
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

        Ok(Self {
            profile,
            forwarded,
            gitignore: values.gitignore,
            excludes: values.excludes,
        })
    }
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
            | "--exclude-hubs" | "--format" => {
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
    let options = HistoryBuildOptions::from_values(values).map_err(|error| error.to_string())?;
    Ok(ParsedBuildCommand {
        revision,
        format,
        replace_corrupt,
        options,
    })
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
    let resolved =
        compass_semantic::resolve_builtin_backend(&name, &environment, values.model.as_deref())
            .map_err(|error| HistoryError::InvalidFingerprint(error.to_string()))?;
    values.backend = Some(name);
    values.model = Some(resolved.model);
    Ok(())
}

pub(crate) struct NativeCompleteGraphBuilder {
    executable: PathBuf,
    forwarded: Vec<String>,
    gitignore: bool,
    excludes: Vec<String>,
}

impl CompleteGraphBuilder for NativeCompleteGraphBuilder {
    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError> {
        if let Some(seed) = seed {
            seed.write_seed(&output_root.join("graphify-out"), &seed_completion(seed))?;
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
            .env("GRAPHIFY_OUT", "graphify-out")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = bounded_output(command)?;
        if !output.status.success() {
            return Err(MaterializeError::BuilderProcess {
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }

        let output_dir = output_root.join("graphify-out");
        let detection = detect(
            checkout,
            &DetectOptions {
                gitignore: self.gitignore,
                ignore_policy: IgnorePolicy::HistoricalCommit,
                extra_excludes: self.excludes.clone(),
                cache_root: Some(output_root.to_path_buf()),
                output_name: "graphify-out".to_owned(),
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
}
