use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{CandidateRelation, DeclarationFact, SemanticRole};

use super::{
    RawFrameworkAnchor, RawFrameworkAnnotationFact, RawFrameworkFact, UniversalDetectionContext,
};

const PACK_ID: &str = "spring-java";
const FRAMEWORK: &str = "spring";
const SPRING_PREFIX: &str = "org.springframework.";

pub(super) fn detect(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    detect_language(
        context,
        "java",
        super::pack::SPRING_JAVA_DESCRIPTOR.dependency_markers,
    )
}

pub(super) fn detect_kotlin(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let mut facts = detect_language(
        context,
        "kotlin",
        super::pack::SPRING_KOTLIN_DESCRIPTOR.dependency_markers,
    );
    for fact in &mut facts {
        set_fact_pack(fact, "spring-kotlin");
    }
    facts
}

fn detect_language(
    context: &UniversalDetectionContext<'_, '_>,
    language: &str,
    dependency_markers: &[&str],
) -> Vec<RawFrameworkFact> {
    if context.evidence.pipeline.language != language {
        return Vec::new();
    }
    let declarations = context
        .evidence
        .declarations
        .iter()
        .map(|declaration| (declaration.id.as_str(), declaration))
        .collect::<HashMap<_, _>>();
    let candidates = context
        .evidence
        .candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .occurrence_id
                .as_deref()
                .map(|occurrence| (occurrence, candidate))
        })
        .collect::<HashMap<_, _>>();
    let activated = context
        .project
        .is_some_and(|project| project.has_any_dependency(dependency_markers))
        || context
            .evidence
            .bindings
            .iter()
            .any(|binding| is_framework_qualified_name(&binding.qualified_target))
        || candidates.values().any(|candidate| {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .is_some_and(is_framework_qualified_name)
        });
    if !activated {
        return Vec::new();
    }

    let mut annotation_nodes = BTreeMap::new();
    collect_annotation_nodes(context.root, &mut annotation_nodes);
    let mut facts = context
        .evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Annotation)
        .filter_map(|occurrence| {
            let declaration = declarations.get(occurrence.owner_declaration_id.as_str())?;
            let start = usize::try_from(occurrence.range.start_byte).ok()?;
            let annotation = annotation_nodes.get(&start).copied();
            let arguments = annotation
                .map(|node| annotation_arguments(node, context.source))
                .unwrap_or_default();
            let candidate = candidates.get(occurrence.id.as_str());
            let qualified_name = candidate
                .and_then(|candidate| candidate.constraints.qualified_name.clone())
                .or_else(|| imported_annotation_name(context, &occurrence.spelling));
            let mut detail = Map::new();
            detail.insert(
                "occurrenceId".to_owned(),
                Value::String(occurrence.id.clone()),
            );
            if let Some(scope_id) = occurrence.scope_id.as_ref() {
                detail.insert("scopeId".to_owned(), Value::String(scope_id.clone()));
            }
            let binding_map = unique_binding_map(context);
            if !binding_map.is_empty() {
                detail.insert("bindings".to_owned(), Value::Object(binding_map));
            }
            if declaration.kind == "annotation_type" {
                detail.insert("declaration".to_owned(), Value::Bool(true));
                if let Some(attribute) =
                    annotation.and_then(|node| annotation_attribute_name(node, context.source))
                {
                    detail.insert("annotationAttribute".to_owned(), Value::String(attribute));
                }
            }
            let targets = injection_targets(context, declaration.id.as_str(), &candidates);
            if !targets.is_empty() {
                detail.insert(
                    "targetReferences".to_owned(),
                    Value::Array(targets.into_iter().map(Value::String).collect()),
                );
            }
            Some(RawFrameworkFact::Annotation(annotation_fact(
                declaration,
                &occurrence.spelling,
                qualified_name,
                anchor(&occurrence.range),
                arguments,
                detail,
            )))
        })
        .collect::<Vec<_>>();
    facts.extend(constructor_facts(context, &candidates));
    facts.extend(constant_facts(context));
    facts.extend(producer_facts(context, &declarations, &candidates));
    facts.extend(repository_facts(context, &declarations));
    facts.sort_by(|left, right| annotation_fact_key(left).cmp(&annotation_fact_key(right)));
    facts
}

