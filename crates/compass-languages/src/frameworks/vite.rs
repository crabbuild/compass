use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value};
use tree_sitter::Node;

use super::typescript_syntax::{StaticValue, TypeScriptSyntax};
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkConfigurationFact, RawFrameworkFact,
    RawFrameworkFileSetFact, RawFrameworkOrigin,
};
use crate::{Extraction, ProjectEvidence};

const MAX_CONFIG_ITEMS: usize = 256;

/// Vite is a build/configuration framework rather than an HTTP router. This
/// adapter uses the already-prepared TypeScript/JavaScript tree and only
/// publishes statically recoverable configuration values.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    project: Option<&ProjectEvidence>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let syntax = TypeScriptSyntax::new(root, source);
    let file_sets = detect_import_meta_globs(path, syntax, project);
    if !is_config(path) {
        return file_sets;
    }
    let imports = import_sources(syntax);
    let define_names = syntax.imported_local_names("vite", "defineConfig");
    let namespace_names = syntax.imported_local_names("vite", "*");
    let define_call = syntax.descendants(root).into_iter().find(|node| {
        let Some(callee) = syntax.call_callee(*node) else {
            return false;
        };
        define_names.iter().any(|name| name == &callee)
            || namespace_names
                .iter()
                .any(|name| callee == format!("{name}.defineConfig"))
    });
    let has_vite_import = imports.contains("vite");
    let source_activates = define_call.is_some() || has_vite_import;
    let project_activates = project.is_none_or(|project| {
        project.has_dependency("vite")
            || project.has_configuration("vite.config.js")
            || project.has_configuration("vite.config.mjs")
            || project.has_configuration("vite.config.ts")
            || project.has_configuration("vite.config.cjs")
    });
    if !project_activates || !source_activates {
        return Vec::new();
    }

    let config_object = define_call
        .and_then(|call| syntax.config_object_from_call(call))
        .or_else(|| syntax.exported_default_config_object());
    let config_anchor = config_object
        .and_then(|node| syntax.range(node))
        .map_or_else(|| anchor(path, source), |range| range_anchor(path, range));

    let mut detail = Map::new();
    let mut configuration_keys = Vec::new();
    let mut aliases = BTreeMap::<String, Value>::new();
    let mut ordered_aliases = Vec::new();
    let plugin_modules = imports
        .iter()
        .filter(|value| is_plugin_package(value))
        .cloned()
        .map(Value::String)
        .collect::<Vec<_>>();
    let plugin_bindings = plugin_import_bindings(syntax, &imports);
    let mut plugins = plugin_modules;
    plugins.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    plugins.dedup();
    let mut configuration_facts = file_sets;

    if let Some(object) = config_object {
        for (ordinal, pair) in object_pairs(syntax, object)
            .into_iter()
            .take(MAX_CONFIG_ITEMS)
            .enumerate()
        {
            let (Some(name), Some(value_node)) = (
                syntax.property_name(pair),
                pair.child_by_field_name("value")
                    .or_else(|| pair.named_child(1)),
            ) else {
                continue;
            };
            let value = syntax.static_value(value_node);
            if matches!(name.as_str(), "resolve" | "plugins") {
                configuration_keys.push(Value::String(name.clone()));
            }
            if name == "resolve" {
                collect_aliases(&value, &mut aliases, &mut ordered_aliases);
            }
            if name == "plugins"
                && let Some(values) = value.array()
            {
                for plugin in values.iter().filter_map(StaticValue::as_string) {
                    if is_plugin_package(plugin) {
                        plugins.push(Value::String(plugin.to_owned()));
                    }
                }
            }
            let Some(ordinal) = u32::try_from(ordinal).ok() else {
                break;
            };
            configuration_facts.push(RawFrameworkFact::Configuration(
                RawFrameworkConfigurationFact {
                    pack_id: "vite-config".to_owned(),
                    framework: "vite".to_owned(),
                    config_id: path.to_string_lossy().replace('\\', "/"),
                    field: name,
                    anchor: syntax
                        .range(pair)
                        .map_or_else(|| config_anchor.clone(), |range| range_anchor(path, range)),
                    ordinal,
                    complete: !matches!(value, StaticValue::Incomplete),
                    value: static_to_json(&value),
                    origin: RawFrameworkOrigin::Config,
                    detail: Map::new(),
                },
            ));
        }
    }
    for call in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "call_expression")
    {
        let Some(callee) = syntax.call_callee(call) else {
            continue;
        };
        if let Some(module) = plugin_bindings.get(&callee) {
            plugins.push(Value::String(module.clone()));
        }
    }
    plugins.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    plugins.dedup();
    if !configuration_keys.is_empty() {
        configuration_keys.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        configuration_keys.dedup();
        detail.insert(
            "configuration_keys".to_owned(),
            Value::Array(configuration_keys),
        );
    }
    if !aliases.is_empty() {
        detail.insert(
            "aliases".to_owned(),
            Value::Object(aliases.into_iter().collect()),
        );
    }
    if !ordered_aliases.is_empty() {
        detail.insert(
            "aliases_ordered".to_owned(),
            Value::Array(ordered_aliases.clone()),
        );
    }
    if !plugins.is_empty() {
        detail.insert("plugins".to_owned(), Value::Array(plugins));
    }

    for fact in &mut configuration_facts {
        if let RawFrameworkFact::Configuration(configuration) = fact
            && configuration.field == "resolve"
            && !ordered_aliases.is_empty()
        {
            configuration.detail.insert(
                "aliases_ordered".to_owned(),
                Value::Array(ordered_aliases.clone()),
            );
        }
    }

    let portable = path.to_string_lossy().replace('\\', "/");
    configuration_facts.push(RawFrameworkFact::Domain(RawDomainFact {
        framework: "vite".to_owned(),
        kind: "framework_configuration".to_owned(),
        name: portable.clone(),
        declaring_scope: portable,
        anchor: config_anchor,
        origin: RawFrameworkOrigin::Config,
        detail,
    }));
    configuration_facts.sort_by_key(fact_key);
    configuration_facts
}

