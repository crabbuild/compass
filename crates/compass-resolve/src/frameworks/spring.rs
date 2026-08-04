use std::collections::{BTreeMap, BTreeSet};

use compass_languages::{
    Extraction, RawDomainFact, RawFrameworkAnnotationFact, RawFrameworkFact, RawFrameworkOrigin,
    RawRouteFact,
};
use serde_json::{Map, Value};

use super::FrameworkResolutionError;

const PACK_ID: &str = "spring-java";
const SPRING_WEB_ANNOTATION_PREFIX: &str = "org.springframework.web.bind.annotation.";

#[derive(Clone, Debug)]
struct MappingSpec {
    paths: Vec<String>,
    operations: Vec<String>,
    rule: &'static str,
}

pub(super) fn expand(extraction: &mut Extraction) -> Result<(), FrameworkResolutionError> {
    let mut annotations = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Annotation(annotation) if annotation.pack_id == PACK_ID => {
                Some(annotation.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let repository_beans = derive_repository_beans(extraction);
    if annotations.is_empty() {
        extraction.framework_facts.extend(repository_beans);
        return Ok(());
    }
    let constructors = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact) if fact.kind == "_spring_constructor" => {
                Some(fact.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let constants = spring_constants(extraction);
    resolve_annotation_arguments(&mut annotations, &constants);

    let by_owner = annotations.iter().fold(
        BTreeMap::<String, Vec<&RawFrameworkAnnotationFact>>::new(),
        |mut grouped, annotation| {
            grouped
                .entry(annotation.owner_declaration_id.clone())
                .or_default()
                .push(annotation);
            grouped
        },
    );
    let definitions = mapping_definitions(&annotations, &by_owner);
    let aliases = alias_definitions(&annotations);
    let controller_owners = annotations
        .iter()
        .filter(|annotation| {
            matches!(
                annotation.owner_kind.as_str(),
                "class" | "interface" | "record"
            ) && matches!(
                annotation_terminal(annotation),
                "Controller" | "RestController"
            )
        })
        .map(|annotation| annotation.owner_qualified_name.clone())
        .collect::<BTreeSet<_>>();
    let mappings = annotations
        .iter()
        .filter_map(|annotation| {
            mapping_for(annotation, &definitions, &aliases).map(|mapping| (annotation, mapping))
        })
        .collect::<Vec<_>>();

    let mut class_mappings = BTreeMap::<String, Vec<MappingSpec>>::new();
    for (annotation, mapping) in &mappings {
        if matches!(
            annotation.owner_kind.as_str(),
            "class" | "interface" | "record"
        ) {
            class_mappings
                .entry(annotation.owner_qualified_name.clone())
                .or_default()
                .push(mapping.clone());
        }
    }
    inherit_class_mappings(extraction, &mut class_mappings);

    let mut derived = repository_beans;
    let mut route_keys = BTreeSet::new();
    for (annotation, mapping) in &mappings {
        if annotation.owner_kind != "method" {
            continue;
        }
        let owner = annotation
            .owner_qualified_name
            .rsplit_once("::")
            .map(|(owner, _)| owner)
            .unwrap_or(annotation.owner_qualified_name.as_str());
        if !controller_owners.contains(owner) {
            continue;
        }
        let prefixes = class_mappings
            .get(owner)
            .map(|mappings| {
                mappings
                    .iter()
                    .flat_map(|mapping| mapping.paths.iter().cloned())
                    .collect::<Vec<_>>()
            })
            .filter(|prefixes| !prefixes.is_empty())
            .unwrap_or_else(|| vec![String::new()]);
        for prefix in prefixes {
            for path in &mapping.paths {
                let normalized = join_route_path(&prefix, path);
                for operation in &mapping.operations {
                    let key = (
                        annotation.anchor.source_file.clone(),
                        annotation.anchor.start_byte,
                        operation.clone(),
                        normalized.clone(),
                        annotation.owner_graph_node_id.clone(),
                    );
                    if !route_keys.insert(key) {
                        continue;
                    }
                    let mut detail = Map::new();
                    detail.insert(
                        "frameworkPack".to_owned(),
                        Value::String(PACK_ID.to_owned()),
                    );
                    detail.insert(
                        "target_qualified_name".to_owned(),
                        Value::String(annotation.owner_qualified_name.clone()),
                    );
                    if let Some(signature) = annotation.owner_signature.as_deref() {
                        detail.insert(
                            "target_signature_qualified".to_owned(),
                            Value::String(format!(
                                "{}{}",
                                annotation.owner_qualified_name,
                                signature
                                    .find('(')
                                    .map(|offset| &signature[offset..])
                                    .unwrap_or_default()
                            )),
                        );
                    }
                    derived.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: "spring".to_owned(),
                        operation: operation.clone(),
                        raw_path: path.clone(),
                        normalized_path: normalized.clone(),
                        declaring_scope: owner.to_owned(),
                        anchor: annotation.anchor.clone(),
                        handler_reference: annotation.owner_graph_node_id.clone(),
                        middleware_references: Vec::new(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: Some(mapping.rule.to_owned()),
                        detail,
                    }));
                }
            }
        }
    }
    derive_inherited_method_routes(
        extraction,
        &mappings,
        &class_mappings,
        &controller_owners,
        &mut route_keys,
        &mut derived,
    );

    derive_beans_and_domains(&annotations, &by_owner, &constructors, &mut derived);
    extraction.framework_facts.retain(|fact| {
        !matches!(fact, RawFrameworkFact::Annotation(annotation) if annotation.pack_id == PACK_ID)
            && !matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "_spring_constructor")
            && !matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "_spring_constant")
    });
    extraction.framework_facts.extend(derived);
    Ok(())
}

