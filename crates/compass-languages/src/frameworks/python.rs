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
    scope_id: Option<String>,
    framework: &'static str,
    constructor: String,
    prefix: String,
    start_byte: u64,
    end_byte: u64,
    constructor_start_byte: u64,
    constructor_end_byte: u64,
    stages: Vec<RawRouteStageFact>,
}

#[derive(Clone, Debug)]
struct CeleryTask {
    declaration_id: String,
    graph_node_id: String,
    bind: bool,
}

#[derive(Default)]
struct DjangoPatternFlow {
    calls: BTreeSet<usize>,
    collections_by_call: BTreeMap<usize, String>,
    local_collections: BTreeSet<String>,
    i18n_calls: BTreeSet<usize>,
    imported_collections: Vec<DjangoImportedPatternCollection>,
}

struct DjangoImportedPatternCollection {
    target: String,
    collection: Option<String>,
    in_i18n: bool,
    anchor: RawFrameworkAnchor,
}

struct DjangoInclude {
    target: String,
    collection: Option<String>,
    application_name: Option<String>,
    namespace: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PythonFramework {
    Django,
    FastApi,
    Flask,
    Starlette,
}

pub(super) fn detect_django(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let mut facts = detect_universal(context, PythonFramework::Django);
    collect_django_model_facts(context, &mut facts);
    collect_django_signal_facts(context, &mut facts);
    facts
}

pub(super) fn detect_drf(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let receivers = receiver_declarations(context);
    let flow = django_pattern_flow(context.root, context.source, context.path, context.evidence);
    let mut facts = Vec::new();
    collect_drf_router_registrations(context, &receivers, &mut facts);
    collect_drf_router_mounts(context, &receivers, &flow, &mut facts);
    collect_drf_viewset_and_serializer_facts(context, &mut facts);
    facts
}

pub(super) fn detect_fastapi(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    detect_universal(context, PythonFramework::FastApi)
}

pub(super) fn detect_flask(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let mut facts = detect_universal(context, PythonFramework::Flask);
    let receivers = receiver_declarations(context);
    collect_flask_factory_roles(context.root, context, &receivers, &mut facts);
    facts
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

pub(super) fn detect_sqlalchemy(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    collect_sqlalchemy_facts(context)
}

pub(super) fn detect_celery(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    collect_celery_facts(context)
}

fn collect_sqlalchemy_facts(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let declarative_bases =
        python_direct_descendants_of(context.evidence, &["sqlalchemy.orm.DeclarativeBase"]);
    let mut models = python_descendants_of(context.evidence, &["sqlalchemy.orm.DeclarativeBase"]);
    for base in &declarative_bases {
        models.remove(base);
    }

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
        let Some(definition) = declaration_node(context.root, model) else {
            continue;
        };
        let fields = sqlalchemy_model_field_facts(definition, context, model, &models, &mut facts);
        let schema = direct_class_assignment_value(definition, "__table_args__", context.source)
            .and_then(|value| dictionary_value_node(value, "schema", context.source))
            .and_then(|value| string_literal(node_text(value, context.source)))
            .unwrap_or_default();
        let table = direct_class_assignment_value(definition, "__tablename__", context.source)
            .and_then(|value| {
                string_literal(node_text(value, context.source))
                    .map(|name| (value, name, schema.clone()))
            });
        let mut detail = Map::from_iter([
            ("declaration_id".into(), Value::String(model.id.clone())),
            ("fields".into(), Value::Array(fields)),
        ]);
        if let Some((_, table_name, schema)) = &table {
            detail.insert("database_table".into(), Value::String(table_name.clone()));
            if !schema.is_empty() {
                detail.insert("database_schema".into(), Value::String(schema.clone()));
            }
        }
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "sqlalchemy-python".to_owned(),
            framework: "sqlalchemy".to_owned(),
            role: "model".to_owned(),
            subject_reference: Some(model.graph_node_id.clone()),
            context: model.module_or_package.clone(),
            anchor: evidence_anchor(&model.range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail,
        }));
        if let Some((table_node, table_name, schema)) = table {
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "sqlalchemy".to_owned(),
                kind: "orm_mapping".to_owned(),
                name: model.name.clone(),
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, table_node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "model_reference".into(),
                        Value::String(model.graph_node_id.clone()),
                    ),
                    ("database_table".into(), Value::String(table_name)),
                    ("database_schema".into(), Value::String(schema)),
                    ("explicit".into(), Value::Bool(true)),
                    (
                        "pack_id".into(),
                        Value::String("sqlalchemy-python".to_owned()),
                    ),
                ]),
            }));
        }
    }
    facts
}

fn python_direct_descendants_of(
    evidence: &SemanticEvidenceBatch,
    exact_bases: &[&str],
) -> BTreeSet<String> {
    evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Extends)
        .filter(|candidate| {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .is_some_and(|target| exact_bases.contains(&target))
                || exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
                    .is_some_and(|target| exact_bases.contains(&target))
        })
        .map(|candidate| candidate.source_declaration_id.clone())
        .collect()
}

fn sqlalchemy_model_field_facts(
    definition: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    model: &DeclarationFact,
    models: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) -> Vec<Value> {
    let Some(body) = definition.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut cursor = body.walk();
    for assignment in body
        .children(&mut cursor)
        .filter(|child| matches!(child.kind(), "assignment" | "annotated_assignment"))
    {
        let (Some(name), Some(value)) = (
            assignment.child_by_field_name("left"),
            assignment.child_by_field_name("right"),
        ) else {
            continue;
        };
        if name.kind() != "identifier" || value.kind() != "call" {
            continue;
        }
        let Some(function) = value.child_by_field_name("function") else {
            continue;
        };
        let Some(constructor) = exact_static_reference_in_scope(
            context.evidence,
            function,
            context.source,
            model.scope_id.as_deref(),
        ) else {
            continue;
        };
        if !matches!(
            constructor.as_str(),
            "sqlalchemy.orm.mapped_column" | "sqlalchemy.orm.relationship"
        ) {
            continue;
        }
        let field_name = node_text(name, context.source).to_owned();
        let mut detail = Map::from_iter([
            ("name".into(), Value::String(field_name.clone())),
            ("constructor".into(), Value::String(constructor.clone())),
            (
                "anchor".into(),
                serde_json::to_value(anchor(context.path, assignment)).unwrap_or(Value::Null),
            ),
        ]);
        if let Some(annotation) = assignment.child_by_field_name("type")
            && node_contains_exact_reference(
                annotation,
                context,
                model.scope_id.as_deref(),
                "sqlalchemy.orm.Mapped",
            )
        {
            detail.insert(
                "annotation".into(),
                Value::String(node_text(annotation, context.source).to_owned()),
            );
        }
        if constructor == "sqlalchemy.orm.mapped_column"
            && let Some(foreign_key) =
                find_exact_call(value, context.evidence, "sqlalchemy.ForeignKey")
            && let Some(target) = call_argument_node(foreign_key, context.source, 0, "column")
                .and_then(|target| string_literal(node_text(target, context.source)))
        {
            detail.insert("foreign_key".into(), Value::String(target));
        }
        if constructor == "sqlalchemy.orm.relationship"
            && let Some(target) = call_argument_node(value, context.source, 0, "argument")
                .and_then(|target_node| {
                    exact_python_lexical_reference_evidence(
                        context.evidence,
                        target_node,
                        context.source,
                    )
                })
                .or_else(|| {
                    assignment
                        .child_by_field_name("type")
                        .and_then(|annotation| {
                            exact_sqlalchemy_annotation_model(annotation, context, models)
                        })
                })
            && let Some(declaration) = unique_declaration_for_reference(context.evidence, &target)
            && models.contains(declaration.id.as_str())
        {
            detail.insert(
                "target_reference".into(),
                Value::String(declaration.graph_node_id.clone()),
            );
            append_exact_framework_relation(
                facts,
                "sqlalchemy-python",
                "sqlalchemy",
                model,
                &declaration.graph_node_id,
                &format!("relationship:{field_name}"),
                anchor(context.path, assignment),
                context.evidence,
            );
        }
        fields.push(Value::Object(detail));
    }
    fields
}

fn exact_sqlalchemy_annotation_model(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    models: &BTreeSet<String>,
) -> Option<String> {
    let mut targets = BTreeSet::new();
    collect_exact_sqlalchemy_annotation_models(node, context, models, &mut targets);
    (targets.len() == 1)
        .then(|| targets.first().cloned())
        .flatten()
}

fn collect_exact_sqlalchemy_annotation_models(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    models: &BTreeSet<String>,
    targets: &mut BTreeSet<String>,
) {
    if node.kind() == "identifier"
        && let Some(reference) =
            exact_python_lexical_reference_evidence(context.evidence, node, context.source)
        && let Some(declaration) = unique_declaration_for_reference(context.evidence, &reference)
        && models.contains(declaration.id.as_str())
    {
        targets.insert(declaration.graph_node_id.clone());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_exact_sqlalchemy_annotation_models(child, context, models, targets);
    }
}

fn node_contains_exact_reference(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    scope_id: Option<&str>,
    expected: &str,
) -> bool {
    if exact_static_reference_in_scope(context.evidence, node, context.source, scope_id).as_deref()
        == Some(expected)
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .any(|child| node_contains_exact_reference(child, context, scope_id, expected))
}

fn collect_celery_facts(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let receivers = receiver_declarations(context);
    let mut facts = Vec::new();
    let mut tasks = Vec::new();
    collect_celery_task_declarations(context.root, context, &receivers, &mut tasks, &mut facts);
    collect_celery_invocations(context.root, context, &receivers, &tasks, &mut facts);
    collect_celery_beat_schedules(context.root, context, &receivers, &mut facts);
    tasks.sort_by(|left, right| left.declaration_id.cmp(&right.declaration_id));
    tasks.dedup_by(|left, right| left.declaration_id == right.declaration_id);
    facts
}

fn collect_celery_task_declarations(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    tasks: &mut Vec<CeleryTask>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "decorated_definition"
        && let Some(definition) = named_child_of_kind(node, "function_definition")
        && let Some(name) = definition.child_by_field_name("name")
        && let Some(handler) = context.evidence.declarations.iter().find(|declaration| {
            matches!(declaration.kind.as_str(), "function" | "method")
                && declaration.range.start_byte == name.start_byte() as u64
                && declaration.range.end_byte == name.end_byte() as u64
        })
    {
        let mut cursor = node.walk();
        for decorator in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let Some((call, bind)) = exact_celery_task_decorator(decorator, context, receivers)
            else {
                continue;
            };
            let configured_name = call
                .and_then(|call| keyword_argument_node(call, "name", context.source))
                .and_then(|value| string_literal(node_text(value, context.source)));
            let queue = call
                .and_then(|call| keyword_argument_node(call, "queue", context.source))
                .and_then(|value| string_literal(node_text(value, context.source)));
            let task_name = configured_name.unwrap_or_else(|| handler.qualified_name.clone());
            let mut detail = Map::from_iter([
                (
                    "handler_reference".into(),
                    Value::String(handler.graph_node_id.clone()),
                ),
                ("pack_id".into(), Value::String("celery-python".to_owned())),
                ("bind".into(), Value::Bool(bind)),
            ]);
            if let Some(queue) = &queue {
                detail.insert("queue".into(), Value::String(queue.clone()));
            }
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "celery".to_owned(),
                kind: "job".to_owned(),
                name: task_name.clone(),
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, decorator),
                origin: RawFrameworkOrigin::Ast,
                detail,
            }));
            facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
                pack_id: "celery-python".to_owned(),
                framework: "celery".to_owned(),
                role: "consumer".to_owned(),
                subject_reference: Some(handler.graph_node_id.clone()),
                context: Some("task".to_owned()),
                anchor: anchor(context.path, decorator),
                origin: RawFrameworkOrigin::Ast,
                evidence_class: "exact".to_owned(),
                detail: Map::from_iter([
                    ("task_name".into(), Value::String(task_name)),
                    ("bind".into(), Value::Bool(bind)),
                ]),
            }));
            if let Some(queue) = queue {
                facts.push(celery_queue_fact(
                    context,
                    decorator,
                    &queue,
                    &handler.graph_node_id,
                    "consumes",
                ));
            }
            let task = CeleryTask {
                declaration_id: handler.id.clone(),
                graph_node_id: handler.graph_node_id.clone(),
                bind,
            };
            if bind {
                collect_celery_retry_facts(definition, context, &task, facts);
            }
            tasks.push(task);
            break;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_celery_task_declarations(child, context, receivers, tasks, facts);
    }
}

