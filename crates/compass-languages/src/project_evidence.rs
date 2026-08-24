use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tree_sitter::Parser;

use crate::frameworks::typescript_syntax::{StaticValue, TypeScriptSyntax};
use crate::json_config::parse_jsonc;

pub const FRAMEWORK_PROJECT_EVIDENCE_EXTENSION: &str = "_compass_framework_project_evidence";

const EVIDENCE_SCHEMA: &str = "compass.framework-project-evidence/1";
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DEPENDENCIES_PER_PROJECT: usize = 10_000;
const MAX_PROJECT_CONFIGURATIONS: usize = 256;
const MAX_PROJECT_CONFIGURATION_KEYS: usize = 2_000;
const MAX_PROJECT_ALIASES: usize = 2_000;
const MAX_PROJECT_PLUGINS: usize = 2_000;
const MAX_PROJECT_ROUTE_ROOTS: usize = 256;
const MAX_COMPOSER_AUTOLOAD_ROOTS: usize = 4_096;
const MAX_COMPOSER_ROOTS_PER_PREFIX: usize = 64;
const MAX_COMPOSER_NAMESPACE_PREFIX_BYTES: usize = 1_024;
const MAX_COMPOSER_DIRECTORY_BYTES: usize = 4_096;
const MAX_PROJECT_EVIDENCE_DIAGNOSTICS: usize = 4_096;
const MAX_PROJECT_SCAN_DIRECTORIES: usize = 4_096;
const MAX_PROJECT_DIRECTORY_ENTRIES: usize = 4_096;
const FIXED_MANIFEST_NAMES: &[&str] = &[
    "package.json",
    "composer.json",
    "pyproject.toml",
    "requirements.txt",
    "requirements.in",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Cargo.toml",
    "go.mod",
    "Package.swift",
    "pubspec.yaml",
    "pubspec.yml",
    "build.sbt",
];
const FIXED_CONFIGURATION_NAMES: &[&str] = &[
    "application.properties",
    "application.yml",
    "application.yaml",
    "astro.config.js",
    "astro.config.mjs",
    "astro.config.ts",
    "bootstrap.yml",
    "bootstrap.yaml",
    "jsconfig.json",
    "next.config.js",
    "next.config.mjs",
    "next.config.ts",
    "nuxt.config.js",
    "nuxt.config.ts",
    "remix.config.js",
    "remix.config.cjs",
    "remix.config.mjs",
    "remix.config.ts",
    "svelte.config.js",
    "svelte.config.ts",
    "tsconfig.json",
    "vite.config.cjs",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.ts",
    "webpack.config.js",
    "webpack.config.ts",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEvidence {
    project_root: PathBuf,
    manifests: Vec<String>,
    ecosystems: Vec<String>,
    metadata: BTreeMap<String, String>,
    source_roots: Vec<String>,
    dependencies: BTreeSet<String>,
    configuration_files: Vec<String>,
    configuration_keys: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    plugins: BTreeSet<String>,
    route_roots: BTreeMap<String, BTreeSet<String>>,
    composer_autoload_roots: Vec<ComposerAutoloadRoot>,
    diagnostics: Vec<ProjectEvidenceDiagnostic>,
    fingerprint: String,
}

/// One repository-contained Composer PSR-4 mapping.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComposerAutoloadRoot {
    pub namespace_prefix: String,
    pub directory: String,
    pub development: bool,
    pub manifest: String,
}

/// A deterministic project-manifest diagnostic retained with cache evidence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectEvidenceDiagnostic {
    pub code: String,
    pub manifest: String,
    pub message: String,
}

impl ProjectEvidence {
    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    #[must_use]
    pub fn manifests(&self) -> &[String] {
        &self.manifests
    }

    #[must_use]
    pub fn ecosystems(&self) -> &[String] {
        &self.ecosystems
    }

    /// Bounded, source-only project metadata such as a package name or an
    /// explicitly declared language/toolchain version. Values are never
    /// obtained by evaluating a build tool or project script.
    #[must_use]
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Project-contained source roots declared by a manifest. The paths are
    /// normalized relative paths and are only advisory to downstream stages.
    #[must_use]
    pub fn source_roots(&self) -> &[String] {
        &self.source_roots
    }

    #[must_use]
    pub fn dependencies(&self) -> &BTreeSet<String> {
        &self.dependencies
    }

    #[must_use]
    pub fn configuration_files(&self) -> &[String] {
        &self.configuration_files
    }

    #[must_use]
    pub fn configuration_keys(&self) -> &BTreeSet<String> {
        &self.configuration_keys
    }

    #[must_use]
    pub fn aliases(&self) -> &BTreeMap<String, String> {
        &self.aliases
    }

    #[must_use]
    pub fn plugins(&self) -> &BTreeSet<String> {
        &self.plugins
    }

    #[must_use]
    pub fn route_roots(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.route_roots
    }

    #[must_use]
    pub fn composer_autoload_roots(&self) -> &[ComposerAutoloadRoot] {
        &self.composer_autoload_roots
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ProjectEvidenceDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[must_use]
    pub fn has_dependency(&self, dependency: &str) -> bool {
        let dependency = normalize_dependency(dependency);
        self.dependencies.contains(&dependency)
    }

    #[must_use]
    pub fn has_any_dependency(&self, dependencies: &[&str]) -> bool {
        dependencies
            .iter()
            .any(|dependency| self.has_dependency(dependency))
    }

    #[must_use]
    pub fn has_configuration(&self, name: &str) -> bool {
        let expected = normalize_project_name(name);
        self.configuration_files.iter().any(|file| {
            normalize_project_name(file) == expected
                || Path::new(file)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|file_name| normalize_project_name(file_name) == expected)
        })
    }

    #[must_use]
    pub fn has_any_configuration(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.has_configuration(name))
    }

    #[must_use]
    pub fn has_plugin(&self, plugin: &str) -> bool {
        self.plugins.contains(&normalize_dependency(plugin))
    }

    #[must_use]
    pub fn has_any_plugin(&self, plugins: &[&str]) -> bool {
        plugins.iter().any(|plugin| self.has_plugin(plugin))
    }

    #[must_use]
    pub fn has_route_root(&self, framework: &str, root: &str) -> bool {
        self.route_roots
            .get(&normalize_dependency(framework))
            .is_some_and(|roots| roots.contains(&normalize_project_path(root)))
    }
}

#[derive(Clone, Debug)]
pub struct ProjectEvidenceIndex {
    repository_root: PathBuf,
    projects: BTreeMap<PathBuf, ProjectEvidence>,
    fallback: ProjectEvidence,
}

