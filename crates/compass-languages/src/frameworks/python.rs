use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};

#[derive(Clone, Debug)]
struct Receiver {
    framework: &'static str,
    prefix: String,
}

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    let text = std::str::from_utf8(source).unwrap_or_default();
    let aliases = import_aliases(root, source);
    let receivers = receiver_declarations(root, source, &aliases);
    let mounts = receiver_mounts(root, source, &receivers);
    let django_import = aliases.values().any(|target| {
        target.starts_with("django.urls.") || target.starts_with("django.conf.urls.")
    });
    let evidence = EvidenceSet::new()
        .direct_if(django_import, "django", EvidenceKind::Import, "django.urls")
        .supporting_if(
            is_django_url_module(path, text),
            "django",
            EvidenceKind::Convention,
            "urls.py with urlpatterns",
        )
        .direct_if(
            receivers
                .values()
                .any(|receiver| receiver.framework == "flask"),
            "flask",
            EvidenceKind::Receiver,
            "flask application receiver",
        )
        .direct_if(
            receivers
                .values()
                .any(|receiver| receiver.framework == "fastapi"),
            "fastapi",
            EvidenceKind::Receiver,
            "fastapi application receiver",
        );
    let mut facts = Vec::new();
    facts.extend(receiver_mount_facts(
        root, source, path, &receivers, &aliases,
    ));
    if evidence.activates("django") {
        collect_django_routes(root, source, path, &aliases, &mut facts);
    }
    if evidence.activates("flask") || evidence.activates("fastapi") {
        collect_decorated_routes(
            root, source, path, &receivers, &mounts, &aliases, &mut facts,
        );
    }
    facts
}

fn collect_django_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    aliases: &HashMap<String, String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call" {
        let function = node
            .child_by_field_name("function")
            .map(|function| node_text(function, source))
            .unwrap_or_default();
        let terminal = function.rsplit('.').next().unwrap_or(function);
        if matches!(terminal, "path" | "re_path" | "url")
            && let Some(arguments) = call_arguments(node, source)
            && let Some(raw_path) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, "route"))
            && let Some(handler) =
                positional_argument(&arguments, 1).or_else(|| keyword_value(&arguments, "view"))
        {
            let mut detail = Map::new();
            detail.insert("django_function".into(), Value::String(terminal.to_owned()));
            let handler = handler.trim();
            let handler_reference = if terminal_call_name(handler) == Some("include") {
                let include = call_text_arguments(handler)
                    .first()
                    .map(String::as_str)
                    .and_then(string_literal)
                    .unwrap_or_else(|| {
                        call_text_arguments(handler)
                            .first()
                            .cloned()
                            .unwrap_or_default()
                    });
                detail.insert("include".into(), Value::String(include.clone()));
                format!("@include:{include}")
            } else {
                let handler = string_literal(handler).unwrap_or_else(|| handler.to_owned());
                expand_alias(&handler, aliases)
            };
            if !handler_reference.is_empty() {
                facts.push(RawFrameworkFact::Route(RawRouteFact {
                    framework: "django".to_owned(),
                    operation: "ANY".to_owned(),
                    raw_path: raw_path.clone(),
                    normalized_path: normalize_django_path(&raw_path, terminal),
                    declaring_scope: module_scope(path),
                    anchor: anchor(path, node),
                    handler_reference,
                    middleware_references: Vec::new(),
                    stages: Vec::new(),
                    origin: RawFrameworkOrigin::Config,
                    rule: None,
                    detail,
                }));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_django_routes(child, source, path, aliases, facts);
    }
}

