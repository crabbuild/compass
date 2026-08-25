use std::collections::BTreeSet;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
    UniversalDetectionContext,
};
use crate::{
    BindingFact, BindingKind, CandidateRelation, DeclarationFact, SemanticEvidenceBatch,
    SemanticRole,
};

#[derive(Clone, Debug)]
struct Receiver {
    declaration_id: String,
    qualified_name: String,
    name: String,
    framework: &'static str,
    prefix: String,
    start_byte: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PythonFramework {
    Django,
    FastApi,
    Flask,
}

pub(super) fn detect_django(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    detect_universal(context, PythonFramework::Django)
}

pub(super) fn detect_fastapi(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    detect_universal(context, PythonFramework::FastApi)
}

pub(super) fn detect_flask(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    detect_universal(context, PythonFramework::Flask)
}

fn detect_universal(
    context: &UniversalDetectionContext<'_, '_>,
    framework: PythonFramework,
) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    if framework == PythonFramework::Django {
        collect_django_routes(
            context.root,
            context.source,
            context.path,
            context.evidence,
            &mut facts,
        );
    } else {
        let receivers = receiver_declarations(context);
        collect_receiver_mount_facts(
            context.root,
            context.source,
            context.path,
            context.evidence,
            &receivers,
            framework,
            &mut facts,
        );
        let local_mounts = facts
            .iter()
            .filter_map(|fact| match fact {
                RawFrameworkFact::Domain(domain) if domain.kind == "router_mount" => {
                    Some(domain.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        collect_decorated_routes(
            context.root,
            context.source,
            context.path,
            context.evidence,
            &receivers,
            &local_mounts,
            framework,
            &mut facts,
        );
    }
    facts
}

fn collect_django_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && contributes_to_urlpatterns(node, source)
        && let Some(function) = node.child_by_field_name("function")
        && let Some(target) = exact_call_target(evidence, function)
    {
        let terminal = target.rsplit('.').next().unwrap_or(target.as_str());
        if matches!(
            target.as_str(),
            "django.urls.path"
                | "django.urls.re_path"
                | "django.conf.urls.url"
                | "django.conf.urls.path"
                | "django.conf.urls.re_path"
        ) && let Some(arguments) = call_arguments(node, source)
            && let Some(raw_path) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, "route"))
            && let Some(handler) =
                positional_argument(&arguments, 1).or_else(|| keyword_value(&arguments, "view"))
        {
            let mut detail = Map::new();
            detail.insert("django_function".into(), Value::String(terminal.to_owned()));
            let handler = handler.trim();
            let handler_node = call_argument_node(node, source, 1, "view");
            let include_call = handler_node.and_then(|handler_node| {
                find_exact_call(handler_node, evidence, "django.urls.include")
                    .or_else(|| find_exact_call(handler_node, evidence, "django.conf.urls.include"))
            });
            let handler_reference = if let Some(include_call) = include_call {
                let include_arguments = call_arguments(include_call, source).unwrap_or_default();
                let include = call_text_arguments(handler)
                    .first()
                    .map(String::as_str)
                    .and_then(string_literal)
                    .unwrap_or_else(|| {
                        positional_argument(&include_arguments, 0)
                            .map(str::to_owned)
                            .unwrap_or_default()
                    });
                detail.insert("include".into(), Value::String(include.clone()));
                format!("@include:{include}")
            } else {
                string_literal(handler).unwrap_or_else(|| handler.to_owned())
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
                    origin: RawFrameworkOrigin::Ast,
                    rule: None,
                    detail,
                }));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_django_routes(child, source, path, evidence, facts);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_decorated_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    receivers: &[Receiver],
    local_mounts: &[RawDomainFact],
    framework: PythonFramework,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "decorated_definition" {
        let mut cursor = node.walk();
        for decorator in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let Some(decorator_occurrence) =
                exact_occurrence(evidence, SemanticRole::Decorator, decorator)
            else {
                continue;
            };
            let text = node_text(decorator, source).trim().trim_start_matches('@');
            let Some((callee, arguments)) = parse_call(text) else {
                continue;
            };
            let Some((receiver_name, method)) = callee.rsplit_once('.') else {
                continue;
            };
            let Some(receiver) = receiver_at(
                receivers,
                evidence,
                receiver_name,
                decorator.start_byte() as u64,
            ) else {
                continue;
            };
            if framework_name(framework) != receiver.framework {
                continue;
            }
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
            let middleware_references = if receiver.framework == "fastapi" {
                fastapi_dependencies(decorator, source, evidence)
            } else {
                Vec::new()
            };
            let Some(handler) = evidence
                .declarations
                .iter()
                .find(|declaration| declaration.id == decorator_occurrence.owner_declaration_id)
            else {
                continue;
            };
            let applicable_mounts = local_mounts
                .iter()
                .filter(|mount| mount.framework == receiver.framework)
                .filter(|mount| {
                    mount
                        .detail
                        .get("target_receiver_id")
                        .and_then(Value::as_str)
                        == Some(receiver.declaration_id.as_str())
                })
                .collect::<Vec<_>>();
            let route_mounts = if applicable_mounts.is_empty() {
                vec![None]
            } else {
                applicable_mounts.into_iter().map(Some).collect()
            };
            for mount in route_mounts {
                let mount_prefix = mount
                    .and_then(|mount| mount.detail.get("mount_prefix"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let composed_prefix = join_route_paths(mount_prefix, &receiver.prefix);
                let normalized_path = join_route_paths(&composed_prefix, &raw_path);
                for operation in &operations {
                    let mut detail = Map::from_iter([
                        ("receiver".into(), Value::String(receiver.name.clone())),
                        (
                            "receiver_id".into(),
                            Value::String(receiver.declaration_id.clone()),
                        ),
                        (
                            "receiver_qualified_name".into(),
                            Value::String(receiver.qualified_name.clone()),
                        ),
                        (
                            "mount_prefix".into(),
                            Value::String(mount_prefix.to_owned()),
                        ),
                    ]);
                    if let Some(mount) = mount {
                        detail.insert(
                            "mounted_receiver_id".into(),
                            mount
                                .detail
                                .get("parent_receiver_id")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        detail.insert(
                            "mounted_receiver_qualified_name".into(),
                            mount
                                .detail
                                .get("parent_receiver_qualified_name")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        let mount_anchor =
                            serde_json::to_value(&mount.anchor).unwrap_or(Value::Null);
                        detail.insert("mount_anchor".into(), mount_anchor.clone());
                        detail.insert("mount_anchors".into(), Value::Array(vec![mount_anchor]));
                    }
                    facts.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: receiver.framework.to_owned(),
                        operation: operation.clone(),
                        raw_path: raw_path.clone(),
                        normalized_path: normalized_path.clone(),
                        declaring_scope: module_scope(path),
                        anchor: anchor(path, decorator),
                        handler_reference: handler.name.clone(),
                        middleware_references: middleware_references.clone(),
                        stages: Vec::new(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: mount.map(|_| "python-receiver-mount".to_owned()),
                        detail,
                    }));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_decorated_routes(
            child,
            source,
            path,
            evidence,
            receivers,
            local_mounts,
            framework,
            facts,
        );
    }
}

fn collect_receiver_mount_facts(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    receivers: &[Receiver],
    framework: PythonFramework,
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
            && exact_occurrence(evidence, SemanticRole::Call, function).is_some()
            && let Some(parent_receiver) =
                receiver_at(receivers, evidence, parent, node.start_byte() as u64)
            && parent_receiver.framework == framework_name(framework)
            && let Some(target) = positional_argument(&arguments, 0)
            && is_dotted_identifier(target.trim())
            && let Some(prefix) = keyword_string(&arguments, prefix_key)
            && let Some(target_qualified_name) = exact_receiver_reference(
                evidence,
                receivers,
                target.trim(),
                node.start_byte() as u64,
            )
        {
            let target_receiver = target.trim().rsplit('.').next().unwrap_or(target.trim());
            let target_module = target_qualified_name
                .rsplit_once('.')
                .map(|(module, _)| module.to_owned())
                .unwrap_or_default();
            let target_receiver_id = receiver_at(
                receivers,
                evidence,
                target_receiver,
                node.start_byte() as u64,
            )
            .map(|receiver| receiver.declaration_id.clone());
            let mut detail = Map::from_iter([
                (
                    "target_receiver".into(),
                    Value::String(target_receiver.to_owned()),
                ),
                ("target_module".into(), Value::String(target_module)),
                (
                    "target_receiver_qualified_name".into(),
                    Value::String(target_qualified_name),
                ),
                ("mount_prefix".into(), Value::String(prefix)),
                (
                    "parent_receiver".into(),
                    Value::String(parent_receiver.name.clone()),
                ),
                (
                    "parent_receiver_id".into(),
                    Value::String(parent_receiver.declaration_id.clone()),
                ),
                (
                    "parent_receiver_qualified_name".into(),
                    Value::String(parent_receiver.qualified_name.clone()),
                ),
            ]);
            if let Some(target_receiver_id) = target_receiver_id {
                detail.insert(
                    "target_receiver_id".into(),
                    Value::String(target_receiver_id),
                );
            }
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: parent_receiver.framework.to_owned(),
                kind: "router_mount".to_owned(),
                name: target_receiver.to_owned(),
                declaring_scope: module_scope(path),
                anchor: anchor(path, node),
                origin: RawFrameworkOrigin::Ast,
                detail,
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receiver_mount_facts(child, source, path, evidence, receivers, framework, facts);
    }
}

fn receiver_declarations(context: &UniversalDetectionContext<'_, '_>) -> Vec<Receiver> {
    let mut receivers = Vec::new();
    collect_receivers(
        context.root,
        context.source,
        context.evidence,
        &mut receivers,
    );
    receivers.sort_by(|left, right| {
        (left.start_byte, &left.declaration_id).cmp(&(right.start_byte, &right.declaration_id))
    });
    receivers
}

fn collect_receivers(
    node: Node<'_>,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
    receivers: &mut Vec<Receiver>,
) {
    if node.kind() == "assignment"
        && node
            .parent()
            .is_some_and(|parent| parent.kind() == "module")
        && let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        )
        && left.kind() == "identifier"
        && right.kind() == "call"
        && let Some(function) = right.child_by_field_name("function")
        && let Some(constructor) = exact_call_target(evidence, function)
        && let Some((framework, prefix_key)) = match constructor.as_str() {
            "flask.Flask" => Some(("flask", "url_prefix")),
            "flask.Blueprint" => Some(("flask", "url_prefix")),
            "fastapi.FastAPI" => Some(("fastapi", "prefix")),
            "fastapi.APIRouter" => Some(("fastapi", "prefix")),
            _ => None,
        }
        && let Some(declaration) = exact_variable_declaration(evidence, left)
        && receiver_type_is_exact(evidence, declaration, &constructor)
    {
        let arguments = call_arguments(right, source).unwrap_or_default();
        receivers.push(Receiver {
            declaration_id: declaration.id.clone(),
            qualified_name: declaration.qualified_name.clone(),
            name: declaration.name.clone(),
            framework,
            prefix: keyword_string(&arguments, prefix_key).unwrap_or_default(),
            start_byte: declaration.range.start_byte,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receivers(child, source, evidence, receivers);
    }
}

fn exact_variable_declaration<'a>(
    evidence: &'a SemanticEvidenceBatch,
    node: Node<'_>,
) -> Option<&'a DeclarationFact> {
    let mut matches = evidence.declarations.iter().filter(|declaration| {
        declaration.kind == "variable"
            && declaration.range.start_byte == node.start_byte() as u64
            && declaration.range.end_byte == node.end_byte() as u64
    });
    let declaration = matches.next()?;
    matches.next().is_none().then_some(declaration)
}

fn receiver_type_is_exact(
    evidence: &SemanticEvidenceBatch,
    declaration: &DeclarationFact,
    expected: &str,
) -> bool {
    let targets = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::TypeOf
                && candidate.source_declaration_id == declaration.id
        })
        .filter_map(|candidate| {
            exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
        })
        .collect::<BTreeSet<_>>();
    targets.len() == 1 && targets.first().is_some_and(|target| *target == expected)
}

fn receiver_at<'a>(
    receivers: &'a [Receiver],
    evidence: &SemanticEvidenceBatch,
    name: &str,
    use_start: u64,
) -> Option<&'a Receiver> {
    let declarations = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.name == name && declaration.range.start_byte < use_start)
        .collect::<Vec<_>>();
    if declarations.len() != 1 {
        return None;
    }
    receivers
        .iter()
        .find(|receiver| receiver.declaration_id == declarations[0].id)
}

