use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const FRAMEWORK_PROJECT_EVIDENCE_EXTENSION: &str = "_compass_framework_project_evidence";

const EVIDENCE_SCHEMA: &str = "compass.framework-project-evidence/1";
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DEPENDENCIES_PER_PROJECT: usize = 10_000;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEvidence {
    project_root: PathBuf,
    manifests: Vec<String>,
    ecosystems: Vec<String>,
    dependencies: BTreeSet<String>,
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
        let mut manifests = BTreeSet::new();
        directories.insert(repository_root.clone());

        for source in sources {
            let source = absolute_path(&repository_root, source);
            if is_recognized_manifest(&source) {
                manifests.insert(source.clone());
            }
            let directory = source.parent().unwrap_or(&repository_root);
            for ancestor in directory.ancestors() {
                if !ancestor.starts_with(&repository_root) {
                    break;
                }
                directories.insert(ancestor.to_path_buf());
                if ancestor == repository_root {
                    break;
                }
            }
        }

        for directory in &directories {
            for name in FIXED_MANIFEST_NAMES {
                let candidate = directory.join(name);
                if regular_manifest(&candidate) {
                    manifests.insert(candidate);
                }
            }
        }

        let mut builders = BTreeMap::<PathBuf, ProjectBuilder>::new();
        for manifest in manifests {
            let project_root = manifest.parent().unwrap_or(&repository_root).to_path_buf();
            let builder = builders.entry(project_root).or_default();
            builder.manifests.insert(
                manifest
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned(),
            );
            let Some(parsed) = parse_manifest(&manifest) else {
                continue;
            };
            builder.ecosystems.insert(parsed.ecosystem.to_owned());
            let remaining = MAX_DEPENDENCIES_PER_PROJECT.saturating_sub(builder.dependencies.len());
            builder
                .dependencies
                .extend(parsed.dependencies.into_iter().take(remaining));
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
}

struct ParsedManifest {
    ecosystem: &'static str,
    dependencies: BTreeSet<String>,
}

fn finish_project(
    repository_root: &Path,
    project_root: PathBuf,
    builder: ProjectBuilder,
) -> ProjectEvidence {
    let manifests = builder.manifests.into_iter().collect::<Vec<_>>();
    let ecosystems = builder.ecosystems.into_iter().collect::<Vec<_>>();
    let dependencies = builder.dependencies;
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
    ProjectEvidence {
        project_root,
        manifests,
        ecosystems,
        dependencies,
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

fn regular_manifest(path: &Path) -> bool {
    is_recognized_manifest(path)
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
}