fn collect_decorated_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashMap<String, Receiver>,
    mounts: &HashMap<String, Vec<String>>,
    aliases: &HashMap<String, String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "decorated_definition"
        && let Some(definition) = named_child(node, &["function_definition", "class_definition"])
        && let Some(name) = definition
            .child_by_field_name("name")
            .map(|name| node_text(name, source).to_owned())
    {
        let mut cursor = node.walk();
        for decorator in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let text = node_text(decorator, source).trim().trim_start_matches('@');
            let Some((callee, arguments)) = parse_call(text) else {
                continue;
            };
            let Some((receiver_name, method)) = callee.rsplit_once('.') else {
                continue;
            };
            let Some(receiver) = receivers.get(receiver_name) else {
                continue;
            };
            let path_keyword = if receiver.framework == "flask" {
                "rule"
            } else {
                "path"
            };
            let Some(raw_path) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, path_keyword))
            else {
                continue;
            };
            let operations = operations(receiver.framework, method, &arguments);
            if operations.is_empty() {
                continue;
            }
            let mount_prefixes = mounts
                .get(receiver_name)
                .cloned()
                .unwrap_or_else(|| vec![String::new()]);
            let middleware_references = if receiver.framework == "fastapi" {
                fastapi_dependencies(&arguments)
                    .into_iter()
                    .map(|reference| expand_alias(&reference, aliases))
                    .collect()
            } else {
                Vec::new()
            };
            for mount_prefix in mount_prefixes {
                let composed_prefix = join_route_paths(&mount_prefix, &receiver.prefix);
                let normalized_path = join_route_paths(&composed_prefix, &raw_path);
                for operation in &operations {
                    facts.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: receiver.framework.to_owned(),
                        operation: operation.clone(),
                        raw_path: raw_path.clone(),
                        normalized_path: normalized_path.clone(),
                        declaring_scope: module_scope(path),
                        anchor: anchor(path, decorator),
                        handler_reference: name.clone(),
                        middleware_references: middleware_references.clone(),
                        stages: Vec::new(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: (!mount_prefix.is_empty())
                            .then(|| "python-mounted-router".to_owned()),
                        detail: Map::from_iter([
                            ("receiver".into(), Value::String(receiver_name.to_owned())),
                            ("mount_prefix".into(), Value::String(mount_prefix.clone())),
                        ]),
                    }));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_decorated_routes(child, source, path, receivers, mounts, aliases, facts);
    }
}

fn receiver_mounts(
    root: Node<'_>,
    source: &[u8],
    receivers: &HashMap<String, Receiver>,
) -> HashMap<String, Vec<String>> {
    let mut mounts = HashMap::<String, Vec<String>>::new();
    collect_receiver_mounts(root, source, receivers, &mut mounts);
    for values in mounts.values_mut() {
        values.sort();
        values.dedup();
    }
    mounts
}

fn receiver_mount_facts(
    root: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashMap<String, Receiver>,
    aliases: &HashMap<String, String>,
) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    collect_receiver_mount_facts(root, source, path, receivers, aliases, &mut facts);
    facts
}

fn collect_receiver_mount_facts(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashMap<String, Receiver>,
    aliases: &HashMap<String, String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(arguments) = call_arguments(node, source)
    {
        let function_text = node_text(function, source);
        let method = function_text.rsplit('.').next().unwrap_or(function_text);
        let prefix_key = match method {
            "include_router" => Some("prefix"),
            "register_blueprint" => Some("url_prefix"),
            _ => None,
        };
        if let Some(prefix_key) = prefix_key
            && let Some(parent) = function_text.rsplit_once('.').map(|(parent, _)| parent)
            && let Some(parent_receiver) = receivers.get(parent)
            && let Some(target) = positional_argument(&arguments, 0)
            && is_identifier(target.trim())
            && let Some(prefix) = keyword_string(&arguments, prefix_key)
        {
            let expanded = expand_alias(target.trim(), aliases);
            let target_module = expanded
                .rsplit_once('.')
                .map(|(module, _)| module.to_owned())
                .unwrap_or_default();
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: parent_receiver.framework.to_owned(),
                kind: "router_mount".to_owned(),
                name: target.trim().to_owned(),
                declaring_scope: module_scope(path),
                anchor: anchor(path, node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "target_receiver".into(),
                        Value::String(target.trim().to_owned()),
                    ),
                    ("target_module".into(), Value::String(target_module)),
                    ("mount_prefix".into(), Value::String(prefix)),
                    ("parent_receiver".into(), Value::String(parent.to_owned())),
                ]),
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receiver_mount_facts(child, source, path, receivers, aliases, facts);
    }
}

