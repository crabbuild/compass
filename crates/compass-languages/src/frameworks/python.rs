use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin,
    RawFrameworkRelationFact, RawFrameworkRoleFact, RawRouteFact, RawRouteStageFact,
    RawRouteStageRole, UniversalDetectionContext,
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
    constructor_start_byte: u64,
    constructor_end_byte: u64,
    stages: Vec<RawRouteStageFact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PythonFramework {
    Django,
    FastApi,
    Flask,
    Starlette,
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

pub(super) fn detect_starlette(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    detect_universal(context, PythonFramework::Starlette)
}

pub(super) fn detect_pydantic(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    collect_pydantic_facts(context)
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
        collect_imperative_routes(
            context.root,
            context.source,
            context.path,
            context.evidence,
            &receivers,
            &local_mounts,
            framework,
            &mut facts,
        );
        if framework == PythonFramework::Starlette {
            collect_starlette_constructor_routes(
                context.root,
                context.source,
                context.path,
                context.evidence,
                &receivers,
                &local_mounts,
                &mut facts,
            );
        }
        if framework == PythonFramework::FastApi {
            collect_fastapi_provider_facts(context, &mut facts);
        }
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
            let Some(handler) = evidence
                .declarations
                .iter()
                .find(|declaration| declaration.id == decorator_occurrence.owner_declaration_id)
            else {
                continue;
            };
            let route_dependencies = if receiver.framework == "fastapi" {
                let mut stages = dependency_stages(decorator, source, path, evidence);
                if let Some(definition) = node
                    .child_by_field_name("definition")
                    .or_else(|| named_child_of_kind(node, "function_definition"))
                    && let Some(parameters) = definition.child_by_field_name("parameters")
                {
                    stages.extend(dependency_stages(parameters, source, path, evidence));
                }
                stages
            } else {
                Vec::new()
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
                let mut stage_groups = Vec::new();
                if let Some(mount) = mount {
                    stage_groups.push(mount_stages(mount, "parent_stages"));
                    stage_groups.push(mount_stages(mount, "mount_stages"));
                }
                stage_groups.push(receiver.stages.clone());
                stage_groups.push(route_dependencies.clone());
                let mut stages = ordered_stages(stage_groups);
                stages.push(handler_stage(
                    handler,
                    u32::try_from(stages.len()).unwrap_or(u32::MAX),
                ));
                append_dependency_graph_facts(facts, receiver.framework, handler, &stages, "route");
                append_handler_schema_relations(
                    facts, handler, decorator, &arguments, source, evidence, path,
                );
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
                        middleware_references: Vec::new(),
                        stages: stages.clone(),
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

#[allow(clippy::too_many_arguments)]
fn collect_imperative_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    receivers: &[Receiver],
    local_mounts: &[RawDomainFact],
    framework: PythonFramework,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_occurrence(evidence, SemanticRole::Call, function).is_some()
        && let Some((receiver_name, method)) = node_text(function, source).rsplit_once('.')
        && let Some(receiver) =
            receiver_at(receivers, evidence, receiver_name, node.start_byte() as u64)
        && receiver.framework == framework_name(framework)
        && let Some(arguments) = call_arguments(node, source)
    {
        let operations = match (receiver.framework, method) {
            ("fastapi", "add_api_route") | ("starlette", "add_route") => {
                let methods = keyword_string_list(&arguments, "methods");
                if methods.is_empty() {
                    vec!["GET".to_owned()]
                } else {
                    methods
                }
            }
            ("fastapi", "add_api_websocket_route") | ("starlette", "add_websocket_route") => {
                vec!["WEBSOCKET".to_owned()]
            }
            _ => Vec::new(),
        };
        if !operations.is_empty()
            && let Some(raw_path) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, "path"))
            && let Some(handler_reference) = positional_argument(&arguments, 1)
                .or_else(|| keyword_value(&arguments, "endpoint"))
                .map(str::trim)
                .filter(|reference| is_dotted_identifier(reference))
        {
            let handler = exact_local_declaration_reference(
                evidence,
                handler_reference,
                node.start_byte() as u64,
                &["function", "method"],
            );
            let dependencies = if receiver.framework == "fastapi" {
                let mut stages = dependency_stages(node, source, path, evidence);
                if let Some(handler) = handler
                    && let Some(definition) = declaration_node(context_root(node), handler)
                    && let Some(parameters) = definition.child_by_field_name("parameters")
                {
                    stages.extend(dependency_stages(parameters, source, path, evidence));
                }
                stages
            } else {
                Vec::new()
            };
            push_receiver_route_facts(
                facts,
                path,
                source,
                evidence,
                receiver,
                local_mounts,
                node,
                &raw_path,
                &operations,
                handler_reference,
                handler,
                dependencies,
                &arguments,
            );
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_imperative_routes(
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

#[allow(clippy::too_many_arguments)]
fn collect_starlette_constructor_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    receivers: &[Receiver],
    local_mounts: &[RawDomainFact],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(receiver) = receivers.iter().find(|receiver| {
            receiver.framework == "starlette"
                && receiver.constructor_start_byte == node.start_byte() as u64
                && receiver.constructor_end_byte == node.end_byte() as u64
        })
    {
        collect_starlette_route_calls(
            node,
            node,
            source,
            path,
            evidence,
            receiver,
            local_mounts,
            facts,
        );
        collect_starlette_mount_calls(node, source, path, evidence, receiver, receivers, facts);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_starlette_constructor_routes(
            child,
            source,
            path,
            evidence,
            receivers,
            local_mounts,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_starlette_route_calls(
    constructor: Node<'_>,
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    receiver: &Receiver,
    local_mounts: &[RawDomainFact],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && node.id() != constructor.id()
        && let Some(function) = node.child_by_field_name("function")
        && let Some(target) = exact_call_target(evidence, function)
        && matches!(
            target.as_str(),
            "starlette.routing.Route" | "starlette.routing.WebSocketRoute"
        )
        && let Some(arguments) = call_arguments(node, source)
        && let Some(path_fragment) = positional_argument(&arguments, 0)
            .and_then(string_literal)
            .or_else(|| keyword_string(&arguments, "path"))
        && let Some(handler_reference) = positional_argument(&arguments, 1)
            .or_else(|| keyword_value(&arguments, "endpoint"))
            .map(str::trim)
            .filter(|reference| is_dotted_identifier(reference))
    {
        let mount_prefix = enclosing_starlette_mount_prefix(constructor, node, source, evidence);
        let raw_path = join_route_paths(&mount_prefix, &path_fragment);
        let operations = if target.ends_with("WebSocketRoute") {
            vec!["WEBSOCKET".to_owned()]
        } else {
            let methods = keyword_string_list(&arguments, "methods");
            if methods.is_empty() {
                vec!["GET".to_owned()]
            } else {
                methods
            }
        };
        let handler = exact_local_declaration_reference(
            evidence,
            handler_reference,
            node.start_byte() as u64,
            &["function", "method"],
        );
        push_receiver_route_facts(
            facts,
            path,
            source,
            evidence,
            receiver,
            local_mounts,
            node,
            &raw_path,
            &operations,
            handler_reference,
            handler,
            Vec::new(),
            &arguments,
        );
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_starlette_route_calls(
            constructor,
            child,
            source,
            path,
            evidence,
            receiver,
            local_mounts,
            facts,
        );
    }
}

fn collect_starlette_mount_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    parent: &Receiver,
    receivers: &[Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_call_target(evidence, function).as_deref() == Some("starlette.routing.Mount")
        && let Some(arguments) = call_arguments(node, source)
        && let Some(prefix) = positional_argument(&arguments, 0)
            .and_then(string_literal)
            .or_else(|| keyword_string(&arguments, "path"))
        && let Some(target) = keyword_value(&arguments, "app")
            .or_else(|| positional_argument(&arguments, 1))
            .map(str::trim)
            .filter(|target| is_dotted_identifier(target))
        && let Some(target_qualified_name) =
            exact_receiver_reference(evidence, receivers, target, node.start_byte() as u64)
    {
        let target_name = target.rsplit('.').next().unwrap_or(target);
        let target_id = receiver_at(receivers, evidence, target_name, node.start_byte() as u64)
            .map(|receiver| receiver.declaration_id.clone());
        let mut detail = Map::from_iter([
            (
                "target_receiver".into(),
                Value::String(target_name.to_owned()),
            ),
            (
                "target_receiver_qualified_name".into(),
                Value::String(target_qualified_name),
            ),
            ("mount_prefix".into(), Value::String(prefix)),
            ("parent_receiver".into(), Value::String(parent.name.clone())),
            (
                "parent_receiver_id".into(),
                Value::String(parent.declaration_id.clone()),
            ),
            (
                "parent_receiver_qualified_name".into(),
                Value::String(parent.qualified_name.clone()),
            ),
            ("parent_stages".into(), Value::Array(Vec::new())),
            ("mount_stages".into(), Value::Array(Vec::new())),
        ]);
        if let Some(target_id) = target_id {
            detail.insert("target_receiver_id".into(), Value::String(target_id));
        }
        facts.push(RawFrameworkFact::Domain(RawDomainFact {
            framework: "starlette".to_owned(),
            kind: "router_mount".to_owned(),
            name: target_name.to_owned(),
            declaring_scope: module_scope(path),
            anchor: anchor(path, node),
            origin: RawFrameworkOrigin::Ast,
            detail,
        }));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_starlette_mount_calls(child, source, path, evidence, parent, receivers, facts);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_receiver_route_facts(
    facts: &mut Vec<RawFrameworkFact>,
    path: &Path,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
    receiver: &Receiver,
    local_mounts: &[RawDomainFact],
    route_node: Node<'_>,
    raw_path: &str,
    operations: &[String],
    handler_reference: &str,
    handler: Option<&DeclarationFact>,
    dependencies: Vec<RawRouteStageFact>,
    arguments: &[String],
) {
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
        let mut groups = Vec::new();
        if let Some(mount) = mount {
            groups.push(mount_stages(mount, "parent_stages"));
            groups.push(mount_stages(mount, "mount_stages"));
        }
        groups.push(receiver.stages.clone());
        groups.push(dependencies.clone());
        let mut stages = ordered_stages(groups);
        stages.push(handler.map_or_else(
            || RawRouteStageFact {
                role: RawRouteStageRole::Handler,
                position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
                reference: handler_reference.to_owned(),
                anchor: anchor(path, route_node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::new(),
            },
            |handler| handler_stage(handler, u32::try_from(stages.len()).unwrap_or(u32::MAX)),
        ));
        if let Some(handler) = handler {
            append_dependency_graph_facts(facts, receiver.framework, handler, &stages, "route");
            append_handler_schema_relations(
                facts, handler, route_node, arguments, source, evidence, path,
            );
        }
        let normalized_path =
            join_route_paths(mount_prefix, &join_route_paths(&receiver.prefix, raw_path));
        for operation in operations {
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
            }
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: receiver.framework.to_owned(),
                operation: operation.clone(),
                raw_path: raw_path.to_owned(),
                normalized_path: normalized_path.clone(),
                declaring_scope: module_scope(path),
                anchor: anchor(path, route_node),
                handler_reference: handler_reference.to_owned(),
                middleware_references: Vec::new(),
                stages: stages.clone(),
                origin: RawFrameworkOrigin::Ast,
                rule: mount.map(|_| "python-receiver-mount".to_owned()),
                detail,
            }));
        }
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
        let mount_arguments = match method {
            "include_router" => positional_argument(&arguments, 0).map(|target| {
                (
                    target,
                    keyword_string(&arguments, "prefix").unwrap_or_default(),
                )
            }),
            "register_blueprint" => positional_argument(&arguments, 0).map(|target| {
                (
                    target,
                    keyword_string(&arguments, "url_prefix").unwrap_or_default(),
                )
            }),
            "mount" => positional_argument(&arguments, 1)
                .or_else(|| keyword_value(&arguments, "app"))
                .zip(positional_argument(&arguments, 0).and_then(string_literal)),
            _ => None,
        };
        if let Some((target, prefix)) = mount_arguments
            && let Some(parent) = function_text.rsplit_once('.').map(|(parent, _)| parent)
            && exact_occurrence(evidence, SemanticRole::Call, function).is_some()
            && let Some(parent_receiver) =
                receiver_at(receivers, evidence, parent, node.start_byte() as u64)
            && parent_receiver.framework == framework_name(framework)
            && is_dotted_identifier(target.trim())
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
            let mount_stages = if parent_receiver.framework == "fastapi" {
                dependency_stages(node, source, path, evidence)
            } else {
                Vec::new()
            };
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
                (
                    "parent_stages".into(),
                    serde_json::to_value(&parent_receiver.stages).unwrap_or(Value::Null),
                ),
                (
                    "mount_stages".into(),
                    serde_json::to_value(&mount_stages).unwrap_or(Value::Null),
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
        context.path,
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
    path: &Path,
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
            "starlette.applications.Starlette" => Some(("starlette", "prefix")),
            "starlette.routing.Router" => Some(("starlette", "prefix")),
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
            constructor_start_byte: right.start_byte() as u64,
            constructor_end_byte: right.end_byte() as u64,
            stages: if framework == "fastapi" {
                dependency_stages(right, source, path, evidence)
            } else {
                Vec::new()
            },
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_receivers(child, source, path, evidence, receivers);
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
        PythonFramework::Starlette => "starlette",
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
    if framework == "starlette" {
        let operation = match method {
            "route" => None,
            "websocket_route" => Some("WEBSOCKET".to_owned()),
            _ => return Vec::new(),
        };
        return operation
            .map(|operation| vec![operation])
            .unwrap_or_else(|| {
                let methods = keyword_string_list(arguments, "methods");
                if methods.is_empty() {
                    vec!["GET".to_owned()]
                } else {
                    methods
                }
            });
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

fn dependency_stages(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
) -> Vec<RawRouteStageFact> {
    let mut stages = Vec::new();
    collect_dependency_stages(node, source, path, evidence, &mut stages);
    stages
}

fn collect_dependency_stages(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    stages: &mut Vec<RawRouteStageFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(target) = exact_call_target(evidence, function)
        && matches!(target.as_str(), "fastapi.Depends" | "fastapi.Security")
        && let Some(reference_node) = call_argument_node(node, source, 0, "dependency")
        && let Some(reference) = exact_static_reference_hint(evidence, reference_node, source)
    {
        let mut detail = Map::new();
        if let Some(provider) =
            exact_reference_declaration(evidence, reference_node).or_else(|| {
                evidence
                    .declarations
                    .iter()
                    .find(|declaration| declaration.graph_node_id == reference)
            })
            && let Some(definition) = declaration_node(context_root(node), provider)
            && contains_node_kind(definition, "yield")
        {
            detail.insert("lifecycle".into(), Value::String("yield".to_owned()));
        }
        if target == "fastapi.Security"
            && let Some(scopes) = call_arguments(node, source)
                .and_then(|arguments| keyword_value(&arguments, "scopes").map(str::to_owned))
        {
            detail.insert("scopes".into(), Value::String(scopes));
        }
        stages.push(RawRouteStageFact {
            role: if target == "fastapi.Security" {
                RawRouteStageRole::Security
            } else {
                RawRouteStageRole::Dependency
            },
            position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
            reference,
            anchor: anchor(path, node),
            origin: RawFrameworkOrigin::Ast,
            detail,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_dependency_stages(child, source, path, evidence, stages);
    }
}

fn ordered_stages(groups: Vec<Vec<RawRouteStageFact>>) -> Vec<RawRouteStageFact> {
    groups
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(position, mut stage)| {
            stage.position = u32::try_from(position).unwrap_or(u32::MAX);
            stage
        })
        .collect()
}

fn mount_stages(mount: &RawDomainFact, key: &str) -> Vec<RawRouteStageFact> {
    mount
        .detail
        .get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn handler_stage(handler: &DeclarationFact, position: u32) -> RawRouteStageFact {
    RawRouteStageFact {
        role: RawRouteStageRole::Handler,
        position,
        reference: handler.name.clone(),
        anchor: evidence_anchor(&handler.range),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([("declaration_id".into(), Value::String(handler.id.clone()))]),
    }
}

fn append_dependency_graph_facts(
    facts: &mut Vec<RawFrameworkFact>,
    framework: &str,
    source: &DeclarationFact,
    stages: &[RawRouteStageFact],
    context: &str,
) {
    if framework != "fastapi" {
        return;
    }
    for stage in stages.iter().filter(|stage| {
        matches!(
            stage.role,
            RawRouteStageRole::Dependency | RawRouteStageRole::Security
        )
    }) {
        let role = if stage.role == RawRouteStageRole::Security {
            "middleware"
        } else {
            "service"
        };
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "fastapi-python".to_owned(),
            framework: "fastapi".to_owned(),
            role: role.to_owned(),
            subject_reference: Some(stage.reference.clone()),
            context: Some(context.to_owned()),
            anchor: stage.anchor.clone(),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: stage.detail.clone(),
        }));
        facts.push(RawFrameworkFact::Relation(RawFrameworkRelationFact {
            pack_id: "fastapi-python".to_owned(),
            framework: "fastapi".to_owned(),
            relation: "depends_on".to_owned(),
            source_reference: Some(source.graph_node_id.clone()),
            target_hint: Some(stage.reference.clone()),
            context: Some(context.to_owned()),
            anchor: stage.anchor.clone(),
            target_anchor: Some(stage.anchor.clone()),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            ambiguity_policy: "require_exact".to_owned(),
            detail: stage.detail.clone(),
        }));
    }
}

fn collect_fastapi_provider_facts(
    context: &UniversalDetectionContext<'_, '_>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    for declaration in context
        .evidence
        .declarations
        .iter()
        .filter(|declaration| matches!(declaration.kind.as_str(), "function" | "method"))
    {
        let Some(definition) = declaration_node(context.root, declaration) else {
            continue;
        };
        let Some(parameters) = definition.child_by_field_name("parameters") else {
            continue;
        };
        let stages = dependency_stages(parameters, context.source, context.path, context.evidence);
        append_dependency_graph_facts(facts, "fastapi", declaration, &stages, "subdependency");
    }
}

fn append_handler_schema_relations(
    facts: &mut Vec<RawFrameworkFact>,
    handler: &DeclarationFact,
    route_node: Node<'_>,
    arguments: &[String],
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
    path: &Path,
) {
    let models = pydantic_model_declarations(evidence);
    if models.is_empty() {
        return;
    }
    let handler_scope = evidence
        .scopes
        .iter()
        .find(|scope| scope.owner_declaration_id.as_deref() == Some(handler.id.as_str()))
        .map(|scope| scope.id.as_str());
    let parameter_ids = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "parameter" && declaration.scope_id.as_deref() == handler_scope
        })
        .map(|declaration| declaration.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut dependencies = BTreeSet::<(String, String)>::new();
    for candidate in &evidence.candidates {
        let context = if candidate.relation == CandidateRelation::Returns
            && candidate.source_declaration_id == handler.id
        {
            Some("response_model")
        } else if candidate.relation == CandidateRelation::TypeOf
            && parameter_ids.contains(candidate.source_declaration_id.as_str())
        {
            Some("request_model")
        } else {
            None
        };
        let Some(context) = context else {
            continue;
        };
        if let Some(model) = exact_model_candidate(evidence, candidate, &models) {
            dependencies.insert((context.to_owned(), model.id.clone()));
        }
    }
    if keyword_value(arguments, "response_model").is_some()
        && let Some(value) = keyword_argument_node(route_node, "response_model", source)
        && let Some(model) = exact_reference_declaration(evidence, value)
        && models.contains(model.id.as_str())
    {
        dependencies.insert(("response_model".to_owned(), model.id.clone()));
    }
    for (context, model_id) in dependencies {
        let Some(model) = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == model_id)
        else {
            continue;
        };
        facts.push(RawFrameworkFact::Relation(RawFrameworkRelationFact {
            pack_id: "pydantic-python".to_owned(),
            framework: "pydantic".to_owned(),
            relation: "depends_on".to_owned(),
            source_reference: Some(handler.graph_node_id.clone()),
            target_hint: Some(model.graph_node_id.clone()),
            context: Some(context.clone()),
            anchor: anchor(path, route_node),
            target_anchor: Some(evidence_anchor(&model.range)),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            ambiguity_policy: "require_exact".to_owned(),
            detail: Map::from_iter([("schema_direction".into(), Value::String(context))]),
        }));
    }
}

fn collect_pydantic_facts(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let models = pydantic_model_declarations(context.evidence);
    let mut facts = Vec::new();
    for model_id in &models {
        let Some(model) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == *model_id)
        else {
            continue;
        };
        let model_scope = context
            .evidence
            .scopes
            .iter()
            .find(|scope| scope.owner_declaration_id.as_deref() == Some(model.id.as_str()))
            .map(|scope| scope.id.as_str());
        let fields = context
            .evidence
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.kind == "field" && declaration.scope_id.as_deref() == model_scope
            })
            .map(|declaration| Value::String(declaration.graph_node_id.clone()))
            .collect::<Vec<_>>();
        let members = pydantic_member_decorators(context.evidence, model_scope);
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "pydantic-python".to_owned(),
            framework: "pydantic".to_owned(),
            role: "model".to_owned(),
            subject_reference: Some(model.graph_node_id.clone()),
            context: model.module_or_package.clone(),
            anchor: evidence_anchor(&model.range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::from_iter([
                ("declaration_id".into(), Value::String(model.id.clone())),
                ("fields".into(), Value::Array(fields)),
                ("members".into(), Value::Array(members)),
            ]),
        }));
    }
    facts
}

