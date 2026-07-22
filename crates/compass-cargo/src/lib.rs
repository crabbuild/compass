//! Deterministic Cargo workspace dependency introspection.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CargoGraph {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
}

impl CargoGraph {
    #[must_use]
    pub fn into_fragment(self) -> Value {
        serde_json::json!({
            "nodes": self.nodes,
            "edges": self.edges,
            "hyperedges": [],
            "input_tokens": 0,
            "output_tokens": 0,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CargoIntrospectionError {
    #[error("could not resolve Cargo workspace {path}: {source}")]
    Resolve {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read Cargo manifest {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse Cargo manifest {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid Cargo workspace member pattern {pattern:?}: {source}")]
    GlobPattern {
        pattern: String,
        source: glob::PatternError,
    },
    #[error("could not inspect Cargo workspace member for pattern {pattern:?}: {source}")]
    GlobWalk {
        pattern: String,
        source: glob::GlobError,
    },
    #[error("Cargo workspace member {path} is outside {root}")]
    OutsideRoot { path: PathBuf, root: PathBuf },
}

#[derive(Clone)]
struct CrateManifest {
    id: String,
    path: PathBuf,
    data: toml::Value,
}

/// Return crate nodes and workspace-internal dependency edges from Cargo manifests.
pub fn introspect_cargo(root: &Path) -> Result<CargoGraph, CargoIntrospectionError> {
    let root = fs::canonicalize(root).map_err(|source| CargoIntrospectionError::Resolve {
        path: root.to_path_buf(),
        source,
    })?;
    let root_manifest = root.join("Cargo.toml");
    let root_data = load_toml(&root_manifest)?;
    let manifests = member_manifest_paths(&root, &root_data)?;
    let mut crates = BTreeMap::<String, CrateManifest>::new();

    for manifest in manifests {
        let data = if manifest == root_manifest {
            root_data.clone()
        } else {
            load_toml(&manifest)?
        };
        let Some(name) = data
            .get("package")
            .and_then(toml::Value::as_table)
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        crates.insert(
            name.to_owned(),
            CrateManifest {
                id: format!("crate:{name}"),
                path: manifest,
                data,
            },
        );
    }

    let mut nodes = Vec::with_capacity(crates.len());
    for (name, manifest) in &crates {
        nodes.push(NodeRecord {
            id: manifest.id.clone(),
            attributes: Map::from_iter([
                ("label".to_owned(), Value::String(name.clone())),
                (
                    "source_file".to_owned(),
                    Value::String(relative_posix(&manifest.path, &root)?),
                ),
                ("source_location".to_owned(), Value::String("L1".to_owned())),
            ]),
        });
    }

    let mut edges = Vec::new();
    for manifest in crates.values() {
        let Some(dependencies) = manifest
            .data
            .get("dependencies")
            .and_then(toml::Value::as_table)
        else {
            continue;
        };
        let source_file = relative_posix(&manifest.path, &root)?;
        let sorted_dependencies = dependencies.iter().collect::<BTreeMap<_, _>>();
        for (dependency_name, specification) in sorted_dependencies {
            let real_name = specification
                .as_table()
                .and_then(|table| table.get("package"))
                .and_then(toml::Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or(dependency_name);
            let Some(target) = crates.get(real_name) else {
                continue;
            };
            edges.push(EdgeRecord {
                source: manifest.id.clone(),
                target: target.id.clone(),
                attributes: Map::from_iter([
                    (
                        "relation".to_owned(),
                        Value::String("crate_depends_on".to_owned()),
                    ),
                    (
                        "context".to_owned(),
                        Value::String("cargo_dependency".to_owned()),
                    ),
                    ("weight".to_owned(), Value::from(1.0)),
                    (
                        "confidence".to_owned(),
                        Value::String("EXTRACTED".to_owned()),
                    ),
                    ("source_file".to_owned(), Value::String(source_file.clone())),
                    ("source_location".to_owned(), Value::String("L1".to_owned())),
                ]),
            });
        }
    }
    Ok(CargoGraph { nodes, edges })
}

fn load_toml(path: &Path) -> Result<toml::Value, CargoIntrospectionError> {
    let text = fs::read_to_string(path).map_err(|source| CargoIntrospectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| CargoIntrospectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn member_manifest_paths(
    root: &Path,
    root_data: &toml::Value,
) -> Result<Vec<PathBuf>, CargoIntrospectionError> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    if root_data
        .get("package")
        .and_then(toml::Value::as_table)
        .is_some()
    {
        let manifest = root.join("Cargo.toml");
        seen.insert(manifest.clone());
        paths.push(manifest);
    }
    let members = root_data
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array);
    let Some(members) = members else {
        return Ok(paths);
    };
    for member in members.iter().filter_map(toml::Value::as_str) {
        let joined = root.join(member);
        let pattern = joined.to_string_lossy().into_owned();
        let matches =
            glob::glob(&pattern).map_err(|source| CargoIntrospectionError::GlobPattern {
                pattern: member.to_owned(),
                source,
            })?;
        let mut matched = matches
            .map(|result| {
                result.map_err(|source| CargoIntrospectionError::GlobWalk {
                    pattern: member.to_owned(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        matched.sort();
        for directory in matched {
            let manifest = directory.join("Cargo.toml");
            if manifest.is_file() && seen.insert(manifest.clone()) {
                paths.push(manifest);
            }
        }
    }
    Ok(paths)
}

fn relative_posix(path: &Path, root: &Path) -> Result<String, CargoIntrospectionError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| CargoIntrospectionError::OutsideRoot {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
        })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_dependencies_and_package_renames_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::create_dir_all(directory.path().join("crates/app"))?;
        fs::create_dir_all(directory.path().join("crates/storage"))?;
        fs::write(
            directory.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )?;
        fs::write(
            directory.path().join("crates/app/Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\ndb = { path = \"../storage\", package = \"internal-storage\" }\nserde = \"1\"\n",
        )?;
        fs::write(
            directory.path().join("crates/storage/Cargo.toml"),
            "[package]\nname = \"internal-storage\"\nversion = \"0.1.0\"\n",
        )?;
        let graph = introspect_cargo(directory.path())?;
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["crate:app", "crate:internal-storage"]
        );
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].source, "crate:app");
        assert_eq!(graph.edges[0].target, "crate:internal-storage");
        Ok(())
    }

    #[test]
    fn empty_and_scalar_dependency_manifests_degrade_safely()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = \"bad\"\n",
        )?;
        let graph = introspect_cargo(directory.path())?;
        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.edges.is_empty());
        Ok(())
    }

    #[test]
    fn dependency_edges_are_sorted_like_python() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        for name in ["app", "alpha", "zeta"] {
            fs::create_dir_all(directory.path().join(name))?;
            let dependencies = if name == "app" {
                "[dependencies]\nzeta={path=\"../zeta\"}\nalpha={path=\"../alpha\"}\n"
            } else {
                ""
            };
            fs::write(
                directory.path().join(name).join("Cargo.toml"),
                format!("[package]\nname=\"{name}\"\nversion=\"0.1.0\"\n{dependencies}"),
            )?;
        }
        fs::write(
            directory.path().join("Cargo.toml"),
            "[workspace]\nmembers=[\"*\"]\n",
        )?;
        let graph = introspect_cargo(directory.path())?;
        assert_eq!(
            graph
                .edges
                .iter()
                .map(|edge| edge.target.as_str())
                .collect::<Vec<_>>(),
            ["crate:alpha", "crate:zeta"]
        );
        Ok(())
    }
}