fn exact_celery_task_decorator<'tree>(
    decorator: Node<'tree>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
) -> Option<(Option<Node<'tree>>, bool)> {
    let expression = decorator.named_child(0)?;
    if expression.kind() == "call" {
        let function = expression.child_by_field_name("function")?;
        let scope = reference_evaluation_scope(context.evidence, function, false);
        if exact_static_reference_in_scope(context.evidence, function, context.source, scope)
            .as_deref()
            == Some("celery.shared_task")
        {
            let bind = call_arguments(expression, context.source)
                .and_then(|arguments| {
                    keyword_value(&arguments, "bind").and_then(static_python_bool)
                })
                .unwrap_or(false);
            return Some((Some(expression), bind));
        }
        if function.kind() == "attribute"
            && function
                .child_by_field_name("attribute")
                .is_some_and(|attribute| node_text(attribute, context.source) == "task")
            && let Some(object) = function.child_by_field_name("object")
            && let Some(receiver) = receiver_at(
                receivers,
                context.evidence,
                node_text(object, context.source),
                function.start_byte() as u64,
            )
            && receiver.framework == "celery"
        {
            let bind = call_arguments(expression, context.source)
                .and_then(|arguments| {
                    keyword_value(&arguments, "bind").and_then(static_python_bool)
                })
                .unwrap_or(false);
            return Some((Some(expression), bind));
        }
        return None;
    }
    let scope = reference_evaluation_scope(context.evidence, expression, false);
    (exact_static_reference_in_scope(context.evidence, expression, context.source, scope)
        .as_deref()
        == Some("celery.shared_task"))
    .then_some((None, false))
}

fn collect_celery_invocations(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    tasks: &[CeleryTask],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && function.kind() == "attribute"
        && exact_occurrence(context.evidence, SemanticRole::Call, function).is_some()
        && let (Some(object), Some(member)) = (
            function.child_by_field_name("object"),
            function.child_by_field_name("attribute"),
        )
    {
        let member = node_text(member, context.source);
        if matches!(member, "delay" | "apply_async" | "s" | "si" | "signature")
            && let Some(task) = exact_celery_task_reference(object, context, tasks)
            && let Some(owner) = call_owner_declaration(function, context.evidence)
        {
            let relation_context = if matches!(member, "s" | "si" | "signature") {
                celery_canvas_context(node, context)
                    .unwrap_or_else(|| format!("signature:{member}"))
            } else {
                format!("invocation:{member}")
            };
            let mut detail =
                Map::from_iter([("invocation".into(), Value::String(member.to_owned()))]);
            let queue = keyword_argument_node(node, "queue", context.source)
                .and_then(|value| string_literal(node_text(value, context.source)));
            if let Some(queue) = &queue {
                detail.insert("queue".into(), Value::String(queue.clone()));
            }
            append_celery_trigger_relation(
                facts,
                context,
                owner,
                task,
                &relation_context,
                node,
                detail,
            );
            if let Some(queue) = queue {
                facts.push(celery_queue_fact(
                    context,
                    node,
                    &queue,
                    &owner.graph_node_id,
                    "produces",
                ));
            }
        } else if member == "send_task"
            && let Some(receiver) = receiver_at(
                receivers,
                context.evidence,
                node_text(object, context.source),
                function.start_byte() as u64,
            )
            && receiver.framework == "celery"
            && let Some(task_name) = call_argument_node(node, context.source, 0, "name")
                .and_then(|value| string_literal(node_text(value, context.source)))
            && let Some(owner) = call_owner_declaration(function, context.evidence)
        {
            let queue = keyword_argument_node(node, "queue", context.source)
                .and_then(|value| string_literal(node_text(value, context.source)));
            let mut detail = Map::from_iter([
                ("transport".into(), Value::String("celery".to_owned())),
                ("subject".into(), Value::String(task_name.clone())),
                (
                    "handler_reference".into(),
                    Value::String(owner.graph_node_id.clone()),
                ),
                ("relationship".into(), Value::String("produces".to_owned())),
                ("pack_id".into(), Value::String("celery-python".to_owned())),
            ]);
            if let Some(queue) = &queue {
                detail.insert("queue".into(), Value::String(queue.clone()));
            }
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "celery".to_owned(),
                kind: "message".to_owned(),
                name: task_name,
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, node),
                origin: RawFrameworkOrigin::Ast,
                detail,
            }));
            if let Some(queue) = queue {
                facts.push(celery_queue_fact(
                    context,
                    node,
                    &queue,
                    &owner.graph_node_id,
                    "produces",
                ));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_celery_invocations(child, context, receivers, tasks, facts);
    }
}

fn exact_celery_task_reference<'a>(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    tasks: &'a [CeleryTask],
) -> Option<&'a CeleryTask> {
    let reference = exact_python_lexical_reference(context, node)?;
    let mut matches = tasks
        .iter()
        .filter(|task| task.graph_node_id == reference || task.declaration_id == reference);
    let task = matches.next()?;
    matches.next().is_none().then_some(task)
}

fn call_owner_declaration<'a>(
    function: Node<'_>,
    evidence: &'a SemanticEvidenceBatch,
) -> Option<&'a DeclarationFact> {
    let occurrence = exact_occurrence(evidence, SemanticRole::Call, function)?;
    let mut matches = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.id == occurrence.owner_declaration_id);
    let owner = matches.next()?;
    matches.next().is_none().then_some(owner)
}

fn celery_canvas_context(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "function_definition" | "class_definition") {
            break;
        }
        if parent.kind() == "call"
            && let Some(function) = parent.child_by_field_name("function")
            && let Some(target) = exact_call_target_in_scope(function, context)
            && matches!(
                target.as_str(),
                "celery.chain" | "celery.group" | "celery.chord"
            )
        {
            return Some(format!(
                "canvas:{}",
                target.rsplit('.').next().unwrap_or_default()
            ));
        }
        current = parent.parent();
    }
    None
}

fn exact_call_target_in_scope(
    function: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
) -> Option<String> {
    exact_python_lexical_reference(context, function)
}

fn exact_python_lexical_reference(
    context: &UniversalDetectionContext<'_, '_>,
    node: Node<'_>,
) -> Option<String> {
    exact_python_lexical_reference_evidence(context.evidence, node, context.source)
}

fn exact_python_lexical_reference_evidence(
    evidence: &SemanticEvidenceBatch,
    node: Node<'_>,
    source: &[u8],
) -> Option<String> {
    let reference = node_text(node, source).trim();
    if !is_identifier(reference) {
        return None;
    }
    let mut scope = reference_evaluation_scope(evidence, node, false);
    while let Some(scope_id) = scope {
        let declarations = evidence
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.name == reference
                    && declaration.range.start_byte < node.start_byte() as u64
                    && declaration.scope_id.as_deref() == Some(scope_id)
            })
            .collect::<Vec<_>>();
        let bindings = evidence
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                    && binding.spelling == reference
                    && binding.range.end_byte <= node.start_byte() as u64
                    && binding.scope_id.as_deref() == Some(scope_id)
            })
            .collect::<Vec<_>>();
        if !declarations.is_empty() || !bindings.is_empty() {
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
                ([], [binding])
                    if !has_intervening_python_name_binding(
                        context_root(node),
                        reference,
                        binding.range.end_byte,
                        node.start_byte() as u64,
                        source,
                    ) =>
                {
                    Some(binding.qualified_target.clone())
                }
                _ => None,
            };
        }
        scope = evidence
            .scopes
            .iter()
            .find(|candidate| candidate.id == scope_id)
            .and_then(|candidate| candidate.parent_scope_id.as_deref());
    }
    None
}

fn append_celery_trigger_relation(
    facts: &mut Vec<RawFrameworkFact>,
    context: &UniversalDetectionContext<'_, '_>,
    source: &DeclarationFact,
    target: &CeleryTask,
    relation_context: &str,
    node: Node<'_>,
    detail: Map<String, Value>,
) {
    let target_anchor = context
        .evidence
        .declarations
        .iter()
        .find(|declaration| declaration.id == target.declaration_id)
        .map(|declaration| evidence_anchor(&declaration.range));
    facts.push(RawFrameworkFact::Relation(RawFrameworkRelationFact {
        pack_id: "celery-python".to_owned(),
        framework: "celery".to_owned(),
        relation: "triggers".to_owned(),
        source_reference: Some(source.graph_node_id.clone()),
        target_hint: Some(target.graph_node_id.clone()),
        context: Some(relation_context.to_owned()),
        anchor: anchor(context.path, node),
        target_anchor,
        origin: RawFrameworkOrigin::Ast,
        evidence_class: "exact".to_owned(),
        ambiguity_policy: "require_exact".to_owned(),
        detail,
    }));
}

fn celery_queue_fact(
    context: &UniversalDetectionContext<'_, '_>,
    node: Node<'_>,
    queue: &str,
    handler_reference: &str,
    relationship: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Domain(RawDomainFact {
        framework: "celery".to_owned(),
        kind: "queue".to_owned(),
        name: queue.to_owned(),
        declaring_scope: module_scope(context.path),
        anchor: anchor(context.path, node),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            ("transport".into(), Value::String("celery".to_owned())),
            ("subject".into(), Value::String(queue.to_owned())),
            (
                "handler_reference".into(),
                Value::String(handler_reference.to_owned()),
            ),
            (
                "relationship".into(),
                Value::String(relationship.to_owned()),
            ),
            ("pack_id".into(), Value::String("celery-python".to_owned())),
        ]),
    })
}

fn collect_celery_retry_facts(
    definition: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    task: &CeleryTask,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if !task.bind {
        return;
    }
    let Some(parameters) = definition.child_by_field_name("parameters") else {
        return;
    };
    let Some((receiver_name, receiver_binding_end)) = first_parameter(parameters, context.source)
    else {
        return;
    };
    collect_celery_retry_nodes(
        definition,
        definition,
        context,
        task,
        &receiver_name,
        receiver_binding_end,
        facts,
    );
}

fn collect_celery_retry_nodes(
    root: Node<'_>,
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    task: &CeleryTask,
    receiver_name: &str,
    receiver_binding_end: u64,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.id() != root.id() && matches!(node.kind(), "function_definition" | "class_definition") {
        return;
    }
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && function.kind() == "attribute"
        && function
            .child_by_field_name("object")
            .is_some_and(|object| node_text(object, context.source) == receiver_name)
        && function
            .child_by_field_name("attribute")
            .is_some_and(|member| node_text(member, context.source) == "retry")
        && !has_python_name_binding_between(
            definition_body(root).unwrap_or(root),
            receiver_name,
            receiver_binding_end,
            function.start_byte() as u64,
            context.source,
        )
        && let Some(source) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == task.declaration_id)
    {
        append_celery_trigger_relation(
            facts,
            context,
            source,
            task,
            "retry",
            node,
            Map::from_iter([("invocation".into(), Value::String("retry".to_owned()))]),
        );
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_celery_retry_nodes(
            root,
            child,
            context,
            task,
            receiver_name,
            receiver_binding_end,
            facts,
        );
    }
}

fn first_parameter(parameters: Node<'_>, source: &[u8]) -> Option<(String, u64)> {
    let mut cursor = parameters.walk();
    for parameter in parameters
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if parameter.kind() == "identifier" {
            return Some((
                node_text(parameter, source).to_owned(),
                parameter.end_byte() as u64,
            ));
        }
        if let Some(name) = parameter.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            return Some((node_text(name, source).to_owned(), name.end_byte() as u64));
        }
    }
    None
}

