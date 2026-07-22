use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value, json};

use crate::{ExtractError, Extraction, make_id};

const MAX_BYTES: u64 = 2_000_000;

pub(crate) fn extract(path: &Path) -> Result<Extraction, ExtractError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > MAX_BYTES as usize {
        return Ok(failure("manifest too large to index"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (ecosystem, parsed) = match name.as_str() {
        "apm.yml" | "apm.yaml" => ("apm", parse_apm(&text)),
        "pyproject.toml" => ("python", parse_pyproject(&text)),
        "go.mod" => ("go", Ok(parse_go_mod(&text))),
        "pom.xml" => ("maven", parse_pom(&text)),
        _ => return Err(ExtractError::Unsupported(path.to_path_buf())),
    };
    let Some(info) = (match parsed {
        Ok(info) => info,
        Err(error) => return Ok(failure(&format!("manifest parse error: {error}"))),
    }) else {
        return Ok(empty());
    };
    if info.name.is_empty() {
        return Ok(empty());
    }

    let source_file = path.to_string_lossy().into_owned();
    let package_node_id = package_id(&info.name);
    let mut attributes = Map::new();
    attributes.insert("label".to_owned(), Value::String(info.name.clone()));
    attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
    attributes.insert("type".to_owned(), Value::String("package".to_owned()));
    attributes.insert("ecosystem".to_owned(), Value::String(ecosystem.to_owned()));
    attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
    attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
    if let Some(version) = info.version.filter(is_truthy) {
        attributes.insert("version".to_owned(), version);
    }
    let mut extraction = empty();
    extraction.nodes.push(NodeRecord {
        id: package_node_id.clone(),
        attributes,
    });
    let mut seen = HashSet::new();
    for dependency in info.dependencies {
        if dependency.is_empty() {
            continue;
        }
        let dependency_id = package_id(&dependency);
        if dependency_id == package_node_id || !seen.insert(dependency_id.clone()) {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert(
            "relation".to_owned(),
            Value::String("depends_on".to_owned()),
        );
        attributes.insert("context".to_owned(), Value::String("dependency".to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert("confidence_score".to_owned(), json!(1.0));
        attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
        attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
        attributes.insert("weight".to_owned(), json!(1.0));
        extraction.edges.push(EdgeRecord {
            source: package_node_id.clone(),
            target: dependency_id,
            attributes,
        });
    }
    Ok(extraction)
}

struct PackageInfo {
    name: String,
    version: Option<Value>,
    dependencies: Vec<String>,
}

type ParseResult = Result<Option<PackageInfo>, String>;

fn parse_apm(text: &str) -> ParseResult {
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|error| error.to_string())?;
    let Some(mapping) = value.as_mapping() else {
        return Ok(None);
    };
    let key = |name: &str| serde_yaml_ng::Value::String(name.to_owned());
    let Some(name) = mapping.get(key("name")).and_then(yaml_string) else {
        return Ok(None);
    };
    let version = mapping.get(key("version")).and_then(yaml_to_json_scalar);
    let dependencies = mapping
        .get(key("dependencies"))
        .map_or_else(Vec::new, yaml_dependencies);
    Ok(Some(PackageInfo {
        name,
        version,
        dependencies,
    }))
}

fn yaml_string(value: &serde_yaml_ng::Value) -> Option<String> {
    match value {
        serde_yaml_ng::Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn yaml_dependencies(value: &serde_yaml_ng::Value) -> Vec<String> {
    match value {
        serde_yaml_ng::Value::Mapping(mapping) => {
            mapping.keys().filter_map(yaml_python_string).collect()
        }
        serde_yaml_ng::Value::Sequence(sequence) => sequence
            .iter()
            .filter_map(|item| match item {
                serde_yaml_ng::Value::String(value) => Some(value.clone()),
                serde_yaml_ng::Value::Mapping(mapping) => {
                    mapping.keys().next().and_then(yaml_python_string)
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn yaml_python_string(value: &serde_yaml_ng::Value) -> Option<String> {
    match value {
        serde_yaml_ng::Value::String(value) => Some(value.clone()),
        serde_yaml_ng::Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        serde_yaml_ng::Value::Number(value) => Some(value.to_string()),
        serde_yaml_ng::Value::Null => Some("None".to_owned()),
        _ => None,
    }
}

fn yaml_to_json_scalar(value: &serde_yaml_ng::Value) -> Option<Value> {
    match value {
        serde_yaml_ng::Value::Null => Some(Value::Null),
        serde_yaml_ng::Value::Bool(value) => Some(Value::Bool(*value)),
        serde_yaml_ng::Value::Number(value) => serde_json::to_value(value).ok(),
        serde_yaml_ng::Value::String(value) => Some(Value::String(value.clone())),
        _ => None,
    }
}

fn parse_pyproject(text: &str) -> ParseResult {
    let root: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
    let project = root.get("project").and_then(toml::Value::as_table);
    let poetry = root
        .get("tool")
        .and_then(toml::Value::as_table)
        .and_then(|tool| tool.get("poetry"))
        .and_then(toml::Value::as_table);
    let name = project
        .and_then(|project| project.get("name"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            poetry
                .and_then(|poetry| poetry.get("name"))
                .and_then(toml::Value::as_str)
        });
    let Some(name) = name else {
        return Ok(None);
    };
    let version = project
        .and_then(|project| project.get("version"))
        .or_else(|| poetry.and_then(|poetry| poetry.get("version")))
        .and_then(toml_to_json_scalar);
    let mut dependencies = project
        .and_then(|project| project.get("dependencies"))
        .and_then(toml::Value::as_array)
        .map_or_else(Vec::new, |dependencies| {
            dependencies
                .iter()
                .filter_map(toml::Value::as_str)
                .map(pep508_name)
                .collect()
        });
    if let Some(poetry_dependencies) = poetry
        .and_then(|poetry| poetry.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        dependencies.extend(
            poetry_dependencies
                .keys()
                .filter(|dependency| !dependency.eq_ignore_ascii_case("python"))
                .cloned(),
        );
    }
    Ok(Some(PackageInfo {
        name: name.to_owned(),
        version,
        dependencies,
    }))
}

fn toml_to_json_scalar(value: &toml::Value) -> Option<Value> {
    match value {
        toml::Value::String(value) => Some(Value::String(value.clone())),
        toml::Value::Integer(value) => Some(json!(value)),
        toml::Value::Float(value) => Some(json!(value)),
        toml::Value::Boolean(value) => Some(json!(value)),
        toml::Value::Datetime(value) => Some(Value::String(value.to_string())),
        toml::Value::Array(_) | toml::Value::Table(_) => None,
    }
}

fn pep508_name(specification: &str) -> String {
    specification
        .trim()
        .split(|character: char| {
            character.is_whitespace()
                || matches!(character, '<' | '>' | '=' | '!' | '~' | ';' | '[' | '(')
        })
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn parse_go_mod(text: &str) -> Option<PackageInfo> {
    let mut name = None;
    let mut dependencies = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let line = line.trim();
        if name.is_none()
            && let Some(module) = line.strip_prefix("module ")
            && let Some(module) = module.split_whitespace().next()
        {
            name = Some(module.to_owned());
            continue;
        }
        if line.starts_with("require") && line["require".len()..].trim_start().starts_with('(') {
            in_require = true;
            continue;
        }
        if in_require {
            if line.starts_with(')') {
                in_require = false;
                continue;
            }
            if let Some(dependency) = versioned_go_dependency(line) {
                dependencies.push(dependency);
            }
        } else if let Some(requirement) = line.strip_prefix("require ")
            && let Some(dependency) = versioned_go_dependency(requirement)
        {
            dependencies.push(dependency);
        }
    }
    name.map(|name| PackageInfo {
        name,
        version: None,
        dependencies,
    })
}

fn versioned_go_dependency(line: &str) -> Option<String> {
    let mut fields = line.split_whitespace();
    let dependency = fields.next()?;
    let version = fields.next()?;
    version.starts_with('v').then(|| dependency.to_owned())
}

fn parse_pom(text: &str) -> ParseResult {
    let document = roxmltree::Document::parse(text).map_err(|error| error.to_string())?;
    let root = document.root_element();
    let direct = |name: &str| {
        root.children()
            .find(|node| node.is_element() && node.tag_name().name() == name)
            .and_then(|node| node.text())
            .map(str::to_owned)
    };
    let Some(artifact) = direct("artifactId") else {
        return Ok(None);
    };
    let group = direct("groupId");
    let name = group.map_or_else(|| artifact.clone(), |group| format!("{group}:{artifact}"));
    let mut dependencies = Vec::new();
    for dependency in root
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "dependencies")
        .flat_map(|dependencies| {
            dependencies
                .children()
                .filter(|node| node.is_element() && node.tag_name().name() == "dependency")
        })
    {
        let child = |name: &str| {
            dependency
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == name)
                .and_then(|node| node.text())
        };
        if let Some(artifact) = child("artifactId") {
            dependencies.push(child("groupId").map_or_else(
                || artifact.to_owned(),
                |group| format!("{group}:{artifact}"),
            ));
        }
    }
    Ok(Some(PackageInfo {
        name,
        version: direct("version").map(Value::String),
        dependencies,
    }))
}

fn package_id(name: &str) -> String {
    make_id(&["pkg", name])
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn empty() -> Extraction {
    Extraction {
        raw_calls: None,
        ..Extraction::default()
    }
}

fn failure(message: &str) -> Extraction {
    let mut extraction = empty();
    extraction.error = Some(message.to_owned());
    extraction
}
