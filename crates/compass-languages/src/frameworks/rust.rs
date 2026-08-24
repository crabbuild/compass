use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, literal, normalize_route_path, text};
use super::{RawDomainFact, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect_axum(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    detect_selected(path, source, root, Some("axum"))
}

pub(super) fn detect_non_axum(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    detect_selected(path, source, root, Some("non-axum"))
}

fn detect_selected(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    selected: Option<&str>,
) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let evidence = EvidenceSet::new()
        .direct_if(
            body.contains("axum::") || body.contains("use axum"),
            "axum",
            EvidenceKind::Import,
            "axum",
        )
        .direct_if(
            body.contains("actix_web"),
            "actix",
            EvidenceKind::Import,
            "actix_web",
        )
        .direct_if(
            body.contains("rocket::"),
            "rocket",
            EvidenceKind::Import,
            "rocket",
        )
        .direct_if(
            body.contains("#[rocket::"),
            "rocket",
            EvidenceKind::Macro,
            "rocket route attribute",
        );
    let axum = selected.is_none_or(|framework| framework == "axum") && evidence.activates("axum");
    let actix = selected.is_none_or(|framework| framework == "actix" || framework == "non-axum")
        && evidence.activates("actix");
    let rocket = selected.is_none_or(|framework| framework == "rocket" || framework == "non-axum")
        && evidence.activates("rocket");
    if !axum && !actix && !rocket {
        return Vec::new();
    }
    let Ok(attribute) = Regex::new(
        r#"(?s)#\[(?:(rocket|actix_web)::)?(get|post|put|patch|delete|head)\(\s*["']([^"']+)["'][^\]]*\)\]"#,
    ) else {
        return Vec::new();
    };
    let Ok(function) = Regex::new(r"\b(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)") else {
        return Vec::new();
    };

    let framework = if axum {
        "axum"
    } else if actix {
        "actix"
    } else {
        "rocket"
    };
    let mut facts = Vec::new();
    collect_rust_calls(root, source, path, framework, "", &mut facts);
    if axum {
        collect_axum_router_composition(root, source, path, &mut facts);
    }
    let masked_body = masked_rust_source(root, source);

    let functions = function
        .captures_iter(&masked_body)
        .filter_map(|capture| {
            let whole = capture.get(0)?;
            let handler = capture.get(1)?;
            Some((whole.start(), handler.as_str().to_owned()))
        })
        .collect::<Vec<_>>();
    for capture in attribute.captures_iter(&masked_body) {
        let Some(whole) = capture.get(0) else {
            continue;
        };
        let (Some(operation), Some(raw_path)) = (capture.get(2), capture.get(3)) else {
            continue;
        };
        let Some((_, handler)) = functions.iter().find(|(start, _)| *start >= whole.end()) else {
            continue;
        };
        let framework = match capture.get(1).map(|value| value.as_str()) {
            Some("rocket") => "rocket",
            Some("actix_web") => "actix",
            _ if actix => "actix",
            _ => "rocket",
        };
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: framework.to_owned(),
            operation: operation.as_str().to_ascii_uppercase(),
            raw_path: raw_path.as_str().to_owned(),
            normalized_path: normalize_route_path(raw_path.as_str()),
            declaring_scope: path.to_string_lossy().into_owned(),
            anchor: anchor(path, source, whole.start(), whole.end()),
            handler_reference: handler.clone(),
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: Some("rust-route-attribute".to_owned()),
            detail: Map::new(),
        }));
    }
    if let Some(selected) = selected {
        facts.retain(|fact| {
            let framework = match fact {
                RawFrameworkFact::Route(route) => route.framework.as_str(),
                RawFrameworkFact::Domain(domain) => domain.framework.as_str(),
                RawFrameworkFact::Annotation(annotation) => annotation.framework.as_str(),
            };
            if selected == "non-axum" {
                framework != "axum"
            } else {
                framework == selected
            }
        });
    }
    facts
}