fn definition_body(definition: Node<'_>) -> Option<Node<'_>> {
    definition.child_by_field_name("body")
}

fn has_python_name_binding_between(
    node: Node<'_>,
    reference: &str,
    binding_end: u64,
    use_start: u64,
    source: &[u8],
) -> bool {
    if node.end_byte() as u64 <= binding_end || node.start_byte() as u64 >= use_start {
        return false;
    }
    if matches!(
        node.kind(),
        "function_definition" | "class_definition" | "decorated_definition"
    ) {
        return false;
    }
    if node.start_byte() as u64 > binding_end
        && node.end_byte() as u64 <= use_start
        && crate::engine::python_bound_names(node, source, true).contains(reference)
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .any(|child| {
            has_python_name_binding_between(child, reference, binding_end, use_start, source)
        })
}

fn collect_celery_beat_schedules(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "assignment"
        && let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        )
        && right.kind() == "dictionary"
        && let Some(receiver_name) =
            node_text(left, context.source).strip_suffix(".conf.beat_schedule")
        && is_identifier(receiver_name)
        && let Some(receiver) = receiver_at(
            receivers,
            context.evidence,
            receiver_name,
            left.start_byte() as u64,
        )
        && receiver.framework == "celery"
    {
        let mut cursor = right.walk();
        for pair in right
            .children(&mut cursor)
            .filter(|child| child.kind() == "pair")
        {
            let (Some(key), Some(value)) = (
                pair.child_by_field_name("key"),
                pair.child_by_field_name("value"),
            ) else {
                continue;
            };
            let Some(schedule_name) = string_literal(node_text(key, context.source)) else {
                continue;
            };
            let Some(task_node) = dictionary_value_node(value, "task", context.source) else {
                continue;
            };
            let Some(task_name) = string_literal(node_text(task_node, context.source)) else {
                continue;
            };
            let Some(schedule_node) = dictionary_value_node(value, "schedule", context.source)
            else {
                continue;
            };
            let Some(schedule) = exact_celery_schedule_value(schedule_node, context) else {
                continue;
            };
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "celery".to_owned(),
                kind: "job".to_owned(),
                name: schedule_name,
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, pair),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    ("scheduled_task".into(), Value::String(task_name)),
                    ("schedule".into(), Value::String(schedule)),
                    ("pack_id".into(), Value::String("celery-python".to_owned())),
                ]),
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_celery_beat_schedules(child, context, receivers, facts);
    }
}

fn dictionary_value_node<'tree>(
    dictionary: Node<'tree>,
    key: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    if dictionary.kind() != "dictionary" {
        return None;
    }
    let mut cursor = dictionary.walk();
    let mut matches = dictionary.children(&mut cursor).filter_map(|pair| {
        if pair.kind() != "pair" {
            return None;
        }
        let pair_key = pair.child_by_field_name("key")?;
        (string_literal(node_text(pair_key, source)).as_deref() == Some(key))
            .then(|| pair.child_by_field_name("value"))
            .flatten()
    });
    let value = matches.next()?;
    matches.next().is_none().then_some(value)
}

fn exact_celery_schedule_value(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
) -> Option<String> {
    if matches!(node.kind(), "integer" | "float" | "string") {
        return Some(node_text(node, context.source).to_owned());
    }
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(target) = exact_call_target_in_scope(function, context)
        && matches!(
            target.as_str(),
            "celery.schedules.crontab" | "celery.schedules.solar"
        )
    {
        return Some(node_text(node, context.source).to_owned());
    }
    None
}

fn collect_flask_receiver_stages(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &mut [Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "decorated_definition" {
        let mut cursor = node.walk();
        for decorator in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let Some(occurrence) =
                exact_occurrence(context.evidence, SemanticRole::Decorator, decorator)
            else {
                continue;
            };
            let Some(handler) = context
                .evidence
                .declarations
                .iter()
                .find(|declaration| declaration.id == occurrence.owner_declaration_id)
            else {
                continue;
            };
            let expression = decorator.named_child(0);
            let (function, arguments) = if let Some(expression) = expression
                && expression.kind() == "call"
            {
                (
                    expression.child_by_field_name("function"),
                    call_arguments(expression, context.source),
                )
            } else {
                (expression, Some(Vec::new()))
            };
            let (Some(function), Some(arguments)) = (function, arguments) else {
                continue;
            };
            let Some((receiver_name, hook)) = node_text(function, context.source).rsplit_once('.')
            else {
                continue;
            };
            let Some(receiver) = receiver_at(
                receivers,
                context.evidence,
                receiver_name,
                decorator.start_byte() as u64,
            ) else {
                continue;
            };
            if receiver.framework != "flask" {
                continue;
            }
            let receiver_id = receiver.declaration_id.clone();
            let role = match hook {
                "before_request"
                | "before_app_request"
                | "after_request"
                | "after_app_request"
                | "teardown_request"
                | "teardown_app_request" => RawRouteStageRole::Middleware,
                "errorhandler" | "app_errorhandler" => {
                    if positional_argument(&arguments, 0)
                        .filter(|value| static_flask_error_code(value))
                        .is_none()
                    {
                        continue;
                    }
                    RawRouteStageRole::ErrorBoundary
                }
                _ => continue,
            };
            append_flask_hook_stage(
                receivers,
                &receiver_id,
                handler,
                hook,
                role,
                anchor(context.path, decorator),
                facts,
            );
        }
    }
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_occurrence(context.evidence, SemanticRole::Call, function).is_some()
        && let Some((receiver_name, hook)) = node_text(function, context.source).rsplit_once('.')
        && let Some(receiver) = receiver_at(
            receivers,
            context.evidence,
            receiver_name,
            node.start_byte() as u64,
        )
        && receiver.framework == "flask"
    {
        let receiver_id = receiver.declaration_id.clone();
        let handler_position = match hook {
            "before_request" | "after_request" | "teardown_request" => 0,
            "register_error_handler" => 1,
            _ => usize::MAX,
        };
        if handler_position != usize::MAX
            && (hook != "register_error_handler"
                || call_argument_node(node, context.source, 0, "code_or_exception")
                    .is_some_and(|value| static_flask_error_code(node_text(value, context.source))))
            && let Some(handler_node) =
                call_argument_node(node, context.source, handler_position, "f")
            && let Some(handler) = exact_reference_declaration(context.evidence, handler_node)
        {
            let role = if hook == "register_error_handler" {
                RawRouteStageRole::ErrorBoundary
            } else {
                RawRouteStageRole::Middleware
            };
            append_flask_hook_stage(
                receivers,
                &receiver_id,
                handler,
                hook,
                role,
                anchor(context.path, node),
                facts,
            );
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_flask_receiver_stages(child, context, receivers, facts);
    }
}

fn append_flask_hook_stage(
    receivers: &mut [Receiver],
    receiver_id: &str,
    handler: &DeclarationFact,
    hook: &str,
    role: RawRouteStageRole,
    hook_anchor: RawFrameworkAnchor,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let Some(receiver) = receivers
        .iter_mut()
        .find(|receiver| receiver.declaration_id == receiver_id)
    else {
        return;
    };
    receiver.stages.push(RawRouteStageFact {
        role,
        position: u32::try_from(receiver.stages.len()).unwrap_or(u32::MAX),
        reference: handler.graph_node_id.clone(),
        anchor: hook_anchor.clone(),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([("hook".into(), Value::String(hook.to_owned()))]),
    });
    facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
        pack_id: "flask-python".to_owned(),
        framework: "flask".to_owned(),
        role: "hook".to_owned(),
        subject_reference: Some(handler.graph_node_id.clone()),
        context: Some(hook.to_owned()),
        anchor: hook_anchor,
        origin: RawFrameworkOrigin::Ast,
        evidence_class: "exact".to_owned(),
        detail: Map::from_iter([("receiver_id".into(), Value::String(receiver_id.to_owned()))]),
    }));
}

fn static_flask_error_code(value: &str) -> bool {
    let value = value.trim();
    value.parse::<u16>().is_ok() || string_literal(value).is_some()
}