impl ProjectEvidenceIndex {
    #[must_use]
    pub fn build(repository_root: &Path, sources: &[PathBuf]) -> Self {
        let repository_root = absolute_path(repository_root, repository_root);
        let mut directories = BTreeSet::new();
        let mut project_files = BTreeSet::new();
        directories.insert(repository_root.clone());

        for source in sources {
            let source = absolute_path(&repository_root, source);
            if is_project_file(&source) {
                project_files.insert(source.clone());
            }
            let directory = source.parent().unwrap_or(&repository_root);
            for ancestor in directory.ancestors() {
                if !ancestor.starts_with(&repository_root) {
                    break;
                }
                if directories.len() < MAX_PROJECT_SCAN_DIRECTORIES
                    || directories.contains(ancestor)
                {
                    directories.insert(ancestor.to_path_buf());
                }
                if ancestor == repository_root {
                    break;
                }
            }
        }

        let seed_directories = directories.iter().cloned().collect::<Vec<_>>();
        for directory in seed_directories {
            for suffix in ["resources", "config", "conf"] {
                if directories.len() >= MAX_PROJECT_SCAN_DIRECTORIES {
                    break;
                }
                let candidate = directory.join(suffix);
                if candidate.starts_with(&repository_root) {
                    directories.insert(candidate);
                }
            }
        }

        for directory in &directories {
            for name in FIXED_MANIFEST_NAMES.iter().chain(FIXED_CONFIGURATION_NAMES) {
                let candidate = directory.join(name);
                if regular_project_file(&candidate) {
                    project_files.insert(candidate);
                }
            }
            if let Ok(entries) = fs::read_dir(directory) {
                for entry in entries.flatten().take(MAX_PROJECT_DIRECTORY_ENTRIES) {
                    let candidate = entry.path();
                    if regular_project_file(&candidate) {
                        project_files.insert(candidate);
                    }
                }
            }
        }

        let mut builders = BTreeMap::<PathBuf, ProjectBuilder>::new();
        let manifest_roots = project_files
            .iter()
            .filter(|path| is_recognized_manifest(path))
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        for project_file in project_files {
            let project_root =
                project_root_for_file(&repository_root, &manifest_roots, &project_file);
            let builder = builders.entry(project_root.clone()).or_default();
            if is_recognized_manifest(&project_file) {
                builder.manifests.insert(file_name(&project_file));
                if let Some(parsed) = parse_manifest(&project_file) {
                    builder.ecosystems.insert(parsed.ecosystem.to_owned());
                    for (key, value) in parsed.metadata {
                        if builder.metadata.len() < MAX_PROJECT_CONFIGURATION_KEYS
                            || builder.metadata.contains_key(&key)
                        {
                            builder.metadata.insert(key, value);
                        }
                    }
                    for root in parsed.source_roots {
                        let Some(root) =
                            contained_manifest_root(&repository_root, &project_root, &root)
                        else {
                            continue;
                        };
                        if builder.source_roots.len() < MAX_PROJECT_ROUTE_ROOTS {
                            builder.source_roots.insert(root);
                        }
                    }
                    let remaining =
                        MAX_DEPENDENCIES_PER_PROJECT.saturating_sub(builder.dependencies.len());
                    builder
                        .dependencies
                        .extend(parsed.dependencies.into_iter().take(remaining));
                }
                if file_name(&project_file).eq_ignore_ascii_case("composer.json") {
                    let (roots, mut diagnostics) = parse_composer_autoload_roots(
                        &repository_root,
                        &project_root,
                        &project_file,
                    );
                    let remaining = MAX_COMPOSER_AUTOLOAD_ROOTS
                        .saturating_sub(builder.composer_autoload_roots.len());
                    if roots.len() > remaining {
                        diagnostics.insert(project_diagnostic(
                            "composer_psr4_total_limit",
                            &relative_project_file(&repository_root, &project_file),
                            "Composer PSR-4 roots exceed the project-wide bounded limit",
                        ));
                    }
                    builder
                        .composer_autoload_roots
                        .extend(roots.into_iter().take(remaining));
                    let diagnostic_capacity =
                        MAX_PROJECT_EVIDENCE_DIAGNOSTICS.saturating_sub(builder.diagnostics.len());
                    builder
                        .diagnostics
                        .extend(diagnostics.into_iter().take(diagnostic_capacity));
                }
                if let Some(parsed) = parse_configuration(&project_file) {
                    builder.configuration_keys.extend(
                        parsed.configuration_keys.into_iter().take(
                            MAX_PROJECT_CONFIGURATION_KEYS
                                .saturating_sub(builder.configuration_keys.len()),
                        ),
                    );
                    builder.aliases.extend(
                        parsed
                            .aliases
                            .into_iter()
                            .take(MAX_PROJECT_ALIASES.saturating_sub(builder.aliases.len())),
                    );
                    builder.plugins.extend(
                        parsed
                            .plugins
                            .into_iter()
                            .take(MAX_PROJECT_PLUGINS.saturating_sub(builder.plugins.len())),
                    );
                }
            } else {
                if builder.configuration_files.len() < MAX_PROJECT_CONFIGURATIONS {
                    builder
                        .configuration_files
                        .insert(relative_project_file(&project_root, &project_file));
                }
                if let Some(parsed) = parse_configuration(&project_file) {
                    builder.configuration_keys.extend(
                        parsed.configuration_keys.into_iter().take(
                            MAX_PROJECT_CONFIGURATION_KEYS
                                .saturating_sub(builder.configuration_keys.len()),
                        ),
                    );
                    builder.aliases.extend(
                        parsed
                            .aliases
                            .into_iter()
                            .take(MAX_PROJECT_ALIASES.saturating_sub(builder.aliases.len())),
                    );
                    builder.plugins.extend(
                        parsed
                            .plugins
                            .into_iter()
                            .take(MAX_PROJECT_PLUGINS.saturating_sub(builder.plugins.len())),
                    );
                }
            }
        }

        for source in sources {
            let source = absolute_path(&repository_root, source);
            let project_root = project_root_for_source(&repository_root, &builders, &source);
            let Some((framework, route_root)) =
                route_root_for_source(&project_root, &source, &builders)
            else {
                continue;
            };
            let builder = builders.entry(project_root).or_default();
            let roots = builder.route_roots.entry(framework).or_default();
            if roots.len() < MAX_PROJECT_ROUTE_ROOTS {
                roots.insert(route_root);
            }
        }

        let projects = builders
            .into_iter()
            .map(|(project_root, builder)| {
                let evidence = finish_project(&repository_root, project_root.clone(), builder);
                (project_root, evidence)
            })
            .collect();
        let fallback = finish_project(
            &repository_root,
            repository_root.clone(),
            ProjectBuilder::default(),
        );
        Self {
            repository_root,
            projects,
            fallback,
        }
    }

    #[must_use]
    pub fn evidence_for(&self, path: &Path) -> &ProjectEvidence {
        let path = absolute_path(&self.repository_root, path);
        let directory = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(&self.repository_root)
        };
        directory
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&self.repository_root))
            .find_map(|ancestor| self.projects.get(ancestor))
            .unwrap_or(&self.fallback)
    }

    #[must_use]
    pub fn fingerprint_for(&self, path: &Path) -> &str {
        self.evidence_for(path).fingerprint()
    }

    #[must_use]
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }
}

#[derive(Default)]
struct ProjectBuilder {
    manifests: BTreeSet<String>,
    ecosystems: BTreeSet<String>,
    metadata: BTreeMap<String, String>,
    source_roots: BTreeSet<String>,
    dependencies: BTreeSet<String>,
    configuration_files: BTreeSet<String>,
    configuration_keys: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    plugins: BTreeSet<String>,
    route_roots: BTreeMap<String, BTreeSet<String>>,
    composer_autoload_roots: BTreeSet<ComposerAutoloadRoot>,
    diagnostics: BTreeSet<ProjectEvidenceDiagnostic>,
}

struct ParsedManifest {
    ecosystem: &'static str,
    dependencies: BTreeSet<String>,
    metadata: BTreeMap<String, String>,
    source_roots: BTreeSet<String>,
}

#[derive(Default)]
struct ParsedConfiguration {
    configuration_keys: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    plugins: BTreeSet<String>,
}