fn pydantic_model_declarations(evidence: &SemanticEvidenceBatch) -> BTreeSet<String> {
    let mut models = BTreeSet::new();
    for candidate in evidence.candidates.iter().filter(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && (candidate.constraints.qualified_name.as_deref() == Some("pydantic.BaseModel")
                || exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
                    == Some("pydantic.BaseModel"))
    }) {
        models.insert(candidate.source_declaration_id.clone());
    }
    for _ in 0..evidence.declarations.len() {
        let mut qualified_models = BTreeMap::<&str, usize>::new();
        for declaration in evidence
            .declarations
            .iter()
            .filter(|declaration| models.contains(declaration.id.as_str()))
        {
            *qualified_models
                .entry(declaration.qualified_name.as_str())
                .or_default() += 1;
        }
        let inherited = evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.relation == CandidateRelation::Extends)
            .filter(|candidate| {
                candidate
                    .constraints
                    .exact_target_declaration_id
                    .as_ref()
                    .is_some_and(|target| models.contains(target))
                    || candidate
                        .constraints
                        .qualified_name
                        .as_deref()
                        .is_some_and(|target| qualified_models.get(target) == Some(&1))
            })
            .map(|candidate| candidate.source_declaration_id.clone())
            .collect::<Vec<_>>();
        let previous = models.len();
        models.extend(inherited);
        if models.len() == previous {
            break;
        }
    }
    models
}