fn collect_flask_factory_roles(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "function_definition"
        && let Some(name) = node.child_by_field_name("name")
        && let Some(factory) = context.evidence.declarations.iter().find(|declaration| {
            declaration.kind == "function"
                && declaration.range.start_byte == name.start_byte() as u64
                && declaration.range.end_byte == name.end_byte() as u64
        })
        && let Some(body) = node.child_by_field_name("body")
    {
        let mut cursor = body.walk();
        let returned = body
            .children(&mut cursor)
            .filter(|statement| statement.kind() == "return_statement")
            .filter_map(|statement| statement.named_child(0).map(|value| (statement, value)))
            .filter_map(|(statement, value)| {
                receiver_at(
                    receivers,
                    context.evidence,
                    node_text(value, context.source),
                    statement.start_byte() as u64,
                )
                .filter(|receiver| receiver.framework == "flask")
                .map(|receiver| (statement, receiver))
            })
            .collect::<Vec<_>>();
        if let [(statement, receiver)] = returned.as_slice() {
            facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
                pack_id: "flask-python".to_owned(),
                framework: "flask".to_owned(),
                role: "service".to_owned(),
                subject_reference: Some(factory.graph_node_id.clone()),
                context: Some("application_factory".to_owned()),
                anchor: anchor(context.path, *statement),
                origin: RawFrameworkOrigin::Ast,
                evidence_class: "exact".to_owned(),
                detail: Map::from_iter([(
                    "receiver_id".into(),
                    Value::String(receiver.declaration_id.clone()),
                )]),
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if child.kind() != "function_definition" || node.kind() != "function_definition" {
            collect_flask_factory_roles(child, context, receivers, facts);
        }
    }
}

fn detect_universal(
    context: &UniversalDetectionContext<'_, '_>,
    framework: PythonFramework,
) -> Vec<RawFrameworkFact> {
    let mut facts = Vec::new();
    if framework == PythonFramework::Django {
        let receivers = receiver_declarations(context);
        let flow =
            django_pattern_flow(context.root, context.source, context.path, context.evidence);
        collect_django_routes(
            context.root,
            context.source,
            context.path,
            context.evidence,
            &receivers,
            &flow,
            &mut facts,
        );
        collect_imported_django_pattern_routes(context.path, &flow, &mut facts);
    } else {
        let mut receivers = receiver_declarations(context);
        if framework == PythonFramework::Flask {
            collect_flask_receiver_stages(context.root, context, &mut receivers, &mut facts);
        }
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
    receivers: &[Receiver],
    flow: &DjangoPatternFlow,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && flow.calls.contains(&node.id())
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
            let is_drf_router_include = include_call
                .and_then(|include_call| {
                    call_argument_node(include_call, source, 0, "urlconf_module")
                })
                .and_then(|router_urls| {
                    exact_local_drf_router_urls(router_urls, source, evidence, receivers)
                })
                .is_some();
            let handler_reference = if let Some(include_call) = include_call {
                let Some(include) =
                    django_include_target(include_call, source, path, evidence, flow)
                else {
                    return;
                };
                detail.insert("include".into(), Value::String(include.target.clone()));
                if let Some(collection) = include.collection {
                    detail.insert("include_collection".into(), Value::String(collection));
                }
                if let Some(application_name) = include.application_name {
                    detail.insert("application_name".into(), Value::String(application_name));
                }
                if let Some(namespace) = include.namespace {
                    detail.insert("namespace".into(), Value::String(namespace));
                }
                format!("@include:{}", include.target)
            } else {
                string_literal(handler).unwrap_or_else(|| handler.to_owned())
            };
            if let Some(collection) = flow.collections_by_call.get(&node.id()) {
                detail.insert(
                    "django_collection".into(),
                    Value::String(collection.clone()),
                );
            }
            if flow.i18n_calls.contains(&node.id()) {
                detail.insert("i18n".into(), Value::Bool(true));
            }
            if !handler_reference.is_empty() && !is_drf_router_include {
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
        collect_django_routes(child, source, path, evidence, receivers, flow, facts);
    }
}

fn django_pattern_flow(
    root: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
) -> DjangoPatternFlow {
    let mut definitions = BTreeMap::<String, Vec<Node<'_>>>::new();
    let mut roots = Vec::<Node<'_>>::new();
    let mut root_assignment_count = 0_usize;
    let mut root_start = None;
    let mut cursor = root.walk();
    for statement in root.children(&mut cursor).filter(|child| child.is_named()) {
        if statement.kind() == "assignment"
            && let (Some(left), Some(right)) = (
                statement.child_by_field_name("left"),
                statement.child_by_field_name("right"),
            )
            && left.kind() == "identifier"
        {
            let name = node_text(left, source).to_owned();
            if name == "urlpatterns" {
                root_assignment_count = root_assignment_count.saturating_add(1);
                root_start = Some(statement.start_byte());
                roots.push(right);
            } else {
                definitions.entry(name).or_default().push(right);
            }
        } else if statement.kind() == "augmented_assignment"
            && let (Some(left), Some(right)) = (
                statement.child_by_field_name("left"),
                statement.child_by_field_name("right"),
            )
            && left.kind() == "identifier"
            && node_text(left, source) == "urlpatterns"
            && root_start.is_some_and(|start| start < statement.start_byte())
            && (node_has_unnamed_child(statement, "+") || node_has_unnamed_child(statement, "+="))
        {
            roots.push(right);
        }
    }
    if root_assignment_count != 1 {
        return DjangoPatternFlow::default();
    }
    let mut flow = DjangoPatternFlow::default();
    let mut visiting = BTreeSet::new();
    for root in roots {
        if !collect_django_pattern_expression(
            root,
            "urlpatterns",
            false,
            source,
            path,
            evidence,
            &definitions,
            &mut visiting,
            &mut flow,
        ) {
            return DjangoPatternFlow::default();
        }
    }
    flow
}

#[allow(clippy::too_many_arguments)]
fn collect_django_pattern_expression<'tree>(
    node: Node<'tree>,
    collection: &str,
    in_i18n: bool,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    definitions: &BTreeMap<String, Vec<Node<'tree>>>,
    visiting: &mut BTreeSet<String>,
    flow: &mut DjangoPatternFlow,
) -> bool {
    match node.kind() {
        "list" | "tuple" | "set" | "parenthesized_expression" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                let _ = collect_django_pattern_expression(
                    child,
                    collection,
                    in_i18n,
                    source,
                    path,
                    evidence,
                    definitions,
                    visiting,
                    flow,
                );
            }
            true
        }
        "binary_operator" if node_has_unnamed_child(node, "+") => {
            let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) else {
                return false;
            };
            let _ = collect_django_pattern_expression(
                left,
                collection,
                in_i18n,
                source,
                path,
                evidence,
                definitions,
                visiting,
                flow,
            );
            let _ = collect_django_pattern_expression(
                right,
                collection,
                in_i18n,
                source,
                path,
                evidence,
                definitions,
                visiting,
                flow,
            );
            true
        }
        "identifier" => {
            let name = node_text(node, source);
            let Some([definition]) = definitions.get(name).map(Vec::as_slice) else {
                let Some(target) = exact_import_reference_hint(evidence, node, source) else {
                    return false;
                };
                let (target, imported_collection) = imported_django_collection_target(&target);
                flow.imported_collections
                    .push(DjangoImportedPatternCollection {
                        target,
                        collection: imported_collection,
                        in_i18n,
                        anchor: anchor(path, node),
                    });
                return true;
            };
            if !visiting.insert(name.to_owned()) {
                return false;
            }
            flow.local_collections.insert(name.to_owned());
            let complete = collect_django_pattern_expression(
                *definition,
                name,
                in_i18n,
                source,
                path,
                evidence,
                definitions,
                visiting,
                flow,
            );
            visiting.remove(name);
            complete
        }
        "call" => {
            let Some(function) = node.child_by_field_name("function") else {
                return false;
            };
            let Some(target) = exact_call_target(evidence, function) else {
                return false;
            };
            if matches!(
                target.as_str(),
                "django.urls.path"
                    | "django.urls.re_path"
                    | "django.conf.urls.url"
                    | "django.conf.urls.path"
                    | "django.conf.urls.re_path"
            ) {
                flow.calls.insert(node.id());
                flow.collections_by_call
                    .insert(node.id(), collection.to_owned());
                if in_i18n {
                    flow.i18n_calls.insert(node.id());
                }
                if let Some(local_collection) =
                    django_local_include_collection(node, source, evidence, definitions)
                {
                    if !visiting.insert(local_collection.0.to_owned()) {
                        return false;
                    }
                    flow.local_collections.insert(local_collection.0.to_owned());
                    let complete = collect_django_pattern_expression(
                        local_collection.1,
                        local_collection.0,
                        in_i18n,
                        source,
                        path,
                        evidence,
                        definitions,
                        visiting,
                        flow,
                    );
                    visiting.remove(local_collection.0);
                    return complete;
                }
                return true;
            }
            if target != "django.conf.urls.i18n.i18n_patterns" {
                return false;
            }
            let Some(arguments) = node.child_by_field_name("arguments") else {
                return false;
            };
            let mut cursor = arguments.walk();
            arguments
                .children(&mut cursor)
                .filter(|child| child.is_named())
                .all(|argument| {
                    if argument.kind() == "keyword_argument" {
                        return argument.child_by_field_name("name").is_some_and(|name| {
                            node_text(name, source) == "prefix_default_language"
                        }) && argument.child_by_field_name("value").is_some_and(|value| {
                            static_python_bool(node_text(value, source)).is_some()
                        });
                    }
                    collect_django_pattern_expression(
                        argument,
                        collection,
                        true,
                        source,
                        path,
                        evidence,
                        definitions,
                        visiting,
                        flow,
                    )
                })
        }
        _ => false,
    }
}

fn imported_django_collection_target(target: &str) -> (String, Option<String>) {
    if let Some(module) = target.strip_suffix(".urlpatterns") {
        return (module.to_owned(), None);
    }
    target.rsplit_once('.').map_or_else(
        || (target.to_owned(), None),
        |(module, collection)| (module.to_owned(), Some(collection.to_owned())),
    )
}

fn collect_imported_django_pattern_routes(
    path: &Path,
    flow: &DjangoPatternFlow,
    facts: &mut Vec<RawFrameworkFact>,
) {
    for imported in &flow.imported_collections {
        let mut detail = Map::from_iter([
            ("include".into(), Value::String(imported.target.clone())),
            (
                "django_collection".into(),
                Value::String("urlpatterns".to_owned()),
            ),
            ("imported_pattern_collection".into(), Value::Bool(true)),
        ]);
        if let Some(collection) = &imported.collection {
            detail.insert(
                "include_collection".into(),
                Value::String(collection.clone()),
            );
        }
        if imported.in_i18n {
            detail.insert("i18n".into(), Value::Bool(true));
        }
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "django".to_owned(),
            operation: "ANY".to_owned(),
            raw_path: "/".to_owned(),
            normalized_path: "/".to_owned(),
            declaring_scope: module_scope(path),
            anchor: imported.anchor.clone(),
            handler_reference: format!("@include:{}", imported.target),
            middleware_references: Vec::new(),
            stages: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: None,
            detail,
        }));
    }
}

fn django_local_include_collection<'tree, 'source>(
    route_call: Node<'tree>,
    source: &'source [u8],
    evidence: &SemanticEvidenceBatch,
    definitions: &BTreeMap<String, Vec<Node<'tree>>>,
) -> Option<(&'source str, Node<'tree>)> {
    let handler = call_argument_node(route_call, source, 1, "view")?;
    let include_call = find_exact_call(handler, evidence, "django.urls.include")
        .or_else(|| find_exact_call(handler, evidence, "django.conf.urls.include"))?;
    let argument = call_argument_node(include_call, source, 0, "urlconf_module")?;
    let collection_node = if argument.kind() == "tuple" {
        let mut cursor = argument.walk();
        argument
            .children(&mut cursor)
            .find(|child| child.is_named())?
    } else {
        argument
    };
    if collection_node.kind() != "identifier" {
        return None;
    }
    let name = node_text(collection_node, source);
    let [definition] = definitions.get(name)?.as_slice() else {
        return None;
    };
    Some((name, *definition))
}

fn django_include_target(
    include_call: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
    flow: &DjangoPatternFlow,
) -> Option<DjangoInclude> {
    let arguments = call_arguments(include_call, source)?;
    let namespace = optional_literal_keyword(&arguments, "namespace")?;
    let argument = call_argument_node(include_call, source, 0, "urlconf_module")?;
    let (target_node, application_name) = if argument.kind() == "tuple" {
        let mut cursor = argument.walk();
        let values = argument
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .collect::<Vec<_>>();
        let target = *values.first()?;
        let application_name = match values.get(1) {
            Some(value) => Some(string_literal(node_text(*value, source))?),
            None => None,
        };
        if values.len() > 2 {
            return None;
        }
        (target, application_name)
    } else {
        (argument, None)
    };
    let text = node_text(target_node, source).trim();
    if let Some(target) = string_literal(text) {
        return Some(DjangoInclude {
            target,
            collection: None,
            application_name,
            namespace,
        });
    }
    if is_identifier(text) && flow.local_collections.contains(text) {
        return Some(DjangoInclude {
            target: module_scope(path),
            collection: Some(text.to_owned()),
            application_name,
            namespace,
        });
    }
    exact_static_reference_hint(evidence, target_node, source).map(|target| DjangoInclude {
        target,
        collection: None,
        application_name,
        namespace,
    })
}

fn node_has_unnamed_child(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == kind)
}

fn collect_drf_router_registrations(
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    let viewsets = python_descendants_of(
        context.evidence,
        &[
            "rest_framework.viewsets.ViewSet",
            "rest_framework.viewsets.GenericViewSet",
            "rest_framework.viewsets.ModelViewSet",
            "rest_framework.viewsets.ReadOnlyModelViewSet",
        ],
    );
    collect_drf_router_registration_nodes(context.root, context, receivers, &viewsets, facts);
}

fn collect_drf_router_registration_nodes(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    viewsets: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
        && exact_occurrence(context.evidence, SemanticRole::Call, function).is_some()
        && let Some((receiver_name, "register")) =
            node_text(function, context.source).rsplit_once('.')
        && let Some(receiver) = receiver_at(
            receivers,
            context.evidence,
            receiver_name,
            node.start_byte() as u64,
        )
        && receiver.framework == "django-rest-framework"
        && let Some(arguments) = call_arguments(node, context.source)
        && let Some(prefix) = positional_argument(&arguments, 0)
            .and_then(string_literal)
            .or_else(|| keyword_string(&arguments, "prefix"))
        && let Some(viewset_node) = call_argument_node(node, context.source, 1, "viewset")
        && let Some(viewset_reference) =
            exact_static_reference_hint(context.evidence, viewset_node, context.source)
        && let Some(viewset) =
            unique_declaration_for_reference(context.evidence, &viewset_reference)
        && viewsets.contains(viewset.id.as_str())
    {
        let basename = positional_argument(&arguments, 2)
            .and_then(string_literal)
            .or_else(|| keyword_string(&arguments, "basename"));
        let lookup_parameter = drf_viewset_lookup_parameter(context, viewset);
        if let (Some(basename), Some(lookup_parameter)) = (basename, lookup_parameter) {
            let methods = drf_viewset_method_templates(context, viewset);
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "django-rest-framework".to_owned(),
                kind: "drf_router_registration".to_owned(),
                name: format!("{}:{prefix}", receiver.qualified_name),
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "pack_id".into(),
                        Value::String("django-rest-framework-python".to_owned()),
                    ),
                    (
                        "router_receiver_id".into(),
                        Value::String(receiver.declaration_id.clone()),
                    ),
                    (
                        "router_receiver_qualified_name".into(),
                        Value::String(receiver.qualified_name.clone()),
                    ),
                    (
                        "router_template".into(),
                        Value::String(match receiver.constructor.as_str() {
                            "rest_framework.routers.DefaultRouter" => {
                                "drf-default-router-v1".to_owned()
                            }
                            _ => "drf-simple-router-v1".to_owned(),
                        }),
                    ),
                    ("prefix".into(), Value::String(prefix)),
                    (
                        "viewset_declaration_id".into(),
                        Value::String(viewset.id.clone()),
                    ),
                    (
                        "viewset_reference".into(),
                        Value::String(viewset.graph_node_id.clone()),
                    ),
                    ("methods".into(), Value::Array(methods)),
                    ("lookup_parameter".into(), Value::String(lookup_parameter)),
                    ("basename".into(), Value::String(basename)),
                ]),
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_drf_router_registration_nodes(child, context, receivers, viewsets, facts);
    }
}