fn set_fact_pack(fact: &mut RawFrameworkFact, pack_id: &str) {
    match fact {
        RawFrameworkFact::Annotation(annotation) => annotation.pack_id = pack_id.to_owned(),
        RawFrameworkFact::Route(route) => {
            route.detail.insert(
                "frameworkPack".to_owned(),
                Value::String(pack_id.to_owned()),
            );
        }
        RawFrameworkFact::Domain(domain) => {
            domain.detail.insert(
                "frameworkPack".to_owned(),
                Value::String(pack_id.to_owned()),
            );
        }
    }
}

fn repository_facts(
    context: &UniversalDetectionContext<'_, '_>,
    declarations: &HashMap<&str, &DeclarationFact>,
) -> Vec<RawFrameworkFact> {
    context
        .evidence
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::Extends | CandidateRelation::Implements
            )
        })
        .filter_map(|candidate| {
            let qualified = candidate.constraints.qualified_name.as_deref()?;
            if !qualified.starts_with("org.springframework.data.")
                || !qualified.ends_with("Repository")
            {
                return None;
            }
            let declaration = declarations.get(candidate.source_declaration_id.as_str())?;
            Some(RawFrameworkFact::Domain(crate::RawDomainFact {
                framework: FRAMEWORK.to_owned(),
                kind: "bean_definition".to_owned(),
                name: decapitalize_name(&declaration.name),
                declaring_scope: declaration.qualified_name.clone(),
                anchor: anchor(&declaration.range),
                origin: crate::RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    (
                        "frameworkPack".to_owned(),
                        Value::String(PACK_ID.to_owned()),
                    ),
                    (
                        "bean_kind".to_owned(),
                        Value::String("SpringDataRepository".to_owned()),
                    ),
                    (
                        "handler_reference".to_owned(),
                        Value::String(declaration.graph_node_id.clone()),
                    ),
                    (
                        "owner_kind".to_owned(),
                        Value::String("interface".to_owned()),
                    ),
                    (
                        "relationship".to_owned(),
                        Value::String("registers".to_owned()),
                    ),
                    ("primary".to_owned(), Value::Bool(false)),
                    (
                        "repository_base".to_owned(),
                        Value::String(qualified.to_owned()),
                    ),
                ]),
            }))
        })
        .collect()
}

fn decapitalize_name(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_lowercase().chain(chars).collect()
}

fn is_framework_qualified_name(qualified: &str) -> bool {
    qualified.starts_with(SPRING_PREFIX)
        || qualified.starts_with("jakarta.persistence.")
        || qualified.starts_with("javax.persistence.")
        || qualified.starts_with("jakarta.transaction.")
        || qualified.starts_with("javax.transaction.")
}

fn unique_binding_map(context: &UniversalDetectionContext<'_, '_>) -> Map<String, Value> {
    let grouped = context.evidence.bindings.iter().fold(
        BTreeMap::<&str, Vec<&str>>::new(),
        |mut grouped, binding| {
            grouped
                .entry(binding.spelling.as_str())
                .or_default()
                .push(binding.qualified_target.as_str());
            grouped
        },
    );
    grouped
        .into_iter()
        .filter_map(|(spelling, mut targets)| {
            targets.sort_unstable();
            targets.dedup();
            (targets.len() == 1)
                .then(|| (spelling.to_owned(), Value::String(targets[0].to_owned())))
        })
        .collect()
}