fn exact_receiver_reference(
    evidence: &SemanticEvidenceBatch,
    receivers: &[Receiver],
    reference: &str,
    use_start: u64,
) -> Option<String> {
    if is_identifier(reference)
        && let Some(receiver) = receiver_at(receivers, evidence, reference, use_start)
    {
        return Some(receiver.qualified_name.clone());
    }
    let (head, suffix) = reference
        .split_once('.')
        .map_or((reference, ""), |(head, suffix)| (head, suffix));
    let targets = evidence
        .bindings
        .iter()
        .filter(|binding| {
            matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                && binding.spelling == head
                && binding.range.end_byte <= use_start
        })
        .map(|binding| {
            if suffix.is_empty() {
                binding.qualified_target.clone()
            } else {
                format!("{}.{}", binding.qualified_target, suffix)
            }
        })
        .collect::<BTreeSet<_>>();
    (targets.len() == 1)
        .then(|| targets.first().cloned())
        .flatten()
}

fn exact_call_target(evidence: &SemanticEvidenceBatch, function: Node<'_>) -> Option<String> {
    let occurrence = exact_occurrence(evidence, SemanticRole::Call, function)?;
    let targets = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.occurrence_id.as_deref() == Some(occurrence.id.as_str())
        })
        .filter_map(|candidate| {
            exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    (targets.len() == 1)
        .then(|| targets.first().cloned())
        .flatten()
}

fn exact_candidate_binding_target<'a>(
    evidence: &'a SemanticEvidenceBatch,
    binding_id: Option<&str>,
) -> Option<&'a str> {
    let binding_id = binding_id?;
    let mut matches = evidence
        .bindings
        .iter()
        .filter(|binding| binding.id == binding_id)
        .filter(|binding| matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias));
    let binding: &BindingFact = matches.next()?;
    matches
        .next()
        .is_none()
        .then_some(binding.qualified_target.as_str())
}