fn collect_drf_router_mounts(
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    flow: &DjangoPatternFlow,
    facts: &mut Vec<RawFrameworkFact>,
) {
    collect_drf_router_mount_nodes(context.root, context, receivers, flow, facts);
    collect_direct_drf_router_urlpatterns(context, receivers, facts);
}

fn collect_drf_router_mount_nodes(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    flow: &DjangoPatternFlow,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call"
        && flow.calls.contains(&node.id())
        && let Some(function) = node.child_by_field_name("function")
        && let Some(target) = exact_call_target(context.evidence, function)
        && matches!(target.as_str(), "django.urls.path" | "django.urls.re_path")
        && let Some(arguments) = call_arguments(node, context.source)
        && let Some(prefix) = positional_argument(&arguments, 0)
            .and_then(string_literal)
            .or_else(|| keyword_string(&arguments, "route"))
        && let Some(handler_node) = call_argument_node(node, context.source, 1, "view")
        && let Some(include_call) =
            find_exact_call(handler_node, context.evidence, "django.urls.include")
        && let Some(router_urls) =
            call_argument_node(include_call, context.source, 0, "urlconf_module")
        && let Some(receiver) =
            exact_local_drf_router_urls(router_urls, context.source, context.evidence, receivers)
    {
        let include_arguments = call_arguments(include_call, context.source).unwrap_or_default();
        let namespace = optional_literal_keyword(&include_arguments, "namespace");
        if let Some(namespace) = namespace {
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "django-rest-framework".to_owned(),
                kind: "drf_router_mount".to_owned(),
                name: receiver.qualified_name.clone(),
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "pack_id".into(),
                        Value::String("django-rest-framework-python".to_owned()),
                    ),
                    (
                        "router_receiver_id".into(),
                        Value::String(receiver.declaration_id.clone()),
                    ),
                    (
                        "router_receiver_qualified_name".into(),
                        Value::String(receiver.qualified_name.clone()),
                    ),
                    (
                        "mount_prefix".into(),
                        Value::String(normalize_django_path(&prefix, "path")),
                    ),
                    (
                        "namespace".into(),
                        namespace.map_or(Value::Null, Value::String),
                    ),
                ]),
            }));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_drf_router_mount_nodes(child, context, receivers, flow, facts);
    }
}

fn collect_direct_drf_router_urlpatterns(
    context: &UniversalDetectionContext<'_, '_>,
    receivers: &[Receiver],
    facts: &mut Vec<RawFrameworkFact>,
) {
    let mut assignments = Vec::new();
    let mut cursor = context.root.walk();
    for statement in context
        .root
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        if statement.kind() == "assignment"
            && let (Some(left), Some(right)) = (
                statement.child_by_field_name("left"),
                statement.child_by_field_name("right"),
            )
            && left.kind() == "identifier"
            && node_text(left, context.source) == "urlpatterns"
        {
            assignments.push((statement, right));
        }
    }
    let [(assignment, right)] = assignments.as_slice() else {
        return;
    };
    let Some(receiver) =
        exact_local_drf_router_urls(*right, context.source, context.evidence, receivers)
    else {
        return;
    };
    append_drf_router_mount(context, receiver, *assignment, String::new(), None, facts);
}

fn append_drf_router_mount(
    context: &UniversalDetectionContext<'_, '_>,
    receiver: &Receiver,
    source_node: Node<'_>,
    prefix: String,
    namespace: Option<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    facts.push(RawFrameworkFact::Domain(RawDomainFact {
        framework: "django-rest-framework".to_owned(),
        kind: "drf_router_mount".to_owned(),
        name: receiver.qualified_name.clone(),
        declaring_scope: module_scope(context.path),
        anchor: anchor(context.path, source_node),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            (
                "pack_id".into(),
                Value::String("django-rest-framework-python".to_owned()),
            ),
            (
                "router_receiver_id".into(),
                Value::String(receiver.declaration_id.clone()),
            ),
            (
                "router_receiver_qualified_name".into(),
                Value::String(receiver.qualified_name.clone()),
            ),
            ("mount_prefix".into(), Value::String(prefix)),
            (
                "namespace".into(),
                namespace.map_or(Value::Null, Value::String),
            ),
        ]),
    }));
}

fn exact_local_drf_router_urls<'a>(
    node: Node<'_>,
    source: &[u8],
    evidence: &SemanticEvidenceBatch,
    receivers: &'a [Receiver],
) -> Option<&'a Receiver> {
    let node = if node.kind() == "tuple" {
        let mut cursor = node.walk();
        let values = node
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .collect::<Vec<_>>();
        if values.len() != 2 || string_literal(node_text(values[1], source)).is_none() {
            return None;
        }
        values[0]
    } else {
        node
    };
    let reference = node_text(node, source).trim();
    let receiver_name = reference.strip_suffix(".urls")?;
    if !is_identifier(receiver_name) {
        return None;
    }
    receiver_at(receivers, evidence, receiver_name, node.start_byte() as u64)
        .filter(|receiver| receiver.framework == "django-rest-framework")
}

fn collect_drf_viewset_and_serializer_facts(
    context: &UniversalDetectionContext<'_, '_>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let viewsets = python_descendants_of(
        context.evidence,
        &[
            "rest_framework.viewsets.ViewSet",
            "rest_framework.viewsets.GenericViewSet",
            "rest_framework.viewsets.ModelViewSet",
            "rest_framework.viewsets.ReadOnlyModelViewSet",
        ],
    );
    let serializers = python_descendants_of(
        context.evidence,
        &[
            "rest_framework.serializers.Serializer",
            "rest_framework.serializers.ModelSerializer",
        ],
    );
    let django_models = python_descendants_of(context.evidence, &["django.db.models.Model"]);
    for viewset_id in &viewsets {
        let Some(viewset) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == *viewset_id)
        else {
            continue;
        };
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "django-rest-framework-python".to_owned(),
            framework: "django-rest-framework".to_owned(),
            role: "controller".to_owned(),
            subject_reference: Some(viewset.graph_node_id.clone()),
            context: viewset.module_or_package.clone(),
            anchor: evidence_anchor(&viewset.range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::from_iter([("declaration_id".into(), Value::String(viewset.id.clone()))]),
        }));
        let Some(definition) = declaration_node(context.root, viewset) else {
            continue;
        };
        for (field, role) in [
            ("serializer_class", None),
            ("permission_classes", Some("middleware")),
            ("authentication_classes", Some("middleware")),
            ("filter_backends", Some("service")),
            ("throttle_classes", Some("service")),
        ] {
            let Some(value) = direct_class_assignment_value(definition, field, context.source)
            else {
                continue;
            };
            for target in exact_static_references_in_scope(
                context.evidence,
                value,
                context.source,
                viewset.scope_id.as_deref(),
            ) {
                append_exact_framework_relation(
                    facts,
                    "django-rest-framework-python",
                    "django-rest-framework",
                    viewset,
                    &target,
                    field,
                    anchor(context.path, value),
                    context.evidence,
                );
                if let Some(role) = role {
                    facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
                        pack_id: "django-rest-framework-python".to_owned(),
                        framework: "django-rest-framework".to_owned(),
                        role: role.to_owned(),
                        subject_reference: Some(target.clone()),
                        context: Some(field.to_owned()),
                        anchor: anchor(context.path, value),
                        origin: RawFrameworkOrigin::Ast,
                        evidence_class: "exact".to_owned(),
                        detail: Map::new(),
                    }));
                }
            }
        }
    }
    for serializer_id in &serializers {
        let Some(serializer) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == *serializer_id)
        else {
            continue;
        };
        let Some(definition) = declaration_node(context.root, serializer) else {
            continue;
        };
        let Some(meta) = direct_nested_class(definition, "Meta", context.source) else {
            continue;
        };
        let Some(model_node) = direct_class_assignment_value(meta, "model", context.source) else {
            continue;
        };
        let targets = exact_static_references_in_scope(
            context.evidence,
            model_node,
            context.source,
            serializer.scope_id.as_deref(),
        );
        if let [target] = targets.as_slice()
            && let Some(model) = unique_declaration_for_reference(context.evidence, target)
            && django_models.contains(model.id.as_str())
        {
            append_exact_framework_relation(
                facts,
                "django-rest-framework-python",
                "django-rest-framework",
                serializer,
                target,
                "serializer_model",
                anchor(context.path, model_node),
                context.evidence,
            );
        }
    }
}

fn collect_django_model_facts(
    context: &UniversalDetectionContext<'_, '_>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let models = python_descendants_of(context.evidence, &["django.db.models.Model"]);
    let managers = python_descendants_of(context.evidence, &["django.db.models.Manager"]);
    for manager_id in &managers {
        let Some(manager) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == *manager_id)
        else {
            continue;
        };
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "django-python".to_owned(),
            framework: "django".to_owned(),
            role: "service".to_owned(),
            subject_reference: Some(manager.graph_node_id.clone()),
            context: Some("model_manager".to_owned()),
            anchor: evidence_anchor(&manager.range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::from_iter([("declaration_id".into(), Value::String(manager.id.clone()))]),
        }));
    }
    for model_id in &models {
        let Some(model) = context
            .evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == *model_id)
        else {
            continue;
        };
        let Some(definition) = declaration_node(context.root, model) else {
            continue;
        };
        let fields = django_model_field_facts(definition, context, model);
        facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "django-python".to_owned(),
            framework: "django".to_owned(),
            role: "model".to_owned(),
            subject_reference: Some(model.graph_node_id.clone()),
            context: model.module_or_package.clone(),
            anchor: evidence_anchor(&model.range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::from_iter([
                ("declaration_id".into(), Value::String(model.id.clone())),
                ("fields".into(), Value::Array(fields)),
            ]),
        }));
        if let Some(meta) = direct_nested_class(definition, "Meta", context.source)
            && let Some(table_node) =
                direct_class_assignment_value(meta, "db_table", context.source)
            && let Some(table_name) = string_literal(node_text(table_node, context.source))
        {
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "django-orm".to_owned(),
                kind: "orm_mapping".to_owned(),
                name: model.name.clone(),
                declaring_scope: module_scope(context.path),
                anchor: anchor(context.path, table_node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "model_reference".into(),
                        Value::String(model.graph_node_id.clone()),
                    ),
                    ("database_table".into(), Value::String(table_name)),
                    ("database_schema".into(), Value::String(String::new())),
                    ("explicit".into(), Value::Bool(true)),
                    ("pack_id".into(), Value::String("django-python".to_owned())),
                ]),
            }));
        }
        collect_django_model_relationships(definition, context, model, &models, &managers, facts);
    }
}

