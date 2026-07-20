use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{Engine, ExtractError, Extraction, file_stem, make_id};

pub(crate) fn extract(
    engine: &mut Engine,
    path: &Path,
    language: &str,
) -> Result<Extraction, ExtractError> {
    match language {
        "razor" => extract_razor(path),
        "blade" => extract_blade(path),
        "vue" => extract_vue(engine, path),
        "svelte" => extract_svelte_or_astro(engine, path, false),
        "astro" => extract_svelte_or_astro(engine, path, true),
        _ => Err(ExtractError::Unsupported(path.to_path_buf())),
    }
}

fn read(path: &Path) -> Result<String, ExtractError> {
    let bytes = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract_razor(path: &Path) -> Result<Extraction, ExtractError> {
    let source = read(path)?;
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = Extraction {
        raw_calls: None,
        ..Extraction::default()
    };
    extraction.nodes.push(node(
        &file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        Some(&source_file),
        None,
        false,
    ));
    let mut seen = HashSet::from([file_id.clone()]);

    let (Ok(using), Ok(inject), Ok(inherits), Ok(model), Ok(page)) = (
        Regex::new(r"^@using\s+([\w.]+)"),
        Regex::new(r"^@inject\s+([\w.<>\[\]]+)\s+(\w+)"),
        Regex::new(r"^@inherits\s+([\w.<>\[\]]+)"),
        Regex::new(r"^@model\s+([\w.<>\[\]]+)"),
        Regex::new(r#"^@page\s+"([^"]+)""#),
    ) else {
        return Ok(extraction);
    };
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let directive = [
            (&using, "imports"),
            (&inject, "imports"),
            (&inherits, "inherits"),
            (&model, "references"),
        ]
        .into_iter()
        .find_map(|(pattern, relation)| {
            pattern
                .captures(line)
                .and_then(|capture| capture.get(1))
                .map(|value| (value.as_str(), relation))
        });
        if let Some((target, relation)) = directive {
            add_razor_ref(
                &mut extraction,
                &mut seen,
                &file_id,
                &source_file,
                target,
                relation,
                line_number,
            );
            continue;
        }
        if let Some(route) = page
            .captures(line)
            .and_then(|capture| capture.get(1))
            .map(|value| value.as_str())
        {
            let route_id = make_id(&["route", route]);
            if !route_id.is_empty() && seen.insert(route_id.clone()) {
                extraction.nodes.push(node(
                    &route_id,
                    &format!("route:{route}"),
                    "concept",
                    Some(&source_file),
                    Some(line_number),
                    false,
                ));
                extraction.edges.push(edge(
                    &file_id,
                    &route_id,
                    "references",
                    &source_file,
                    None,
                    false,
                ));
            }
        }
    }

    let Ok(component) = Regex::new(r"<([A-Z][A-Za-z0-9]+)[\s/>]") else {
        return Ok(extraction);
    };
    const HTML_TAGS: &[&str] = &[
        "DOCTYPE", "Html", "Head", "Body", "Div", "Span", "Table", "Form", "Input", "Button",
        "Select", "Option", "Label", "Textarea", "Script", "Style", "Link", "Meta", "Title",
        "Header", "Footer", "Nav", "Main", "Section", "Article", "Aside",
    ];
    for capture in component.captures_iter(&source) {
        let (Some(matched), Some(name)) = (capture.get(0), capture.get(1)) else {
            continue;
        };
        let name = name.as_str();
        if HTML_TAGS.contains(&name) {
            continue;
        }
        let line = source[..matched.start()]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        add_razor_ref(
            &mut extraction,
            &mut seen,
            &file_id,
            &source_file,
            name,
            "calls",
            line,
        );
    }

    let (Ok(code), Ok(method)) = (
        Regex::new(r"@code\s*\{"),
        Regex::new(
            r"(?:public|private|protected|internal|static|async|override|virtual|abstract)\s+[\w<>\[\],\s]+\s+(\w+)\s*\(",
        ),
    ) else {
        return Ok(extraction);
    };
    for code_match in code.find_iter(&source) {
        let block_start = code_match.end();
        let Some(block_end) = matching_brace(&source, block_start) else {
            continue;
        };
        let block = &source[block_start..block_end];
        for capture in method.captures_iter(block) {
            let (Some(matched), Some(name)) = (capture.get(0), capture.get(1)) else {
                continue;
            };
            let name = name.as_str();
            let absolute = block_start + matched.start();
            let line = source[..absolute]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            let method_id = make_id(&[&file_stem(path), name]);
            if !method_id.is_empty() && seen.insert(method_id.clone()) {
                extraction.nodes.push(node(
                    &method_id,
                    name,
                    "code",
                    Some(&source_file),
                    Some(line),
                    false,
                ));
                extraction.edges.push(edge(
                    &file_id,
                    &method_id,
                    "contains",
                    &source_file,
                    None,
                    false,
                ));
            }
        }
    }
    Ok(extraction)
}

fn add_razor_ref(
    extraction: &mut Extraction,
    seen: &mut HashSet<String>,
    file_id: &str,
    source_file: &str,
    target: &str,
    relation: &str,
    line: usize,
) {
    let target_id = make_id(&[target]);
    if target_id.is_empty() {
        return;
    }
    if seen.insert(target_id.clone()) {
        extraction.nodes.push(node(
            &target_id,
            target,
            "code",
            Some(source_file),
            Some(line),
            false,
        ));
    }
    extraction.edges.push(edge(
        file_id,
        &target_id,
        relation,
        source_file,
        Some(line),
        false,
    ));
}

fn extract_blade(path: &Path) -> Result<Extraction, ExtractError> {
    let source = read(path)?;
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = Extraction {
        raw_calls: None,
        ..Extraction::default()
    };
    extraction.nodes.push(node(
        &file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        Some(&source_file),
        None,
        false,
    ));
    let mut seen = HashSet::from([file_id.clone()]);
    let patterns = [
        (r#"@include\(['"]([^'"]+)['"]"#, "includes", true),
        (r"<livewire:([\w.\-]+)", "uses_component", false),
        (r#"wire:click=["']([^"']+)["']"#, "binds_method", false),
    ];
    for (raw_pattern, relation, slash_id) in patterns {
        let Ok(pattern) = Regex::new(raw_pattern) else {
            continue;
        };
        for capture in pattern.captures_iter(&source) {
            let Some(label) = capture.get(1).map(|value| value.as_str()) else {
                continue;
            };
            let id_input = if slash_id {
                label.replace('.', "/")
            } else {
                label.to_owned()
            };
            let target_id = make_id(&[&id_input]);
            if seen.insert(target_id.clone()) {
                extraction.nodes.push(node(
                    &target_id,
                    label,
                    "code",
                    Some(&source_file),
                    None,
                    false,
                ));
            }
            extraction.edges.push(edge(
                &file_id,
                &target_id,
                relation,
                &source_file,
                None,
                true,
            ));
        }
    }
    Ok(extraction)
}

fn extract_vue(engine: &mut Engine, path: &Path) -> Result<Extraction, ExtractError> {
    let source = read(path)?;
    let (masked, language) = mask_vue(&source);
    let (language, grammar) = match language.as_deref() {
        Some("tsx") => ("tsx", "tsx"),
        Some("js" | "jsx") => ("javascript", "javascript"),
        _ => ("typescript", "typescript"),
    };
    let mut extraction =
        engine.extract_embedded_script(path, masked.as_bytes(), language, grammar)?;
    add_dynamic_imports(&mut extraction, path, &source);
    Ok(extraction)
}

fn extract_svelte_or_astro(
    engine: &mut Engine,
    path: &Path,
    astro: bool,
) -> Result<Extraction, ExtractError> {
    let source = read(path)?;
    let mut extraction =
        engine.extract_embedded_script(path, source.as_bytes(), "javascript", "javascript")?;
    add_dynamic_imports(&mut extraction, path, &source);
    let mut regions = Vec::new();
    if astro && let Some(frontmatter) = astro_frontmatter(&source) {
        regions.push(frontmatter);
    }
    regions.extend(script_bodies(&source));
    add_static_imports(&mut extraction, path, &regions);
    Ok(extraction)
}

fn mask_vue(source: &str) -> (String, Option<String>) {
    let Ok(pattern) =
        Regex::new(r#"(?is)(<script\b(?:"[^"]*"|'[^']*'|[^>"'])*>)(.*?)(</script\s*>)"#)
    else {
        return (blank_except_newlines(source), None);
    };
    let Ok(language_pattern) = Regex::new(r#"(?i)\blang\s*=\s*['"]?([A-Za-z]+)['"]?"#) else {
        return (blank_except_newlines(source), None);
    };
    let mut masked = String::with_capacity(source.len());
    let mut position = 0;
    let mut language = None;
    for capture in pattern.captures_iter(source) {
        let (Some(full), Some(open), Some(body), Some(close)) = (
            capture.get(0),
            capture.get(1),
            capture.get(2),
            capture.get(3),
        ) else {
            continue;
        };
        masked.push_str(&blank_except_newlines(&source[position..full.start()]));
        masked.push_str(&blank_except_newlines(open.as_str()));
        masked.push_str(body.as_str());
        masked.push_str(&blank_except_newlines(close.as_str()));
        position = full.end();
        if language.is_none() {
            language = language_pattern
                .captures(open.as_str())
                .and_then(|value| value.get(1))
                .map(|value| value.as_str().to_ascii_lowercase());
        }
    }
    masked.push_str(&blank_except_newlines(&source[position..]));
    (masked, language)
}

fn blank_except_newlines(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' => character,
            _ => ' ',
        })
        .collect()
}

fn script_bodies(source: &str) -> Vec<&str> {
    let Ok(pattern) = Regex::new(r"(?is)<script\b[^>]*>(.*?)</script\s*>") else {
        return Vec::new();
    };
    pattern
        .captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .collect()
}

fn astro_frontmatter(source: &str) -> Option<&str> {
    let Ok(pattern) = Regex::new(r"(?s)\A\s*---\s*\r?\n(.*?)\r?\n---\s*(?:\r?\n|\z)") else {
        return None;
    };
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str())
}

fn add_dynamic_imports(extraction: &mut Extraction, path: &Path, source: &str) {
    let Ok(pattern) = Regex::new(r#"import\(\s*['"]([^'"]+)['"]\s*\)"#) else {
        return;
    };
    let imports: Vec<_> = pattern
        .captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
        .collect();
    for raw in imports {
        add_template_import(extraction, path, &raw, "dynamic_import", true);
    }
}

fn add_static_imports(extraction: &mut Extraction, path: &Path, regions: &[&str]) {
    let Ok(pattern) = Regex::new(r#"import\s+(?:[^'"`;]+?\s+from\s+)?['"]([^'"]+)['"]"#) else {
        return;
    };
    for region in regions {
        let imports: Vec<_> = pattern
            .captures_iter(region)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
            .collect();
        for raw in imports {
            add_template_import(extraction, path, &raw, "imports_from", false);
        }
    }
}

fn add_template_import(
    extraction: &mut Extraction,
    path: &Path,
    raw: &str,
    relation: &str,
    dynamic: bool,
) {
    if raw.is_empty() {
        return;
    }
    let (target_path, target_id) = if raw.starts_with('.') {
        let joined = lexical_normalize(&path.parent().unwrap_or_else(|| Path::new(".")).join(raw));
        let resolved = if dynamic {
            resolve_js_path(&joined)
        } else {
            rewrite_js_extension(joined)
        };
        let target_id = make_id(&[&resolved.to_string_lossy()]);
        (resolved.to_string_lossy().into_owned(), target_id)
    } else {
        let module = raw.rsplit('/').next().unwrap_or_default();
        if module.is_empty() {
            return;
        }
        (raw.to_owned(), make_id(&[module]))
    };
    let file_id = make_id(&[&path.to_string_lossy()]);
    let exists = extraction.nodes.iter().any(|node| node.id == target_id);
    if !exists {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(raw.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert("source_file".to_owned(), Value::String(target_path));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        extraction.nodes.push(NodeRecord {
            id: target_id.clone(),
            attributes,
        });
    }
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String("EXTRACTED".to_owned()),
    );
    attributes.insert(
        "source_file".to_owned(),
        Value::String(path.to_string_lossy().into_owned()),
    );
    extraction.edges.push(EdgeRecord {
        source: file_id,
        target: target_id,
        attributes,
    });
}

fn rewrite_js_extension(mut path: PathBuf) -> PathBuf {
    match path.extension().and_then(|value| value.to_str()) {
        Some("js") => path.set_extension("ts"),
        Some("jsx") => path.set_extension("tsx"),
        _ => false,
    };
    path
}

fn resolve_js_path(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js")
    ) {
        let candidate = path.with_extension("ts");
        if candidate.is_file() {
            return candidate;
        }
    } else if matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("jsx")
    ) {
        let candidate = path.with_extension("tsx");
        if candidate.is_file() {
            return candidate;
        }
    }
    for extension in [
        "ts", "tsx", "mts", "cts", "svelte", "js", "jsx", "mjs", "cjs",
    ] {
        let candidate = path.with_file_name(format!(
            "{}.{extension}",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        ));
        if candidate.is_file() {
            return candidate;
        }
    }
    if path.is_dir() {
        for name in [
            "index.ts",
            "index.tsx",
            "index.svelte",
            "index.js",
            "index.jsx",
            "index.mjs",
        ] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path.to_path_buf()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn matching_brace(source: &str, start: usize) -> Option<usize> {
    let mut depth = 1_usize;
    for (offset, byte) in source.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn node(
    id: &str,
    label: &str,
    file_type: &str,
    source_file: Option<&str>,
    line: Option<usize>,
    confidence: bool,
) -> NodeRecord {
    let mut attributes = Map::new();
    attributes.insert("label".to_owned(), Value::String(label.to_owned()));
    attributes.insert("file_type".to_owned(), Value::String(file_type.to_owned()));
    attributes.insert(
        "source_file".to_owned(),
        source_file.map_or(Value::Null, |value| Value::String(value.to_owned())),
    );
    attributes.insert(
        "source_location".to_owned(),
        line.map_or(Value::Null, |value| Value::String(format!("L{value}"))),
    );
    if confidence {
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
    }
    NodeRecord {
        id: id.to_owned(),
        attributes,
    }
}

fn edge(
    source: &str,
    target: &str,
    relation: &str,
    source_file: &str,
    line: Option<usize>,
    confidence_score: bool,
) -> EdgeRecord {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String("EXTRACTED".to_owned()),
    );
    if confidence_score {
        attributes.insert("confidence_score".to_owned(), Value::from(1.0));
    }
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    if let Some(line) = line {
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
    } else if confidence_score {
        attributes.insert("source_location".to_owned(), Value::Null);
    }
    attributes.insert("weight".to_owned(), Value::from(1.0));
    EdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes,
    }
}