fn constant_facts(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    if context.evidence.pipeline.language == "kotlin" {
        let mut values = BTreeMap::new();
        collect_kotlin_constant_values(context.root, context.source, &mut values);
        return context
            .evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "constant")
            .filter_map(|declaration| {
                let expression = values.get(&declaration.range.start_byte)?;
                Some(RawFrameworkFact::Domain(crate::RawDomainFact {
                    framework: FRAMEWORK.to_owned(),
                    kind: "_spring_constant".to_owned(),
                    name: declaration.qualified_name.clone(),
                    declaring_scope: declaration
                        .qualified_name
                        .rsplit_once("::")
                        .map(|(owner, _)| owner)
                        .unwrap_or(declaration.qualified_name.as_str())
                        .to_owned(),
                    anchor: anchor(&declaration.range),
                    origin: crate::RawFrameworkOrigin::Ast,
                    detail: Map::from_iter([
                        (
                            "frameworkPack".to_owned(),
                            Value::String(PACK_ID.to_owned()),
                        ),
                        ("expression".to_owned(), Value::String(expression.clone())),
                        (
                            "handler_reference".to_owned(),
                            Value::String(declaration.graph_node_id.clone()),
                        ),
                    ]),
                }))
            })
            .collect();
    }
    let mut declarations = BTreeMap::new();
    for declaration in &context.evidence.declarations {
        if matches!(declaration.kind.as_str(), "field" | "constant") {
            declarations.insert(declaration.range.start_byte, declaration);
        }
    }
    let mut nodes = Vec::new();
    collect_constant_nodes(context.root, context.source, &mut nodes);
    nodes
        .into_iter()
        .filter_map(|declarator| {
            let name = declarator.child_by_field_name("name")?;
            let value = declarator.child_by_field_name("value")?;
            let declaration = declarations.get(&u64::try_from(name.start_byte()).ok()?)?;
            let expression = context
                .source
                .get(value.start_byte()..value.end_byte())
                .and_then(|value| std::str::from_utf8(value).ok())?
                .trim();
            (!expression.is_empty()).then(|| {
                RawFrameworkFact::Domain(crate::RawDomainFact {
                    framework: FRAMEWORK.to_owned(),
                    kind: "_spring_constant".to_owned(),
                    name: declaration.qualified_name.clone(),
                    declaring_scope: declaration
                        .qualified_name
                        .rsplit_once("::")
                        .map(|(owner, _)| owner)
                        .unwrap_or(declaration.qualified_name.as_str())
                        .to_owned(),
                    anchor: anchor(&declaration.range),
                    origin: crate::RawFrameworkOrigin::Ast,
                    detail: Map::from_iter([
                        (
                            "frameworkPack".to_owned(),
                            Value::String(PACK_ID.to_owned()),
                        ),
                        (
                            "expression".to_owned(),
                            Value::String(expression.to_owned()),
                        ),
                        (
                            "handler_reference".to_owned(),
                            Value::String(declaration.graph_node_id.clone()),
                        ),
                    ]),
                })
            })
        })
        .collect()
}

fn collect_kotlin_constant_values(
    node: Node<'_>,
    source: &[u8],
    output: &mut BTreeMap<u64, String>,
) {
    if node.kind() == "property_declaration" {
        let text = source
            .get(node.start_byte()..node.end_byte())
            .and_then(|value| std::str::from_utf8(value).ok())
            .unwrap_or_default();
        if text.split_whitespace().any(|part| part == "const") {
            let mut variables = Vec::new();
            collect_named_kind(node, "variable_declaration", &mut variables);
            if let Some(variable) = variables.first().copied()
                && let Some(name) = first_kotlin_identifier(variable)
                && let Some(expression) = split_assignment(text).map(|(_, value)| value.trim())
                && let Ok(start) = u64::try_from(name.start_byte())
            {
                output.insert(start, expression.to_owned());
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_kotlin_constant_values(child, source, output);
    }
}

fn collect_named_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_named_kind(child, kind, output);
    }
}

fn first_kotlin_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "simple_identifier" | "type_identifier") {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(first_kotlin_identifier)
}

fn collect_constant_nodes<'tree>(node: Node<'tree>, source: &[u8], output: &mut Vec<Node<'tree>>) {
    if matches!(node.kind(), "field_declaration" | "constant_declaration") {
        let text = node
            .child_by_field_name("modifiers")
            .or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|child| child.kind() == "modifiers")
            })
            .and_then(|modifiers| source.get(modifiers.start_byte()..modifiers.end_byte()))
            .and_then(|value| std::str::from_utf8(value).ok())
            .unwrap_or_default();
        if node.kind() == "constant_declaration"
            || (text.split_whitespace().any(|part| part == "static")
                && text.split_whitespace().any(|part| part == "final"))
        {
            let mut cursor = node.walk();
            output.extend(
                node.named_children(&mut cursor)
                    .filter(|child| child.kind() == "variable_declarator"),
            );
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_constant_nodes(child, source, output);
    }
}

fn annotation_attribute_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "annotation_type_element_declaration" {
            let name = parent.child_by_field_name("name").or_else(|| {
                let mut cursor = parent.walk();
                parent
                    .named_children(&mut cursor)
                    .find(|child| child.kind() == "identifier")
            })?;
            return source
                .get(name.start_byte()..name.end_byte())
                .and_then(|value| std::str::from_utf8(value).ok())
                .map(str::to_owned);
        }
        if parent.kind() == "annotation_type_declaration" {
            break;
        }
        current = parent.parent();
    }
    None
}