fn collect_django_model_relationships(
    definition: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    model: &DeclarationFact,
    models: &BTreeSet<String>,
    managers: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let Some(body) = definition.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for assignment in body
        .children(&mut cursor)
        .filter(|child| matches!(child.kind(), "assignment" | "annotated_assignment"))
    {
        let Some(field) = assignment.child_by_field_name("left") else {
            continue;
        };
        let Some(value) = assignment.child_by_field_name("right") else {
            continue;
        };
        if value.kind() != "call" {
            continue;
        }
        let Some(function) = value.child_by_field_name("function") else {
            continue;
        };
        if let Some(manager_reference) = exact_static_reference_in_scope(
            context.evidence,
            function,
            context.source,
            model.scope_id.as_deref(),
        ) && let Some(manager) =
            unique_declaration_for_reference(context.evidence, &manager_reference)
            && managers.contains(manager.id.as_str())
        {
            append_exact_framework_relation(
                facts,
                "django-python",
                "django",
                model,
                &manager_reference,
                &format!("manager:{}", node_text(field, context.source)),
                anchor(context.path, assignment),
                context.evidence,
            );
            continue;
        }
        let Some(target) = exact_static_reference_in_scope(
            context.evidence,
            function,
            context.source,
            model.scope_id.as_deref(),
        ) else {
            continue;
        };
        if !matches!(
            target.as_str(),
            "django.db.models.ForeignKey"
                | "django.db.models.ManyToManyField"
                | "django.db.models.OneToOneField"
        ) {
            continue;
        }
        let Some(related_node) = call_argument_node(value, context.source, 0, "to") else {
            continue;
        };
        let related = exact_static_references_in_scope(
            context.evidence,
            related_node,
            context.source,
            model.scope_id.as_deref(),
        );
        if let [related] = related.as_slice()
            && let Some(declaration) = unique_declaration_for_reference(context.evidence, related)
            && models.contains(declaration.id.as_str())
        {
            let relation_anchor = anchor(context.path, field);
            append_exact_framework_relation(
                facts,
                "django-python",
                "django",
                model,
                related,
                target.rsplit('.').next().unwrap_or("relationship"),
                relation_anchor,
                context.evidence,
            );
        }
    }
}

fn django_model_field_facts(
    definition: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    model: &DeclarationFact,
) -> Vec<Value> {
    let Some(body) = definition.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut cursor = body.walk();
    for assignment in body
        .children(&mut cursor)
        .filter(|child| matches!(child.kind(), "assignment" | "annotated_assignment"))
    {
        let (Some(name), Some(value)) = (
            assignment.child_by_field_name("left"),
            assignment.child_by_field_name("right"),
        ) else {
            continue;
        };
        if name.kind() != "identifier" || value.kind() != "call" {
            continue;
        }
        let Some(function) = value.child_by_field_name("function") else {
            continue;
        };
        let Some(constructor) = exact_static_reference_in_scope(
            context.evidence,
            function,
            context.source,
            model.scope_id.as_deref(),
        ) else {
            continue;
        };
        let terminal = constructor.rsplit('.').next().unwrap_or_default();
        if !constructor.starts_with("django.db.models.")
            || !(terminal.ends_with("Field") || matches!(terminal, "ForeignKey" | "OneToOneField"))
        {
            continue;
        }
        fields.push(Value::Object(Map::from_iter([
            (
                "name".into(),
                Value::String(node_text(name, context.source).to_owned()),
            ),
            ("constructor".into(), Value::String(constructor)),
            (
                "anchor".into(),
                serde_json::to_value(anchor(context.path, assignment)).unwrap_or(Value::Null),
            ),
        ])));
    }
    fields
}

fn collect_django_signal_facts(
    context: &UniversalDetectionContext<'_, '_>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let models = python_descendants_of(context.evidence, &["django.db.models.Model"]);
    collect_django_signal_nodes(context.root, context, &models, facts);
}

fn collect_django_signal_nodes(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    models: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call" {
        collect_django_signal_connection(node, context, models, facts);
    }
    if node.kind() == "decorated_definition" {
        let mut cursor = node.walk();
        for decorator in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
        {
            let Some(receiver_call) =
                find_exact_call(decorator, context.evidence, "django.dispatch.receiver")
            else {
                continue;
            };
            let Some(signal_node) = call_argument_node(receiver_call, context.source, 0, "signal")
            else {
                continue;
            };
            let Some(signal) =
                exact_import_reference_hint(context.evidence, signal_node, context.source)
            else {
                continue;
            };
            if !signal.starts_with("django.db.models.signals.") {
                continue;
            }
            let Some(occurrence) =
                exact_occurrence(context.evidence, SemanticRole::Decorator, decorator)
            else {
                continue;
            };
            let Some(handler) = context
                .evidence
                .declarations
                .iter()
                .find(|declaration| declaration.id == occurrence.owner_declaration_id)
            else {
                continue;
            };
            append_django_signal_subscription(
                context,
                models,
                facts,
                handler,
                signal,
                anchor(context.path, decorator),
                call_argument_node(receiver_call, context.source, usize::MAX, "sender"),
            );
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_django_signal_nodes(child, context, models, facts);
    }
}

fn collect_django_signal_connection(
    call: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    models: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let Some(function) = call.child_by_field_name("function") else {
        return;
    };
    if function.kind() != "attribute" {
        return;
    }
    let Some(method) = function.child_by_field_name("attribute") else {
        return;
    };
    if node_text(method, context.source) != "connect" {
        return;
    }
    let Some(receiver) = function.child_by_field_name("object") else {
        return;
    };
    let Some(signal) = exact_django_signal_reference_hint(context, receiver) else {
        return;
    };
    if !signal.starts_with("django.db.models.signals.") {
        return;
    }
    let Some(handler_node) = call_argument_node(call, context.source, 0, "receiver") else {
        return;
    };
    let Some(handler_reference) =
        exact_static_reference_hint(context.evidence, handler_node, context.source)
    else {
        return;
    };
    let Some(handler) = unique_declaration_for_reference(context.evidence, &handler_reference)
    else {
        return;
    };
    if !matches!(handler.kind.as_str(), "function" | "method") {
        return;
    }
    append_django_signal_subscription(
        context,
        models,
        facts,
        handler,
        signal,
        anchor(context.path, call),
        call_argument_node(call, context.source, usize::MAX, "sender"),
    );
}

fn exact_django_signal_reference_hint(
    context: &UniversalDetectionContext<'_, '_>,
    node: Node<'_>,
) -> Option<String> {
    let reference = node_text(node, context.source).trim();
    if !is_dotted_identifier(reference) {
        return None;
    }
    let (head, suffix) = reference
        .split_once('.')
        .map_or((reference, None), |(head, suffix)| (head, Some(suffix)));
    let reference_scope = reference_evaluation_scope(context.evidence, node, false);
    let targets = context
        .evidence
        .bindings
        .iter()
        .filter(|binding| {
            matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                && binding.spelling == head
                && binding.range.end_byte <= node.start_byte() as u64
                && binding.scope_id.as_deref() == reference_scope
                && !has_intervening_python_name_binding(
                    context.root,
                    head,
                    binding.range.end_byte,
                    node.start_byte() as u64,
                    context.source,
                )
        })
        .map(|binding| {
            suffix.map_or_else(
                || binding.qualified_target.clone(),
                |suffix| format!("{}.{}", binding.qualified_target, suffix),
            )
        })
        .collect::<BTreeSet<_>>();
    let target = targets.first()?;
    (targets.len() == 1 && target.starts_with("django.db.models.signals.")).then(|| target.clone())
}

