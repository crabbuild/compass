use std::collections::HashSet;
use std::path::Path;

use crate::{Extraction, RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::Node;

use crate::make_id;

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
        file_id: file_id.clone(),
        owner_id: file_id.clone(),
        extraction: empty(),
        seen_nodes: HashSet::new(),
        seen_edges: HashSet::new(),
        pair_count: 0,
    };
    state.add_file_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    );
    if let Some((name, dialect)) = schema_document(path, document, source) {
        state.add_schema_node(&name, &dialect);
    }
    state.walk_object(document, &file_id, "", None, 0);
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
    file_id: String,
    owner_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String, usize)>,
    pair_count: usize,
}

impl State<'_> {
    fn walk_object(
        &mut self,
        object: Node<'_>,
        parent_id: &str,
        prefix: &str,
        parent_key: Option<&str>,
        depth: usize,
    ) {
        if depth > 12 {
            return;
        }
        let mut cursor = object.walk();
        for pair in object.children(&mut cursor) {
            if pair.kind() != "pair" {
                continue;
            }
            if self.pair_count >= 2_000 {
                return;
            }
            self.pair_count += 1;
            let Some(key) = pair_key(pair, self.source) else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            let key_path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            let key_id = make_id(&["config-key", &self.source_file, &key_path]);
            let line = pair.start_position().row + 1;
            self.add_config_key(&key_id, &key, &key_path, line);
            self.add_edge(&self.owner_id.clone(), &key_id, "contains", line, None);
            if parent_id != self.file_id {
                self.add_edge(parent_id, &key_id, "references", line, Some("config-child"));
            }

            let Some(value) = pair.child_by_field_name("value") else {
                continue;
            };
            match value.kind() {
                "object" => {
                    self.walk_object(value, &key_id, &key_path, Some(&key), depth + 1);
                }
                "array" => self.add_array_references(value, &key_id),
                "string" => {
                    self.add_string_reference(value, &key, parent_key, &key_id);
                }
                _ => {}
            }
        }
    }

    fn add_array_references(&mut self, array: Node<'_>, key_id: &str) {
        let mut cursor = array.walk();
        for item in array.children(&mut cursor) {
            if item.kind() != "string" {
                continue;
            }
            let reference = string_text(item, self.source);
            if reference.is_empty() {
                continue;
            }
            let line = item.start_position().row + 1;
            let reference_id = self.add_resource(&reference, line);
            self.add_edge(
                key_id,
                &reference_id,
                "references",
                line,
                Some("config-array"),
            );
        }
    }

    fn add_string_reference(
        &mut self,
        value: Node<'_>,
        key: &str,
        parent_key: Option<&str>,
        key_id: &str,
    ) {
        let text = string_text(value, self.source);
        if text.is_empty() {
            return;
        }
        let line = value.start_position().row + 1;
        if matches!(key, "extends" | "$ref" | "$schema") {
            let reference_id = self.add_resource(&text, line);
            self.add_edge(
                key_id,
                &reference_id,
                "references",
                line,
                Some("config-reference"),
            );
        } else if parent_key.is_some_and(|parent| DEPENDENCY_KEYS.contains(&parent)) {
            let dependency_id = self.add_package_resource(key, line);
            self.add_edge(key_id, &dependency_id, "imports", line, Some("dependency"));
        }
    }

    fn add_file_node(&mut self, id: String, name: &str) {
        let mut attributes = self.base_attributes("file", name, name, 1);
        attributes.insert("file_type".into(), Value::String("code".into()));
        self.push_node(id, attributes);
    }

    fn add_config_key(&mut self, id: &str, name: &str, key_path: &str, line: usize) {
        let mut attributes = self.base_attributes("config_key", name, key_path, line);
        attributes.insert("format".into(), Value::String("json".into()));
        attributes.insert("key_path".into(), Value::String(key_path.to_owned()));
        self.push_node(id.to_owned(), attributes);
    }

    fn add_schema_node(&mut self, name: &str, dialect: &str) {
        let id = make_id(&["config-schema", &self.source_file]);
        let mut attributes = self.base_attributes("schema", name, name, 1);
        attributes.insert("dialect".into(), Value::String(dialect.to_owned()));
        attributes.insert("namespace".into(), Value::String(self.source_file.clone()));
        self.push_node(id.clone(), attributes);
        self.add_edge(&self.file_id.clone(), &id, "contains", 1, None);
        self.owner_id = id;
    }

    fn add_resource(&mut self, reference: &str, line: usize) -> String {
        let id = make_id(&["config-resource", reference]);
        let mut attributes = self.base_attributes("resource", reference, reference, line);
        attributes.insert("file_type".into(), Value::String("concept".into()));
        attributes.insert("uri".into(), Value::String(reference.to_owned()));
        self.push_node(id.clone(), attributes);
        id
    }

    fn add_package_resource(&mut self, package: &str, line: usize) -> String {
        let id = make_id(&["package-resource", package]);
        let mut attributes = self.base_attributes("resource", package, package, line);
        attributes.insert("file_type".into(), Value::String("concept".into()));
        attributes.insert("uri".into(), Value::String(format!("pkg:{package}")));
        self.push_node(id.clone(), attributes);
        id
    }

    fn base_attributes(
        &self,
        kind: &str,
        name: &str,
        qualified_name: &str,
        line: usize,
    ) -> Map<String, Value> {
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(name.to_owned()));
        attributes.insert("name".into(), Value::String(name.to_owned()));
        attributes.insert(
            "qualified_name".into(),
            Value::String(qualified_name.to_owned()),
        );
        attributes.insert("symbol_kind".into(), Value::String(kind.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert("language".into(), Value::String("json".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{line}")));
        attributes.insert("line_start".into(), Value::from(line));
        attributes.insert("line_end".into(), Value::from(line));
        attributes.insert("_origin".into(), Value::String("config".into()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.json-config".into()),
        );
        attributes
    }

    fn push_node(&mut self, id: String, attributes: Map<String, Value>) {
        if self.seen_nodes.insert(id.clone()) {
            self.extraction.nodes.push(NodeRecord { id, attributes });
        }
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        if source.is_empty()
            || target.is_empty()
            || source == target
            || !self.seen_edges.insert((
                source.to_owned(),
                target.to_owned(),
                relation.to_owned(),
                line,
            ))
        {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{line}")));
        attributes.insert("weight".into(), json!(1.0));
        attributes.insert("_origin".into(), Value::String("config".into()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.json-config".into()),
        );
        if let Some(context) = context {
            attributes.insert("context".into(), Value::String(context.to_owned()));
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

fn schema_document(path: &Path, object: Node<'_>, source: &[u8]) -> Option<(String, String)> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("schema");
    let mut cursor = object.walk();
    let pairs = object
        .children(&mut cursor)
        .filter(|child| child.kind() == "pair")
        .collect::<Vec<_>>();
    for pair in pairs {
        let key = pair_key(pair, source)?;
        if key == "$schema" {
            let dialect = pair
                .child_by_field_name("value")
                .map(|value| string_text(value, source))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "json-schema".to_owned());
            return Some((file_name.to_owned(), dialect));
        }
        if matches!(key.as_str(), "openapi" | "swagger") {
            return Some((file_name.to_owned(), key));
        }
    }
    file_name
        .to_ascii_lowercase()
        .ends_with(".schema.json")
        .then(|| (file_name.to_owned(), "json-schema".to_owned()))
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