fn producer_facts(
    context: &UniversalDetectionContext<'_, '_>,
    declarations: &HashMap<&str, &DeclarationFact>,
    candidates: &HashMap<&str, &crate::RelationshipCandidate>,
) -> Vec<RawFrameworkFact> {
    let mut call_nodes = BTreeMap::new();
    if context.evidence.pipeline.language == "kotlin" {
        collect_kotlin_call_nodes(context.root, &mut call_nodes);
    } else {
        collect_named_nodes(context.root, "method_invocation", "name", &mut call_nodes);
    }
    context
        .evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call)
        .filter_map(|occurrence| {
            let candidate = candidates.get(occurrence.id.as_str())?;
            if candidate.relation != CandidateRelation::Calls {
                return None;
            }
            let qualified = candidate.constraints.qualified_name.clone().or_else(|| {
                if let crate::HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name,
                    ..
                } = candidate.constraints.hierarchy.as_ref()?
                {
                    Some(format!(
                        "{receiver_qualified_name}::{}",
                        candidate.target_spelling
                    ))
                } else {
                    None
                }
            })?;
            let (kind, transport, relationship, mut argument_index) =
                producer_signature(&qualified)?;
            if qualified.ends_with("RabbitTemplate::convertAndSend") {
                argument_index =
                    usize::from(candidate.constraints.argument_count.unwrap_or(0) >= 3);
            }
            let declaration = declarations.get(occurrence.owner_declaration_id.as_str())?;
            let call = call_nodes
                .get(&usize::try_from(occurrence.range.start_byte).ok()?)
                .copied()?;
            let subject = call_argument(
                call,
                argument_index,
                context.source,
                context.evidence.pipeline.language.as_str(),
            )?;
            let mut detail = Map::new();
            detail.insert(
                "handler_reference".to_owned(),
                Value::String(declaration.graph_node_id.clone()),
            );
            detail.insert("transport".to_owned(), Value::String(transport.to_owned()));
            detail.insert(
                "relationship".to_owned(),
                Value::String(relationship.to_owned()),
            );
            detail.insert(
                "frameworkPack".to_owned(),
                Value::String(PACK_ID.to_owned()),
            );
            Some(RawFrameworkFact::Domain(crate::RawDomainFact {
                framework: FRAMEWORK.to_owned(),
                kind: kind.to_owned(),
                name: subject,
                declaring_scope: declaration.qualified_name.clone(),
                anchor: anchor(&occurrence.range),
                origin: crate::RawFrameworkOrigin::Ast,
                detail,
            }))
        })
        .collect()
}

fn producer_signature(
    qualified: &str,
) -> Option<(&'static str, &'static str, &'static str, usize)> {
    if qualified.ends_with("ApplicationEventPublisher::publishEvent") {
        Some(("event", "spring", "publishes", 0))
    } else if qualified.ends_with("KafkaTemplate::send") {
        Some(("topic", "kafka", "produces", 0))
    } else if qualified.ends_with("RabbitTemplate::convertAndSend") {
        Some(("queue", "rabbitmq", "produces", 1))
    } else {
        None
    }
}

fn call_argument(node: Node<'_>, index: usize, source: &[u8], language: &str) -> Option<String> {
    let arguments = if language == "kotlin" {
        first_named_descendant(node, "value_arguments")?
    } else {
        node.child_by_field_name("arguments")?
    };
    let mut cursor = arguments.walk();
    let argument = arguments
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .nth(index)
        .or_else(|| {
            if index == 1 {
                let mut fallback = arguments.walk();
                arguments.named_children(&mut fallback).next()
            } else {
                None
            }
        })?;
    let argument = if language == "kotlin" && argument.kind() == "value_argument" {
        let mut cursor = argument.walk();
        argument
            .named_children(&mut cursor)
            .last()
            .unwrap_or(argument)
    } else {
        argument
    };
    source
        .get(argument.start_byte()..argument.end_byte())
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(canonical_argument)
        .filter(|value| !value.is_empty())
}

fn collect_kotlin_call_nodes<'tree>(node: Node<'tree>, output: &mut BTreeMap<usize, Node<'tree>>) {
    if node.kind() == "call_expression"
        && let Some(callee) = node.named_child(0)
        && let Some(name) = if callee.kind() == "navigation_expression" {
            first_named_descendant(callee, "navigation_suffix").and_then(first_kotlin_identifier)
        } else {
            first_kotlin_identifier(callee)
        }
    {
        output.insert(name.start_byte(), node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_kotlin_call_nodes(child, output);
    }
}

fn first_named_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(|child| first_named_descendant(child, kind))
}