fn collect_receiver_mounts(
    node: Node<'_>,
    source: &[u8],
    receivers: &HashMap<String, Receiver>,
    mounts: &mut HashMap<String, Vec<String>>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(arguments) = call_arguments(node, source)
    {
        let function = node_text(function, source);
        let method = function.rsplit('.').next().unwrap_or(function);
        let prefix_key = match method {
            "include_router" => Some("prefix"),
            "register_blueprint" => Some("url_prefix"),
            _ => None,
        };
        if let Some(prefix_key) = prefix_key
            && let Some(parent) = function.rsplit_once('.').map(|(parent, _)| parent)
            && receivers.contains_key(parent)
            && let Some(target) = positional_argument(&arguments, 0)
            && is_identifier(target.trim())
            && let Some(prefix) = keyword_string(&arguments, prefix_key)
        {
            mounts
                .entry(target.trim().to_owned())
                .or_default()
                .push(prefix);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receiver_mounts(child, source, receivers, mounts);
    }
}

fn receiver_declarations(
    root: Node<'_>,
    source: &[u8],
    aliases: &HashMap<String, String>,
) -> HashMap<String, Receiver> {
    let mut receivers = HashMap::new();
    collect_receivers(root, source, aliases, &mut receivers);
    receivers
}

fn import_aliases(root: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    collect_import_aliases(root, source, &mut aliases);
    aliases
}

fn collect_import_aliases(node: Node<'_>, source: &[u8], aliases: &mut HashMap<String, String>) {
    if node.kind() == "import_from_statement" {
        let module = node
            .child_by_field_name("module_name")
            .map(|module| node_text(module, source).trim_matches('.'))
            .unwrap_or_default();
        let mut past_import = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import" {
                past_import = true;
                continue;
            }
            if !past_import {
                continue;
            }
            let (imported, local) = if child.kind() == "aliased_import" {
                (
                    child
                        .child_by_field_name("name")
                        .map(|name| node_text(name, source)),
                    child
                        .child_by_field_name("alias")
                        .map(|name| node_text(name, source)),
                )
            } else if matches!(child.kind(), "identifier" | "dotted_name") {
                let name = node_text(child, source);
                (Some(name), Some(name))
            } else {
                (None, None)
            };
            if let (Some(imported), Some(local)) = (imported, local)
                && imported != "*"
            {
                let qualified = if module.is_empty() {
                    imported.to_owned()
                } else {
                    format!("{module}.{imported}")
                };
                aliases.insert(local.to_owned(), qualified);
            }
        }
    } else if node.kind() == "import_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "aliased_import"
                && let (Some(name), Some(alias)) = (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("alias"),
                )
            {
                aliases.insert(
                    node_text(alias, source).to_owned(),
                    node_text(name, source).to_owned(),
                );
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_import_aliases(child, source, aliases);
    }
}

fn expand_alias(reference: &str, aliases: &HashMap<String, String>) -> String {
    let split = reference.find('.').unwrap_or(reference.len());
    aliases.get(&reference[..split]).map_or_else(
        || reference.to_owned(),
        |expanded| format!("{expanded}{}", &reference[split..]),
    )
}

