use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const FRAMEWORK_PROJECT_EVIDENCE_EXTENSION: &str = "_compass_framework_project_evidence";

const EVIDENCE_SCHEMA: &str = "compass.framework-project-evidence/1";
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DEPENDENCIES_PER_PROJECT: usize = 10_000;
const MAX_PROJECT_CONFIGURATIONS: usize = 256;
const MAX_PROJECT_CONFIGURATION_KEYS: usize = 2_000;
const MAX_PROJECT_ALIASES: usize = 2_000;
const MAX_PROJECT_PLUGINS: usize = 2_000;
const MAX_PROJECT_ROUTE_ROOTS: usize = 256;
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
    dependencies: BTreeSet<String>,
    configuration_files: Vec<String>,
    configuration_keys: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    plugins: BTreeSet<String>,
    route_roots: BTreeMap<String, BTreeSet<String>>,
    fingerprint: String,
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
                    let remaining =
                        MAX_DEPENDENCIES_PER_PROJECT.saturating_sub(builder.dependencies.len());
                    builder
                        .dependencies
                        .extend(parsed.dependencies.into_iter().take(remaining));
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
    dependencies: BTreeSet<String>,
    configuration_files: BTreeSet<String>,
    configuration_keys: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    plugins: BTreeSet<String>,
    route_roots: BTreeMap<String, BTreeSet<String>>,
}

struct ParsedManifest {
    ecosystem: &'static str,
    dependencies: BTreeSet<String>,
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
    let dependencies = builder.dependencies;
    let configuration_files = builder.configuration_files.into_iter().collect::<Vec<_>>();
    let configuration_keys = builder.configuration_keys;
    let aliases = builder.aliases;
    let plugins = builder.plugins;
    let route_roots = builder.route_roots;
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
    ProjectEvidence {
        project_root,
        manifests,
        ecosystems,
        dependencies,
        configuration_files,
        configuration_keys,
        aliases,
        plugins,
        route_roots,
        fingerprint: format!("sha256:{:x}", digest.finalize()),
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
    let (ecosystem, dependencies) = match lower.as_str() {
        "package.json" => ("npm", json_dependencies(&source, NPM_DEPENDENCY_KEYS)?),
        "composer.json" => (
            "composer",
            json_dependencies(&source, &["require", "require-dev"])?,
        ),
        "pyproject.toml" => ("python", pyproject_dependencies(&source)?),
        "requirements.txt" | "requirements.in" => ("python", requirements_dependencies(&source)),
        "gemfile" => ("ruby", gemfile_dependencies(&source)),
        "pom.xml" => ("maven", pom_dependencies(&source)?),
        "build.gradle" | "build.gradle.kts" => ("gradle", gradle_dependencies(&source)),
        "cargo.toml" => ("cargo", cargo_dependencies(&source)?),
        "go.mod" => ("go", go_mod_dependencies(&source)),
        "package.swift" => ("swift", swift_package_dependencies(&source)),
        _ if lower.ends_with(".csproj") => ("dotnet", csproj_dependencies(&source)?),
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
        name if name.starts_with("vite.config.") => parse_vite_configuration(&source, &mut parsed),
        name if name.starts_with("next.config.") => parse_next_configuration(&source, &mut parsed),
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
    let Ok(root) = serde_json::from_str::<Value>(source) else {
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

fn parse_vite_configuration(source: &str, output: &mut ParsedConfiguration) {
    if source.contains("resolve") && source.contains("alias") {
        output.configuration_keys.insert("resolve.alias".to_owned());
    }
    if source.contains("plugins") {
        output.configuration_keys.insert("plugins".to_owned());
    }
    collect_quoted_aliases(source, &mut output.aliases);
    collect_config_plugins(source, output);
}

fn parse_next_configuration(source: &str, output: &mut ParsedConfiguration) {
    for key in [
        "rewrites",
        "redirects",
        "headers",
        "experimental",
        "pageExtensions",
    ] {
        if source.contains(key) {
            output.configuration_keys.insert(key.to_owned());
        }
    }
    collect_config_plugins(source, output);
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
    lower.contains("plugin")
        || lower.starts_with("@vitejs/")
        || lower.starts_with("vite-plugin-")
        || lower.starts_with("next-plugin-")
        || matches!(
            lower.as_str(),
            "react" | "vue" | "svelte" | "solid" | "legacy"
        )
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
}