fn collect_axum_router_composition(
    root: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let mut functions = Vec::new();
    collect_nodes_of_kind(root, "function_item", &mut functions);
    let local_functions = functions
        .iter()
        .filter_map(|function| function.child_by_field_name("name"))
        .map(|name| node_text(name, source).to_owned())
        .filter(|name| !name.is_empty())
        .collect::<BTreeSet<_>>();
    let import_aliases = rust_import_aliases(root, source);
    let mut owners = BTreeMap::<(u64, u64), String>::new();
    let mut mounts = Vec::new();
    for function in functions {
        let Some(name) = function.child_by_field_name("name") else {
            continue;
        };
        let Some(body) = function.child_by_field_name("body") else {
            continue;
        };
        let function_name = node_text(name, source);
        if function_name.is_empty() {
            continue;
        }
        let mut bindings = BTreeMap::new();
        let mut local_routers = BTreeSet::new();
        collect_router_bindings(
            body,
            source,
            function_name,
            &mut bindings,
            &mut local_routers,
        );
        collect_router_calls(
            body,
            source,
            path,
            function_name,
            &bindings,
            &local_routers,
            &local_functions,
            &import_aliases,
            &mut owners,
            &mut mounts,
        );
        if let Some(target) =
            returned_router_target(body, source, function_name, &bindings, &local_routers)
        {
            mounts.push(router_mount_fact(
                path,
                source,
                body,
                &function_router_owner(function_name),
                target,
                "",
                "alias",
            ));
        }
    }
    for fact in facts.iter_mut() {
        let RawFrameworkFact::Route(route) = fact else {
            continue;
        };
        if route.framework != "axum" {
            continue;
        }
        if let Some(owner) = owners.get(&(route.anchor.start_byte, route.anchor.end_byte)) {
            route.detail.insert(
                "router_owner".to_owned(),
                serde_json::Value::String(owner.clone()),
            );
        }
    }
    facts.extend(mounts);
}

fn collect_nodes_of_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_nodes_of_kind(child, kind, output);
    }
}