fn finish_project(
    repository_root: &Path,
    project_root: PathBuf,
    builder: ProjectBuilder,
) -> ProjectEvidence {
    let manifests = builder.manifests.into_iter().collect::<Vec<_>>();
    let ecosystems = builder.ecosystems.into_iter().collect::<Vec<_>>();
    let metadata = builder.metadata;
    let source_roots = builder.source_roots.into_iter().collect::<Vec<_>>();
    let dependencies = builder.dependencies;
    let configuration_files = builder.configuration_files.into_iter().collect::<Vec<_>>();
    let configuration_keys = builder.configuration_keys;
    let aliases = builder.aliases;
    let plugins = builder.plugins;
    let route_roots = builder.route_roots;
    let composer_autoload_roots = builder
        .composer_autoload_roots
        .into_iter()
        .collect::<Vec<_>>();
    let diagnostics = builder.diagnostics.into_iter().collect::<Vec<_>>();
    let relative_root = project_root
        .strip_prefix(repository_root)
        .unwrap_or(&project_root)
        .to_string_lossy()
        .replace('\\', "/");
    let mut digest = Sha256::new();
    digest.update(EVIDENCE_SCHEMA.as_bytes());
    digest.update([0]);
    digest.update(relative_root.as_bytes());
    digest.update([0]);
    for manifest in &manifests {
        digest.update(manifest.as_bytes());
        digest.update([0]);
    }
    for ecosystem in &ecosystems {
        digest.update(ecosystem.as_bytes());
        digest.update([0]);
    }
    for (key, value) in &metadata {
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    for source_root in &source_roots {
        digest.update(source_root.as_bytes());
        digest.update([0]);
    }
    for dependency in &dependencies {
        digest.update(dependency.as_bytes());
        digest.update([0]);
    }
    for configuration_file in &configuration_files {
        digest.update(configuration_file.as_bytes());
        digest.update([0]);
    }
    for configuration_key in &configuration_keys {
        digest.update(configuration_key.as_bytes());
        digest.update([0]);
    }
    for (alias, target) in &aliases {
        digest.update(alias.as_bytes());
        digest.update([0]);
        digest.update(target.as_bytes());
        digest.update([0]);
    }
    for plugin in &plugins {
        digest.update(plugin.as_bytes());
        digest.update([0]);
    }
    for (framework, roots) in &route_roots {
        digest.update(framework.as_bytes());
        digest.update([0]);
        for root in roots {
            digest.update(root.as_bytes());
            digest.update([0]);
        }
    }
    for root in &composer_autoload_roots {
        digest.update(root.namespace_prefix.as_bytes());
        digest.update([0]);
        digest.update(root.directory.as_bytes());
        digest.update([0]);
        digest.update([u8::from(root.development)]);
        digest.update(root.manifest.as_bytes());
        digest.update([0]);
    }
    for diagnostic in &diagnostics {
        digest.update(diagnostic.code.as_bytes());
        digest.update([0]);
        digest.update(diagnostic.manifest.as_bytes());
        digest.update([0]);
        digest.update(diagnostic.message.as_bytes());
        digest.update([0]);
    }
    ProjectEvidence {
        project_root,
        manifests,
        ecosystems,
        metadata,
        source_roots,
        dependencies,
        configuration_files,
        configuration_keys,
        aliases,
        plugins,
        route_roots,
        composer_autoload_roots,
        diagnostics,
        fingerprint: format!("sha256:{:x}", digest.finalize()),
    }
}

fn parse_composer_autoload_roots(
    repository_root: &Path,
    project_root: &Path,
    manifest: &Path,
) -> (
    BTreeSet<ComposerAutoloadRoot>,
    BTreeSet<ProjectEvidenceDiagnostic>,
) {
    let manifest_name = relative_project_file(repository_root, manifest);
    let mut roots = BTreeSet::new();
    let mut diagnostics = BTreeSet::new();
    let metadata = match fs::symlink_metadata(manifest) {
        Ok(metadata)
            if metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX_MANIFEST_BYTES =>
        {
            metadata
        }
        Ok(_) => {
            diagnostics.insert(project_diagnostic(
                "composer_manifest_rejected",
                &manifest_name,
                "Composer manifest is not a bounded regular file",
            ));
            return (roots, diagnostics);
        }
        Err(_) => return (roots, diagnostics),
    };
    let _ = metadata;
    let source = match fs::read_to_string(manifest) {
        Ok(source) => source,
        Err(_) => {
            diagnostics.insert(project_diagnostic(
                "composer_manifest_unreadable",
                &manifest_name,
                "Composer manifest could not be read as UTF-8",
            ));
            return (roots, diagnostics);
        }
    };
    let document = match serde_json::from_str::<Value>(&source) {
        Ok(Value::Object(document)) => document,
        Ok(_) | Err(_) => {
            diagnostics.insert(project_diagnostic(
                "composer_manifest_invalid",
                &manifest_name,
                "Composer manifest is not a valid JSON object",
            ));
            return (roots, diagnostics);
        }
    };
    for (section, development) in [("autoload", false), ("autoload-dev", true)] {
        let Some(psr4) = document
            .get(section)
            .and_then(Value::as_object)
            .and_then(|autoload| autoload.get("psr-4"))
        else {
            continue;
        };
        let Some(entries) = psr4.as_object() else {
            diagnostics.insert(project_diagnostic(
                "composer_psr4_invalid",
                &manifest_name,
                &format!("{section}.psr-4 must be an object"),
            ));
            continue;
        };
        for (prefix, directories) in entries {
            let Some(prefix) = normalize_composer_prefix(prefix) else {
                diagnostics.insert(project_diagnostic(
                    "composer_psr4_prefix_invalid",
                    &manifest_name,
                    &format!("invalid PSR-4 namespace prefix {prefix:?}"),
                ));
                continue;
            };
            let values = match directories {
                Value::String(directory) => vec![directory.as_str()],
                Value::Array(values) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .take(MAX_COMPOSER_ROOTS_PER_PREFIX.saturating_add(1))
                    .collect::<Vec<_>>(),
                _ => {
                    diagnostics.insert(project_diagnostic(
                        "composer_psr4_directory_invalid",
                        &manifest_name,
                        &format!("PSR-4 root for {prefix:?} must be a string or string array"),
                    ));
                    continue;
                }
            };
            if values.len() > MAX_COMPOSER_ROOTS_PER_PREFIX {
                diagnostics.insert(project_diagnostic(
                    "composer_psr4_root_limit",
                    &manifest_name,
                    &format!("PSR-4 root count for {prefix:?} exceeds the bounded limit"),
                ));
                continue;
            }
            for directory in values {
                let Some(directory) =
                    contained_composer_directory(repository_root, project_root, directory)
                else {
                    diagnostics.insert(project_diagnostic(
                        "composer_psr4_directory_rejected",
                        &manifest_name,
                        &format!("PSR-4 root {directory:?} is invalid or leaves the repository"),
                    ));
                    continue;
                };
                if roots.len() >= MAX_COMPOSER_AUTOLOAD_ROOTS {
                    diagnostics.insert(project_diagnostic(
                        "composer_psr4_total_limit",
                        &manifest_name,
                        "Composer PSR-4 roots exceed the project-wide bounded limit",
                    ));
                    return (roots, diagnostics);
                }
                roots.insert(ComposerAutoloadRoot {
                    namespace_prefix: prefix.clone(),
                    directory,
                    development,
                    manifest: manifest_name.clone(),
                });
            }
        }
    }
    (roots, diagnostics)
}

fn normalize_composer_prefix(prefix: &str) -> Option<String> {
    let prefix = prefix
        .trim()
        .trim_start_matches('\\')
        .trim_end_matches('\\');
    if prefix.is_empty() {
        return Some(String::new());
    }
    if prefix.len() > MAX_COMPOSER_NAMESPACE_PREFIX_BYTES {
        return None;
    }
    if !prefix.split('\\').all(valid_php_identifier) {
        return None;
    }
    Some(format!("{prefix}\\"))
}

fn valid_php_identifier(segment: &str) -> bool {
    let mut bytes = segment.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn contained_composer_directory(
    repository_root: &Path,
    project_root: &Path,
    directory: &str,
) -> Option<String> {
    if directory.len() > MAX_COMPOSER_DIRECTORY_BYTES
        || directory.starts_with(['/', '\\'])
        || directory.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }
    let mut candidate = project_root.to_path_buf();
    for component in Path::new(directory).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => candidate.push(segment),
            std::path::Component::ParentDir => {
                if candidate == repository_root || !candidate.pop() {
                    return None;
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    if !candidate.starts_with(repository_root) {
        return None;
    }
    let canonical_repository = fs::canonicalize(repository_root).ok()?;
    let mut existing_ancestor = candidate.clone();
    while !existing_ancestor.exists() {
        if existing_ancestor == repository_root || !existing_ancestor.pop() {
            return None;
        }
    }
    let canonical_ancestor = fs::canonicalize(existing_ancestor).ok()?;
    if !canonical_ancestor.starts_with(canonical_repository) {
        return None;
    }
    let relative = candidate.strip_prefix(repository_root).ok()?;
    let normalized = normalize_project_path(&relative.to_string_lossy());
    Some(if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    })
}

fn project_diagnostic(code: &str, manifest: &str, message: &str) -> ProjectEvidenceDiagnostic {
    ProjectEvidenceDiagnostic {
        code: code.to_owned(),
        manifest: manifest.to_owned(),
        message: message.to_owned(),
    }
}

fn parse_manifest(path: &Path) -> Option<ParsedManifest> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let source = fs::read_to_string(path).ok()?;
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    let (ecosystem, dependencies, metadata, source_roots) = match lower.as_str() {
        "package.json" => (
            "npm",
            json_dependencies(&source, NPM_DEPENDENCY_KEYS)?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "composer.json" => (
            "composer",
            json_dependencies(&source, &["require", "require-dev"])?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "pyproject.toml" => (
            "python",
            pyproject_dependencies(&source)?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "requirements.txt" | "requirements.in" => (
            "python",
            requirements_dependencies(&source),
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "gemfile" => (
            "ruby",
            gemfile_dependencies(&source),
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "pom.xml" => (
            "maven",
            pom_dependencies(&source)?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "build.gradle" | "build.gradle.kts" => {
            let (metadata, source_roots) = gradle_project_metadata(&source);
            (
                "gradle",
                gradle_dependencies(&source),
                metadata,
                source_roots,
            )
        }
        "cargo.toml" => (
            "cargo",
            cargo_dependencies(&source)?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "go.mod" => (
            "go",
            go_mod_dependencies(&source),
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        "package.swift" => {
            let (metadata, source_roots) = swift_package_metadata(&source);
            (
                "swift",
                swift_package_dependencies(&source),
                metadata,
                source_roots,
            )
        }
        "pubspec.yaml" | "pubspec.yml" => {
            let parsed = pubspec_metadata(&source)?;
            (
                "dart",
                parsed.dependencies,
                parsed.metadata,
                parsed.source_roots,
            )
        }
        "build.sbt" => {
            let (dependencies, metadata, source_roots) = sbt_project_metadata(&source);
            ("scala", dependencies, metadata, source_roots)
        }
        _ if lower.ends_with(".csproj") => (
            "dotnet",
            csproj_dependencies(&source)?,
            BTreeMap::new(),
            BTreeSet::new(),
        ),
        _ => return None,
    };
    Some(ParsedManifest {
        ecosystem,
        dependencies: dependencies
            .into_iter()
            .map(|dependency| normalize_dependency(&dependency))
            .filter(|dependency| !dependency.is_empty())
            .take(MAX_DEPENDENCIES_PER_PROJECT)
            .collect(),
        metadata,
        source_roots,
    })
}

const NPM_DEPENDENCY_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

fn json_dependencies(source: &str, keys: &[&str]) -> Option<Vec<String>> {
    let root = serde_json::from_str::<Value>(source).ok()?;
    let object = root.as_object()?;
    Some(
        keys.iter()
            .filter_map(|key| object.get(*key).and_then(Value::as_object))
            .flat_map(|dependencies| dependencies.keys().cloned())
            .collect(),
    )
}

fn pyproject_dependencies(source: &str) -> Option<Vec<String>> {
    let root = toml::from_str::<toml::Table>(source).ok()?;
    let mut dependencies = root
        .get("project")
        .and_then(toml::Value::as_table)
        .and_then(|project| project.get("dependencies"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(python_requirement_name)
        .collect::<Vec<_>>();
    if let Some(poetry) = root
        .get("tool")
        .and_then(toml::Value::as_table)
        .and_then(|tool| tool.get("poetry"))
        .and_then(toml::Value::as_table)
        .and_then(|poetry| poetry.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        dependencies.extend(
            poetry
                .keys()
                .filter(|dependency| !dependency.eq_ignore_ascii_case("python"))
                .cloned(),
        );
    }
    Some(dependencies)
}

fn requirements_dependencies(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(['#', '-']))
        .map(python_requirement_name)
        .collect()
}

fn python_requirement_name(value: &str) -> String {
    value
        .split(|character: char| {
            character.is_whitespace()
                || matches!(character, '<' | '>' | '=' | '!' | '~' | ';' | '[' | '(')
        })
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn gemfile_dependencies(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("gem").map(str::trim_start))
        .filter_map(first_quoted)
        .collect()
}

fn pom_dependencies(source: &str) -> Option<Vec<String>> {
    let document = roxmltree::Document::parse(source).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "dependency")
            .filter_map(|dependency| {
                let child = |name: &str| {
                    dependency
                        .children()
                        .find(|node| node.is_element() && node.tag_name().name() == name)
                        .and_then(|node| node.text())
                };
                let artifact = child("artifactId")?;
                Some(child("groupId").map_or_else(
                    || artifact.to_owned(),
                    |group| format!("{group}:{artifact}"),
                ))
            })
            .collect(),
    )
}

fn gradle_dependencies(source: &str) -> Vec<String> {
    quoted_values(source)
        .filter(|value| value.matches(':').count() >= 1)
        .filter_map(|value| {
            let mut parts = value.split(':');
            let group = parts.next()?;
            let artifact = parts.next()?;
            (!group.is_empty() && !artifact.is_empty()).then(|| format!("{group}:{artifact}"))
        })
        .collect()
}

fn cargo_dependencies(source: &str) -> Option<Vec<String>> {
    let root = toml::from_str::<toml::Table>(source).ok()?;
    let mut dependencies = Vec::new();
    collect_cargo_dependency_table(&root, &mut dependencies);
    if let Some(targets) = root.get("target").and_then(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            collect_cargo_dependency_table(target, &mut dependencies);
        }
    }
    Some(dependencies)
}

fn collect_cargo_dependency_table(table: &toml::Table, dependencies: &mut Vec<String>) {
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(values) = table.get(key).and_then(toml::Value::as_table) {
            dependencies.extend(values.keys().cloned());
        }
    }
}

fn go_mod_dependencies(source: &str) -> Vec<String> {
    let mut dependencies = Vec::new();
    let mut in_require = false;
    for line in source.lines().map(str::trim) {
        if line == "require (" {
            in_require = true;
            continue;
        }
        if in_require && line == ")" {
            in_require = false;
            continue;
        }
        let requirement = if in_require {
            line
        } else if let Some(requirement) = line.strip_prefix("require ") {
            requirement
        } else {
            continue;
        };
        if let Some(dependency) = requirement.split_whitespace().next() {
            dependencies.push(dependency.to_owned());
        }
    }
    dependencies
}

fn csproj_dependencies(source: &str) -> Option<Vec<String>> {
    let document = roxmltree::Document::parse(source).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "PackageReference")
            .filter_map(|node| {
                node.attribute("Include")
                    .or_else(|| node.attribute("Update"))
            })
            .map(str::to_owned)
            .collect(),
    )
}

fn swift_package_dependencies(source: &str) -> Vec<String> {
    quoted_values(source)
        .filter(|value| value.contains("://") || value.ends_with(".git"))
        .filter_map(|value| {
            value
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .map(|name| name.trim_end_matches(".git").to_owned())
        })
        .collect()
}

fn swift_package_metadata(source: &str) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut metadata = BTreeMap::new();
    let mut source_roots = default_source_roots(["Sources", "Tests"]);
    let Ok(package_name) = Regex::new(r#"\bname\s*:\s*[\"']([^\"']+)[\"']"#) else {
        return (metadata, source_roots);
    };
    if let Some(name) = package_name
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty() && value.len() <= 512)
    {
        metadata.insert("swift.package.name".to_owned(), name.to_owned());
    }
    let Ok(paths) = Regex::new(r#"\bpath\s*:\s*[\"']([^\"']+)[\"']"#) else {
        return (metadata, source_roots);
    };
    for capture in paths.captures_iter(source).take(MAX_PROJECT_ROUTE_ROOTS) {
        let Some(path) = capture
            .get(1)
            .and_then(|value| bounded_source_root(value.as_str()))
        else {
            continue;
        };
        source_roots.insert(path);
    }
    (metadata, source_roots)
}

struct ParsedPubspec {
    dependencies: Vec<String>,
    metadata: BTreeMap<String, String>,
    source_roots: BTreeSet<String>,
}

fn pubspec_metadata(source: &str) -> Option<ParsedPubspec> {
    let root = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(source).ok()?;
    let mapping = root.as_mapping()?;
    let mut dependencies = Vec::new();
    for section in ["dependencies", "dev_dependencies", "dependency_overrides"] {
        let Some(values) =
            yaml_mapping_value(mapping, section).and_then(|value| value.as_mapping())
        else {
            continue;
        };
        dependencies.extend(
            values
                .keys()
                .filter_map(|key| key.as_str().map(str::to_owned)),
        );
    }
    let mut metadata = BTreeMap::new();
    if let Some(name) = yaml_mapping_value(mapping, "name")
        .and_then(serde_yaml_ng::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512)
    {
        metadata.insert("dart.package.name".to_owned(), name.to_owned());
    }
    if let Some(sdk) = yaml_mapping_value(mapping, "environment")
        .and_then(serde_yaml_ng::Value::as_mapping)
        .and_then(|environment| yaml_mapping_value(environment, "sdk"))
        .and_then(serde_yaml_ng::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512)
    {
        metadata.insert("dart.sdk".to_owned(), sdk.to_owned());
    }
    if let Some(flutter) = yaml_mapping_value(mapping, "flutter")
        .and_then(serde_yaml_ng::Value::as_mapping)
        .and_then(|flutter| yaml_mapping_value(flutter, "module"))
        .and_then(serde_yaml_ng::Value::as_str)
        .and_then(bounded_source_root)
    {
        metadata.insert("flutter.module".to_owned(), flutter);
    }
    Some(ParsedPubspec {
        dependencies,
        metadata,
        source_roots: default_source_roots(["lib", "bin", "test", "tool", "web"]),
    })
}

fn yaml_mapping_value<'value>(
    mapping: &'value serde_yaml_ng::Mapping,
    key: &str,
) -> Option<&'value serde_yaml_ng::Value> {
    mapping.get(serde_yaml_ng::Value::String(key.to_owned()))
}