fn exact_occurrence<'a>(
    evidence: &'a SemanticEvidenceBatch,
    role: SemanticRole,
    node: Node<'_>,
) -> Option<&'a crate::OccurrenceFact> {
    let mut matches = evidence.occurrences.iter().filter(|occurrence| {
        occurrence.role == role
            && occurrence.range.start_byte == node.start_byte() as u64
            && occurrence.range.end_byte == node.end_byte() as u64
    });
    let occurrence = matches.next()?;
    matches.next().is_none().then_some(occurrence)
}

fn framework_name(framework: PythonFramework) -> &'static str {
    match framework {
        PythonFramework::Django => "django",
        PythonFramework::FastApi => "fastapi",
        PythonFramework::Flask => "flask",
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
            vec!["GET".to_owned()]
        } else {
            methods
        };
    }
    Vec::new()
}

fn fastapi_dependencies(
    decorator: Node<'_>,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
) -> Vec<String> {
    let mut dependencies = Vec::new();
    collect_fastapi_dependencies(decorator, source, evidence, &mut dependencies);
    dependencies
}

fn collect_fastapi_dependencies(
    node: Node<'_>,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
    dependencies: &mut Vec<String>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_call_target(evidence, function).as_deref() == Some("fastapi.Depends")
        && let Some(arguments) = call_arguments(node, source)
        && let Some(reference) = positional_argument(&arguments, 0)
        && is_dotted_identifier(reference.trim())
    {
        dependencies.push(reference.trim().to_owned());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_fastapi_dependencies(child, source, evidence, dependencies);
    }
}

fn contributes_to_urlpatterns(node: Node<'_>, source: &[u8]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "assignment" {
            return parent
                .child_by_field_name("left")
                .is_some_and(|left| node_text(left, source).trim() == "urlpatterns");
        }
        current = parent.parent();
    }
    false
}

fn call_argument_node<'tree>(
    call: Node<'tree>,
    source: &[u8],
    position: usize,
    keyword: &str,
) -> Option<Node<'tree>> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut positional_index = 0_usize;
    let mut cursor = arguments.walk();
    for argument in arguments
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if argument.kind() == "keyword_argument" {
            let name = argument.child_by_field_name("name")?;
            if node_text(name, source) == keyword {
                return argument.child_by_field_name("value");
            }
            continue;
        }
        if positional_index == position {
            return Some(argument);
        }
        positional_index = positional_index.saturating_add(1);
    }
    None
}

fn find_exact_call<'tree>(
    node: Node<'tree>,
    evidence: &SemanticEvidenceBatch,
    expected: &str,
) -> Option<Node<'tree>> {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_call_target(evidence, function).as_deref() == Some(expected)
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if let Some(found) = find_exact_call(child, evidence, expected) {
            return Some(found);
        }
    }
    None
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