fn collect_router_bindings(
    node: Node<'_>,
    source: &[u8],
    function_name: &str,
    references: &mut BTreeMap<String, String>,
    local_routers: &mut BTreeSet<String>,
) {
    if node.kind() == "let_declaration"
        && let (Some(pattern), Some(value)) = (
            node.child_by_field_name("pattern"),
            node.child_by_field_name("value"),
        )
        && let Some(binding) = rust_binding_name(pattern, source)
    {
        if let Some(reference) = router_factory_reference(value, source) {
            references.insert(binding.to_owned(), reference);
        } else if contains_router_constructor(value, source) {
            local_routers.insert(local_router_owner(function_name, binding));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_router_bindings(child, source, function_name, references, local_routers);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_router_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    function_name: &str,
    references: &BTreeMap<String, String>,
    local_routers: &BTreeSet<String>,
    local_functions: &BTreeSet<String>,
    import_aliases: &BTreeMap<String, String>,
    owners: &mut BTreeMap<(u64, u64), String>,
    mounts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(method) = rust_call_method(function, source)
    {
        let owner = router_call_owner(node, source, function_name);
        if method == "route" {
            owners.insert(
                (node.start_byte() as u64, node.end_byte() as u64),
                owner.clone(),
            );
        }
        if matches!(method.as_str(), "nest" | "merge") {
            let arguments = node
                .child_by_field_name("arguments")
                .map(named_children)
                .unwrap_or_default();
            let (prefix, target) = if method == "nest" {
                (
                    arguments
                        .first()
                        .and_then(|argument| literal(node_text(*argument, source)))
                        .unwrap_or_default(),
                    arguments.get(1).copied(),
                )
            } else {
                (String::new(), arguments.first().copied())
            };
            if let Some(target) = target.and_then(|target| {
                router_target(target, source, function_name, references, local_routers)
            }) {
                mounts.push(router_mount_fact(
                    path, source, node, &owner, target, &prefix, &method,
                ));
            }
        }
        if matches!(method.as_str(), "layer" | "route_layer")
            && let Some(middleware) = node
                .child_by_field_name("arguments")
                .and_then(|arguments| named_children(arguments).first().copied())
                .and_then(|argument| {
                    axum_middleware_reference(
                        argument,
                        source,
                        path,
                        local_functions,
                        import_aliases,
                    )
                })
        {
            mounts.push(router_middleware_fact(
                path,
                source,
                node,
                &owner,
                &middleware,
                &method,
            ));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_router_calls(
            child,
            source,
            path,
            function_name,
            references,
            local_routers,
            local_functions,
            import_aliases,
            owners,
            mounts,
        );
    }
}

fn router_call_owner(node: Node<'_>, source: &[u8], function_name: &str) -> String {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "let_declaration"
            && let Some(pattern) = parent.child_by_field_name("pattern")
            && let Some(binding) = rust_binding_name(pattern, source)
        {
            return local_router_owner(function_name, binding);
        }
        if parent.kind() == "assignment_expression"
            && let Some(left) = parent.child_by_field_name("left")
            && let Some(binding) = rust_binding_name(left, source)
        {
            return local_router_owner(function_name, binding);
        }
        if parent.kind() == "function_item" {
            break;
        }
        current = parent;
    }
    function_router_owner(function_name)
}

fn returned_router_target(
    body: Node<'_>,
    source: &[u8],
    function_name: &str,
    references: &BTreeMap<String, String>,
    local_routers: &BTreeSet<String>,
) -> Option<RouterTarget> {
    let tail = named_children(body).last().copied()?;
    if tail.kind() == "let_declaration" || tail.kind() == "assignment_expression" {
        return None;
    }
    if let Some(reference) = rooted_router_factory_reference(tail, source) {
        return Some(RouterTarget::Reference(reference));
    }
    let binding = root_router_binding(tail, source)?;
    let local = local_router_owner(function_name, &binding);
    if local_routers.contains(&local) {
        return Some(RouterTarget::Owner(local));
    }
    references
        .get(&binding)
        .cloned()
        .map(RouterTarget::Reference)
}

fn rooted_router_factory_reference(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(reference) = router_factory_reference(node, source) {
        return Some(reference);
    }
    if node.kind() != "call_expression" {
        return None;
    }
    node.child_by_field_name("function")
        .and_then(|function| function.child_by_field_name("value"))
        .and_then(|value| rooted_router_factory_reference(value, source))
}

fn root_router_binding(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(node_text(node, source).to_owned());
    }
    if node.kind() == "call_expression" {
        return node
            .child_by_field_name("function")
            .and_then(|function| root_router_binding(function, source));
    }
    if node.kind() == "field_expression" {
        return node
            .child_by_field_name("value")
            .and_then(|value| root_router_binding(value, source));
    }
    named_children(node)
        .first()
        .copied()
        .and_then(|child| root_router_binding(child, source))
}

fn router_target(
    node: Node<'_>,
    source: &[u8],
    function_name: &str,
    references: &BTreeMap<String, String>,
    local_routers: &BTreeSet<String>,
) -> Option<RouterTarget> {
    if node.kind() == "identifier" {
        let binding = node_text(node, source);
        if let Some(reference) = references.get(binding) {
            return Some(RouterTarget::Reference(reference.clone()));
        }
        let owner = local_router_owner(function_name, binding);
        return local_routers
            .contains(&owner)
            .then_some(RouterTarget::Owner(owner));
    }
    router_factory_reference(node, source).map(RouterTarget::Reference)
}

fn router_factory_reference(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if !matches!(function.kind(), "identifier" | "scoped_identifier") {
        return None;
    }
    let reference = node_text(function, source).trim();
    let terminal = reference.rsplit("::").next().unwrap_or_default();
    (!matches!(terminal, "new" | "default") && !reference.is_empty()).then(|| reference.to_owned())
}

fn contains_router_constructor(node: Node<'_>, source: &[u8]) -> bool {
    node_text(node, source).contains("Router::new")
}

fn rust_binding_name<'source>(node: Node<'_>, source: &'source [u8]) -> Option<&'source str> {
    if node.kind() == "identifier" {
        let value = node_text(node, source);
        return (!value.is_empty()).then_some(value);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| rust_binding_name(child, source))
}

fn function_router_owner(function_name: &str) -> String {
    format!("fn:{function_name}")
}

fn local_router_owner(function_name: &str, binding: &str) -> String {
    format!("fn:{function_name}#local:{binding}")
}

#[derive(Clone)]
enum RouterTarget {
    Owner(String),
    Reference(String),
}

fn router_mount_fact(
    path: &Path,
    source: &[u8],
    node: Node<'_>,
    parent: &str,
    target: RouterTarget,
    prefix: &str,
    operation: &str,
) -> RawFrameworkFact {
    let mut detail = Map::new();
    detail.insert(
        "parent_router".to_owned(),
        serde_json::Value::String(parent.to_owned()),
    );
    match target {
        RouterTarget::Owner(owner) => {
            detail.insert("target_router".to_owned(), serde_json::Value::String(owner));
        }
        RouterTarget::Reference(reference) => {
            detail.insert(
                "target_reference".to_owned(),
                serde_json::Value::String(reference),
            );
        }
    }
    detail.insert(
        "mount_prefix".to_owned(),
        serde_json::Value::String(prefix.to_owned()),
    );
    detail.insert(
        "mount_operation".to_owned(),
        serde_json::Value::String(operation.to_owned()),
    );
    RawFrameworkFact::Domain(RawDomainFact {
        framework: "axum".to_owned(),
        kind: "router_mount".to_owned(),
        name: parent.to_owned(),
        declaring_scope: path.to_string_lossy().replace('\\', "/"),
        anchor: anchor(path, source, node.start_byte(), node.end_byte()),
        origin: RawFrameworkOrigin::Ast,
        detail,
    })
}

fn axum_middleware_reference(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    local_functions: &BTreeSet<String>,
    import_aliases: &BTreeMap<String, String>,
) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    let method = rust_call_method(function, source)?;
    if !matches!(method.as_str(), "from_fn" | "from_fn_with_state") {
        return None;
    }
    let handler = node
        .child_by_field_name("arguments")
        .and_then(|arguments| named_children(arguments).last().copied())
        .map(|handler| clean_rust_handler(node_text(handler, source)))?;
    if handler.is_empty() {
        return None;
    }
    if handler.contains('.') || !local_functions.contains(&handler) {
        return Some(import_aliases.get(&handler).cloned().unwrap_or(handler));
    }
    rust_module_name(path).map_or(Some(handler.clone()), |module| {
        Some(format!("{module}.{handler}"))
    })
}