fn derive_repository_beans(extraction: &Extraction) -> Vec<RawFrameworkFact> {
    let nodes = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    extraction
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.string("relation").as_str(),
                "implements" | "inherits" | "extends"
            )
        })
        .filter_map(|edge| {
            let source = nodes.get(edge.source.as_str())?;
            let target = nodes.get(edge.target.as_str())?;
            let target_qualified = target.string("qualified_name");
            if !target_qualified.starts_with("org.springframework.data.")
                || !target_qualified.ends_with("Repository")
                || source.string("symbol_kind") != "interface"
                || !seen.insert(source.id.clone())
            {
                return None;
            }
            let qualified = source.string("qualified_name");
            Some(RawFrameworkFact::Domain(RawDomainFact {
                framework: "spring".to_owned(),
                kind: "bean_definition".to_owned(),
                name: decapitalize(terminal(&qualified)),
                declaring_scope: qualified,
                anchor: compass_languages::RawFrameworkAnchor {
                    source_file: edge.string("source_file"),
                    start_byte: edge
                        .attributes
                        .get("start_byte")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    end_byte: edge
                        .attributes
                        .get("end_byte")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    start_line: edge
                        .attributes
                        .get("line_start")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(1),
                    start_column: edge
                        .attributes
                        .get("column_start")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or_default(),
                    end_line: edge
                        .attributes
                        .get("line_end")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(1),
                    end_column: edge
                        .attributes
                        .get("column_end")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or_default(),
                },
                origin: RawFrameworkOrigin::Ast,
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
                        Value::String(source.id.clone()),
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
                ]),
            }))
        })
        .collect()
}

#[derive(Default)]
struct SpringConstants {
    by_qualified: BTreeMap<String, String>,
    by_terminal: BTreeMap<String, Vec<String>>,
}

fn spring_constants(extraction: &Extraction) -> SpringConstants {
    let mut constants = SpringConstants::default();
    for fact in &extraction.framework_facts {
        let RawFrameworkFact::Domain(fact) = fact else {
            continue;
        };
        if fact.kind != "_spring_constant" {
            continue;
        }
        let Some(expression) = fact.detail.get("expression").and_then(Value::as_str) else {
            continue;
        };
        let qualified = fact.name.replace("::", ".");
        constants
            .by_terminal
            .entry(terminal(&qualified).to_owned())
            .or_default()
            .push(qualified.clone());
        constants
            .by_qualified
            .insert(qualified, expression.to_owned());
    }
    for qualified in constants.by_terminal.values_mut() {
        qualified.sort();
        qualified.dedup();
    }
    constants
}

fn resolve_annotation_arguments(
    annotations: &mut [RawFrameworkAnnotationFact],
    constants: &SpringConstants,
) {
    for annotation in annotations {
        let bindings = annotation
            .detail
            .get("bindings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (argument_name, value) in &mut annotation.arguments {
            let Some(values) = value.as_array_mut() else {
                continue;
            };
            for value in values {
                let Some(expression) = value.as_str() else {
                    continue;
                };
                if let Some(resolved) = evaluate_constant_expression(
                    expression,
                    &bindings,
                    constants,
                    &mut BTreeSet::new(),
                    0,
                ) {
                    *value = Value::String(resolved);
                } else if matches!(argument_name.as_str(), "path" | "value")
                    && looks_like_unresolved_constant(expression)
                {
                    *value = Value::String("__COMPASS_UNRESOLVED_CONSTANT__".to_owned());
                }
            }
        }
    }
}

fn looks_like_unresolved_constant(value: &str) -> bool {
    value.contains('+')
        || (!value.starts_with('/')
            && value
                .chars()
                .any(|character| character.is_ascii_uppercase())
            && value.chars().all(|character| {
                character.is_ascii_uppercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '.')
            }))
}