fn sbt_project_metadata(source: &str) -> (Vec<String>, BTreeMap<String, String>, BTreeSet<String>) {
    let mut dependencies = Vec::new();
    let mut metadata = BTreeMap::new();
    let mut source_roots = default_source_roots(["src/main/scala", "src/test/scala"]);
    for line in source.lines().take(MAX_PROJECT_CONFIGURATION_KEYS) {
        let values = quoted_values(line).take(4).collect::<Vec<_>>();
        if line.contains('%') && values.len() >= 2 {
            dependencies.push(format!("{}:{}", values[0], values[1]));
        }
    }
    collect_sbt_metadata(source, "scalaVersion", "scala.version", &mut metadata);
    collect_sbt_metadata(source, "sbtVersion", "sbt.version", &mut metadata);
    collect_sbt_metadata(source, "organization", "scala.organization", &mut metadata);
    let Ok(paths) = Regex::new(
        r#"(?:scalaSource|javaSource|sourceDirectory)\s*:?=\s*[^\n\r]*?[\"']([^\"']+)[\"']"#,
    ) else {
        return (dependencies, metadata, source_roots);
    };
    for capture in paths.captures_iter(source).take(MAX_PROJECT_ROUTE_ROOTS) {
        if let Some(path) = capture
            .get(1)
            .and_then(|value| bounded_source_root(value.as_str()))
        {
            source_roots.insert(path);
        }
    }
    (dependencies, metadata, source_roots)
}