fn collect_named_nodes<'tree>(
    node: Node<'tree>,
    expected_kind: &str,
    field: &str,
    output: &mut BTreeMap<usize, Node<'tree>>,
) {
    if node.kind() == expected_kind
        && let Some(name) = node.child_by_field_name(field)
    {
        output.insert(name.start_byte(), node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_named_nodes(child, expected_kind, field, output);
    }
}

fn injection_targets(
    context: &UniversalDetectionContext<'_, '_>,
    owner_declaration_id: &str,
    candidates: &HashMap<&str, &crate::RelationshipCandidate>,
) -> Vec<String> {
    let mut targets = context
        .evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.owner_declaration_id == owner_declaration_id
                && occurrence.role == SemanticRole::TypeReference
                && matches!(
                    occurrence.context.as_deref(),
                    Some(
                        "type" | "parameter_type" | "property_type" | "constructor_parameter_type"
                    )
                )
        })
        .filter_map(|occurrence| {
            candidates
                .get(occurrence.id.as_str())
                .and_then(|candidate| candidate.constraints.qualified_name.clone())
                .or_else(|| Some(occurrence.spelling.clone()))
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

fn constructor_facts(
    context: &UniversalDetectionContext<'_, '_>,
    candidates: &HashMap<&str, &crate::RelationshipCandidate>,
) -> Vec<RawFrameworkFact> {
    context
        .evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "constructor")
        .filter_map(|declaration| {
            let mut targets = injection_targets(context, declaration.id.as_str(), candidates);
            if targets.is_empty()
                && context.evidence.pipeline.language == "kotlin"
                && let Some(owner) = declaration
                    .qualified_name
                    .rsplit_once("::")
                    .map(|(owner, _)| owner)
                && let Some(owner) = context.evidence.declarations.iter().find(|candidate| {
                    candidate.qualified_name == owner
                        && matches!(candidate.kind.as_str(), "class" | "annotation_type")
                })
            {
                targets = injection_targets(context, owner.id.as_str(), candidates);
            }
            (!targets.is_empty()).then(|| {
                let mut detail = Map::new();
                detail.insert(
                    "frameworkPack".to_owned(),
                    Value::String(PACK_ID.to_owned()),
                );
                detail.insert(
                    "handler_reference".to_owned(),
                    Value::String(declaration.graph_node_id.clone()),
                );
                detail.insert(
                    "targetReferences".to_owned(),
                    Value::Array(targets.into_iter().map(Value::String).collect()),
                );
                detail.insert(
                    "parameterCount".to_owned(),
                    Value::from(declaration.parameter_count.unwrap_or_default()),
                );
                RawFrameworkFact::Domain(crate::RawDomainFact {
                    framework: FRAMEWORK.to_owned(),
                    kind: "_spring_constructor".to_owned(),
                    name: declaration.qualified_name.clone(),
                    declaring_scope: declaration
                        .qualified_name
                        .rsplit_once("::")
                        .map(|(owner, _)| owner)
                        .unwrap_or(declaration.qualified_name.as_str())
                        .to_owned(),
                    anchor: anchor(&declaration.range),
                    origin: crate::RawFrameworkOrigin::Ast,
                    detail,
                })
            })
        })
        .collect()
}

fn annotation_fact(
    owner: &DeclarationFact,
    annotation_name: &str,
    annotation_qualified_name: Option<String>,
    anchor: RawFrameworkAnchor,
    arguments: Map<String, Value>,
    detail: Map<String, Value>,
) -> RawFrameworkAnnotationFact {
    RawFrameworkAnnotationFact {
        pack_id: PACK_ID.to_owned(),
        framework: FRAMEWORK.to_owned(),
        annotation_name: annotation_name.to_owned(),
        annotation_qualified_name,
        owner_declaration_id: owner.id.clone(),
        owner_graph_node_id: owner.graph_node_id.clone(),
        owner_qualified_name: owner.qualified_name.clone(),
        owner_kind: owner.kind.clone(),
        owner_signature: owner.signature.clone(),
        anchor,
        arguments,
        detail,
    }
}

fn imported_annotation_name(
    context: &UniversalDetectionContext<'_, '_>,
    spelling: &str,
) -> Option<String> {
    context
        .evidence
        .bindings
        .iter()
        .filter(|binding| binding.spelling == spelling)
        .map(|binding| binding.qualified_target.as_str())
        .find(|qualified| qualified.starts_with(SPRING_PREFIX))
        .map(str::to_owned)
}

fn annotation_fact_key(fact: &RawFrameworkFact) -> (&str, u64, &str, &str) {
    match fact {
        RawFrameworkFact::Annotation(fact) => (
            fact.anchor.source_file.as_str(),
            fact.anchor.start_byte,
            fact.owner_declaration_id.as_str(),
            fact.annotation_name.as_str(),
        ),
        RawFrameworkFact::Route(fact) => (
            fact.anchor.source_file.as_str(),
            fact.anchor.start_byte,
            fact.declaring_scope.as_str(),
            fact.operation.as_str(),
        ),
        RawFrameworkFact::Domain(fact) => (
            fact.anchor.source_file.as_str(),
            fact.anchor.start_byte,
            fact.declaring_scope.as_str(),
            fact.kind.as_str(),
        ),
    }
}

fn anchor(range: &crate::EvidenceRange) -> RawFrameworkAnchor {
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

fn collect_annotation_nodes<'tree>(node: Node<'tree>, output: &mut BTreeMap<usize, Node<'tree>>) {
    if matches!(node.kind(), "annotation" | "marker_annotation")
        && let Some(name) = annotation_name_node(node)
    {
        output.insert(name.start_byte(), node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_annotation_nodes(child, output);
    }
}

fn annotation_name_node(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find_map(|child| {
            matches!(
                child.kind(),
                "identifier" | "type_identifier" | "scoped_identifier"
            )
            .then_some(child)
            .or_else(|| annotation_name_node(child))
        })
    })
}

