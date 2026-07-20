use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, file_stem, make_id, normalize_id};

const CONFIG_NAMES: &[&str] = &[
    "package.json",
    "tsconfig.json",
    "jsconfig.json",
    "composer.json",
    "deno.json",
    "deno.jsonc",
    "bower.json",
    "manifest.json",
    "app.json",
    "now.json",
    "vercel.json",
    "angular.json",
    "nest-cli.json",
    "biome.json",
    "biome.jsonc",
    "renovate.json",
    ".babelrc",
    ".babelrc.json",
    ".eslintrc.json",
    ".prettierrc.json",
    ".prettierrc",
    "babel.config.json",
];
const CONFIG_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
    "bundleDependencies",
    "bundledDependencies",
    "extends",
    "$ref",
    "$schema",
    "compilerOptions",
];
const DEPENDENCY_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
    "bundleDependencies",
    "bundledDependencies",
];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let document = if root.kind() == "document" {
        root.child(0).unwrap_or(root)
    } else {
        root
    };
    if document.kind() != "object" {
        return skipped("data json (non-object root)");
    }
    if !is_config(path, document, source) {
        return skipped("data json (not a config/manifest)");
    }

    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut state = State {
        source,
        source_file,
        stem: file_stem(path),
        file_id: file_id.clone(),
        extraction: empty(),
        seen_nodes: HashSet::new(),
        pair_count: 0,
    };
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        "code",
    );
    state.walk_object(document, &file_id, None, 0);
    state.extraction
}

pub(crate) fn error(message: &str) -> Extraction {
    let mut extraction = empty();
    extraction.error = Some(message.to_owned());
    extraction
}

fn skipped(message: &str) -> Extraction {
    let mut extraction = empty();
    extraction
        .extensions
        .insert("skipped".to_owned(), Value::String(message.to_owned()));
    extraction
}

fn empty() -> Extraction {
    Extraction {
        raw_calls: None,
        ..Extraction::default()
    }
}

fn is_config(path: &Path, object: Node<'_>, source: &[u8]) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if CONFIG_NAMES.contains(&name.as_str())
        || [
            ".eslintrc.json",
            ".prettierrc.json",
            ".babelrc.json",
            "tsconfig.json",
            "jsconfig.json",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return true;
    }
    let mut cursor = object.walk();
    object.children(&mut cursor).any(|child| {
        child.kind() == "pair"
            && pair_key(child, source).is_some_and(|key| CONFIG_KEYS.contains(&key.as_str()))
    })
}

struct State<'source> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    pair_count: usize,
}

impl State<'_> {
    fn walk_object(
        &mut self,
        object: Node<'_>,
        parent_id: &str,
        parent_key: Option<&str>,
        depth: usize,
    ) {
        if depth > 6 {
            return;
        }
        let mut cursor = object.walk();
        for pair in object.children(&mut cursor) {
            if pair.kind() != "pair" {
                continue;
            }
            if self.pair_count >= 500 {
                return;
            }
            self.pair_count += 1;
            let Some(key) = pair_key(pair, self.source) else {
                continue;
            };
            if key.is_empty() || normalize_id(&key).is_empty() {
                continue;
            }
            let key_id = parent_key.map_or_else(
                || make_id(&[&self.stem, &key]),
                |parent| make_id(&[&self.stem, parent, &key]),
            );
            if key_id.is_empty() {
                continue;
            }
            let line = pair.start_position().row + 1;
            self.add_node(key_id.clone(), &key, line, "code");
            self.add_edge(parent_id, &key_id, "contains", line, None);

            let Some(value) = pair.child_by_field_name("value") else {
                continue;
            };
            match value.kind() {
                "object" => self.walk_object(value, &key_id, Some(&key), depth + 1),
                "array" => self.add_array_references(value, &key_id, line),
                "string" => self.add_string_reference(value, &key, parent_key, &key_id, line),
                _ => {}
            }
        }
    }

    fn add_array_references(&mut self, array: Node<'_>, key_id: &str, line: usize) {
        let mut cursor = array.walk();
        for item in array.children(&mut cursor) {
            if item.kind() != "string" {
                continue;
            }
            let reference = string_text(item, self.source);
            if reference.is_empty() {
                continue;
            }
            let reference_id = make_id(&["ref", &reference]);
            if reference_id.is_empty() {
                continue;
            }
            self.add_node(reference_id.clone(), &reference, line, "concept");
            self.add_edge(key_id, &reference_id, "extends", line, Some("import"));
        }
    }

    fn add_string_reference(
        &mut self,
        value: Node<'_>,
        key: &str,
        parent_key: Option<&str>,
        key_id: &str,
        line: usize,
    ) {
        let text = string_text(value, self.source);
        if text.is_empty() {
            return;
        }
        if key == "extends" {
            let reference_id = make_id(&["ref", &text]);
            if !reference_id.is_empty() {
                self.add_node(reference_id.clone(), &text, line, "concept");
                self.add_edge(
                    &self.file_id.clone(),
                    &reference_id,
                    "extends",
                    line,
                    Some("import"),
                );
            }
        } else if key == "$ref" {
            let reference_id = make_id(&["ref", &text]);
            if !reference_id.is_empty() {
                self.add_edge(key_id, &reference_id, "references", line, None);
            }
        } else if parent_key.is_some_and(|parent| DEPENDENCY_KEYS.contains(&parent)) {
            let dependency_id = make_id(&[key]);
            if !dependency_id.is_empty() {
                self.add_node(dependency_id.clone(), key, line, "concept");
                self.add_edge(key_id, &dependency_id, "imports", line, Some("import"));
            }
        }
    }

    fn add_node(&mut self, id: String, label: &str, line: usize, file_type: &str) {
        if id.is_empty() || !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String(file_type.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
        self.extraction.nodes.push(NodeRecord { id, attributes });
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
        attributes.insert("weight".to_owned(), json!(1.0));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }
}

fn pair_key(pair: Node<'_>, source: &[u8]) -> Option<String> {
    let key = pair.child_by_field_name("key")?;
    Some(if key.kind() == "string" {
        string_text(key, source)
    } else {
        text(key, source).to_owned()
    })
}

fn string_text(node: Node<'_>, source: &[u8]) -> String {
    node.child_by_field_name("string_content").map_or_else(
        || text(node, source).trim_matches(['"', '\'']).to_owned(),
        |content| text(content, source).to_owned(),
    )
}

fn text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}