fn detect_import_meta_globs(
    path: &Path,
    syntax: TypeScriptSyntax<'_, '_>,
    project: Option<&ProjectEvidence>,
) -> Vec<RawFrameworkFact> {
    let project_activates = project.is_none_or(|project| {
        project.has_dependency("vite")
            || project.has_any_configuration(&[
                "vite.config.js",
                "vite.config.mjs",
                "vite.config.ts",
                "vite.config.cjs",
            ])
    });
    if !project_activates {
        return Vec::new();
    }
    let mut facts = Vec::new();
    for call in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "call_expression")
    {
        let Some(callee) = syntax.call_callee(call) else {
            continue;
        };
        let (eager, lazy) = match callee.as_str() {
            "import.meta.glob" => (false, true),
            "import.meta.globEager" => (true, false),
            _ => continue,
        };
        let Some(arguments) = call.child_by_field_name("arguments") else {
            continue;
        };
        let mut cursor = arguments.walk();
        let args = arguments.named_children(&mut cursor).collect::<Vec<_>>();
        let Some(pattern_node) = args.first().copied() else {
            continue;
        };
        let (patterns, complete) = glob_patterns(syntax, pattern_node);
        if patterns.is_empty() {
            continue;
        }
        let options = args
            .get(1)
            .map(|node| syntax.static_value(*node))
            .unwrap_or(StaticValue::Incomplete);
        let (eager, lazy, import_mode, query_mode) = glob_options(&options, eager, lazy);
        let Some(range) = syntax.range(call) else {
            continue;
        };
        let package_scope =
            project.map(|project| project.project_root().to_string_lossy().replace('\\', "/"));
        let mut detail = Map::from_iter([
            ("callee".to_owned(), Value::String(callee)),
            ("complete".to_owned(), Value::Bool(complete)),
        ]);
        if let Some(project) = project {
            let configurations = project
                .vite_aliases()
                .iter()
                .map(|rule| rule.configuration.as_str())
                .collect::<BTreeSet<_>>();
            if configurations.len() == 1 && !project.vite_aliases().is_empty() {
                detail.insert(
                    "aliases_ordered".to_owned(),
                    Value::Array(
                        project
                            .vite_aliases()
                            .iter()
                            .map(|rule| {
                                Value::Object(Map::from_iter([
                                    ("find".to_owned(), Value::String(rule.find.clone())),
                                    (
                                        "replacement".to_owned(),
                                        Value::String(rule.replacement.clone()),
                                    ),
                                    (
                                        "kind".to_owned(),
                                        Value::String(rule.kind.as_str().to_owned()),
                                    ),
                                    (
                                        "configuration".to_owned(),
                                        Value::String(rule.configuration.clone()),
                                    ),
                                    ("ordinal".to_owned(), Value::from(rule.ordinal)),
                                ]))
                            })
                            .collect(),
                    ),
                );
            }
        }
        if let Some(options) = static_to_json(&options) {
            detail.insert("options".to_owned(), options);
        }
        facts.push(RawFrameworkFact::FileSet(RawFrameworkFileSetFact {
            pack_id: "vite-config".to_owned(),
            framework: "vite".to_owned(),
            owner_reference: format!("import.meta.glob@{}", range.start_byte),
            patterns: patterns
                .iter()
                .filter(|pattern| !pattern.starts_with('!'))
                .cloned()
                .collect(),
            negative_patterns: patterns
                .into_iter()
                .filter_map(|pattern| pattern.strip_prefix('!').map(str::to_owned))
                .collect(),
            anchor: range_anchor(path, range),
            package_scope,
            eager,
            lazy,
            import_mode,
            query_mode,
            origin: RawFrameworkOrigin::Ast,
            detail,
        }));
    }
    facts
}