fn append_django_signal_subscription(
    context: &UniversalDetectionContext<'_, '_>,
    models: &BTreeSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
    handler: &DeclarationFact,
    signal: String,
    subscription_anchor: RawFrameworkAnchor,
    sender_node: Option<Node<'_>>,
) {
    append_exact_framework_relation_kind(
        facts,
        "django-python",
        "django",
        "subscribes",
        handler,
        &signal,
        "signal",
        subscription_anchor.clone(),
        context.evidence,
    );
    facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
        pack_id: "django-python".to_owned(),
        framework: "django".to_owned(),
        role: "subscriber".to_owned(),
        subject_reference: Some(handler.graph_node_id.clone()),
        context: Some(signal),
        anchor: subscription_anchor,
        origin: RawFrameworkOrigin::Ast,
        evidence_class: "exact".to_owned(),
        detail: Map::new(),
    }));
    let Some(sender_node) = sender_node else {
        return;
    };
    let senders = exact_static_references_in_scope(
        context.evidence,
        sender_node,
        context.source,
        handler.scope_id.as_deref(),
    );
    if let [sender] = senders.as_slice()
        && let Some(sender_declaration) = unique_declaration_for_reference(context.evidence, sender)
        && models.contains(sender_declaration.id.as_str())
    {
        append_exact_framework_relation(
            facts,
            "django-python",
            "django",
            handler,
            sender,
            "signal_sender",
            anchor(context.path, sender_node),
            context.evidence,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_exact_framework_relation(
    facts: &mut Vec<RawFrameworkFact>,
    pack_id: &str,
    framework: &str,
    source: &DeclarationFact,
    target: &str,
    context: &str,
    anchor: RawFrameworkAnchor,
    evidence: &SemanticEvidenceBatch,
) {
    append_exact_framework_relation_kind(
        facts,
        pack_id,
        framework,
        "depends_on",
        source,
        target,
        context,
        anchor,
        evidence,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_exact_framework_relation_kind(
    facts: &mut Vec<RawFrameworkFact>,
    pack_id: &str,
    framework: &str,
    relation: &str,
    source: &DeclarationFact,
    target: &str,
    context: &str,
    anchor: RawFrameworkAnchor,
    evidence: &SemanticEvidenceBatch,
) {
    let target_anchor = unique_declaration_for_reference(evidence, target)
        .map(|declaration| evidence_anchor(&declaration.range));
    facts.push(RawFrameworkFact::Relation(RawFrameworkRelationFact {
        pack_id: pack_id.to_owned(),
        framework: framework.to_owned(),
        relation: relation.to_owned(),
        source_reference: Some(source.graph_node_id.clone()),
        target_hint: Some(target.to_owned()),
        context: Some(context.to_owned()),
        anchor,
        target_anchor,
        origin: RawFrameworkOrigin::Ast,
        evidence_class: "exact".to_owned(),
        ambiguity_policy: "require_exact".to_owned(),
        detail: Map::new(),
    }));
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
                append_dependency_graph_facts(
                    facts,
                    receiver.framework,
                    handler,
                    &stages,
                    "route",
                    evidence,
                );
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
                    append_flask_implicit_methods(
                        &mut detail,
                        receiver.framework,
                        operation,
                        &arguments,
                    );
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
            ("flask", "add_url_rule") => operations("flask", method, &arguments),
            _ => Vec::new(),
        };
        if receiver.framework == "flask"
            && method == "add_url_rule"
            && let Some(raw_path) = positional_argument(&arguments, 0)
                .and_then(string_literal)
                .or_else(|| keyword_string(&arguments, "rule"))
            && let Some(handler_node) = call_argument_node(node, source, 2, "view_func")
            && let Some((handler, inferred_operations)) =
                exact_flask_view_declaration(handler_node, source, evidence)
        {
            let operations = if keyword_value(&arguments, "methods").is_some() {
                operations.clone()
            } else {
                inferred_operations.unwrap_or_else(|| operations.clone())
            };
            if !operations.is_empty() {
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
                    &handler.graph_node_id,
                    Some(handler),
                    Vec::new(),
                    &arguments,
                );
            }
        } else if !operations.is_empty()
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

fn exact_flask_view_declaration<'a>(
    node: Node<'_>,
    source: &[u8],
    evidence: &'a SemanticEvidenceBatch,
) -> Option<(&'a DeclarationFact, Option<Vec<String>>)> {
    if let Some(reference) = exact_python_lexical_reference_evidence(evidence, node, source)
        && let Some(handler) = unique_declaration_for_reference(evidence, &reference)
        && matches!(handler.kind.as_str(), "function" | "method")
    {
        return Some((handler, None));
    }
    if node.kind() != "call" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if function.kind() != "attribute"
        || function
            .child_by_field_name("attribute")
            .is_none_or(|member| node_text(member, source) != "as_view")
        || exact_occurrence(evidence, SemanticRole::Call, function).is_none()
    {
        return None;
    }
    let view_node = function.child_by_field_name("object")?;
    let view_reference = exact_python_lexical_reference_evidence(evidence, view_node, source)?;
    let view = unique_declaration_for_reference(evidence, &view_reference)?;
    let method_views = python_descendants_of(evidence, &["flask.views.MethodView"]);
    if !method_views.contains(view.id.as_str()) {
        return None;
    }
    let view_scope = evidence
        .scopes
        .iter()
        .find(|scope| scope.owner_declaration_id.as_deref() == Some(view.id.as_str()))
        .map(|scope| scope.id.as_str());
    let mut operations = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "method" && declaration.scope_id.as_deref() == view_scope
        })
        .filter_map(|declaration| match declaration.name.as_str() {
            "get" | "post" | "put" | "patch" | "delete" | "options" | "head" => {
                Some(declaration.name.to_ascii_uppercase())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    operations.sort();
    operations.dedup();
    (!operations.is_empty()).then_some((view, Some(operations)))
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
            append_dependency_graph_facts(
                facts,
                receiver.framework,
                handler,
                &stages,
                "route",
                evidence,
            );
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
            append_flask_implicit_methods(&mut detail, receiver.framework, operation, arguments);
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
            "include_router" => optional_literal_keyword(&arguments, "prefix").and_then(|prefix| {
                positional_argument(&arguments, 0)
                    .map(|target| (target, prefix.unwrap_or_default()))
            }),
            "register_blueprint" => {
                optional_literal_keyword(&arguments, "url_prefix").and_then(|prefix| {
                    positional_argument(&arguments, 0)
                        .map(|target| (target, prefix.unwrap_or_default()))
                })
            }
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
        && let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        )
        && left.kind() == "identifier"
    {
        let name = node_text(left, source);
        let scope_id = reference_evaluation_scope(evidence, left, false).map(str::to_owned);
        for receiver in receivers.iter_mut().filter(|receiver| {
            receiver.name == name && receiver.scope_id == scope_id && receiver.end_byte == u64::MAX
        }) {
            receiver.end_byte = node.start_byte() as u64;
        }
        if right.kind() == "call"
            && let Some(function) = right.child_by_field_name("function")
            && let Some(constructor) = exact_call_target(evidence, function)
                .or_else(|| exact_python_lexical_reference_evidence(evidence, function, source))
            && let Some((framework, prefix_key)) = match constructor.as_str() {
                "flask.Flask" => Some(("flask", "url_prefix")),
                "flask.Blueprint" => Some(("flask", "url_prefix")),
                "fastapi.FastAPI" => Some(("fastapi", "prefix")),
                "fastapi.APIRouter" => Some(("fastapi", "prefix")),
                "starlette.applications.Starlette" => Some(("starlette", "prefix")),
                "starlette.routing.Router" => Some(("starlette", "prefix")),
                "celery.Celery" => Some(("celery", "")),
                "rest_framework.routers.DefaultRouter" | "rest_framework.routers.SimpleRouter" => {
                    Some(("django-rest-framework", "prefix"))
                }
                _ => None,
            }
        {
            let arguments = call_arguments(right, source).unwrap_or_default();
            let prefix = if prefix_key.is_empty() {
                Some(String::new())
            } else {
                optional_literal_keyword(&arguments, prefix_key).map(Option::unwrap_or_default)
            };
            if let Some(prefix) = prefix {
                let declaration = exact_variable_declaration(evidence, left);
                let declaration_id = declaration.map_or_else(
                    || {
                        crate::make_id(&[
                            "python-framework-receiver",
                            &path.to_string_lossy(),
                            scope_id.as_deref().unwrap_or_default(),
                            name,
                            &left.start_byte().to_string(),
                        ])
                    },
                    |declaration| declaration.id.clone(),
                );
                let qualified_name = declaration.map_or_else(
                    || {
                        let owner = scope_id
                            .as_deref()
                            .and_then(|scope_id| {
                                evidence
                                    .scopes
                                    .iter()
                                    .find(|scope| scope.id == scope_id)
                                    .and_then(|scope| scope.owner_declaration_id.as_deref())
                            })
                            .and_then(|owner| {
                                evidence
                                    .declarations
                                    .iter()
                                    .find(|declaration| declaration.id == owner)
                            })
                            .map(|declaration| declaration.qualified_name.clone())
                            .unwrap_or_else(|| module_scope(path));
                        format!("{owner}.{name}")
                    },
                    |declaration| declaration.qualified_name.clone(),
                );
                receivers.push(Receiver {
                    declaration_id,
                    qualified_name,
                    name: name.to_owned(),
                    scope_id,
                    framework,
                    constructor,
                    prefix,
                    start_byte: left.start_byte() as u64,
                    end_byte: u64::MAX,
                    constructor_start_byte: right.start_byte() as u64,
                    constructor_end_byte: right.end_byte() as u64,
                    stages: if framework == "fastapi" {
                        dependency_stages(right, source, path, evidence)
                    } else {
                        Vec::new()
                    },
                });
            }
        }
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

fn receiver_at<'a>(
    receivers: &'a [Receiver],
    evidence: &SemanticEvidenceBatch,
    name: &str,
    use_start: u64,
) -> Option<&'a Receiver> {
    let mut scope_id = evidence
        .scopes
        .iter()
        .filter(|scope| scope.range.start_byte <= use_start && scope.range.end_byte >= use_start)
        .min_by_key(|scope| scope.range.end_byte.saturating_sub(scope.range.start_byte))
        .map(|scope| scope.id.as_str());
    let mut scope_chain = Vec::new();
    while let Some(current) = scope_id {
        scope_chain.push(current);
        scope_id = evidence
            .scopes
            .iter()
            .find(|scope| scope.id == current)
            .and_then(|scope| scope.parent_scope_id.as_deref());
    }
    let mut matches = receivers
        .iter()
        .filter(|receiver| receiver.name == name)
        .filter(|receiver| receiver.start_byte < use_start && receiver.end_byte > use_start)
        .filter_map(|receiver| {
            scope_chain
                .iter()
                .position(|scope| Some(*scope) == receiver.scope_id.as_deref())
                .map(|distance| (distance, receiver))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(distance, receiver)| (*distance, std::cmp::Reverse(receiver.start_byte)));
    let (distance, receiver) = *matches.first()?;
    if matches
        .get(1)
        .is_some_and(|(other_distance, _)| *other_distance == distance)
    {
        return None;
    }
    for shadow_scope in scope_chain.iter().take(distance) {
        if evidence.declarations.iter().any(|declaration| {
            declaration.name == name
                && declaration.range.start_byte < use_start
                && declaration.scope_id.as_deref() == Some(*shadow_scope)
        }) {
            return None;
        }
    }
    let declarations = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.name == name
                && declaration.range.start_byte < use_start
                && declaration.scope_id == receiver.scope_id
        })
        .collect::<Vec<_>>();
    match declarations.as_slice() {
        [] => Some(receiver),
        [declaration] if declaration.id == receiver.declaration_id => Some(receiver),
        _ => None,
    }
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
            "websocket" | "websocket_route" => Some("WEBSOCKET".to_owned()),
            "api_route" | "route" => None,
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
    if framework == "flask" {
        if matches!(
            method,
            "get" | "post" | "put" | "patch" | "delete" | "options"
        ) {
            return vec![method.to_ascii_uppercase()];
        }
        if matches!(method, "route" | "add_url_rule") {
            let methods = keyword_string_list(arguments, "methods");
            return if methods.is_empty() {
                vec!["GET".to_owned()]
            } else {
                methods
            };
        }
    }
    Vec::new()
}

fn append_flask_implicit_methods(
    detail: &mut Map<String, Value>,
    framework: &str,
    operation: &str,
    arguments: &[String],
) {
    if framework != "flask" || operation != "GET" {
        return;
    }
    let mut implicit_methods = vec![Value::String("HEAD".to_owned())];
    if keyword_value(arguments, "provide_automatic_options").and_then(static_python_bool)
        != Some(false)
    {
        implicit_methods.push(Value::String("OPTIONS".to_owned()));
    }
    detail.insert(
        "implicit_methods".to_owned(),
        Value::Array(implicit_methods),
    );
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
    if let Some(mut stage) = direct_dependency_stage(node, source, path, evidence) {
        stage.position = u32::try_from(stages.len()).unwrap_or(u32::MAX);
        stages.push(stage);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_dependency_stages(child, source, path, evidence, stages);
    }
}

fn direct_dependency_stage(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    evidence: &SemanticEvidenceBatch,
) -> Option<RawRouteStageFact> {
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
        return Some(RawRouteStageFact {
            role: if target == "fastapi.Security" {
                RawRouteStageRole::Security
            } else {
                RawRouteStageRole::Dependency
            },
            position: 0,
            reference,
            anchor: anchor(path, node),
            origin: RawFrameworkOrigin::Ast,
            detail,
        });
    }
    None
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
        // Decorator ownership already selected one exact declaration. Keep
        // that stable graph identity through route resolution instead of
        // discarding it and performing a second project-wide name lookup.
        reference: handler.graph_node_id.clone(),
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
    evidence: &SemanticEvidenceBatch,
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
            target_anchor: unique_declaration_for_reference(evidence, &stage.reference)
                .map(|declaration| evidence_anchor(&declaration.range)),
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
    collect_fastapi_dependency_expression_facts(context.root, context, facts);
}

fn collect_fastapi_dependency_expression_facts(
    node: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if let Some(stage) =
        direct_dependency_stage(node, context.source, context.path, context.evidence)
        && let Some(function) = node.child_by_field_name("function")
        && let Some(owner) = call_owner_declaration(function, context.evidence)
    {
        let relation_context = if ancestors_before_definition(node)
            .iter()
            .any(|ancestor| ancestor.kind() == "parameters")
        {
            "subdependency"
        } else {
            "dependency_expression"
        };
        append_dependency_graph_facts(
            facts,
            "fastapi",
            owner,
            std::slice::from_ref(&stage),
            relation_context,
            context.evidence,
        );
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_fastapi_dependency_expression_facts(child, context, facts);
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

fn python_descendants_of(
    evidence: &SemanticEvidenceBatch,
    exact_bases: &[&str],
) -> BTreeSet<String> {
    let mut declarations = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Extends)
        .filter(|candidate| {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .is_some_and(|target| exact_bases.contains(&target))
                || exact_candidate_binding_target(evidence, candidate.binding_id.as_deref())
                    .is_some_and(|target| exact_bases.contains(&target))
        })
        .map(|candidate| candidate.source_declaration_id.clone())
        .collect::<BTreeSet<_>>();
    for _ in 0..evidence.declarations.len() {
        let mut qualified = BTreeMap::<&str, usize>::new();
        for declaration in evidence
            .declarations
            .iter()
            .filter(|declaration| declarations.contains(declaration.id.as_str()))
        {
            *qualified
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
                    .is_some_and(|target| declarations.contains(target))
                    || candidate
                        .constraints
                        .qualified_name
                        .as_deref()
                        .is_some_and(|target| qualified.get(target) == Some(&1))
            })
            .map(|candidate| candidate.source_declaration_id.clone())
            .collect::<Vec<_>>();
        let previous = declarations.len();
        declarations.extend(inherited);
        if declarations.len() == previous {
            break;
        }
    }
    declarations
}

fn drf_viewset_method_templates(
    context: &UniversalDetectionContext<'_, '_>,
    viewset: &DeclarationFact,
) -> Vec<Value> {
    let viewset_scope = context
        .evidence
        .scopes
        .iter()
        .find(|scope| scope.owner_declaration_id.as_deref() == Some(viewset.id.as_str()))
        .map(|scope| scope.id.as_str());
    let mut methods = context
        .evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "method" && declaration.scope_id.as_deref() == viewset_scope
        })
        .collect::<Vec<_>>();
    methods.sort_by_key(|method| (method.range.start_byte, method.range.end_byte));
    let mut templates = Vec::new();
    for method in methods {
        if let Some((operation, detail)) = match method.name.as_str() {
            "list" => Some(("GET", false)),
            "create" => Some(("POST", false)),
            "retrieve" => Some(("GET", true)),
            "update" => Some(("PUT", true)),
            "partial_update" => Some(("PATCH", true)),
            "destroy" => Some(("DELETE", true)),
            _ => None,
        } {
            templates.push(drf_method_template(
                method,
                operation,
                detail,
                None,
                evidence_anchor(&method.range),
            ));
        }
        templates.extend(drf_action_method_templates(context, method));
    }
    templates
}