fn evaluate_constant_expression(
    expression: &str,
    bindings: &Map<String, Value>,
    constants: &SpringConstants,
    visiting: &mut BTreeSet<String>,
    depth: usize,
) -> Option<String> {
    if depth >= 16 {
        return None;
    }
    let expression = expression.trim();
    if expression.len() >= 2 && expression.starts_with('"') && expression.ends_with('"') {
        return serde_json::from_str::<String>(expression).ok();
    }
    let parts = split_expression(expression, '+');
    if parts.len() > 1 {
        let mut value = String::new();
        for part in parts {
            value.push_str(&evaluate_constant_expression(
                part,
                bindings,
                constants,
                visiting,
                depth.saturating_add(1),
            )?);
        }
        return Some(value);
    }
    let normalized = expression.replace("::", ".");
    let bound = bindings
        .get(expression)
        .and_then(Value::as_str)
        .map(|value| value.replace("::", "."));
    let qualified = if constants.by_qualified.contains_key(&normalized) {
        Some(normalized)
    } else if let Some(bound) = bound {
        constants.by_qualified.contains_key(&bound).then_some(bound)
    } else {
        constants
            .by_terminal
            .get(expression)
            .filter(|matches| matches.len() == 1)
            .and_then(|matches| matches.first().cloned())
    }?;
    if !visiting.insert(qualified.clone()) {
        return None;
    }
    let value = constants
        .by_qualified
        .get(&qualified)
        .and_then(|expression| {
            evaluate_constant_expression(
                expression,
                bindings,
                constants,
                visiting,
                depth.saturating_add(1),
            )
        });
    visiting.remove(&qualified);
    value
}

fn split_expression(value: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut escaped = false;
    let mut depth = 0_u16;
    for (offset, character) in value.char_indices() {
        if quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quote = false;
            }
            continue;
        }
        match character {
            '"' => quote = true,
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if character == separator && depth == 0 => {
                parts.push(value[start..offset].trim());
                start = offset.saturating_add(character.len_utf8());
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

fn inherit_class_mappings(
    extraction: &Extraction,
    class_mappings: &mut BTreeMap<String, Vec<MappingSpec>>,
) {
    let qualified_by_id = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            let qualified = node.string("qualified_name");
            (!qualified.is_empty()).then_some((node.id.as_str(), qualified))
        })
        .collect::<BTreeMap<_, _>>();
    for _ in 0..16 {
        let mut additions = Vec::new();
        for edge in &extraction.edges {
            if !matches!(
                edge.string("relation").as_str(),
                "implements" | "inherits" | "extends"
            ) {
                continue;
            }
            let (Some(child), Some(parent)) = (
                qualified_by_id.get(edge.source.as_str()),
                qualified_by_id.get(edge.target.as_str()),
            ) else {
                continue;
            };
            if class_mappings.contains_key(child.as_str()) {
                continue;
            }
            if let Some(mappings) = class_mappings.get(parent.as_str()) {
                additions.push((child.clone(), mappings.clone()));
            }
        }
        if additions.is_empty() {
            break;
        }
        for (child, mappings) in additions {
            class_mappings.entry(child).or_insert(mappings);
        }
    }
}