fn annotation_arguments(node: Node<'_>, source: &[u8]) -> Map<String, Value> {
    let Some(name) = annotation_name_node(node) else {
        return Map::new();
    };
    let start = name.end_byte().min(source.len());
    let end = node.end_byte().min(source.len());
    let Some(text) = source
        .get(start..end)
        .and_then(|value| std::str::from_utf8(value).ok())
    else {
        return Map::new();
    };
    let text = text.trim();
    let Some(text) = text
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Map::new();
    };
    let mut values = BTreeMap::<String, Vec<String>>::new();
    for argument in split_top_level(text, ',') {
        let argument = argument.trim();
        if argument.is_empty() {
            continue;
        }
        let (name, value) = split_assignment(argument)
            .map_or(("value", argument), |(name, value)| {
                (name.trim(), value.trim())
            });
        values
            .entry(name.to_owned())
            .or_default()
            .extend(flatten_argument_values(value));
    }
    values
        .into_iter()
        .map(|(name, values)| {
            (
                name,
                Value::Array(values.into_iter().map(Value::String).collect()),
            )
        })
        .collect()
}

fn split_assignment(value: &str) -> Option<(&str, &str)> {
    top_level_separator(value, '=')
        .map(|offset| (&value[..offset], &value[offset.saturating_add(1)..]))
}

fn flatten_argument_values(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(value);
    split_top_level(value, ',')
        .into_iter()
        .map(|value| canonical_argument(value.trim()))
        .filter(|value| !value.is_empty())
        .collect()
}

fn canonical_argument(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        return value[1..value.len().saturating_sub(1)].to_owned();
    }
    value.split_whitespace().collect()
}

fn split_top_level(value: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0_usize;
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' | '[' | '{' | '<' => stack.push(character),
            ')' | ']' | '}' | '>' => {
                stack.pop();
            }
            _ if character == separator && stack.is_empty() => {
                parts.push(&value[start..offset]);
                start = offset.saturating_add(character.len_utf8());
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

fn top_level_separator(value: &str, separator: char) -> Option<usize> {
    split_top_level_offsets(value, separator).into_iter().next()
}

fn split_top_level_offsets(value: &str, separator: char) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' | '[' | '{' | '<' => stack.push(character),
            ')' | ']' | '}' | '>' => {
                stack.pop();
            }
            _ if character == separator && stack.is_empty() => offsets.push(offset),
            _ => {}
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::{canonical_argument, flatten_argument_values, split_assignment, split_top_level};

    #[test]
    fn annotation_argument_parser_preserves_arrays_constants_and_nested_values() {
        assert_eq!(
            split_top_level(r#"path = {"/a", PREFIX + "/b"}, method = {GET, POST}"#, ','),
            [r#"path = {"/a", PREFIX + "/b"}"#, " method = {GET, POST}"]
        );
        assert_eq!(split_assignment("path = VALUE"), Some(("path ", " VALUE")));
        assert_eq!(
            flatten_argument_values(r#"{"/a", PREFIX + "/b"}"#),
            ["/a", "PREFIX+\"/b\""]
        );
        assert_eq!(
            canonical_argument(" RequestMethod.GET "),
            "RequestMethod.GET"
        );
    }
}