fn drf_viewset_lookup_parameter(
    context: &UniversalDetectionContext<'_, '_>,
    viewset: &DeclarationFact,
) -> Option<String> {
    let definition = declaration_node(context.root, viewset)?;
    if direct_class_has_assignment(definition, "lookup_value_regex", context.source)
        || direct_class_has_assignment(definition, "lookup_value_converter", context.source)
    {
        return None;
    }
    let lookup_field =
        direct_optional_literal_assignment(definition, "lookup_field", context.source)?
            .unwrap_or_else(|| "pk".to_owned());
    let lookup_parameter =
        direct_optional_literal_assignment(definition, "lookup_url_kwarg", context.source)?
            .unwrap_or(lookup_field);
    is_identifier(&lookup_parameter).then_some(lookup_parameter)
}

fn drf_action_method_templates(
    context: &UniversalDetectionContext<'_, '_>,
    method: &DeclarationFact,
) -> Vec<Value> {
    let Some(definition) = declaration_node(context.root, method) else {
        return Vec::new();
    };
    let Some(decorated) = definition
        .parent()
        .filter(|parent| parent.kind() == "decorated_definition")
    else {
        return Vec::new();
    };
    let action_occurrences = context
        .evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Decorates
                && candidate.source_declaration_id == method.id
                && (candidate.constraints.qualified_name.as_deref()
                    == Some("rest_framework.decorators.action")
                    || exact_candidate_binding_target(
                        context.evidence,
                        candidate.binding_id.as_deref(),
                    ) == Some("rest_framework.decorators.action"))
        })
        .filter_map(|candidate| candidate.occurrence_id.as_deref())
        .collect::<BTreeSet<_>>();
    if action_occurrences.len() != 1 {
        return Vec::new();
    }
    let Some(occurrence) = context
        .evidence
        .occurrences
        .iter()
        .find(|occurrence| action_occurrences.contains(occurrence.id.as_str()))
    else {
        return Vec::new();
    };
    let mut cursor = decorated.walk();
    let Some(decorator) = decorated
        .children(&mut cursor)
        .filter(|child| child.kind() == "decorator")
        .find(|decorator| {
            decorator.start_byte() as u64 <= occurrence.range.start_byte
                && decorator.end_byte() as u64 >= occurrence.range.end_byte
        })
    else {
        return Vec::new();
    };
    let text = node_text(decorator, context.source)
        .trim()
        .trim_start_matches('@');
    let Some((_, arguments)) = parse_call(text) else {
        return Vec::new();
    };
    let Some(detail) = keyword_value(&arguments, "detail").and_then(static_python_bool) else {
        return Vec::new();
    };
    let methods = if keyword_value(&arguments, "methods").is_some() {
        let methods = keyword_string_list(&arguments, "methods");
        if methods.is_empty() {
            return Vec::new();
        }
        methods
    } else {
        vec!["GET".to_owned()]
    };
    let url_path = if let Some(value) = keyword_value(&arguments, "url_path") {
        let Some(value) = string_literal(value) else {
            return Vec::new();
        };
        value
    } else {
        method.name.clone()
    };
    methods
        .into_iter()
        .map(|operation| {
            drf_method_template(
                method,
                &operation,
                detail,
                Some(url_path.clone()),
                anchor(context.path, decorator),
            )
        })
        .collect()
}

fn drf_method_template(
    method: &DeclarationFact,
    operation: &str,
    detail: bool,
    url_path: Option<String>,
    anchor: RawFrameworkAnchor,
) -> Value {
    Value::Object(Map::from_iter([
        ("operation".into(), Value::String(operation.to_owned())),
        ("detail".into(), Value::Bool(detail)),
        (
            "handler".into(),
            Value::String(method.graph_node_id.clone()),
        ),
        (
            "handler_declaration_id".into(),
            Value::String(method.id.clone()),
        ),
        (
            "url_path".into(),
            url_path.map_or(Value::Null, Value::String),
        ),
        (
            "anchor".into(),
            serde_json::to_value(anchor).unwrap_or(Value::Null),
        ),
    ]))
}

fn static_python_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

fn optional_literal_keyword(arguments: &[String], key: &str) -> Option<Option<String>> {
    match keyword_value(arguments, key) {
        Some(value) => string_literal(value).map(Some),
        None => Some(None),
    }
}

fn direct_class_assignment_value<'tree>(
    definition: Node<'tree>,
    field: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    let body = definition.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let mut matches = body.children(&mut cursor).filter_map(|child| {
        if !matches!(child.kind(), "assignment" | "annotated_assignment") {
            return None;
        }
        let left = child.child_by_field_name("left")?;
        (node_text(left, source) == field)
            .then(|| child.child_by_field_name("right"))
            .flatten()
    });
    let value = matches.next()?;
    matches.next().is_none().then_some(value)
}

fn direct_class_has_assignment(definition: Node<'_>, field: &str, source: &[u8]) -> bool {
    let Some(body) = definition.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.children(&mut cursor).any(|child| {
        matches!(child.kind(), "assignment" | "annotated_assignment")
            && child
                .child_by_field_name("left")
                .is_some_and(|left| node_text(left, source) == field)
    })
}

fn direct_optional_literal_assignment(
    definition: Node<'_>,
    field: &str,
    source: &[u8],
) -> Option<Option<String>> {
    if !direct_class_has_assignment(definition, field, source) {
        return Some(None);
    }
    direct_class_assignment_value(definition, field, source)
        .and_then(|value| string_literal(node_text(value, source)))
        .map(Some)
}

fn direct_nested_class<'tree>(
    definition: Node<'tree>,
    name: &str,
    source: &[u8],
) -> Option<Node<'tree>> {
    let body = definition.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let mut matches = body.children(&mut cursor).filter(|child| {
        child.kind() == "class_definition"
            && child
                .child_by_field_name("name")
                .is_some_and(|node| node_text(node, source) == name)
    });
    let class = matches.next()?;
    matches.next().is_none().then_some(class)
}

fn exact_static_references_in_scope(
    evidence: &SemanticEvidenceBatch,
    node: Node<'_>,
    source: &[u8],
    scope_id: Option<&str>,
) -> Vec<String> {
    if matches!(node.kind(), "list" | "tuple" | "set") {
        let mut cursor = node.walk();
        let mut references = node
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .flat_map(|child| exact_static_references_in_scope(evidence, child, source, scope_id))
            .collect::<Vec<_>>();
        references.sort();
        references.dedup();
        return references;
    }
    exact_static_reference_in_scope(evidence, node, source, scope_id)
        .into_iter()
        .collect()
}

fn exact_static_reference_in_scope(
    evidence: &SemanticEvidenceBatch,
    node: Node<'_>,
    source: &[u8],
    scope_id: Option<&str>,
) -> Option<String> {
    let reference = node_text(node, source).trim();
    if !is_dotted_identifier(reference) {
        return None;
    }
    if is_identifier(reference) {
        let declarations = evidence
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.name == reference
                    && declaration.range.start_byte < node.start_byte() as u64
                    && declaration.scope_id.as_deref() == scope_id
            })
            .collect::<Vec<_>>();
        let bindings = evidence
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                    && binding.spelling == reference
                    && binding.range.end_byte <= node.start_byte() as u64
                    && binding.scope_id.as_deref() == scope_id
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
            ([], [binding])
                if !has_intervening_python_name_binding(
                    context_root(node),
                    reference,
                    binding.range.end_byte,
                    node.start_byte() as u64,
                    source,
                ) =>
            {
                Some(binding.qualified_target.clone())
            }
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
                && binding.scope_id.as_deref() == scope_id
                && !has_intervening_python_name_binding(
                    context_root(node),
                    head,
                    binding.range.end_byte,
                    node.start_byte() as u64,
                    source,
                )
        })
        .map(|binding| format!("{}.{}", binding.qualified_target, suffix))
        .collect::<BTreeSet<_>>();
    (bindings.len() == 1)
        .then(|| bindings.first().cloned())
        .flatten()
}

fn unique_declaration_for_reference<'a>(
    evidence: &'a SemanticEvidenceBatch,
    reference: &str,
) -> Option<&'a DeclarationFact> {
    let mut matches = evidence.declarations.iter().filter(|declaration| {
        declaration.id == reference
            || declaration.graph_node_id == reference
            || declaration.qualified_name == reference
    });
    let declaration = matches.next()?;
    matches.next().is_none().then_some(declaration)
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
    let mut reference_scope = reference_evaluation_scope(evidence, node, in_signature);
    if is_identifier(reference) {
        while let Some(scope_id) = reference_scope {
            let declarations = evidence
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.name == reference
                        && declaration.range.start_byte < node.start_byte() as u64
                        && declaration.scope_id.as_deref() == Some(scope_id)
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
                        && binding.scope_id.as_deref() == Some(scope_id)
                })
                .collect::<Vec<_>>();
            if !declarations.is_empty() || !bindings.is_empty() {
                return match (declarations.as_slice(), bindings.as_slice()) {
                    ([declaration], [])
                        if (matches!(
                            declaration.kind.as_str(),
                            "function" | "method" | "class"
                        ) || exact_dependency_instance_declaration(evidence, declaration))
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
            reference_scope = evidence
                .scopes
                .iter()
                .find(|scope| scope.id == scope_id)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        return None;
    }
    let (head, suffix) = reference.split_once('.')?;
    while let Some(scope_id) = reference_scope {
        let declarations = evidence
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.name == head
                    && declaration.range.start_byte < node.start_byte() as u64
                    && declaration.scope_id.as_deref() == Some(scope_id)
            })
            .count();
        let bindings = evidence
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                    && binding.spelling == head
                    && binding.range.end_byte <= node.start_byte() as u64
                    && binding.scope_id.as_deref() == Some(scope_id)
            })
            .map(|binding| format!("{}.{}", binding.qualified_target, suffix))
            .collect::<BTreeSet<_>>();
        if declarations > 0 || !bindings.is_empty() {
            return (declarations == 0 && bindings.len() == 1)
                .then(|| bindings.first().cloned())
                .flatten();
        }
        reference_scope = evidence
            .scopes
            .iter()
            .find(|scope| scope.id == scope_id)
            .and_then(|scope| scope.parent_scope_id.as_deref());
    }
    None
}

fn exact_dependency_instance_declaration(
    evidence: &SemanticEvidenceBatch,
    declaration: &DeclarationFact,
) -> bool {
    if declaration.kind != "variable" {
        return false;
    }
    let types = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::TypeOf
                && candidate.source_declaration_id == declaration.id
        })
        .map(|candidate| {
            (
                candidate.constraints.exact_target_declaration_id.as_deref(),
                candidate.constraints.qualified_name.as_deref(),
            )
        })
        .collect::<BTreeSet<_>>();
    types.len() == 1
        && types
            .first()
            .is_some_and(|(declaration, qualified)| declaration.is_some() || qualified.is_some())
}

fn exact_import_reference_hint(
    evidence: &SemanticEvidenceBatch,
    node: Node<'_>,
    source: &[u8],
) -> Option<String> {
    let reference = node_text(node, source).trim();
    if !is_identifier(reference) {
        return None;
    }
    let reference_scope = reference_evaluation_scope(evidence, node, false);
    let bindings = evidence
        .bindings
        .iter()
        .filter(|binding| {
            matches!(binding.kind, BindingKind::Import | BindingKind::ImportAlias)
                && binding.spelling == reference
                && binding.range.end_byte <= node.start_byte() as u64
                && binding.scope_id.as_deref() == reference_scope
        })
        .map(|binding| binding.qualified_target.as_str())
        .collect::<BTreeSet<_>>();
    let target = *bindings.first()?;
    (bindings.len() == 1
        && exact_static_reference_hint(evidence, node, source).as_deref() == Some(target))
    .then(|| target.to_owned())
}

fn has_intervening_python_rebinding(
    root: Node<'_>,
    declaration: &DeclarationFact,
    reference: &str,
    use_start: u64,
    source: &[u8],
) -> bool {
    has_intervening_python_name_binding(
        root,
        reference,
        declaration.range.end_byte,
        use_start,
        source,
    )
}

fn has_intervening_python_name_binding(
    root: Node<'_>,
    reference: &str,
    binding_end: u64,
    use_start: u64,
    source: &[u8],
) -> bool {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|statement| statement.is_named())
        .filter(|statement| statement.start_byte() as u64 > binding_end)
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