fn glob_patterns(syntax: TypeScriptSyntax<'_, '_>, node: Node<'_>) -> (Vec<String>, bool) {
    match syntax.static_value(node) {
        StaticValue::String(value) => (vec![value], true),
        StaticValue::Array(values) => (
            values
                .iter()
                .filter_map(StaticValue::as_string)
                .map(str::to_owned)
                .collect(),
            true,
        ),
        StaticValue::Incomplete if node.kind() == "array" => {
            let mut cursor = node.walk();
            let mut complete = true;
            let patterns = node
                .named_children(&mut cursor)
                .filter_map(|child| match syntax.static_value(child) {
                    StaticValue::String(value) => Some(value),
                    _ => {
                        complete = false;
                        None
                    }
                })
                .collect();
            (patterns, complete)
        }
        _ => (Vec::new(), false),
    }
}

fn glob_options(
    options: &StaticValue,
    default_eager: bool,
    default_lazy: bool,
) -> (bool, bool, bool, bool) {
    let mut eager = default_eager;
    let mut lazy = default_lazy;
    let mut import_mode = false;
    let mut query_mode = false;
    if let Some(entries) = options.object() {
        for (key, value) in entries {
            match (key.as_str(), value) {
                ("eager", StaticValue::Boolean(value)) => {
                    eager = *value;
                    lazy = !*value;
                }
                ("import", StaticValue::String(_)) => import_mode = true,
                ("query", StaticValue::String(_)) => query_mode = true,
                _ => {}
            }
        }
    }
    (eager, lazy, import_mode, query_mode)
}

pub(super) fn is_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("vite.config.")
                && matches!(
                    lower.rsplit_once('.').map(|(_, extension)| extension),
                    Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
                )
        })
}

fn object_pairs<'tree, 'source>(
    syntax: TypeScriptSyntax<'tree, 'source>,
    object: Node<'tree>,
) -> Vec<Node<'tree>> {
    let mut cursor = object.walk();
    object
        .named_children(&mut cursor)
        .filter(|node| node.kind() == "pair" && !syntax.is_incomplete(*node))
        .collect()
}

fn import_sources(syntax: TypeScriptSyntax<'_, '_>) -> BTreeSet<String> {
    syntax
        .descendants(syntax.root())
        .into_iter()
        .filter_map(|node| {
            let import_like = node.kind() == "import_statement"
                || syntax.call_callee(node).as_deref() == Some("require");
            if !import_like {
                return None;
            }
            syntax
                .descendants(node)
                .into_iter()
                .find_map(|child| syntax.literal_string(child))
        })
        .collect()
}

fn plugin_import_bindings(
    syntax: TypeScriptSyntax<'_, '_>,
    imports: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut bindings = BTreeMap::new();
    for module in imports.iter().filter(|module| is_plugin_package(module)) {
        for local in syntax.imported_local_names(module, "default") {
            bindings.insert(local, module.clone());
        }
        for namespace in syntax.imported_local_names(module, "*") {
            for export in ["default", "react", "vue", "svelte", "solid"] {
                bindings.insert(format!("{namespace}.{export}"), module.clone());
            }
        }
    }
    bindings
}