fn collect_sbt_metadata(
    source: &str,
    setting: &str,
    key: &str,
    output: &mut BTreeMap<String, String>,
) {
    let pattern = format!(r#"(?m)\b{setting}\s*:?=\s*[\"']([^\"']+)[\"']"#);
    let Ok(pattern) = Regex::new(&pattern) else {
        return;
    };
    if let Some(value) = pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty() && value.len() <= 512)
    {
        output.insert(key.to_owned(), value.to_owned());
    }
}

fn gradle_project_metadata(source: &str) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut metadata = BTreeMap::new();
    let mut source_roots = default_source_roots([
        "src/main/groovy",
        "src/test/groovy",
        "src/main/java",
        "src/test/java",
    ]);
    for (setting, key) in [("group", "gradle.group"), ("version", "gradle.version")] {
        collect_gradle_metadata(source, setting, key, &mut metadata);
    }
    let Ok(paths) = Regex::new(
        r#"(?:srcDirs|srcDir|srcDirs\.from)\s*(?:=|\+=|\()\s*[^\n\r]*?[\"']([^\"']+)[\"']"#,
    ) else {
        return (metadata, source_roots);
    };
    for capture in paths.captures_iter(source).take(MAX_PROJECT_ROUTE_ROOTS) {
        if let Some(path) = capture
            .get(1)
            .and_then(|value| bounded_source_root(value.as_str()))
        {
            source_roots.insert(path);
        }
    }
    (metadata, source_roots)
}

fn collect_gradle_metadata(
    source: &str,
    setting: &str,
    key: &str,
    output: &mut BTreeMap<String, String>,
) {
    let pattern = format!(
        r#"(?m)^\s*{setting}\s*(?:=|:)\s*[\"']([^\"']+)[\"']|\b{setting}\s*=\s*[\"']([^\"']+)[\"']"#
    );
    let Ok(pattern) = Regex::new(&pattern) else {
        return;
    };
    if let Some(value) = pattern
        .captures(source)
        .and_then(|capture| capture.get(1).or_else(|| capture.get(2)))
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty() && value.len() <= 512)
    {
        output.insert(key.to_owned(), value.to_owned());
    }
}

fn bounded_source_root(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 4_096
        || value.starts_with(['/', '\\'])
        || value.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }
    let mut components = Vec::new();
    for component in Path::new(value).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => components.push(segment.to_string_lossy()),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn default_source_roots<const N: usize>(roots: [&str; N]) -> BTreeSet<String> {
    roots.into_iter().map(str::to_owned).collect()
}

fn contained_manifest_root(
    repository_root: &Path,
    project_root: &Path,
    relative: &str,
) -> Option<String> {
    let candidate = project_root.join(relative);
    if !candidate.starts_with(repository_root) {
        return None;
    }
    if !candidate.is_dir() {
        return None;
    }
    let canonical_repository = fs::canonicalize(repository_root).ok()?;
    let mut existing_ancestor = candidate.clone();
    while !existing_ancestor.exists() {
        if existing_ancestor == repository_root || !existing_ancestor.pop() {
            return None;
        }
    }
    let canonical_ancestor = fs::canonicalize(existing_ancestor).ok()?;
    if !canonical_ancestor.starts_with(canonical_repository) {
        return None;
    }
    Some(normalize_project_path(relative))
}

fn first_quoted(value: &str) -> Option<String> {
    quoted_values(value).next()
}

fn quoted_values(value: &str) -> impl Iterator<Item = String> + '_ {
    let mut rest = value;
    std::iter::from_fn(move || {
        let start = rest.find(['\'', '"'])?;
        let quote = rest.as_bytes()[start];
        let after = &rest[start + 1..];
        let end = after.as_bytes().iter().position(|byte| *byte == quote)?;
        let result = after[..end].to_owned();
        rest = &after[end + 1..];
        Some(result)
    })
}

fn parse_configuration(path: &Path) -> Option<ParsedConfiguration> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return None;
    }
    let source = fs::read_to_string(path).ok()?;
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let mut parsed = ParsedConfiguration::default();
    match name.as_str() {
        "package.json" => parse_package_configuration(&source, &mut parsed),
        name if name.starts_with("tsconfig.") || name.starts_with("jsconfig.") => {
            parse_typescript_configuration(&source, &mut parsed)
        }
        name if name.starts_with("vite.config.") => {
            parse_static_vite_config(path, &source, &mut parsed)
        }
        name if name.starts_with("next.config.") => {
            parse_static_next_config(path, &source, &mut parsed)
        }
        name if (name.starts_with("application.") || name.starts_with("application-"))
            || (name.starts_with("bootstrap.") || name.starts_with("bootstrap-")) =>
        {
            parse_spring_configuration(&source, &mut parsed)
        }
        _ => parse_generic_configuration(&source, &mut parsed),
    }
    Some(parsed)
}

fn parse_package_configuration(source: &str, output: &mut ParsedConfiguration) {
    let Ok(root) = serde_json::from_str::<Value>(source) else {
        return;
    };
    let Some(object) = root.as_object() else {
        return;
    };
    for key in ["imports", "_moduleAliases", "_module_aliases"] {
        if let Some(values) = object.get(key).and_then(Value::as_object) {
            output.configuration_keys.insert(key.to_owned());
            collect_json_aliases(values, &mut output.aliases);
        }
    }
    for dependency_key in ["dependencies", "devDependencies", "peerDependencies"] {
        let Some(dependencies) = object.get(dependency_key).and_then(Value::as_object) else {
            continue;
        };
        for dependency in dependencies.keys() {
            if is_plugin_name(dependency) {
                output.plugins.insert(normalize_dependency(dependency));
            }
        }
    }
}

fn parse_typescript_configuration(source: &str, output: &mut ParsedConfiguration) {
    let Some(root) = parse_jsonc(source) else {
        return;
    };
    let Some(options) = root.get("compilerOptions").and_then(Value::as_object) else {
        return;
    };
    if options.contains_key("baseUrl") {
        output
            .configuration_keys
            .insert("compilerOptions.baseUrl".to_owned());
    }
    if let Some(paths) = options.get("paths").and_then(Value::as_object) {
        output
            .configuration_keys
            .insert("compilerOptions.paths".to_owned());
        collect_json_aliases(paths, &mut output.aliases);
    }
}

fn parse_static_vite_config(path: &Path, source: &str, output: &mut ParsedConfiguration) {
    inspect_frontend_config(path, source, |syntax| {
        let Some(object) = config_object(syntax) else {
            return;
        };
        for pair in object_pairs(syntax, object) {
            let Some(name) = syntax.property_name(pair) else {
                continue;
            };
            let Some(value_node) = pair
                .child_by_field_name("value")
                .or_else(|| pair.named_child(1))
            else {
                continue;
            };
            let value = syntax.static_value(value_node);
            match name.as_str() {
                "resolve" => {
                    if let Some((_, aliases)) = value
                        .object()
                        .and_then(|values| values.iter().find(|(key, _)| key == "alias"))
                    {
                        collect_static_aliases(aliases, &mut output.aliases);
                        output.configuration_keys.insert("resolve.alias".to_owned());
                    }
                }
                "plugins" => {
                    output.configuration_keys.insert("plugins".to_owned());
                    collect_static_plugins(&value, output);
                }
                _ => {}
            }
        }
        collect_syntax_plugins(syntax, output);
    });
    // `path.resolve(import.meta.dirname, "./src")` is intentionally treated
    // as an opaque call by the bounded static evaluator. The alias itself is
    // still recoverable without executing the config, so retain that
    // source-backed mapping for downstream Vite glob resolution. Dynamic
    // aliases remain unresolved and therefore fail closed at the matcher.
    collect_quoted_aliases(source, &mut output.aliases);
}