fn pydantic_member_decorators(
    evidence: &SemanticEvidenceBatch,
    model_scope: Option<&str>,
) -> Vec<Value> {
    let member_ids = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            matches!(declaration.kind.as_str(), "method" | "property")
                && declaration.scope_id.as_deref() == model_scope
        })
        .map(|declaration| (declaration.id.as_str(), declaration.graph_node_id.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut members = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Decorates)
        .filter_map(|candidate| {
            let member = member_ids.get(candidate.source_declaration_id.as_str())?;
            let qualified = candidate
                .constraints
                .qualified_name
                .as_deref()
                .or_else(|| {
                    exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
                })?;
            matches!(
                qualified,
                "pydantic.field_validator"
                    | "pydantic.model_validator"
                    | "pydantic.field_serializer"
                    | "pydantic.model_serializer"
                    | "pydantic.computed_field"
            )
            .then(|| {
                Value::Object(Map::from_iter([
                    ("member".into(), Value::String((*member).to_owned())),
                    ("decorator".into(), Value::String(qualified.to_owned())),
                ]))
            })
        })
        .collect::<Vec<_>>();
    members.sort_by_key(Value::to_string);
    members.dedup();
    members
}

fn exact_model_candidate<'a>(
    evidence: &'a SemanticEvidenceBatch,
    candidate: &crate::RelationshipCandidate,
    models: &BTreeSet<String>,
) -> Option<&'a DeclarationFact> {
    if let Some(target) = candidate
        .constraints
        .exact_target_declaration_id
        .as_deref()
        .filter(|target| models.contains(*target))
    {
        return evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == target);
    }
    let qualified = candidate.constraints.qualified_name.as_deref()?;
    let mut matches = evidence.declarations.iter().filter(|declaration| {
        models.contains(declaration.id.as_str()) && declaration.qualified_name == qualified
    });
    let model = matches.next()?;
    matches.next().is_none().then_some(model)
}

