use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{ExtractError, Extraction, make_id};

const MAX_XML_BYTES: u64 = 2 * 1024 * 1024;

static PROJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Project\("[^"]*"\)\s*=\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([^"]*)""#)
        .unwrap_or_else(|error| unreachable!("static solution project regex is invalid: {error}"))
});
static PROJECT_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Project\("[^"]*"\)\s*=\s*"[^"]+"\s*,\s*"[^"]+"\s*,\s*"\{([^}]+)\}""#)
        .unwrap_or_else(|error| {
            unreachable!("static solution project-line regex is invalid: {error}")
        })
});
static DEPENDENCY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{([0-9a-fA-F-]+)\}\s*=\s*\{([0-9a-fA-F-]+)\}").unwrap_or_else(|error| {
        unreachable!("static solution dependency regex is invalid: {error}")
    })
});

pub(crate) fn extract_solution(path: &Path) -> Result<Extraction, ExtractError> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("slnx"))
    {
        extract_slnx(path)
    } else {
        extract_sln(path)
    }
}

fn extract_sln(path: &Path) -> Result<Extraction, ExtractError> {
    let data = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8_lossy(&data);
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        &source_file,
    ));
    let mut seen = HashSet::from([file_id.clone()]);
    let mut guid_to_id = HashMap::new();
    for captures in PROJECT.captures_iter(&text) {
        let (Some(name), Some(project_path), Some(guid)) =
            (captures.get(1), captures.get(2), captures.get(3))
        else {
            continue;
        };
        let name = name.as_str();
        let project_path = project_path.as_str().replace('\\', "/");
        let project_source = if project_path == name {
            name.to_owned()
        } else {
            resolve_path(path, &project_path)
        };
        let project_id = make_id(&[&project_source]);
        if !project_id.is_empty() && seen.insert(project_id.clone()) {
            extraction
                .nodes
                .push(node(project_id.clone(), name, "code", &project_source));
            extraction
                .edges
                .push(edge(&file_id, &project_id, "contains", &source_file));
        }
        let guid = guid.as_str().trim_matches(['{', '}']);
        if !guid.is_empty() {
            guid_to_id.insert(guid.to_ascii_lowercase(), project_id);
        }
    }

    let mut in_dependencies = false;
    let mut current_project = None;
    for line in text.lines() {
        if let Some(guid) = PROJECT_LINE
            .captures(line)
            .and_then(|captures| captures.get(1))
        {
            current_project = Some(guid.as_str().to_ascii_lowercase());
            continue;
        }
        if line.trim() == "EndProject" {
            current_project = None;
            continue;
        }
        if line.contains("ProjectSection(ProjectDependencies)") {
            in_dependencies = true;
            continue;
        }
        if in_dependencies && line.contains("EndProjectSection") {
            in_dependencies = false;
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let (Some(current), Some(captures)) = (&current_project, DEPENDENCY.captures(line)) else {
            continue;
        };
        let Some(target_guid) = captures.get(1) else {
            continue;
        };
        let (Some(source), Some(target)) = (
            guid_to_id.get(current),
            guid_to_id.get(&target_guid.as_str().to_ascii_lowercase()),
        ) else {
            continue;
        };
        if source != target {
            extraction
                .edges
                .push(edge(source, target, "imports", &source_file));
        }
    }
    Ok(extraction)
}

fn extract_slnx(path: &Path) -> Result<Extraction, ExtractError> {
    let source = match read_bounded_xml(path)? {
        XmlRead::Data(source) => source,
        XmlRead::TooLarge => return Ok(failure("project file too large")),
    };
    if let Some(error) = unsafe_xml_error(&source) {
        return Ok(failure(error));
    }
    let text = String::from_utf8_lossy(&source);
    let document = match roxmltree::Document::parse(&text) {
        Ok(document) => document,
        Err(error) => return Ok(failure(&format!("XML parse error: {error}"))),
    };
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        &source_file,
    ));
    let mut seen = HashSet::from([file_id.clone()]);
    let mut project_ids = HashSet::new();
    for project in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Project")
    {
        let Some(project_path) = project.attribute("Path") else {
            continue;
        };
        let project_source = resolve_path(path, project_path);
        let project_id = make_id(&[&project_source]);
        if !project_id.is_empty() && seen.insert(project_id.clone()) {
            extraction.nodes.push(node(
                project_id.clone(),
                Path::new(project_path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
                "code",
                &project_source,
            ));
            extraction
                .edges
                .push(edge(&file_id, &project_id, "contains", &source_file));
        }
        if !project_id.is_empty() {
            project_ids.insert(project_id);
        }
    }
    for project in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Project")
    {
        let Some(project_path) = project.attribute("Path") else {
            continue;
        };
        let source_id = make_id(&[&resolve_path(path, project_path)]);
        for dependency in project
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "BuildDependency")
        {
            let Some(dependency_path) = dependency.attribute("Project") else {
                continue;
            };
            let target_id = make_id(&[&resolve_path(path, dependency_path)]);
            if !source_id.is_empty()
                && !target_id.is_empty()
                && source_id != target_id
                && project_ids.contains(&target_id)
            {
                extraction
                    .edges
                    .push(edge(&source_id, &target_id, "imports", &source_file));
            }
        }
    }
    Ok(extraction)
}