fn rust_import_aliases(root: Node<'_>, source: &[u8]) -> BTreeMap<String, String> {
    let mut declarations = Vec::new();
    collect_nodes_of_kind(root, "use_declaration", &mut declarations);
    let mut aliases = BTreeMap::new();
    for declaration in declarations {
        let value = node_text(declaration, source)
            .trim()
            .trim_start_matches("use")
            .trim()
            .trim_end_matches(';')
            .trim();
        collect_rust_imports(value, "", &mut aliases);
    }
    aliases
}

fn collect_rust_imports(value: &str, prefix: &str, aliases: &mut BTreeMap<String, String>) {
    let value = value.trim();
    if value.is_empty() || value == "*" {
        return;
    }
    if let Some(open) = value.find('{')
        && value.ends_with('}')
    {
        let head = value[..open].trim().trim_end_matches("::");
        let next_prefix = join_rust_import(prefix, head);
        for item in split_top_level(&value[open + 1..value.len() - 1]) {
            collect_rust_imports(item, &next_prefix, aliases);
        }
        return;
    }
    let (path, alias) = value
        .rsplit_once(" as ")
        .map_or((value, None), |(path, alias)| {
            (path.trim(), Some(alias.trim()))
        });
    if path == "self" {
        if let Some(name) = prefix.rsplit("::").next().filter(|name| !name.is_empty()) {
            aliases.insert(
                alias.unwrap_or(name).to_owned(),
                rust_import_reference(prefix),
            );
        }
        return;
    }
    let qualified = join_rust_import(prefix, path);
    let Some(name) = alias.or_else(|| path.rsplit("::").next()) else {
        return;
    };
    let reference = rust_import_reference(&qualified);
    if !name.is_empty() && !reference.is_empty() {
        aliases.insert(name.to_owned(), reference);
    }
}