fn exact_reference_declaration<'a>(
    evidence: &'a SemanticEvidenceBatch,
    node: Node<'_>,
) -> Option<&'a DeclarationFact> {
    let occurrence_ids = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.range.start_byte >= node.start_byte() as u64
                && occurrence.range.end_byte <= node.end_byte() as u64
        })
        .map(|occurrence| occurrence.id.as_str())
        .collect::<BTreeSet<_>>();
    let targets = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::References)
        .filter(|candidate| {
            candidate
                .occurrence_id
                .as_deref()
                .is_some_and(|occurrence| occurrence_ids.contains(occurrence))
        })
        .filter_map(|candidate| candidate.constraints.exact_target_declaration_id.as_deref())
        .collect::<BTreeSet<_>>();
    if targets.len() != 1 {
        return None;
    }
    let target = *targets.first()?;
    evidence
        .declarations
        .iter()
        .find(|declaration| declaration.id == target)
}

fn exact_static_reference_hint(
    evidence: &SemanticEvidenceBatch,
    node: Node<'_>,
    source: &[u8],
) -> Option<String> {
    let reference = node_text(node, source).trim();
    if !is_dotted_identifier(reference) {
        return None;
    }
    let in_signature = ancestors_before_definition(node)
        .iter()
        .any(|ancestor| ancestor.kind() == "parameters");
    let reference_scope = reference_evaluation_scope(evidence, node, in_signature);
    if is_identifier(reference) {
        let declarations = evidence
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.name == reference
                    && declaration.range.start_byte < node.start_byte() as u64
                    && declaration.scope_id.as_deref() == reference_scope
                    && !(in_signature && declaration.kind == "parameter")
            })
            .collect::<Vec<_>>();
        let bindings = evidence
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                    && binding.spelling == reference
                    && binding.range.end_byte <= node.start_byte() as u64
                    && binding.scope_id.as_deref() == reference_scope
            })
            .collect::<Vec<_>>();
        return match (declarations.as_slice(), bindings.as_slice()) {
            ([declaration], [])
                if matches!(declaration.kind.as_str(), "function" | "method" | "class")
                    && !has_intervening_python_rebinding(
                        context_root(node),
                        declaration,
                        reference,
                        node.start_byte() as u64,
                        source,
                    ) =>
            {
                Some(declaration.graph_node_id.clone())
            }
            ([], [binding]) => Some(binding.qualified_target.clone()),
            _ => None,
        };
    }
    let (head, suffix) = reference.split_once('.')?;
    let bindings = evidence
        .bindings
        .iter()
        .filter(|binding| {
            matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                && binding.spelling == head
                && binding.range.end_byte <= node.start_byte() as u64
                && binding.scope_id.as_deref() == reference_scope
        })
        .map(|binding| format!("{}.{}", binding.qualified_target, suffix))
        .collect::<BTreeSet<_>>();
    (bindings.len() == 1)
        .then(|| bindings.first().cloned())
        .flatten()
}

