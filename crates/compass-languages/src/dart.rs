use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value};

use crate::{Extraction, file_stem, make_id};

const SCALAR_TYPES: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "void",
];
const COLLECTION_TYPES: &[&str] = &["List", "Map", "Set", "Future", "Stream"];

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct State<'a> {
    path: &'a Path,
    text: String,
    source_file: String,
    stem: String,
    file_id: String,
    is_part: bool,
    extraction: Extraction,
    defined: HashSet<String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &[u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let text = strip_comments(std::str::from_utf8(source).unwrap_or_default());
        let mut stem = file_stem(path);
        let mut file_id = make_id(&[&source_file]);
        let mut is_part = false;
        if let Some(parent) = part_parent(path, &text) {
            stem = file_stem(&parent);
            file_id = make_id(&[&parent.to_string_lossy()]);
            is_part = true;
        }
        Self {
            path,
            text,
            source_file,
            stem,
            file_id,
            is_part,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            defined: HashSet::new(),
        }
    }

    fn run(mut self) -> Extraction {
        if !self.is_part {
            let label = self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let file_id = self.file_id.clone();
            let source_file = self.source_file.clone();
            self.push_node(&file_id, &label, "code", Some(&source_file));
        }
        self.add_classes();
        self.add_annotations();
        self.add_typedefs();
        self.add_extensions();
        self.add_variables();
        self.add_functions();
        self.add_imports_exports();
        self.add_generic_lookups();
        self.extraction
    }

    fn add_classes(&mut self) {
        let Ok(pattern) = Regex::new(
            r"(?m)^\s*(?:(?:abstract|sealed|base|interface|final|mixin)\s+)*(?:class|mixin|enum|extension\s+type)\s+(\w+)",
        ) else {
            return;
        };
        let matches: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((
                    capture.get(0)?.start(),
                    capture.get(0)?.end(),
                    capture.get(1)?.as_str().to_owned(),
                ))
            })
            .collect();
        for (start, end, name) in matches {
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, "code", Some(self.source_file.clone()));
            self.add_edge(&self.file_id.clone(), &id, "defines", None);

            let header_end = safe_end(&self.text, end, 500);
            let mut rest = self.text[end..header_end].to_owned();
            rest = skip_balanced_prefix(rest, '<', '>');
            rest = skip_balanced_prefix(rest, '(', ')');
            let boundary = [rest.find('{'), rest.find(';')]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(rest.len());
            let mut header = rest[..boundary].to_owned();
            let mut base = None;
            let mut generics = None;
            if let Some((matched_end, base_name)) =
                anchored_clause(&header, r"^\s*(?:extends|on)\s+([A-Za-z0-9_.]+)")
            {
                base = Some(base_name);
                let remainder = header[matched_end..].to_owned();
                if remainder.trim_start().starts_with('<') {
                    let open = remainder.find('<').unwrap_or_default();
                    if let Some(close) = matching_delimiter(remainder.as_bytes(), open, b'<', b'>')
                    {
                        generics = Some(remainder[open + 1..close].to_owned());
                        header = remainder[close + 1..].to_owned();
                    } else {
                        header = remainder;
                    }
                } else {
                    header = remainder;
                }
            }
            let mut mixins = Vec::new();
            if let Some((matched_end, _)) = anchored_clause(&header, r"^\s*with\s+()") {
                let remainder = &header[matched_end..];
                if let Some(position) = remainder.find("implements") {
                    mixins = split_types(&remainder[..position]);
                    header = remainder[position..].to_owned();
                } else {
                    mixins = split_types(remainder);
                    header.clear();
                }
            }
            let interfaces = anchored_clause(&header, r"^\s*implements\s+()")
                .map_or_else(Vec::new, |(matched_end, _)| {
                    split_types(&header[matched_end..])
                });

            if let Some(base) = base {
                let target = make_id(&[&base]);
                self.add_node(&target, &base, "code", None);
                self.add_edge(&id, &target, "inherits", None);
                if let Some(generics) = generics {
                    for generic in split_types(&generics) {
                        let clean = generic.split('<').next().unwrap_or_default().trim();
                        if !is_builtin(clean, false) {
                            let target = make_id(&[clean]);
                            self.add_node(&target, clean, "code", None);
                            self.add_edge(&id, &target, "references", None);
                        }
                    }
                }
            }
            for mixin in mixins {
                let clean = mixin.split('<').next().unwrap_or_default().trim();
                let target = make_id(&[clean]);
                self.add_node(&target, clean, "code", None);
                self.add_edge(&id, &target, "mixes_in", None);
            }
            for interface in interfaces {
                let clean = interface.split('<').next().unwrap_or_default().trim();
                let target = make_id(&[clean]);
                self.add_node(&target, clean, "code", None);
                self.add_edge(&id, &target, "implements", None);
            }

            if let Some(open) = self.text[start..].find('{').map(|value| start + value) {
                let semicolon = self.text[start..].find(';').map(|value| start + value);
                if semicolon.is_none_or(|semicolon| open < semicolon) {
                    let close = matching_delimiter(self.text.as_bytes(), open, b'{', b'}')
                        .map_or(self.text.len(), |value| value + 1);
                    let body = self.text[open..close].to_owned();
                    self.add_class_framework(&id, &body);
                }
            }
        }
    }

    fn add_class_framework(&mut self, owner: &str, body: &str) {
        self.add_pattern_relations(
            owner,
            body,
            r"\bon<(\w+)>\s*\(",
            "calls",
            "bloc_event",
            false,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\b(?:emit|yield)\s*\(?\s*(?:const\s+)?([A-Z]\w*)\b",
            "calls",
            "emit_state",
            true,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\b(?:\w*[Bb]loc\w*|context\.read<\w+>\(\))\.add\(\s*(?:const\s+)?([A-Z]\w*)\b",
            "calls",
            "bloc_add_event",
            true,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\bref\.(?:watch|read|listen)\s*\(\s*(\w+)\b",
            "references",
            "riverpod_reference",
            false,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\bBloc(?:Builder|Listener|Consumer|Provider|Selector)\s*<\s*([A-Za-z0-9_]+)\b",
            "references",
            "bloc_widget_binding",
            true,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\b(?:read|watch|select|of)\s*<([A-Za-z0-9_]+)>",
            "references",
            "bloc_lookup",
            true,
        );
    }

    fn add_annotations(&mut self) {
        let Ok(pattern) = Regex::new(r"@(\w+)(?:\([^)]*\))?") else {
            return;
        };
        let annotations: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((capture.get(0)?.end(), capture.get(1)?.as_str().to_owned()))
            })
            .collect();
        let class_pattern = Regex::new(
            r"(?m)^\s*(?:(?:abstract|sealed|base|interface|final|mixin)\s+)*(?:class|mixin|enum|extension\s+type)\s+(\w+)",
        )
        .ok();
        let function_pattern = Regex::new(
            r"(?m)^\s*(?:factory\s+|static\s+|async\s+|external\s+|abstract\s+)?(?:\([^)]+\)|[A-Za-z0-9_<>,.?]+)(?:\s+[A-Za-z0-9_<>,.?]+){0,3}\s+(\w+)\s*\(",
        )
        .ok();
        for (end, annotation) in annotations {
            if matches!(
                annotation.as_str(),
                "override" | "deprecated" | "required" | "protected" | "mustCallSuper"
            ) {
                continue;
            }
            let window_end = safe_end(&self.text, end, 300);
            let window = &self.text[end..window_end];
            let class = class_pattern
                .as_ref()
                .and_then(|pattern| pattern.captures(window))
                .and_then(|capture| {
                    Some((capture.get(0)?.start(), capture.get(1)?.as_str().to_owned()))
                });
            let function = function_pattern
                .as_ref()
                .and_then(|pattern| pattern.captures(window))
                .and_then(|capture| {
                    Some((capture.get(0)?.start(), capture.get(1)?.as_str().to_owned()))
                });
            let (position, target, is_class) = match (class, function) {
                (Some(class), Some(function)) if class.0 < function.0 => (class.0, class.1, true),
                (Some(_), Some(function)) => (function.0, function.1, false),
                (Some(class), None) => (class.0, class.1, true),
                (None, Some(function)) => (function.0, function.1, false),
                (None, None) => continue,
            };
            if window[..position].contains([';', '{', '}']) {
                continue;
            }
            let target_id = make_id(&[&self.stem, &target]);
            let annotation_id = make_id(&["annotation", &annotation.to_ascii_lowercase()]);
            self.add_node(&annotation_id, &format!("@{annotation}"), "concept", None);
            self.add_edge(&target_id, &annotation_id, "configures", None);
            if annotation.eq_ignore_ascii_case("riverpod") {
                let provider = if is_class {
                    lower_first(&target) + "Provider"
                } else {
                    format!("{target}Provider")
                };
                let provider_id = make_id(&[&provider]);
                self.add_node(
                    &provider_id,
                    &provider,
                    "concept",
                    Some(self.source_file.clone()),
                );
                self.add_edge_with_context(
                    &target_id,
                    &provider_id,
                    "defines",
                    "riverpod_provider",
                );
            }
        }
    }

    fn add_typedefs(&mut self) {
        let Ok(pattern) =
            Regex::new(r"(?m)^\s*typedef\s+(\w+)\s*(?:<[^>]+>)?\s*=\s*([A-Za-z0-9_<>,.?\s]+);")
        else {
            return;
        };
        let values: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((
                    capture.get(1)?.as_str().to_owned(),
                    capture.get(2)?.as_str().to_owned(),
                ))
            })
            .collect();
        for (name, target) in values {
            let target = target
                .split('<')
                .next()
                .unwrap_or_default()
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .trim();
            if is_builtin(target, true) || target == "Function" {
                continue;
            }
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, "code", Some(self.source_file.clone()));
            self.add_edge(&self.file_id.clone(), &id, "defines", None);
            let target_id = make_id(&[target]);
            self.add_node(&target_id, target, "code", None);
            self.add_edge_with_context(&id, &target_id, "references", "typedef");
        }
    }

    fn add_extensions(&mut self) {
        let Ok(pattern) = Regex::new(r"(?m)^\s{0,4}extension\s+(\w+)?(?:<[^>]+>)?\s+on\s+(\w+)")
        else {
            return;
        };
        let values: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((
                    capture.get(1).map(|value| value.as_str().to_owned()),
                    capture.get(2)?.as_str().to_owned(),
                ))
            })
            .collect();
        for (name, target) in values {
            let raw_name = name
                .clone()
                .unwrap_or_else(|| format!("{}_anonymous_extension", self.stem));
            let label = name.unwrap_or_else(|| format!("Extension on {target}"));
            let id = make_id(&[&self.stem, &raw_name]);
            self.add_node(&id, &label, "code", Some(self.source_file.clone()));
            self.add_edge(&self.file_id.clone(), &id, "defines", None);
            let target_id = make_id(&[&target]);
            self.add_node(&target_id, &target, "code", None);
            self.add_edge(&id, &target_id, "extends", None);
        }
    }

    fn add_variables(&mut self) {
        let Ok(pattern) = Regex::new(
            r"(?m)^\s{0,2}(?:late\s+)?(?:(?:final|const|var)\s+)?(?:\([^)]+\)\s+|([A-Za-z0-9_<>,.?]+(?:\s+[A-Za-z0-9_<>,.?]+){0,3})\s+)?(?:(\w+)|(?:\w+\s*)?\(([^)]+)\))\s*(?:=|$|;)",
        ) else {
            return;
        };
        let values: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((
                    capture.get(0)?.as_str().to_owned(),
                    capture.get(1).map(|value| value.as_str().to_owned()),
                    capture.get(2).map(|value| value.as_str().to_owned()),
                    capture.get(3).map(|value| value.as_str().to_owned()),
                ))
            })
            .collect();
        let modifier = Regex::new(r"^\s*(?:late|final|const|var)\b").ok();
        for (full, var_type, single, destructured) in values {
            if modifier
                .as_ref()
                .is_none_or(|pattern| !pattern.is_match(&full))
                && var_type.is_none()
            {
                continue;
            }
            if let Some(name) = single {
                if matches!(
                    name.as_str(),
                    "if" | "for" | "while" | "switch" | "catch" | "return"
                ) {
                    continue;
                }
                let id = make_id(&[&self.stem, &name]);
                self.add_node(&id, &name, "code", Some(self.source_file.clone()));
                self.add_edge(&self.file_id.clone(), &id, "defines", None);
                if let Some(var_type) = var_type
                    && !is_builtin(var_type.trim(), true)
                {
                    let clean = var_type
                        .split('<')
                        .next()
                        .unwrap_or_default()
                        .rsplit('.')
                        .next()
                        .unwrap_or_default()
                        .trim();
                    let target = make_id(&[clean]);
                    self.add_node(&target, clean, "code", None);
                    self.add_edge_with_context(
                        &self.file_id.clone(),
                        &target,
                        "references",
                        "variable_type",
                    );
                }
            } else if let Some(names) = destructured {
                for raw in names
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    let name = raw.rsplit(':').next().unwrap_or_default().trim();
                    if valid_lower_identifier(name)
                        && !matches!(name, "if" | "for" | "while" | "switch" | "catch" | "return")
                    {
                        let id = make_id(&[&self.stem, name]);
                        self.add_node(&id, name, "code", Some(self.source_file.clone()));
                        self.add_edge(&self.file_id.clone(), &id, "defines", None);
                    }
                }
            }
        }
    }

    fn add_functions(&mut self) {
        let Ok(pattern) = Regex::new(
            r"(?m)^\s{0,2}(?:factory\s+|static\s+|async\s+|external\s+|abstract\s+)?(?:\([^)]+\)|[A-Za-z0-9_<>,.?]+)(?:\s+[A-Za-z0-9_<>,.?]+){0,3}\s+(\w+(?:\.\w+)?)\s*\(",
        ) else {
            return;
        };
        let functions: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| {
                Some((capture.get(0)?.start(), capture.get(1)?.as_str().to_owned()))
            })
            .collect();
        for (start, raw_name) in functions {
            let name = raw_name.rsplit('.').next().unwrap_or_default();
            if matches!(
                name,
                "if" | "for"
                    | "while"
                    | "switch"
                    | "catch"
                    | "return"
                    | "void"
                    | "dynamic"
                    | "final"
                    | "const"
                    | "get"
                    | "set"
            ) || name.starts_with(|character: char| character.is_ascii_uppercase())
            {
                continue;
            }
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, "code", Some(self.source_file.clone()));
            self.add_edge(&self.file_id.clone(), &id, "defines", None);
            let open = self.text[start..].find('{').map(|value| start + value);
            let semicolon = self.text[start..].find(';').map(|value| start + value);
            let arrow = self.text[start..].find("=>").map(|value| start + value);
            if let Some(open) = open
                && semicolon.is_none_or(|semicolon| open < semicolon)
                && arrow.is_none_or(|arrow| open < arrow)
            {
                let close = matching_delimiter(self.text.as_bytes(), open, b'{', b'}')
                    .map_or(self.text.len(), |value| value + 1);
                let body = self.text[open..close].to_owned();
                self.add_function_framework(&id, &body);
            }
        }
    }

    fn add_function_framework(&mut self, owner: &str, body: &str) {
        self.add_pattern_relations(
            owner,
            body,
            r"\bref\.(?:watch|read|listen)\s*\(\s*(\w+)\b",
            "references",
            "riverpod_reference",
            false,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\b(?:\w*[Bb]loc\w*|context\.read<\w+>\(\))\.add\(\s*(?:const\s+)?([A-Z]\w*)\b",
            "calls",
            "bloc_add_event",
            true,
        );
        self.add_pattern_relations(
            owner,
            body,
            r"\b(?:read|watch|select|of)\s*<([A-Za-z0-9_]+)>",
            "references",
            "bloc_lookup",
            true,
        );
        if let Ok(pattern) = Regex::new(
            r#"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?['"]([A-Za-z0-9_/?=&%-]+)['"]"#,
        ) {
            let values: Vec<_> = pattern
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
                .collect();
            for route in values {
                let normalized = route.replace(['/', '?', '=', '&'], "_");
                let target = make_id(&["route", &normalized]);
                self.add_node(&target, &format!("Route {route}"), "concept", None);
                self.add_edge_with_context(owner, &target, "navigates", "route_path");
            }
        }
        if let Ok(pattern) = Regex::new(
            r"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?([A-Z][A-Za-z0-9_]*\.[A-Za-z0-9_]+)",
        ) {
            let values: Vec<_> = pattern
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
                .collect();
            for route in values {
                let target = make_id(&["route", &route.replace('.', "_")]);
                self.add_node(&target, &route, "concept", None);
                self.add_edge_with_context(owner, &target, "navigates", "route_const");
            }
        }
        if let Ok(pattern) = Regex::new(
            r"\b(?:push|replace)\s*\(\s*(?:context\s*,\s*)?.*?\b([A-Z]\w*(?:Route|Screen|Page))\b",
        ) {
            let values: Vec<_> = pattern
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
                .collect();
            for route in values {
                let target = make_id(&[&route]);
                self.add_node(&target, &route, "code", None);
                self.add_edge_with_context(owner, &target, "navigates", "route_object");
            }
        }
    }

    fn add_imports_exports(&mut self) {
        for (keyword, relation) in [("import", "imports"), ("export", "exports")] {
            let Ok(pattern) = Regex::new(&format!(r#"(?m)^\s*{keyword}\s+['"]([^'"]+)['"]"#))
            else {
                continue;
            };
            let packages: Vec<_> = pattern
                .captures_iter(&self.text)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
                .collect();
            for package in packages {
                let target = make_id(&[&package]);
                self.add_node(&target, &package, "code", None);
                self.add_edge(&self.file_id.clone(), &target, relation, None);
            }
        }
    }

    fn add_generic_lookups(&mut self) {
        let Ok(pattern) = Regex::new(r"\b\w+<([A-Za-z0-9_.]+(?:<[A-Za-z0-9_.,\s<>]+>)?)\s*>\s*\(")
        else {
            return;
        };
        let values: Vec<_> = pattern
            .captures_iter(&self.text)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
            .collect();
        for raw in values {
            let raw = raw.rsplit('.').next().unwrap_or_default().trim();
            let clean = raw.split('<').next().unwrap_or_default().trim();
            if is_builtin(clean, true) {
                continue;
            }
            let target = make_id(&[clean]);
            self.add_node(&target, clean, "code", None);
            self.add_edge_with_context(&self.file_id.clone(), &target, "references", "type_lookup");
        }
    }

    fn add_pattern_relations(
        &mut self,
        owner: &str,
        body: &str,
        pattern: &str,
        relation: &str,
        context: &str,
        filter_builtins: bool,
    ) {
        let Ok(pattern) = Regex::new(pattern) else {
            return;
        };
        let values: Vec<_> = pattern
            .captures_iter(body)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
            .collect();
        for value in values {
            if filter_builtins && is_builtin(&value, true) {
                continue;
            }
            let target = make_id(&[&value]);
            self.add_node(&target, &value, "code", None);
            self.add_edge_with_context(owner, &target, relation, context);
        }
    }

    fn add_node(&mut self, id: &str, label: &str, file_type: &str, source_file: Option<String>) {
        self.push_node(id, label, file_type, source_file.as_deref());
    }

    fn push_node(&mut self, id: &str, label: &str, file_type: &str, source_file: Option<&str>) {
        if !self.defined.insert(id.to_owned()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String(file_type.to_owned()));
        attributes.insert(
            "source_file".into(),
            source_file.map_or(Value::Null, |value| Value::String(value.to_owned())),
        );
        attributes.insert("source_location".into(), Value::Null);
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, context: Option<&str>) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert("confidence_score".into(), Value::from(1.0));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::Null);
        attributes.insert("weight".into(), Value::from(1.0));
        if let Some(context) = context {
            attributes.insert("context".into(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_edge_with_context(&mut self, source: &str, target: &str, relation: &str, context: &str) {
        self.add_edge(source, target, relation, Some(context));
    }
}

fn strip_comments(source: &str) -> String {
    let Ok(pattern) = Regex::new(
        r#"(?s)\"\"\"(?:\\.|.)*?\"\"\"|'''(?:\\.|.)*?'''|\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*'|/\*.*?\*/|//[^\n]*"#,
    ) else {
        return source.to_owned();
    };
    pattern
        .replace_all(source, |captures: &regex::Captures<'_>| {
            let value = captures.get(0).map_or("", |value| value.as_str());
            if value.starts_with('/') {
                String::new()
            } else {
                value.to_owned()
            }
        })
        .into_owned()
}

fn part_parent(path: &Path, source: &str) -> Option<PathBuf> {
    let pattern = Regex::new(r#"(?m)^\s*part\s+of\s+['"]([^'"]+)['"]"#).ok()?;
    let parent = pattern.captures(source)?.get(1)?.as_str();
    if !parent.ends_with(".dart") {
        return None;
    }
    let candidate = path.parent()?.join(parent);
    candidate
        .exists()
        .then(|| fs::canonicalize(&candidate).unwrap_or(candidate))
}

fn split_types(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if character == '<' {
            depth += 1;
        } else if character == '>' {
            depth = depth.saturating_sub(1);
        } else if character == ',' && depth == 0 {
            let item = value[start..index].trim();
            if !item.is_empty() {
                values.push(item.to_owned());
            }
            start = index + 1;
        }
    }
    let item = value[start..].trim();
    if !item.is_empty() {
        values.push(item.to_owned());
    }
    values
}

fn matching_delimiter(value: &[u8], open: usize, opener: u8, closer: u8) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in value.iter().enumerate().skip(open) {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(*byte, b'\'' | b'"') {
            quote = Some(*byte);
        } else if *byte == opener {
            depth += 1;
        } else if *byte == closer {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn skip_balanced_prefix(value: String, opener: char, closer: char) -> String {
    let trimmed = value.trim_start();
    if !trimmed.starts_with(opener) {
        return value;
    }
    let whitespace = value.len() - trimmed.len();
    matching_delimiter(value.as_bytes(), whitespace, opener as u8, closer as u8)
        .map_or(value.clone(), |end| value[end + 1..].to_owned())
}

fn anchored_clause(value: &str, pattern: &str) -> Option<(usize, String)> {
    let capture = Regex::new(pattern).ok()?.captures(value)?;
    let full = capture.get(0)?;
    Some((
        full.end(),
        capture.get(1).map_or("", |value| value.as_str()).to_owned(),
    ))
}

fn safe_end(value: &str, start: usize, length: usize) -> usize {
    let mut end = (start + length).min(value.len());
    while end > start && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn is_builtin(value: &str, collections: bool) -> bool {
    SCALAR_TYPES.contains(&value) || (collections && COLLECTION_TYPES.contains(&value))
}

fn valid_lower_identifier(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_lowercase() || *byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn lower_first(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().collect::<String>() + chars.as_str()
    })
}