pub(crate) fn extract_project(path: &Path) -> Result<Extraction, ExtractError> {
    let source = match read_bounded_xml(path)? {
        XmlRead::Data(source) => source,
        XmlRead::TooLarge => return Ok(failure("project file too large")),
    };
    if let Some(error) = unsafe_xml_error(&source) {
        return Ok(failure(error));
    }
    let text = String::from_utf8_lossy(&source);
    let document = match roxmltree::Document::parse(&text) {
        Ok(document) => document,
        Err(error) => return Ok(failure(&format!("XML parse error: {error}"))),
    };
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        &source_file,
    ));
    let mut seen = HashSet::from([file_id.clone()]);
    for framework in document.descendants().filter(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "TargetFramework" | "TargetFrameworks"
            )
    }) {
        let Some(text) = framework.text() else {
            continue;
        };
        let frameworks: Vec<&str> = if framework.tag_name().name() == "TargetFrameworks" {
            text.trim()
                .split(';')
                .map(str::trim)
                .filter(|framework| !framework.is_empty())
                .collect()
        } else {
            vec![text.trim()]
        };
        for framework in frameworks {
            let framework_id = make_id(&["framework", framework]);
            if !framework_id.is_empty() && seen.insert(framework_id.clone()) {
                extraction.nodes.push(node(
                    framework_id.clone(),
                    framework,
                    "concept",
                    &source_file,
                ));
                extraction
                    .edges
                    .push(edge(&file_id, &framework_id, "references", &source_file));
            }
        }
    }
    for package in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "PackageReference")
    {
        let name = package
            .attribute("Include")
            .or_else(|| package.attribute("include"))
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let version = package
            .attribute("Version")
            .or_else(|| package.attribute("version"))
            .unwrap_or_default();
        let package_id = make_id(&["nuget", name]);
        if !package_id.is_empty() && seen.insert(package_id.clone()) {
            extraction.nodes.push(node(
                package_id.clone(),
                &if version.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name} ({version})")
                },
                "code",
                &source_file,
            ));
        }
        extraction
            .edges
            .push(edge(&file_id, &package_id, "imports", &source_file));
    }
    for project in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "ProjectReference")
    {
        let reference = project
            .attribute("Include")
            .or_else(|| project.attribute("include"))
            .unwrap_or_default();
        if reference.is_empty() {
            continue;
        }
        let project_source = resolve_path(path, reference);
        let project_id = make_id(&[&project_source]);
        if !project_id.is_empty() && seen.insert(project_id.clone()) {
            extraction.nodes.push(node(
                project_id.clone(),
                Path::new(&reference.replace('\\', "/"))
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
                "code",
                &project_source,
            ));
        }
        extraction
            .edges
            .push(edge(&file_id, &project_id, "imports", &source_file));
    }
    if let Some(sdk) = document.root_element().attribute("Sdk")
        && !sdk.is_empty()
    {
        let sdk_id = make_id(&["sdk", sdk]);
        if !sdk_id.is_empty() && seen.insert(sdk_id.clone()) {
            extraction
                .nodes
                .push(node(sdk_id.clone(), sdk, "concept", &source_file));
            extraction
                .edges
                .push(edge(&file_id, &sdk_id, "references", &source_file));
        }
    }
    Ok(extraction)
}

enum XmlRead {
    Data(Vec<u8>),
    TooLarge,
}

fn read_bounded_xml(path: &Path) -> Result<XmlRead, ExtractError> {
    let mut source = Vec::new();
    File::open(path)
        .map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .take(MAX_XML_BYTES + 1)
        .read_to_end(&mut source)
        .map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if source.len() > MAX_XML_BYTES as usize {
        return Ok(XmlRead::TooLarge);
    }
    Ok(XmlRead::Data(source))
}

fn unsafe_xml_error(source: &[u8]) -> Option<&'static str> {
    let lower = source.to_ascii_lowercase();
    (lower.windows(9).any(|window| window == b"<!doctype")
        || lower.windows(8).any(|window| window == b"<!entity"))
    .then_some("refusing XML with DOCTYPE/ENTITY declaration")
}

fn resolve_path(from: &Path, raw: &str) -> String {
    let raw = raw.replace('\\', "/");
    let joined = from.parent().unwrap_or_else(|| Path::new("")).join(raw);
    let absolute = if joined.is_absolute() {
        joined
    } else {
        std::env::current_dir().map_or(joined.clone(), |current| current.join(joined))
    };
    lexical_normalize(&absolute).to_string_lossy().into_owned()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            _ => output.push(component.as_os_str()),
        }
    }
    output
}

fn node(id: String, label: &str, file_type: &str, source_file: &str) -> NodeRecord {
    let mut attributes = Map::new();
    attributes.insert("label".to_owned(), Value::String(label.to_owned()));
    attributes.insert("file_type".to_owned(), Value::String(file_type.to_owned()));
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert("source_location".to_owned(), Value::Null);
    NodeRecord { id, attributes }
}

fn edge(source: &str, target: &str, relation: &str, source_file: &str) -> EdgeRecord {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String("EXTRACTED".to_owned()),
    );
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert("weight".to_owned(), json!(1.0));
    EdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes,
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