fn has_intervening_python_rebinding(
    root: Node<'_>,
    declaration: &DeclarationFact,
    reference: &str,
    use_start: u64,
    source: &[u8],
) -> bool {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|statement| statement.is_named())
        .filter(|statement| statement.start_byte() as u64 > declaration.range.end_byte)
        .filter(|statement| statement.end_byte() as u64 <= use_start)
        .filter(|statement| {
            !matches!(
                statement.kind(),
                "function_definition" | "class_definition" | "decorated_definition"
            )
        })
        .any(|statement| {
            crate::engine::python_bound_names(statement, source, true).contains(reference)
        })
}

fn reference_evaluation_scope<'a>(
    evidence: &'a SemanticEvidenceBatch,
    node: Node<'_>,
    in_signature: bool,
) -> Option<&'a str> {
    let scope = evidence
        .scopes
        .iter()
        .filter(|scope| {
            scope.range.start_byte <= node.start_byte() as u64
                && scope.range.end_byte >= node.end_byte() as u64
        })
        .min_by_key(|scope| scope.range.end_byte.saturating_sub(scope.range.start_byte))?;
    if in_signature && scope.kind == "function" {
        scope.parent_scope_id.as_deref()
    } else {
        Some(scope.id.as_str())
    }
}

fn ancestors_before_definition(mut node: Node<'_>) -> Vec<Node<'_>> {
    let mut ancestors = Vec::new();
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "function_definition" | "class_definition") {
            break;
        }
        ancestors.push(parent);
        node = parent;
    }
    ancestors
}