fn split_top_level(value: &str) -> Vec<&str> {
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut output = Vec::new();
    for (index, character) in value.char_indices() {
        match character {
            '{' => depth = depth.saturating_add(1),
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                output.push(value[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    output.push(value[start..].trim());
    output
}

fn join_rust_import(prefix: &str, value: &str) -> String {
    match (prefix.is_empty(), value.is_empty()) {
        (true, _) => value.to_owned(),
        (_, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}::{value}"),
    }
}

fn rust_import_reference(value: &str) -> String {
    value
        .split("::")
        .filter(|part| !matches!(*part, "crate" | "self" | "super") && !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn rust_module_name(path: &Path) -> Option<String> {
    if path.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        path.parent()?
            .file_name()?
            .to_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    } else {
        path.file_stem()?
            .to_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

fn router_middleware_fact(
    path: &Path,
    source: &[u8],
    node: Node<'_>,
    owner: &str,
    reference: &str,
    operation: &str,
) -> RawFrameworkFact {
    let mut detail = Map::new();
    detail.insert(
        "parent_router".to_owned(),
        serde_json::Value::String(owner.to_owned()),
    );
    detail.insert(
        "middleware_reference".to_owned(),
        serde_json::Value::String(reference.to_owned()),
    );
    detail.insert(
        "middleware_operation".to_owned(),
        serde_json::Value::String(operation.to_owned()),
    );
    RawFrameworkFact::Domain(RawDomainFact {
        framework: "axum".to_owned(),
        kind: "router_middleware".to_owned(),
        name: reference.to_owned(),
        declaring_scope: path.to_string_lossy().replace('\\', "/"),
        anchor: anchor(path, source, node.start_byte(), node.end_byte()),
        origin: RawFrameworkOrigin::Ast,
        detail,
    })
}

fn masked_rust_source(root: Node<'_>, source: &[u8]) -> String {
    let mut masked = source.to_vec();
    mask_rust_comments(root, &mut masked);
    String::from_utf8(masked).unwrap_or_default()
}

fn mask_rust_comments(node: Node<'_>, source: &mut [u8]) {
    if matches!(node.kind(), "line_comment" | "block_comment") {
        let start = node.start_byte().min(source.len());
        let end = node.end_byte().min(source.len());
        for byte in &mut source[start..end] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        mask_rust_comments(child, source);
    }
}

fn collect_rust_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    default_framework: &str,
    prefix: &str,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        let method = rust_call_method(function, source);
        let arguments = node.child_by_field_name("arguments");
        let argument_values = arguments
            .map(|arguments| named_children(arguments))
            .unwrap_or_default();
        // Axum's `nest` mounts its child argument; it does not turn later
        // methods on the parent builder into children of that prefix. Actix
        // `scope`, by contrast, is a receiver whose following routes inherit
        // its path.
        let receiver_prefix = if default_framework == "axum" {
            String::new()
        } else {
            function
                .child_by_field_name("value")
                .map(|value| rust_chain_prefix(value, source))
                .unwrap_or_default()
        };
        let mut child_prefix = append_prefix(prefix, &receiver_prefix);
        if matches!(method.as_deref(), Some("nest" | "scope"))
            && let Some(route_prefix) = argument_values
                .first()
                .and_then(|argument| literal(node_text(*argument, source)))
        {
            child_prefix = join_route_path(prefix, &route_prefix);
        }
        if method.as_deref() == Some("route") {
            let (raw_path, method_expression) = if argument_values
                .first()
                .and_then(|argument| literal(node_text(*argument, source)))
                .is_some()
            {
                (
                    argument_values
                        .first()
                        .and_then(|argument| literal(node_text(*argument, source))),
                    argument_values.get(1).copied(),
                )
            } else {
                (
                    function
                        .child_by_field_name("value")
                        .and_then(|value| rust_resource_path(value, source)),
                    argument_values.first().copied(),
                )
            };
            if let (Some(raw_path), Some(method_expression)) = (raw_path, method_expression) {
                let framework =
                    route_framework(method_expression, function, source, default_framework);
                for (operation, handler) in rust_method_handlers(method_expression, source) {
                    if handler.is_empty() {
                        continue;
                    }
                    let mut fact = route_fact(
                        path,
                        source,
                        framework,
                        &operation,
                        &raw_path,
                        &handler,
                        node.start_byte(),
                        node.end_byte(),
                        "rust-router-call",
                    );
                    if !child_prefix.is_empty()
                        && let RawFrameworkFact::Route(route) = &mut fact
                    {
                        route.normalized_path =
                            join_route_path(&child_prefix, &route.normalized_path);
                    }
                    facts.push(fact);
                }
            }
        }
        let mut cursor = node.walk();
        let function_id = function.id();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            let next_prefix = if function_id == child.id() {
                prefix
            } else {
                &child_prefix
            };
            collect_rust_calls(child, source, path, default_framework, next_prefix, facts);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_rust_calls(child, source, path, default_framework, prefix, facts);
    }
}

fn rust_call_method(function: Node<'_>, source: &[u8]) -> Option<String> {
    if function.kind() == "field_expression" {
        return function
            .child_by_field_name("field")
            .map(|field| node_text(field, source).to_owned());
    }
    if matches!(function.kind(), "scoped_identifier" | "identifier") {
        return Some(
            node_text(function, source)
                .rsplit("::")
                .next()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    None
}

fn append_prefix(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base.to_owned();
    }
    if base.is_empty() || base == suffix {
        return suffix.to_owned();
    }
    let suffix = suffix.trim_matches('/');
    if base.ends_with(&format!("/{suffix}")) {
        base.to_owned()
    } else {
        join_route_path(base, suffix)
    }
}

fn rust_chain_prefix(node: Node<'_>, source: &[u8]) -> String {
    if node.kind() != "call_expression" {
        return node
            .child_by_field_name("value")
            .map(|value| rust_chain_prefix(value, source))
            .unwrap_or_default();
    }
    let Some(function) = node.child_by_field_name("function") else {
        return String::new();
    };
    let base = function
        .child_by_field_name("value")
        .map(|value| rust_chain_prefix(value, source))
        .unwrap_or_default();
    let Some(method) = rust_call_method(function, source) else {
        return base;
    };
    if matches!(method.as_str(), "nest" | "scope")
        && let Some(prefix) = node
            .child_by_field_name("arguments")
            .and_then(|arguments| named_children(arguments).first().copied())
            .and_then(|argument| literal(node_text(argument, source)))
    {
        return append_prefix(&base, &prefix);
    }
    base
}

fn rust_resource_path(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if rust_call_method(function, source).as_deref() != Some("resource") {
        return function
            .child_by_field_name("value")
            .and_then(|value| rust_resource_path(value, source));
    }
    node.child_by_field_name("arguments")
        .and_then(|arguments| named_children(arguments).first().copied())
        .and_then(|argument| literal(node_text(argument, source)))
}

fn rust_method_handlers(node: Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    if node.kind() != "call_expression" {
        return Vec::new();
    }
    let Some(function) = node.child_by_field_name("function") else {
        return Vec::new();
    };
    let Some(method) = rust_call_method(function, source) else {
        return Vec::new();
    };
    let arguments = node
        .child_by_field_name("arguments")
        .map(|arguments| named_children(arguments))
        .unwrap_or_default();
    let value = function.child_by_field_name("value");
    if is_http_method(&method) {
        let mut output = value
            .map(|value| rust_method_handlers(value, source))
            .unwrap_or_default();
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        output.push((method.to_ascii_uppercase(), handler));
        return output;
    }
    if method == "to" {
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        return value
            .map(|value| {
                rust_method_handlers(value, source)
                    .into_iter()
                    .map(|(operation, _)| (operation, handler.clone()))
                    .collect()
            })
            .unwrap_or_default();
    }
    if matches!(function.kind(), "scoped_identifier" | "identifier") && is_http_method(&method) {
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        return vec![(method.to_ascii_uppercase(), handler)];
    }
    value
        .map(|value| rust_method_handlers(value, source))
        .unwrap_or_default()
}

fn is_http_method(method: &str) -> bool {
    matches!(
        method.to_ascii_lowercase().as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace" | "connect"
    )
}

fn clean_rust_handler(value: &str) -> String {
    value.trim().trim_start_matches('&').replace("::", ".")
}

fn route_framework<'framework>(
    method_expression: Node<'_>,
    function: Node<'_>,
    source: &[u8],
    default_framework: &'framework str,
) -> &'framework str {
    let method_text = node_text(method_expression, source);
    let function_text = node_text(function, source);
    if method_text.contains("web::")
        || function_text.contains("web::")
        || function_text.contains("App")
        || function_text.contains("scope")
        || function_text.contains("resource")
    {
        "actix"
    } else {
        default_framework
    }
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn node_text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    std::str::from_utf8(
        &source[node.start_byte().min(source.len())..node.end_byte().min(source.len())],
    )
    .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn route_fact(
    path: &Path,
    source: &[u8],
    framework: &str,
    operation: &str,
    raw_path: &str,
    handler: &str,
    start: usize,
    end: usize,
    rule: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_ascii_uppercase(),
        raw_path: raw_path.to_owned(),
        normalized_path: normalize_route_path(raw_path),
        declaring_scope: path.to_string_lossy().into_owned(),
        anchor: super::text::anchor(path, source, start, end),
        handler_reference: handler.replace("::", "."),
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: Some(rule.to_owned()),
        detail: Map::new(),
    })
}