fn collect_receivers(
    node: Node<'_>,
    source: &[u8],
    aliases: &HashMap<String, String>,
    receivers: &mut HashMap<String, Receiver>,
) {
    if node.kind() == "assignment" {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        if let (Some(left), Some(right)) = (left, right) {
            let variable = node_text(left, source).trim();
            let expression = node_text(right, source).trim();
            if is_identifier(variable)
                && let Some((constructor, arguments)) = parse_call(expression)
            {
                let resolved = expand_alias(constructor, aliases);
                let terminal = resolved.rsplit('.').next().unwrap_or(&resolved);
                let framework = match resolved.as_str() {
                    "flask.Flask" | "flask.Blueprint" => Some("flask"),
                    "fastapi.FastAPI" | "fastapi.APIRouter" => Some("fastapi"),
                    _ => None,
                };
                if let Some(framework) = framework {
                    let prefix_key = if terminal == "Blueprint" {
                        "url_prefix"
                    } else {
                        "prefix"
                    };
                    receivers.insert(
                        variable.to_owned(),
                        Receiver {
                            framework,
                            prefix: keyword_string(&arguments, prefix_key).unwrap_or_default(),
                        },
                    );
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receivers(child, source, aliases, receivers);
    }
}

fn operations(framework: &str, method: &str, arguments: &[String]) -> Vec<String> {
    if framework == "fastapi" {
        let operation = match method {
            "get" | "post" | "put" | "patch" | "delete" | "options" | "head" | "trace" => {
                Some(method.to_ascii_uppercase())
            }
            "api_route" | "route" => None,
            _ => return Vec::new(),
        };
        return operation
            .map(|operation| vec![operation])
            .unwrap_or_else(|| keyword_string_list(arguments, "methods"));
    }
    if framework == "flask" && method == "route" {
        let methods = keyword_string_list(arguments, "methods");
        return if methods.is_empty() {
            vec!["ANY".to_owned()]
        } else {
            methods
        };
    }
    Vec::new()
}

fn fastapi_dependencies(arguments: &[String]) -> Vec<String> {
    let Some(raw) = keyword_value(arguments, "dependencies") else {
        return Vec::new();
    };
    let Ok(regex) = Regex::new(r"\bDepends\s*\(\s*([A-Za-z_][\w.]*)") else {
        return Vec::new();
    };
    regex
        .captures_iter(raw)
        .filter_map(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
        .collect()
}

fn call_arguments(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    let arguments = node.child_by_field_name("arguments")?;
    let text = node_text(arguments, source).trim();
    let text = text.strip_prefix('(')?.strip_suffix(')')?;
    Some(split_arguments(text))
}

fn parse_call(value: &str) -> Option<(&str, Vec<String>)> {
    let open = value.find('(')?;
    let close = matching_close(value, open)?;
    if !value[close + 1..].trim().is_empty() {
        return None;
    }
    let callee = value[..open].trim();
    is_dotted_identifier(callee).then(|| (callee, split_arguments(&value[open + 1..close])))
}

fn call_text_arguments(value: &str) -> Vec<String> {
    parse_call(value)
        .map(|(_, arguments)| arguments)
        .unwrap_or_default()
}

fn terminal_call_name(value: &str) -> Option<&str> {
    parse_call(value).map(|(callee, _)| callee.rsplit('.').next().unwrap_or(callee))
}

fn split_arguments(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                values.push(value[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < value.len() || !value.trim().is_empty() {
        values.push(value[start..].trim().to_owned());
    }
    values
}

fn matching_close(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (relative, character) in value[open..].char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn string_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let (prefix, quoted) = value
        .find(['\'', '"'])
        .map(|index| (&value[..index], &value[index..]))?;
    if !prefix
        .chars()
        .all(|character| matches!(character, 'r' | 'R' | 'u' | 'U'))
    {
        return None;
    }
    let delimiter = quoted.chars().next()?;
    if quoted.len() < 2 || !quoted.ends_with(delimiter) {
        return None;
    }
    let content = &quoted[delimiter.len_utf8()..quoted.len() - delimiter.len_utf8()];
    (!content.contains(['\n', '\r'])).then(|| content.to_owned())
}

fn keyword_value<'a>(arguments: &'a [String], key: &str) -> Option<&'a str> {
    arguments.iter().find_map(|argument| {
        let (name, value) = split_keyword_argument(argument)?;
        (name.trim() == key).then(|| value.trim())
    })
}

fn positional_argument(arguments: &[String], index: usize) -> Option<&str> {
    arguments
        .iter()
        .filter(|argument| split_keyword_argument(argument).is_none())
        .nth(index)
        .map(String::as_str)
}

fn split_keyword_argument(argument: &str) -> Option<(&str, &str)> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in argument.char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => return Some((&argument[..index], &argument[index + 1..])),
            _ => {}
        }
    }
    None
}

fn keyword_string(arguments: &[String], key: &str) -> Option<String> {
    keyword_value(arguments, key).and_then(string_literal)
}

fn keyword_string_list(arguments: &[String], key: &str) -> Vec<String> {
    let Some(value) = keyword_value(arguments, key) else {
        return Vec::new();
    };
    let value = value.trim();
    if !(value.starts_with('[') || value.starts_with('(')) {
        return Vec::new();
    }
    split_arguments(
        value
            .trim_start_matches(['[', '('])
            .trim_end_matches([']', ')']),
    )
    .into_iter()
    .filter_map(|value| string_literal(&value))
    .map(|value| value.to_ascii_uppercase())
    .collect()
}

fn normalize_django_path(path: &str, function: &str) -> String {
    let mut normalized = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    if function == "path"
        && let Ok(parameter) = Regex::new(r"<(?:(?:[^:>]+):)?([^>]+)>")
    {
        normalized = parameter.replace_all(&normalized, "{$1}").into_owned();
    }
    if normalized.len() > 1 {
        normalized = normalized.trim_end_matches('/').to_owned();
    }
    normalized
}

fn join_route_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let joined = if prefix.is_empty() {
        format!("/{path}")
    } else if path.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{path}")
    };
    if joined.is_empty() {
        "/".to_owned()
    } else if joined.starts_with('/') {
        joined
    } else {
        format!("/{joined}")
    }
}

fn is_django_url_module(path: &Path, source: &str) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("urls.py")
        && source.contains("urlpatterns")
}

fn module_scope(path: &Path) -> String {
    path.with_extension("")
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn anchor(path: &Path, node: Node<'_>) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().into_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX),
        start_column: u32::try_from(node.start_position().column).unwrap_or(u32::MAX),
        end_line: u32::try_from(node.end_position().row + 1).unwrap_or(u32::MAX),
        end_column: u32::try_from(node.end_position().column).unwrap_or(u32::MAX),
    }
}

fn named_child<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| kinds.contains(&child.kind()))
}

fn node_text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn is_dotted_identifier(value: &str) -> bool {
    !value.is_empty() && value.split('.').all(is_identifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_parser_preserves_nested_calls_and_collections() {
        assert_eq!(
            split_arguments(
                r#""/x", dependencies=[Depends(auth), Depends(scope)], methods=["GET", "POST"]"#
            ),
            vec![
                r#""/x""#,
                "dependencies=[Depends(auth), Depends(scope)]",
                r#"methods=["GET", "POST"]"#,
            ]
        );
    }

    #[test]
    fn path_normalization_preserves_framework_semantics() {
        assert_eq!(
            normalize_django_path("users/<int:id>/", "path"),
            "/users/{id}"
        );
        assert_eq!(join_route_paths("/api", "/users"), "/api/users");
    }
}