fn collect_aliases(
    value: &StaticValue,
    output: &mut BTreeMap<String, Value>,
    ordered: &mut Vec<Value>,
) {
    let Some(values) = value.object() else {
        return;
    };
    let Some((_, aliases)) = values.iter().find(|(key, _)| key == "alias") else {
        return;
    };
    match aliases {
        StaticValue::Object(values) => {
            for (alias, target) in values.iter().take(MAX_CONFIG_ITEMS) {
                let Some(target_json) = static_to_json(target) else {
                    continue;
                };
                output.entry(alias.clone()).or_insert(target_json.clone());
                ordered.push(Value::Object(Map::from_iter([
                    ("find".to_owned(), Value::String(alias.clone())),
                    ("replacement".to_owned(), target_json),
                    ("kind".to_owned(), Value::String("string".to_owned())),
                ])));
            }
        }
        StaticValue::Array(values) => {
            for value in values.iter().take(MAX_CONFIG_ITEMS) {
                let Some(entries) = value.object() else {
                    continue;
                };
                let find_value = entries
                    .iter()
                    .find(|(key, _)| key == "find")
                    .map(|(_, value)| value);
                let find = find_value.and_then(|value| match value {
                    StaticValue::String(value) | StaticValue::Regex(value) => Some(value.as_str()),
                    _ => None,
                });
                let replacement = entries
                    .iter()
                    .find(|(key, _)| key == "replacement")
                    .and_then(|(_, value)| static_to_json(value));
                let (Some(find), Some(replacement)) = (find, replacement) else {
                    continue;
                };
                let kind = if find_value.is_some_and(|value| matches!(value, StaticValue::Regex(_)))
                {
                    "regex"
                } else {
                    "string"
                };
                ordered.push(Value::Object(Map::from_iter([
                    ("find".to_owned(), Value::String(find.to_owned())),
                    ("replacement".to_owned(), replacement),
                    ("kind".to_owned(), Value::String(kind.to_owned())),
                ])));
            }
        }
        _ => {}
    }
}

fn static_to_json(value: &StaticValue) -> Option<Value> {
    match value {
        StaticValue::String(value) | StaticValue::Regex(value) => {
            Some(Value::String(value.clone()))
        }
        StaticValue::Boolean(value) => Some(Value::Bool(*value)),
        StaticValue::Number(value) => serde_json::from_str(value).ok(),
        StaticValue::Null => Some(Value::Null),
        StaticValue::Array(values) => Some(Value::Array(
            values.iter().filter_map(static_to_json).collect(),
        )),
        StaticValue::Object(values) => Some(Value::Object(
            values
                .iter()
                .filter_map(|(key, value)| static_to_json(value).map(|value| (key.clone(), value)))
                .collect(),
        )),
        StaticValue::Incomplete => None,
    }
}

fn is_plugin_package(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("@vitejs/plugin-")
        || value.starts_with("vite-plugin-")
        || value.starts_with("unplugin-")
}

fn fact_key(fact: &RawFrameworkFact) -> (String, u64, String) {
    (
        fact.anchor().source_file.clone(),
        fact.anchor().start_byte,
        match fact {
            RawFrameworkFact::Configuration(configuration) => configuration.field.clone(),
            RawFrameworkFact::Domain(domain) => domain.kind.clone(),
            _ => String::new(),
        },
    )
}

fn range_anchor(path: &Path, range: super::typescript_syntax::SyntaxRange) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: u64::try_from(range.start_byte).unwrap_or(u64::MAX),
        end_byte: u64::try_from(range.end_byte).unwrap_or(u64::MAX),
        start_line: range.start_line,
        start_column: range.start_column,
        end_line: range.end_line,
        end_column: range.end_column,
    }
}

fn anchor(path: &Path, source: &[u8]) -> RawFrameworkAnchor {
    let end_line = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: 0,
        end_byte: u64::try_from(source.len()).unwrap_or(u64::MAX),
        start_line: 1,
        start_column: 0,
        end_line: u32::try_from(end_line).unwrap_or(u32::MAX),
        end_column: 0,
    }
}