fn parse_static_next_config(path: &Path, source: &str, output: &mut ParsedConfiguration) {
    inspect_frontend_config(path, source, |syntax| {
        let Some(object) = config_object(syntax) else {
            return;
        };
        let names = object_pairs(syntax, object)
            .into_iter()
            .filter_map(|pair| syntax.property_name(pair))
            .collect::<BTreeSet<_>>();
        for key in [
            "rewrites",
            "redirects",
            "headers",
            "experimental",
            "pageExtensions",
        ] {
            if names.contains(key) {
                output.configuration_keys.insert(key.to_owned());
            }
        }
        collect_syntax_plugins(syntax, output);
    });
}

fn inspect_frontend_config(
    path: &Path,
    source: &str,
    inspect: impl FnOnce(TypeScriptSyntax<'_, '_>),
) {
    let language_name = match path.extension().and_then(|extension| extension.to_str()) {
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("jsx") => "jsx",
        Some("js" | "mjs" | "cjs") => "javascript",
        _ => return,
    };
    let Ok(language) = tree_sitter_language_pack::get_language(language_name) else {
        return;
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return;
    };
    inspect(TypeScriptSyntax::new(tree.root_node(), source.as_bytes()));
}

fn config_object<'tree, 'source>(
    syntax: TypeScriptSyntax<'tree, 'source>,
) -> Option<tree_sitter::Node<'tree>> {
    let define_call = syntax.descendants(syntax.root()).into_iter().find(|node| {
        let Some(callee) = syntax.call_callee(*node) else {
            return false;
        };
        (callee == "defineConfig"
            && !syntax
                .imported_local_names("vite", "defineConfig")
                .is_empty())
            || syntax
                .imported_local_names("vite", "*")
                .iter()
                .any(|name| callee == format!("{name}.defineConfig"))
    });
    define_call
        .and_then(|call| syntax.config_object_from_call(call))
        .or_else(|| syntax.exported_default_config_object())
}

fn object_pairs<'tree, 'source>(
    syntax: TypeScriptSyntax<'tree, 'source>,
    object: tree_sitter::Node<'tree>,
) -> Vec<tree_sitter::Node<'tree>> {
    let mut cursor = object.walk();
    object
        .named_children(&mut cursor)
        .filter(|node| {
            matches!(
                node.kind(),
                "pair" | "method_definition" | "public_field_definition"
            ) && !syntax.is_incomplete(*node)
        })
        .collect()
}

fn collect_static_aliases(value: &StaticValue, output: &mut BTreeMap<String, String>) {
    let Some(values) = value.object() else {
        return;
    };
    for (alias, target) in values.iter().take(MAX_PROJECT_ALIASES) {
        let Some(target) = target.as_string() else {
            continue;
        };
        output
            .entry(normalize_alias(alias))
            .or_insert_with(|| normalize_project_path(target));
    }
}

fn collect_static_plugins(value: &StaticValue, output: &mut ParsedConfiguration) {
    let Some(values) = value.array() else {
        return;
    };
    for plugin in values.iter().filter_map(StaticValue::as_string) {
        if is_plugin_name(plugin) {
            output.plugins.insert(normalize_dependency(plugin));
        }
    }
}

fn collect_syntax_plugins(syntax: TypeScriptSyntax<'_, '_>, output: &mut ParsedConfiguration) {
    for node in syntax.descendants(syntax.root()) {
        let import_like = node.kind() == "import_statement"
            || syntax.call_callee(node).as_deref() == Some("require");
        if !import_like {
            continue;
        }
        let Some(value) = syntax
            .descendants(node)
            .into_iter()
            .find_map(|child| syntax.literal_string(child))
        else {
            continue;
        };
        if is_plugin_name(&value) {
            output.plugins.insert(normalize_dependency(&value));
        }
    }
}

fn parse_spring_configuration(source: &str, output: &mut ParsedConfiguration) {
    for line in source.lines().map(str::trim) {
        let key = line
            .split_once(':')
            .map(|(key, _)| key)
            .or_else(|| line.split_once('=').map(|(key, _)| key))
            .map(str::trim)
            .unwrap_or_default();
        if key.starts_with("spring.") || key.starts_with("server.") {
            output.configuration_keys.insert(key.to_owned());
        }
    }
}

fn parse_generic_configuration(source: &str, output: &mut ParsedConfiguration) {
    if source.contains("resolve") && source.contains("alias") {
        output.configuration_keys.insert("resolve.alias".to_owned());
        collect_quoted_aliases(source, &mut output.aliases);
    }
    collect_config_plugins(source, output);
}

fn collect_json_aliases(
    values: &serde_json::Map<String, Value>,
    output: &mut BTreeMap<String, String>,
) {
    for (alias, target) in values {
        let Some(target) = target.as_str().or_else(|| {
            target
                .as_array()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
        }) else {
            continue;
        };
        if output.len() >= MAX_PROJECT_ALIASES && !output.contains_key(alias) {
            break;
        }
        output.insert(normalize_alias(alias), normalize_project_path(target));
    }
}

fn collect_quoted_aliases(source: &str, output: &mut BTreeMap<String, String>) {
    let Ok(pattern) =
        Regex::new(r#"[\"']([^\"']+)[\"']\s*:\s*(?:path\.resolve\([^,]+,\s*)?[\"']([^\"']+)[\"']"#)
    else {
        return;
    };
    for capture in pattern.captures_iter(source) {
        let (Some(alias), Some(target)) = (capture.get(1), capture.get(2)) else {
            continue;
        };
        if output.len() >= MAX_PROJECT_ALIASES && !output.contains_key(alias.as_str()) {
            break;
        }
        output.insert(
            normalize_alias(alias.as_str()),
            normalize_project_path(target.as_str()),
        );
    }
}

fn collect_config_plugins(source: &str, output: &mut ParsedConfiguration) {
    let Ok(imports) = Regex::new(r#"(?:from|require\s*\()\s*[\"']([^\"']+)[\"']"#) else {
        return;
    };
    for capture in imports.captures_iter(source) {
        let Some(value) = capture.get(1).map(|value| value.as_str()) else {
            continue;
        };
        if is_plugin_name(value) {
            output.plugins.insert(normalize_dependency(value));
        }
    }
    let Ok(calls) = Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$-]*)\s*\(") else {
        return;
    };
    for capture in calls.captures_iter(source) {
        let Some(value) = capture.get(1).map(|value| value.as_str()) else {
            continue;
        };
        if is_plugin_name(value) {
            output.plugins.insert(normalize_dependency(value));
        }
    }
}

fn is_plugin_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("@vitejs/plugin-")
        || lower.starts_with("vite-plugin-")
        || lower.starts_with("unplugin-")
        || lower.starts_with("next-plugin-")
}

fn normalize_alias(value: &str) -> String {
    value.trim().replace('\\', "/")
}

fn normalize_project_path(value: &str) -> String {
    value.trim().trim_start_matches("./").replace('\\', "/")
}