fn exact_local_declaration_reference<'a>(
    evidence: &'a SemanticEvidenceBatch,
    reference: &str,
    use_start: u64,
    kinds: &[&str],
) -> Option<&'a DeclarationFact> {
    if !is_identifier(reference) {
        return None;
    }
    let use_scope = evidence
        .scopes
        .iter()
        .filter(|scope| scope.range.start_byte <= use_start && scope.range.end_byte >= use_start)
        .min_by_key(|scope| scope.range.end_byte.saturating_sub(scope.range.start_byte))
        .map(|scope| scope.id.as_str());
    let mut matches = evidence.declarations.iter().filter(|declaration| {
        declaration.name == reference
            && declaration.range.start_byte < use_start
            && declaration.scope_id.as_deref() == use_scope
            && kinds.contains(&declaration.kind.as_str())
    });
    let declaration = matches.next()?;
    matches.next().is_none().then_some(declaration)
}

fn keyword_argument_node<'tree>(
    node: Node<'tree>,
    key: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    if node.kind() == "keyword_argument"
        && node
            .child_by_field_name("name")
            .is_some_and(|name| node_text(name, source) == key)
    {
        return node.child_by_field_name("value");
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if let Some(found) = keyword_argument_node(child, key, source) {
            return Some(found);
        }
    }
    None
}

fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.is_named() && child.kind() == kind)
}

fn context_root(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        node = parent;
    }
    node
}

fn declaration_node<'tree>(
    node: Node<'tree>,
    declaration: &DeclarationFact,
) -> Option<Node<'tree>> {
    if matches!(node.kind(), "function_definition" | "class_definition")
        && node.child_by_field_name("name").is_some_and(|name| {
            name.start_byte() as u64 == declaration.range.start_byte
                && name.end_byte() as u64 == declaration.range.end_byte
        })
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if let Some(found) = declaration_node(child, declaration) {
            return Some(found);
        }
    }
    None
}

fn contains_node_kind(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind || node.kind().strip_suffix("_expression") == Some(kind) {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .any(|child| contains_node_kind(child, kind))
}

fn enclosing_starlette_mount_prefix(
    constructor: Node<'_>,
    node: Node<'_>,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
) -> String {
    let mut prefixes = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.id() == constructor.id() {
            break;
        }
        if parent.kind() == "call"
            && let Some(function) = parent.child_by_field_name("function")
            && exact_call_target(evidence, function).as_deref() == Some("starlette.routing.Mount")
            && let Some(arguments) = call_arguments(parent, source)
            && let Some(prefix) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, "path"))
        {
            prefixes.push(prefix);
        }
        current = parent.parent();
    }
    prefixes
        .into_iter()
        .rev()
        .fold(String::new(), |path, prefix| {
            join_route_paths(&path, &prefix)
        })
}

fn evidence_anchor(range: &crate::EvidenceRange) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: range.source_file.clone(),
        start_byte: range.start_byte,
        end_byte: range.end_byte,
        start_line: range.start_line,
        start_column: range.start_column,
        end_line: range.end_line,
        end_column: range.end_column,
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