fn derive_inherited_method_routes(
    extraction: &Extraction,
    mappings: &[(&RawFrameworkAnnotationFact, MappingSpec)],
    class_mappings: &BTreeMap<String, Vec<MappingSpec>>,
    controller_owners: &BTreeSet<String>,
    route_keys: &mut BTreeSet<(String, u64, String, String, String)>,
    output: &mut Vec<RawFrameworkFact>,
) {
    let nodes = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let types = extraction
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("symbol_kind").as_str(),
                "class" | "interface" | "record" | "type"
            )
        })
        .filter_map(|node| {
            let qualified = node.string("qualified_name");
            (!qualified.is_empty()).then_some((qualified, node.id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut descendants = BTreeMap::<&str, BTreeSet<&str>>::new();
    for edge in &extraction.edges {
        if matches!(
            edge.string("relation").as_str(),
            "implements" | "inherits" | "extends"
        ) {
            descendants
                .entry(edge.target.as_str())
                .or_default()
                .insert(edge.source.as_str());
        }
    }
    for _ in 0..16 {
        let snapshot = descendants.clone();
        let mut changed = false;
        for children in descendants.values_mut() {
            let inherited = children
                .clone()
                .into_iter()
                .flat_map(|child| snapshot.get(child).into_iter().flatten().copied())
                .collect::<Vec<_>>();
            for child in inherited {
                changed |= children.insert(child);
            }
        }
        if !changed {
            break;
        }
    }
    let contained =
        extraction
            .edges
            .iter()
            .fold(BTreeMap::<&str, Vec<&str>>::new(), |mut contained, edge| {
                if edge.string("relation") == "contains" {
                    contained
                        .entry(edge.source.as_str())
                        .or_default()
                        .push(edge.target.as_str());
                }
                contained
            });

    for (annotation, mapping) in mappings {
        if annotation.owner_kind != "method" {
            continue;
        }
        let Some((base_owner, method_name)) = annotation.owner_qualified_name.rsplit_once("::")
        else {
            continue;
        };
        let Some(base_id) = types.get(base_owner) else {
            continue;
        };
        if !controller_owners.contains(base_owner) {
            continue;
        }
        let expected_arity = signature_arity(annotation.owner_signature.as_deref());
        for child_id in descendants.get(base_id).into_iter().flatten() {
            for method_id in contained.get(child_id).into_iter().flatten() {
                let Some(method) = nodes.get(method_id) else {
                    continue;
                };
                if terminal(&method.string("qualified_name")) != method_name
                    || signature_arity(method.attributes.get("signature").and_then(Value::as_str))
                        != expected_arity
                {
                    continue;
                }
                let qualified = method.string("qualified_name");
                let child_owner = qualified
                    .rsplit_once("::")
                    .map(|(owner, _)| owner)
                    .unwrap_or_default();
                if !controller_owners.contains(child_owner) {
                    continue;
                }
                let prefixes = class_mappings
                    .get(child_owner)
                    .map(|mappings| {
                        mappings
                            .iter()
                            .flat_map(|mapping| mapping.paths.iter().cloned())
                            .collect::<Vec<_>>()
                    })
                    .filter(|prefixes| !prefixes.is_empty())
                    .unwrap_or_else(|| vec![String::new()]);
                for prefix in prefixes {
                    for path in &mapping.paths {
                        let normalized = join_route_path(&prefix, path);
                        for operation in &mapping.operations {
                            let key = (
                                annotation.anchor.source_file.clone(),
                                annotation.anchor.start_byte,
                                operation.clone(),
                                normalized.clone(),
                                method.id.clone(),
                            );
                            if !route_keys.insert(key) {
                                continue;
                            }
                            output.push(RawFrameworkFact::Route(RawRouteFact {
                                framework: "spring".to_owned(),
                                operation: operation.clone(),
                                raw_path: path.clone(),
                                normalized_path: normalized.clone(),
                                declaring_scope: child_owner.to_owned(),
                                anchor: annotation.anchor.clone(),
                                handler_reference: method.id.clone(),
                                middleware_references: Vec::new(),
                                origin: RawFrameworkOrigin::Ast,
                                rule: Some("spring-inherited-request-mapping".to_owned()),
                                detail: Map::from_iter([
                                    (
                                        "frameworkPack".to_owned(),
                                        Value::String(PACK_ID.to_owned()),
                                    ),
                                    (
                                        "target_qualified_name".to_owned(),
                                        Value::String(qualified.clone()),
                                    ),
                                ]),
                            }));
                        }
                    }
                }
            }
        }
    }
}

fn signature_arity(signature: Option<&str>) -> Option<usize> {
    let parameters = signature?.split_once('(')?.1.rsplit_once(')')?.0.trim();
    if parameters.is_empty() {
        Some(0)
    } else {
        Some(parameters.split(',').count())
    }
}

fn mapping_definitions(
    annotations: &[RawFrameworkAnnotationFact],
    by_owner: &BTreeMap<String, Vec<&RawFrameworkAnnotationFact>>,
) -> BTreeMap<String, MappingSpec> {
    let mut definitions = BTreeMap::new();
    for annotation in annotations {
        if annotation.owner_kind != "annotation_type" {
            continue;
        }
        let Some(owner_annotations) = by_owner.get(&annotation.owner_declaration_id) else {
            continue;
        };
        if let Some(mapping) = owner_annotations
            .iter()
            .find_map(|candidate| direct_mapping(candidate))
        {
            definitions.insert(annotation.owner_qualified_name.clone(), mapping.clone());
            definitions
                .entry(terminal(&annotation.owner_qualified_name).to_owned())
                .or_insert(mapping);
        }
    }
    definitions
}

fn alias_definitions(
    annotations: &[RawFrameworkAnnotationFact],
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut aliases = BTreeMap::<String, BTreeMap<String, String>>::new();
    for annotation in annotations {
        if !matches!(annotation.owner_kind.as_str(), "method" | "annotation_type")
            || annotation_terminal(annotation) != "AliasFor"
        {
            continue;
        }
        let (annotation_owner, attribute) = if annotation.owner_kind == "method" {
            let Some((owner, attribute)) = annotation.owner_qualified_name.rsplit_once("::") else {
                continue;
            };
            (owner, attribute)
        } else {
            let Some(attribute) = annotation
                .detail
                .get("annotationAttribute")
                .and_then(Value::as_str)
            else {
                continue;
            };
            (annotation.owner_qualified_name.as_str(), attribute)
        };
        let target = argument_values(annotation, "attribute")
            .into_iter()
            .chain(argument_values(annotation, "value"))
            .next()
            .unwrap_or_else(|| attribute.to_owned());
        for owner in [annotation_owner, terminal(annotation_owner)] {
            aliases
                .entry(owner.to_owned())
                .or_default()
                .insert(attribute.to_owned(), target.clone());
        }
    }
    aliases
}

fn mapping_for(
    annotation: &RawFrameworkAnnotationFact,
    definitions: &BTreeMap<String, MappingSpec>,
    aliases: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<MappingSpec> {
    if let Some(mapping) = direct_mapping(annotation) {
        return Some(mapping);
    }
    let qualified = annotation.annotation_qualified_name.as_deref();
    let definition_key = qualified
        .filter(|qualified| definitions.contains_key(*qualified))
        .unwrap_or(annotation.annotation_name.as_str());
    let mut mapping = definitions.get(definition_key)?.clone();
    let alias_key = qualified.unwrap_or(definition_key);
    if let Some(alias_rules) = aliases.get(alias_key) {
        for (source, target) in alias_rules {
            let values = argument_values(annotation, source);
            if values.is_empty() {
                continue;
            }
            match target.as_str() {
                "path" | "value" => mapping.paths = values,
                "method" => mapping.operations = operations_from_values(&values),
                _ => {}
            }
        }
    }
    mapping.rule = "spring-composed-request-mapping";
    Some(mapping)
}

fn direct_mapping(annotation: &RawFrameworkAnnotationFact) -> Option<MappingSpec> {
    let qualified = annotation.annotation_qualified_name.as_deref()?;
    if !qualified.starts_with(SPRING_WEB_ANNOTATION_PREFIX) {
        return None;
    }
    let name = annotation_terminal(annotation);
    let operation = match name {
        "GetMapping" => Some("GET"),
        "PostMapping" => Some("POST"),
        "PutMapping" => Some("PUT"),
        "PatchMapping" => Some("PATCH"),
        "DeleteMapping" => Some("DELETE"),
        "RequestMapping" => None,
        _ => return None,
    };
    let mut paths = argument_values(annotation, "path");
    if paths.is_empty() {
        paths = argument_values(annotation, "value");
    }
    if paths.is_empty() {
        paths.push(String::new());
    }
    paths.retain(|path| !path.contains('+') && path != "__COMPASS_UNRESOLVED_CONSTANT__");
    if paths.is_empty() {
        return None;
    }
    let operations = operation.map_or_else(
        || {
            let methods = argument_values(annotation, "method");
            if methods.is_empty() {
                vec!["ANY".to_owned()]
            } else {
                operations_from_values(&methods)
            }
        },
        |operation| vec![operation.to_owned()],
    );
    Some(MappingSpec {
        paths,
        operations,
        rule: "spring-request-mapping",
    })
}

fn operations_from_values(values: &[String]) -> Vec<String> {
    let mut operations = values
        .iter()
        .filter_map(|value| {
            let operation = value.rsplit(['.', ':']).next().unwrap_or(value).trim();
            (!operation.is_empty()).then(|| operation.to_ascii_uppercase())
        })
        .collect::<Vec<_>>();
    operations.sort();
    operations.dedup();
    if operations.is_empty() {
        operations.push("ANY".to_owned());
    }
    operations
}

fn derive_beans_and_domains(
    annotations: &[RawFrameworkAnnotationFact],
    by_owner: &BTreeMap<String, Vec<&RawFrameworkAnnotationFact>>,
    constructors: &[RawDomainFact],
    output: &mut Vec<RawFrameworkFact>,
) {
    let mut component_definitions = annotations
        .iter()
        .filter(|annotation| annotation.owner_kind == "annotation_type")
        .filter(|annotation| is_component_annotation(annotation))
        .map(|annotation| {
            (
                annotation.owner_qualified_name.clone(),
                annotation_terminal(annotation).to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for _ in 0..16 {
        let additions = annotations
            .iter()
            .filter(|annotation| annotation.owner_kind == "annotation_type")
            .filter_map(|annotation| {
                let kind = annotation
                    .annotation_qualified_name
                    .as_ref()
                    .and_then(|qualified| component_definitions.get(qualified))?;
                (!component_definitions.contains_key(&annotation.owner_qualified_name))
                    .then(|| (annotation.owner_qualified_name.clone(), kind.clone()))
            })
            .collect::<Vec<_>>();
        if additions.is_empty() {
            break;
        }
        component_definitions.extend(additions);
    }
    let component_owners = annotations
        .iter()
        .filter(|annotation| {
            is_component_owner(annotation)
                && (is_component_annotation(annotation)
                    || annotation
                        .annotation_qualified_name
                        .as_ref()
                        .is_some_and(|qualified| component_definitions.contains_key(qualified)))
        })
        .map(|annotation| {
            (
                annotation.owner_qualified_name.clone(),
                annotation.owner_graph_node_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for annotation in annotations {
        let name = annotation_terminal(annotation);
        match name {
            "Component" | "Service" | "Repository" | "Controller" | "RestController"
            | "Configuration"
                if is_spring_annotation(annotation) && is_component_owner(annotation) =>
            {
                let bean_name = argument_values(annotation, "value")
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| decapitalize(terminal(&annotation.owner_qualified_name)));
                output.push(domain_fact(
                    annotation,
                    "bean_definition",
                    &bean_name,
                    Map::from_iter([
                        ("bean_kind".to_owned(), Value::String(name.to_owned())),
                        (
                            "handler_reference".to_owned(),
                            Value::String(annotation.owner_graph_node_id.clone()),
                        ),
                        (
                            "owner_kind".to_owned(),
                            Value::String(annotation.owner_kind.clone()),
                        ),
                        (
                            "relationship".to_owned(),
                            Value::String("registers".to_owned()),
                        ),
                        (
                            "primary".to_owned(),
                            Value::Bool(owner_has_annotation(by_owner, annotation, "Primary")),
                        ),
                    ]),
                ));
            }
            "Bean" if is_spring_annotation(annotation) => {
                let bean_name = argument_values(annotation, "name")
                    .into_iter()
                    .chain(argument_values(annotation, "value"))
                    .next()
                    .unwrap_or_else(|| terminal(&annotation.owner_qualified_name).to_owned());
                output.push(domain_fact(
                    annotation,
                    "bean_definition",
                    &bean_name,
                    Map::from_iter([
                        ("bean_kind".to_owned(), Value::String("Bean".to_owned())),
                        (
                            "handler_reference".to_owned(),
                            Value::String(annotation.owner_graph_node_id.clone()),
                        ),
                        (
                            "owner_kind".to_owned(),
                            Value::String(annotation.owner_kind.clone()),
                        ),
                        (
                            "relationship".to_owned(),
                            Value::String("registers".to_owned()),
                        ),
                        (
                            "primary".to_owned(),
                            Value::Bool(owner_has_annotation(by_owner, annotation, "Primary")),
                        ),
                    ]),
                ));
            }
            "KafkaListener" if is_spring_annotation(annotation) => {
                for topic in values_or(annotation, &["topics", "topicPattern", "value"]) {
                    output.push(message_fact(
                        annotation, "topic", &topic, "kafka", "consumes",
                    ));
                }
            }
            "RabbitListener" if is_spring_annotation(annotation) => {
                for queue in values_or(annotation, &["queues", "queuesToDeclare", "value"]) {
                    output.push(message_fact(
                        annotation, "queue", &queue, "rabbitmq", "consumes",
                    ));
                }
            }
            "EventListener" if is_spring_annotation(annotation) => {
                let mut events = values_or(annotation, &["classes", "value"]);
                if events.is_empty() {
                    events = target_references(&annotation.detail);
                }
                for event in events {
                    output.push(message_fact(
                        annotation, "event", &event, "spring", "handles",
                    ));
                }
            }
            "Scheduled" if is_spring_annotation(annotation) => {
                let schedule = [
                    "cron",
                    "fixedRate",
                    "fixedRateString",
                    "fixedDelay",
                    "fixedDelayString",
                ]
                .into_iter()
                .find_map(|name| argument_values(annotation, name).into_iter().next())
                .unwrap_or_else(|| "unspecified".to_owned());
                output.push(domain_fact(
                    annotation,
                    "job",
                    &annotation.owner_qualified_name,
                    Map::from_iter([
                        (
                            "handler_reference".to_owned(),
                            Value::String(annotation.owner_graph_node_id.clone()),
                        ),
                        (
                            "owner_kind".to_owned(),
                            Value::String(annotation.owner_kind.clone()),
                        ),
                        ("schedule".to_owned(), Value::String(schedule)),
                    ]),
                ));
            }
            "Transactional" if is_transaction_annotation(annotation) => {
                output.push(domain_fact(
                    annotation,
                    "framework_decoration",
                    "transactional",
                    Map::from_iter([
                        (
                            "handler_reference".to_owned(),
                            Value::String(annotation.owner_graph_node_id.clone()),
                        ),
                        (
                            "owner_kind".to_owned(),
                            Value::String(annotation.owner_kind.clone()),
                        ),
                        (
                            "trait".to_owned(),
                            Value::String("transactional".to_owned()),
                        ),
                    ]),
                ));
            }
            "PreAuthorize" | "PostAuthorize" | "Secured" | "RolesAllowed"
                if is_security_annotation(annotation) =>
            {
                output.push(domain_fact(
                    annotation,
                    "framework_decoration",
                    "secured",
                    Map::from_iter([
                        (
                            "handler_reference".to_owned(),
                            Value::String(annotation.owner_graph_node_id.clone()),
                        ),
                        ("trait".to_owned(), Value::String("secured".to_owned())),
                        (
                            "policy".to_owned(),
                            Value::Array(
                                argument_values(annotation, "value")
                                    .into_iter()
                                    .map(Value::String)
                                    .collect(),
                            ),
                        ),
                    ]),
                ));
            }
            "Entity" if is_jpa_annotation(annotation) => {
                let table = by_owner
                    .get(&annotation.owner_declaration_id)
                    .into_iter()
                    .flatten()
                    .find(|candidate| {
                        annotation_terminal(candidate) == "Table" && is_jpa_annotation(candidate)
                    });
                let table_name = table
                    .into_iter()
                    .flat_map(|table| argument_values(table, "name"))
                    .next()
                    .unwrap_or_else(|| terminal(&annotation.owner_qualified_name).to_owned());
                let schema = table
                    .into_iter()
                    .flat_map(|table| argument_values(table, "schema"))
                    .next();
                let mut detail = Map::from_iter([
                    (
                        "model_reference".to_owned(),
                        Value::String(annotation.owner_graph_node_id.clone()),
                    ),
                    ("database_table".to_owned(), Value::String(table_name)),
                ]);
                if let Some(schema) = schema {
                    detail.insert("database_schema".to_owned(), Value::String(schema));
                }
                output.push(domain_fact(
                    annotation,
                    "orm_mapping",
                    &annotation.owner_qualified_name,
                    detail,
                ));
            }
            _ => {}
        }
    }
    for annotation in annotations.iter().filter(|annotation| {
        is_component_owner(annotation)
            && annotation
                .annotation_qualified_name
                .as_ref()
                .is_some_and(|qualified| component_definitions.contains_key(qualified))
    }) {
        let qualified = annotation
            .annotation_qualified_name
            .as_deref()
            .unwrap_or_default();
        let kind = component_definitions
            .get(qualified)
            .map(String::as_str)
            .unwrap_or("Component");
        let bean_name = argument_values(annotation, "value")
            .into_iter()
            .next()
            .unwrap_or_else(|| decapitalize(terminal(&annotation.owner_qualified_name)));
        output.push(domain_fact(
            annotation,
            "bean_definition",
            &bean_name,
            Map::from_iter([
                ("bean_kind".to_owned(), Value::String(kind.to_owned())),
                (
                    "handler_reference".to_owned(),
                    Value::String(annotation.owner_graph_node_id.clone()),
                ),
                (
                    "owner_kind".to_owned(),
                    Value::String(annotation.owner_kind.clone()),
                ),
                (
                    "relationship".to_owned(),
                    Value::String("registers".to_owned()),
                ),
                (
                    "primary".to_owned(),
                    Value::Bool(owner_has_annotation(by_owner, annotation, "Primary")),
                ),
                ("composed".to_owned(), Value::Bool(true)),
            ]),
        ));
    }
    derive_injections(annotations, constructors, &component_owners, output);
}

fn derive_injections(
    annotations: &[RawFrameworkAnnotationFact],
    constructors: &[RawDomainFact],
    component_owners: &BTreeMap<String, String>,
    output: &mut Vec<RawFrameworkFact>,
) {
    let explicit_constructors = annotations
        .iter()
        .filter(|annotation| {
            annotation.owner_kind == "constructor" && is_injection_annotation(annotation)
        })
        .map(|annotation| annotation.owner_qualified_name.as_str())
        .collect::<BTreeSet<_>>();
    let constructor_counts =
        constructors
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, constructor| {
                *counts
                    .entry(constructor.declaring_scope.as_str())
                    .or_default() += 1;
                counts
            });

    for annotation in annotations
        .iter()
        .filter(|annotation| is_injection_annotation(annotation))
    {
        let owner = enclosing_type(annotation);
        let Some(source) = component_owners.get(owner) else {
            continue;
        };
        for target in target_references(&annotation.detail) {
            output.push(injection_fact(
                &annotation.anchor,
                owner,
                source,
                &target,
                "annotated-injection",
            ));
        }
    }

    for constructor in constructors {
        let owner = constructor.declaring_scope.as_str();
        let Some(source) = component_owners.get(owner) else {
            continue;
        };
        let explicit = explicit_constructors.contains(constructor.name.as_str());
        let implicit = constructor_counts.get(owner).copied() == Some(1);
        if !explicit && !implicit {
            continue;
        }
        for target in target_references(&constructor.detail) {
            output.push(injection_fact(
                &constructor.anchor,
                owner,
                source,
                &target,
                if explicit {
                    "annotated-constructor-injection"
                } else {
                    "single-constructor-injection"
                },
            ));
        }
    }
}

fn injection_fact(
    anchor: &compass_languages::RawFrameworkAnchor,
    owner: &str,
    source: &str,
    target: &str,
    rule: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Domain(RawDomainFact {
        framework: "spring".to_owned(),
        kind: "injection".to_owned(),
        name: target.to_owned(),
        declaring_scope: owner.to_owned(),
        anchor: anchor.clone(),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            (
                "frameworkPack".to_owned(),
                Value::String(PACK_ID.to_owned()),
            ),
            (
                "source_reference".to_owned(),
                Value::String(source.to_owned()),
            ),
            (
                "target_reference".to_owned(),
                Value::String(target.to_owned()),
            ),
            (
                "relationship".to_owned(),
                Value::String("depends_on".to_owned()),
            ),
            ("rule".to_owned(), Value::String(rule.to_owned())),
        ]),
    })
}

fn target_references(detail: &Map<String, Value>) -> Vec<String> {
    detail
        .get("targetReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn enclosing_type(annotation: &RawFrameworkAnnotationFact) -> &str {
    if matches!(
        annotation.owner_kind.as_str(),
        "class" | "interface" | "record"
    ) {
        annotation.owner_qualified_name.as_str()
    } else {
        annotation
            .owner_qualified_name
            .rsplit_once("::")
            .map(|(owner, _)| owner)
            .unwrap_or(annotation.owner_qualified_name.as_str())
    }
}

fn is_component_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    matches!(
        annotation_terminal(annotation),
        "Component" | "Service" | "Repository" | "Controller" | "RestController" | "Configuration"
    ) && is_spring_annotation(annotation)
}

fn is_component_owner(annotation: &RawFrameworkAnnotationFact) -> bool {
    matches!(annotation.owner_kind.as_str(), "class" | "record")
}

fn is_injection_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    annotation
        .annotation_qualified_name
        .as_deref()
        .is_some_and(|qualified| {
            matches!(
                qualified,
                "org.springframework.beans.factory.annotation.Autowired"
                    | "jakarta.inject.Inject"
                    | "javax.inject.Inject"
            )
        })
}

fn message_fact(
    annotation: &RawFrameworkAnnotationFact,
    kind: &str,
    subject: &str,
    transport: &str,
    relationship: &str,
) -> RawFrameworkFact {
    domain_fact(
        annotation,
        kind,
        subject,
        Map::from_iter([
            (
                "handler_reference".to_owned(),
                Value::String(annotation.owner_graph_node_id.clone()),
            ),
            ("transport".to_owned(), Value::String(transport.to_owned())),
            (
                "relationship".to_owned(),
                Value::String(relationship.to_owned()),
            ),
        ]),
    )
}

fn domain_fact(
    annotation: &RawFrameworkAnnotationFact,
    kind: &str,
    name: &str,
    mut detail: Map<String, Value>,
) -> RawFrameworkFact {
    detail.insert(
        "frameworkPack".to_owned(),
        Value::String(PACK_ID.to_owned()),
    );
    RawFrameworkFact::Domain(RawDomainFact {
        framework: "spring".to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        declaring_scope: annotation.owner_qualified_name.clone(),
        anchor: annotation.anchor.clone(),
        origin: RawFrameworkOrigin::Ast,
        detail,
    })
}

fn owner_has_annotation(
    by_owner: &BTreeMap<String, Vec<&RawFrameworkAnnotationFact>>,
    annotation: &RawFrameworkAnnotationFact,
    expected: &str,
) -> bool {
    by_owner
        .get(&annotation.owner_declaration_id)
        .is_some_and(|annotations| {
            annotations
                .iter()
                .any(|candidate| annotation_terminal(candidate) == expected)
        })
}

fn values_or(annotation: &RawFrameworkAnnotationFact, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .find_map(|name| {
            let values = argument_values(annotation, name);
            (!values.is_empty()).then_some(values)
        })
        .unwrap_or_default()
}

fn argument_values(annotation: &RawFrameworkAnnotationFact, name: &str) -> Vec<String> {
    annotation
        .arguments
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn annotation_terminal(annotation: &RawFrameworkAnnotationFact) -> &str {
    annotation
        .annotation_qualified_name
        .as_deref()
        .map(terminal)
        .unwrap_or(annotation.annotation_name.as_str())
}

fn terminal(value: &str) -> &str {
    value.rsplit(['.', ':']).next().unwrap_or(value)
}

fn is_spring_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    annotation
        .annotation_qualified_name
        .as_deref()
        .is_some_and(|qualified| qualified.starts_with("org.springframework."))
}

fn is_security_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    annotation
        .annotation_qualified_name
        .as_deref()
        .is_some_and(|qualified| {
            qualified.starts_with("org.springframework.security.")
                || qualified.starts_with("jakarta.annotation.security.")
                || qualified.starts_with("javax.annotation.security.")
        })
}

fn is_transaction_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    annotation
        .annotation_qualified_name
        .as_deref()
        .is_some_and(|qualified| {
            qualified == "org.springframework.transaction.annotation.Transactional"
                || qualified == "jakarta.transaction.Transactional"
                || qualified == "javax.transaction.Transactional"
        })
}

fn is_jpa_annotation(annotation: &RawFrameworkAnnotationFact) -> bool {
    annotation
        .annotation_qualified_name
        .as_deref()
        .is_some_and(|qualified| {
            qualified.starts_with("jakarta.persistence.")
                || qualified.starts_with("javax.persistence.")
        })
}

fn decapitalize(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_lowercase().chain(chars).collect()
}

fn join_route_path(prefix: &str, path: &str) -> String {
    let prefix = normalize_route_path(prefix);
    let path = normalize_route_path(path);
    match (prefix.as_str(), path.as_str()) {
        ("/", "/") => "/".to_owned(),
        ("/", path) => path.to_owned(),
        (prefix, "/") => prefix.to_owned(),
        (prefix, path) => format!("{}{}", prefix.trim_end_matches('/'), path),
    }
}

fn normalize_route_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return "/".to_owned();
    }
    let mut normalized = String::with_capacity(path.len().saturating_add(1));
    if !path.starts_with('/') {
        normalized.push('/');
    }
    let mut slash = false;
    for character in path.chars() {
        if character == '/' {
            if slash {
                continue;
            }
            slash = true;
        } else {
            slash = false;
        }
        normalized.push(character);
    }
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{join_route_path, normalize_route_path, operations_from_values};

    #[test]
    fn spring_paths_and_methods_are_canonical() {
        assert_eq!(normalize_route_path("api//users/"), "/api/users");
        assert_eq!(join_route_path("/api", "/users"), "/api/users");
        assert_eq!(
            operations_from_values(&["RequestMethod.POST".to_owned(), "GET".to_owned()]),
            ["GET", "POST"]
        );
    }
}