fn normalize_project_name(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn relative_project_file(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn project_root_for_file(
    repository_root: &Path,
    manifest_roots: &[PathBuf],
    path: &Path,
) -> PathBuf {
    manifest_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
        .or_else(|| {
            path.parent().map(|parent| {
                if matches!(
                    parent.file_name().and_then(|name| name.to_str()),
                    Some("resources" | "config" | "conf")
                ) {
                    parent.parent().unwrap_or(parent).to_path_buf()
                } else {
                    parent.to_path_buf()
                }
            })
        })
        .unwrap_or_else(|| repository_root.to_path_buf())
}

fn project_root_for_source(
    repository_root: &Path,
    builders: &BTreeMap<PathBuf, ProjectBuilder>,
    path: &Path,
) -> PathBuf {
    builders
        .keys()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
        .unwrap_or_else(|| repository_root.to_path_buf())
}

fn route_root_for_source(
    project_root: &Path,
    source: &Path,
    builders: &BTreeMap<PathBuf, ProjectBuilder>,
) -> Option<(String, String)> {
    let relative = source.strip_prefix(project_root).ok()?;
    let components = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let builder = builders.get(project_root)?;
    let has = |dependency: &str| builder.dependencies.contains(dependency);
    let configured = |name: &str| {
        builder
            .configuration_files
            .iter()
            .any(|file| normalize_project_name(file).ends_with(&normalize_project_name(name)))
    };
    let next_configured = ["next.config.js", "next.config.mjs", "next.config.ts"]
        .iter()
        .any(|name| configured(name));
    let remix_configured = [
        "remix.config.cjs",
        "remix.config.js",
        "remix.config.mjs",
        "remix.config.ts",
    ]
    .iter()
    .any(|name| configured(name));
    if (has("next") || next_configured)
        && (components.first() == Some(&"app") || components.first() == Some(&"pages"))
    {
        return Some(("next".to_owned(), components[0].to_owned()));
    }
    if (has("next") || next_configured)
        && components.first() == Some(&"src")
        && matches!(components.get(1), Some(&"app" | &"pages"))
    {
        return Some(("next".to_owned(), format!("src/{}", components[1])));
    }
    if (remix_configured
        || has_any_dependency(
            builder,
            &[
                "@remix-run/dev",
                "@remix-run/node",
                "@remix-run/react",
                "@remix-run/router",
                "@remix-run/serve",
            ],
        ))
        && components.starts_with(&["app", "routes"])
    {
        return Some(("remix".to_owned(), "app/routes".to_owned()));
    }
    if (remix_configured
        || has_any_dependency(
            builder,
            &[
                "@remix-run/dev",
                "@remix-run/node",
                "@remix-run/react",
                "@remix-run/router",
                "@remix-run/serve",
            ],
        ))
        && components.starts_with(&["routes"])
    {
        return Some(("remix".to_owned(), "routes".to_owned()));
    }
    if (remix_configured
        || has_any_dependency(
            builder,
            &[
                "@remix-run/dev",
                "@remix-run/node",
                "@remix-run/react",
                "@remix-run/router",
                "@remix-run/serve",
            ],
        ))
        && components.starts_with(&["src", "routes"])
    {
        return Some(("remix".to_owned(), "src/routes".to_owned()));
    }
    if has("nuxt") && components.first() == Some(&"pages") {
        return Some(("nuxt".to_owned(), "pages".to_owned()));
    }
    if has("nuxt") && components.starts_with(&["server", "api"]) {
        return Some(("nuxt".to_owned(), "server/api".to_owned()));
    }
    if has("astro") && components.starts_with(&["src", "pages"]) {
        return Some(("astro".to_owned(), "src/pages".to_owned()));
    }
    if has("@sveltejs/kit") && components.starts_with(&["src", "routes"]) {
        return Some(("sveltekit".to_owned(), "src/routes".to_owned()));
    }
    None
}

fn has_any_dependency(builder: &ProjectBuilder, dependencies: &[&str]) -> bool {
    dependencies.iter().any(|dependency| {
        builder
            .dependencies
            .contains(&normalize_dependency(dependency))
    })
}

fn normalize_dependency(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn is_recognized_manifest(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    FIXED_MANIFEST_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
        || name.to_ascii_lowercase().ends_with(".csproj")
}

fn is_recognized_configuration(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    FIXED_CONFIGURATION_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
        || (name.to_ascii_lowercase().starts_with("application.")
            || name.to_ascii_lowercase().starts_with("application-"))
            && matches!(
                name.to_ascii_lowercase()
                    .rsplit_once('.')
                    .map(|(_, extension)| extension),
                Some("properties" | "yml" | "yaml")
            )
        || (name.to_ascii_lowercase().starts_with("bootstrap.")
            || name.to_ascii_lowercase().starts_with("bootstrap-"))
            && matches!(
                name.to_ascii_lowercase()
                    .rsplit_once('.')
                    .map(|(_, extension)| extension),
                Some("properties" | "yml" | "yaml")
            )
        || (name.to_ascii_lowercase().starts_with("tsconfig.")
            && name.to_ascii_lowercase().ends_with(".json"))
        || (name.to_ascii_lowercase().starts_with("jsconfig.")
            && name.to_ascii_lowercase().ends_with(".json"))
}

fn is_project_file(path: &Path) -> bool {
    is_recognized_manifest(path) || is_recognized_configuration(path)
}

fn regular_project_file(path: &Path) -> bool {
    is_project_file(path)
        && fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.file_type().is_file() && !metadata.file_type().is_symlink()
        })
}

fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use tempfile::tempdir;

    use super::ProjectEvidenceIndex;

    #[test]
    fn nearest_project_merges_manifests_and_is_deterministic() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("apps/web/src"))?;
        fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"astro":"5.0.0"}}"#,
        )?;
        fs::write(
            root.join("apps/web/package.json"),
            r#"{"dependencies":{"nuxt":"4.0.0","@scope/example":"1.0.0"}}"#,
        )?;
        fs::write(
            root.join("apps/web/composer.json"),
            r#"{"require":{"laravel/framework":"^12"}}"#,
        )?;
        let source = root.join("apps/web/src/page.ts");
        fs::write(&source, "export default {}")?;

        let first = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let second = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = first.evidence_for(&source);

        assert_eq!(first.project_count(), 2);
        assert_eq!(evidence.project_root(), root.join("apps/web"));
        assert!(evidence.has_dependency("nuxt"));
        assert!(evidence.has_dependency("@scope/example"));
        assert!(evidence.has_dependency("laravel/framework"));
        assert!(!evidence.has_dependency("astro"));
        assert_eq!(
            evidence.fingerprint(),
            second.evidence_for(&source).fingerprint()
        );
        Ok(())
    }

    #[test]
    fn composer_psr4_roots_are_bounded_contained_and_deterministic() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        for path in ["src", "src/Domain", "tests", "packages/shared"] {
            fs::create_dir_all(root.join(path))?;
        }
        let source = root.join("src/Domain/Model.php");
        fs::write(&source, "<?php namespace App\\Domain; class Model {}")?;
        fs::write(
            root.join("composer.json"),
            r#"{
              "require": {"laravel/framework": "^12"},
              "autoload": {"psr-4": {
                "App\\": ["src/", "./src"],
                "App\\Domain\\": "src/Domain",
                "Shared\\": "packages/shared",
                "Root\\": ""
              }},
              "autoload-dev": {"psr-4": {"Tests\\": "tests"}}
            }"#,
        )?;

        let first = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let second = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = first.evidence_for(&source);
        let roots = evidence.composer_autoload_roots();

        assert_eq!(roots.len(), 5, "roots={roots:#?}");
        assert!(roots.iter().any(|entry| {
            entry.namespace_prefix == "App\\" && entry.directory == "src" && !entry.development
        }));
        assert!(roots.iter().any(|entry| {
            entry.namespace_prefix == "App\\Domain\\"
                && entry.directory == "src/Domain"
                && !entry.development
        }));
        assert!(roots.iter().any(|entry| {
            entry.namespace_prefix == "Tests\\" && entry.directory == "tests" && entry.development
        }));
        assert!(roots.iter().any(|entry| {
            entry.namespace_prefix == "Root\\" && entry.directory == "." && !entry.development
        }));
        assert!(evidence.diagnostics().is_empty());
        assert_eq!(
            evidence.fingerprint(),
            second.evidence_for(&source).fingerprint()
        );
        Ok(())
    }

    #[test]
    fn composer_psr4_rejects_escapes_absolute_paths_and_malformed_entries()
    -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src"))?;
        let source = root.join("src/App.php");
        fs::write(&source, "<?php class App {}")?;
        fs::write(
            root.join("composer.json"),
            r#"{
              "autoload": {"psr-4": {
                "": "src",
                "Escape\\": "../../outside",
                "Absolute\\": "/private/source",
                "Drive\\": "C:\\source",
                "Malformed Prefix!\\": "src",
                "WrongShape\\": 42
              }}
            }"#,
        )?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = index.evidence_for(&source);
        assert_eq!(evidence.composer_autoload_roots().len(), 1);
        assert!(
            evidence
                .composer_autoload_roots()
                .iter()
                .any(|entry| { entry.namespace_prefix.is_empty() && entry.directory == "src" })
        );
        let codes = evidence
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(codes.contains("composer_psr4_directory_rejected"));
        assert!(codes.contains("composer_psr4_directory_invalid"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn composer_psr4_rejects_a_directory_symlink_that_leaves_the_repository()
    -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempdir()?;
        let outside = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src"))?;
        symlink(outside.path(), root.join("src/external"))?;
        let source = root.join("src/App.php");
        fs::write(&source, "<?php class App {}")?;
        fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"Escaped\\":"src/external/missing"}}}"#,
        )?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = index.evidence_for(&source);
        assert!(evidence.composer_autoload_roots().is_empty());
        assert!(
            evidence
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code == "composer_psr4_directory_rejected" })
        );
        Ok(())
    }

    #[test]
    fn fallback_fingerprint_changes_when_a_project_manifest_appears() -> Result<(), Box<dyn Error>>
    {
        let directory = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src"))?;
        let source = root.join("src/app.ts");
        fs::write(&source, "")?;
        let without = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let old = without.fingerprint_for(&source).to_owned();

        fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"@sveltejs/kit":"2.0.0"}}"#,
        )?;
        let with = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));

        assert_ne!(old, with.fingerprint_for(&source));
        assert!(with.evidence_for(&source).has_dependency("@sveltejs/kit"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_manifests_do_not_contribute_evidence() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("outside.json"),
            r#"{"dependencies":{"nuxt":"4"}}"#,
        )?;
        symlink(root.join("outside.json"), root.join("package.json"))?;
        let source = root.join("src/app.ts");
        fs::write(&source, "")?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));

        assert!(!index.evidence_for(&source).has_dependency("nuxt"));
        Ok(())
    }

    #[test]
    fn framework_configuration_aliases_plugins_and_route_roots_are_indexed()
    -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join("src/app"))?;
        let source = root.join("src/app/page.tsx");
        fs::write(&source, "export default function Page() { return null }")?;
        fs::write(
            root.join("package.json"),
            r##"{"dependencies":{"next":"15","vite":"7"},"devDependencies":{"@vitejs/plugin-react":"4"},"imports":{"#shared/*":"./src/shared/*"}}"##,
        )?;
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["./src/*"]}}}"#,
        )?;
        fs::write(
            root.join("vite.config.ts"),
            r#"import react from '@vitejs/plugin-react'; export default { plugins: [react()], resolve: { alias: { '~': './src' } } }"#,
        )?;
        fs::write(
            root.join("next.config.mjs"),
            "import withIntl from 'next-plugin-intl'; export default withIntl({ rewrites() { return [] } });",
        )?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = index.evidence_for(&source);

        assert!(evidence.has_configuration("tsconfig.json"));
        assert!(evidence.has_configuration("vite.config.ts"));
        assert!(evidence.has_configuration("next.config.mjs"));
        assert!(
            evidence
                .configuration_keys()
                .contains("compilerOptions.paths")
        );
        assert!(evidence.configuration_keys().contains("resolve.alias"));
        assert!(evidence.configuration_keys().contains("rewrites"));
        assert_eq!(evidence.aliases().get("@/*"), Some(&"src/*".to_owned()));
        assert_eq!(evidence.aliases().get("~"), Some(&"src".to_owned()));
        assert!(evidence.has_plugin("@vitejs/plugin-react"));
        assert!(evidence.has_plugin("next-plugin-intl"));
        assert!(evidence.has_route_root("next", "src/app"));
        Ok(())
    }

    #[test]
    fn typescript_jsonc_configuration_keeps_aliases_and_ignores_comment_text()
    -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        let source = root.join("src/app.ts");
        fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
        fs::write(&source, "export const app = true;\n")?;
        fs::write(
            root.join("tsconfig.json"),
            r##"{
                // The slash in this comment must not start a value.
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "@/*": ["./src/*",],
                    },
                },
            }"##,
        )?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = index.evidence_for(&source);
        assert!(
            evidence
                .configuration_keys()
                .contains("compilerOptions.baseUrl")
        );
        assert!(
            evidence
                .configuration_keys()
                .contains("compilerOptions.paths")
        );
        assert_eq!(evidence.aliases().get("@/*"), Some(&"src/*".to_owned()));
        Ok(())
    }

    #[test]
    fn remix_route_roots_accept_dependency_or_config_evidence() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        let route = root.join("app/routes/users.$id.tsx");
        fs::create_dir_all(route.parent().ok_or("route has no parent")?)?;
        fs::write(&route, "export default function User() { return null }")?;
        fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"@remix-run/dev":"2.10.0"}}"#,
        )?;
        let dependency_index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&route));
        assert!(
            dependency_index
                .evidence_for(&route)
                .has_route_root("remix", "app/routes")
        );

        let config_only = tempdir()?;
        let config_route = config_only.path().join("routes/_index.tsx");
        fs::create_dir_all(config_route.parent().ok_or("config route has no parent")?)?;
        fs::write(
            &config_route,
            "export default function Home() { return null }",
        )?;
        fs::write(
            config_only.path().join("remix.config.ts"),
            "export default { appDirectory: 'app' }",
        )?;
        let config_index =
            ProjectEvidenceIndex::build(config_only.path(), std::slice::from_ref(&config_route));
        assert!(
            config_index
                .evidence_for(&config_route)
                .has_route_root("remix", "routes")
        );
        Ok(())
    }

    #[test]
    fn spring_resource_profiles_are_discovered_from_a_sibling_source_directory()
    -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        let source = root.join("src/main/java/example/App.java");
        fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
        fs::create_dir_all(root.join("src/main/resources"))?;
        fs::write(&source, "class App {}")?;
        fs::write(
            root.join("src/main/resources/application-dev.yml"),
            "spring.application.name: compass\nserver.port: 8080\n",
        )?;

        let index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = index.evidence_for(&source);
        assert!(evidence.has_configuration("application-dev.yml"));
        assert!(
            evidence
                .configuration_keys()
                .contains("spring.application.name")
        );
        assert!(evidence.configuration_keys().contains("server.port"));
        Ok(())
    }

    #[test]
    fn language_manifests_are_bounded_and_never_execute_build_tools() -> Result<(), Box<dyn Error>>
    {
        let directory = tempdir()?;
        let root = directory.path();
        let source = root.join("lib/main.dart");
        fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
        fs::write(&source, "class Main {}\n")?;
        fs::write(
            root.join("pubspec.yaml"),
            "name: sample_app\nenvironment:\n  sdk: ^3.4.0\ndependencies:\n  flutter:\n    sdk: flutter\n  riverpod: ^2.5.0\n",
        )?;
        fs::write(
            root.join("build.sbt"),
            "scalaVersion := \"3.3.3\"\nlibraryDependencies += \"org.typelevel\" %% \"cats-core\" % \"2.12.0\"\n",
        )?;
        fs::write(
            root.join("Package.swift"),
            "let package = Package(name: \"Sample\", targets: [.target(name: \"Sample\", path: \"Sources/Sample\")])\n",
        )?;

        let first = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let second = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source));
        let evidence = first.evidence_for(&source);
        assert!(evidence.has_dependency("riverpod"));
        assert_eq!(
            evidence.metadata().get("dart.package.name"),
            Some(&"sample_app".to_owned())
        );
        assert_eq!(
            evidence.metadata().get("dart.sdk"),
            Some(&"^3.4.0".to_owned())
        );
        assert!(evidence.source_roots().iter().any(|root| root == "lib"));
        assert!(
            evidence
                .manifests()
                .iter()
                .any(|name| name == "pubspec.yaml")
        );
        assert_eq!(
            evidence.fingerprint(),
            second.evidence_for(&source).fingerprint()
        );

        let scala_source = root.join("src/main/scala/Main.scala");
        fs::create_dir_all(scala_source.parent().ok_or("scala source has no parent")?)?;
        fs::write(&scala_source, "object Main {}\n")?;
        let scala_index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&scala_source));
        let scala = scala_index.evidence_for(&scala_source);
        assert!(scala.has_dependency("org.typelevel:cats-core"));
        assert_eq!(
            scala.metadata().get("scala.version"),
            Some(&"3.3.3".to_owned())
        );
        assert!(
            scala
                .source_roots()
                .iter()
                .any(|root| root == "src/main/scala")
        );

        let swift_source = root.join("Sources/Sample/Main.swift");
        fs::create_dir_all(swift_source.parent().ok_or("swift source has no parent")?)?;
        fs::write(&swift_source, "struct Main {}\n")?;
        let swift_index = ProjectEvidenceIndex::build(root, std::slice::from_ref(&swift_source));
        let swift = swift_index.evidence_for(&swift_source);
        assert_eq!(
            swift.metadata().get("swift.package.name"),
            Some(&"Sample".to_owned())
        );
        assert!(
            swift
                .source_roots()
                .iter()
                .any(|root| root == "Sources/Sample")
        );
        Ok(())
    }
}
