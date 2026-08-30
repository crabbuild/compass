//! Project-wide ASP.NET MVC route expansion from universal C# annotations.

use std::collections::{BTreeMap, BTreeSet};

use compass_languages::{
    Extraction, RawFrameworkAnnotationFact, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};
use serde_json::{Map, Value};

use super::FrameworkResolutionError;

const PACK_ID: &str = "aspnet-csharp";

#[derive(Clone, Debug)]
struct Mapping {
    operations: Vec<String>,
    paths: Vec<String>,
    rule: &'static str,
}

pub(super) fn expand(extraction: &mut Extraction) -> Result<(), FrameworkResolutionError> {
    let annotations = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Annotation(annotation) if annotation.pack_id == PACK_ID => {
                Some(annotation.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if annotations.is_empty() {
        return Ok(());
    }
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
    let controllers = controller_types(&annotations);
    let class_routes = class_route_templates(&annotations);
    let mut routes = Vec::new();
    let mut seen = BTreeSet::new();
    for annotation in &annotations {
        if annotation.owner_kind != "method" || terminal(annotation) == "NonAction" {
            continue;
        }
        let owner_type = annotation
            .owner_qualified_name
            .rsplit_once("::")
            .map(|(owner, _)| owner)
            .unwrap_or_default();
        if !controllers.contains(owner_type) {
            continue;
        }
        let Some(owner_annotations) = by_owner.get(&annotation.owner_declaration_id) else {
            continue;
        };
        if owner_annotations
            .iter()
            .any(|annotation| terminal(annotation) == "NonAction")
        {
            continue;
        }
        let http_mappings = owner_annotations
            .iter()
            .filter_map(|annotation| http_mapping(annotation))
            .collect::<Vec<_>>();
        if http_mappings.is_empty() {
            continue;
        }
        let action_routes = owner_annotations
            .iter()
            .filter(|annotation| terminal(annotation) == "Route")
            .flat_map(|annotation| argument_strings(annotation, "0", "Template"))
            .collect::<Vec<_>>();
        let prefixes = class_routes
            .get(owner_type)
            .cloned()
            .filter(|routes| !routes.is_empty())
            .unwrap_or_else(|| vec![String::new()]);
        for mapping in http_mappings {
            let paths = if action_routes.is_empty() {
                mapping.paths.clone()
            } else {
                action_routes.clone()
            };
            for prefix in &prefixes {
                for path in &paths {
                    let expanded_prefix =
                        expand_tokens(prefix, owner_type, &annotation.owner_qualified_name);
                    let expanded_path =
                        expand_tokens(path, owner_type, &annotation.owner_qualified_name);
                    let normalized = compose_route(&expanded_prefix, &expanded_path);
                    for operation in &mapping.operations {
                        let key = (
                            annotation.anchor.source_file.clone(),
                            annotation.anchor.start_byte,
                            operation.clone(),
                            normalized.clone(),
                            annotation.owner_graph_node_id.clone(),
                        );
                        if !seen.insert(key) {
                            continue;
                        }
                        let mut detail = Map::from_iter([
                            (
                                "frameworkPack".to_owned(),
                                Value::String(PACK_ID.to_owned()),
                            ),
                            (
                                "target_qualified_name".to_owned(),
                                Value::String(annotation.owner_qualified_name.clone()),
                            ),
                        ]);
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
                        routes.push(RawFrameworkFact::Route(RawRouteFact {
                            framework: "aspnet".to_owned(),
                            operation: operation.clone(),
                            raw_path: path.clone(),
                            normalized_path: normalized.clone(),
                            declaring_scope: owner_type.to_owned(),
                            anchor: annotation.anchor.clone(),
                            handler_reference: format!(
                                "{}.{}",
                                owner_type.rsplit('.').next().unwrap_or(owner_type),
                                annotation
                                    .owner_qualified_name
                                    .rsplit("::")
                                    .next()
                                    .unwrap_or(&annotation.owner_qualified_name)
                            ),
                            middleware_references: Vec::new(),
                            stages: Vec::new(),
                            origin: RawFrameworkOrigin::Ast,
                            rule: Some(if action_routes.is_empty() {
                                mapping.rule.to_owned()
                            } else {
                                "aspnet-action-route-attribute".to_owned()
                            }),
                            detail,
                        }));
                    }
                }
            }
        }
    }
    extraction.framework_facts.retain(|fact| {
        !matches!(fact, RawFrameworkFact::Annotation(annotation) if annotation.pack_id == PACK_ID)
    });
    extraction.framework_facts.extend(routes);
    Ok(())
}

fn controller_types(annotations: &[RawFrameworkAnnotationFact]) -> BTreeSet<String> {
    let mut controllers = annotations
        .iter()
        .filter(|annotation| matches!(annotation.owner_kind.as_str(), "class" | "record"))
        .filter(|annotation| {
            matches!(
                terminal(annotation),
                "ApiController" | "Controller" | "Route"
            ) || annotation.owner_qualified_name.ends_with("Controller")
        })
        .map(|annotation| annotation.owner_qualified_name.clone())
        .collect::<BTreeSet<_>>();
    controllers.extend(
        annotations
            .iter()
            .filter(|annotation| annotation.owner_kind == "method")
            .filter_map(|annotation| {
                annotation
                    .owner_qualified_name
                    .rsplit_once("::")
                    .map(|(owner, _)| owner)
                    .filter(|owner| {
                        owner
                            .rsplit('.')
                            .next()
                            .is_some_and(|name| name.ends_with("Controller"))
                    })
                    .map(str::to_owned)
            }),
    );
    controllers
}

fn class_route_templates(
    annotations: &[RawFrameworkAnnotationFact],
) -> BTreeMap<String, Vec<String>> {
    let mut output = BTreeMap::new();
    for annotation in annotations {
        if matches!(annotation.owner_kind.as_str(), "class" | "record")
            && terminal(annotation) == "Route"
        {
            output
                .entry(annotation.owner_qualified_name.clone())
                .or_insert_with(Vec::new)
                .extend(argument_strings(annotation, "0", "Template"));
        }
    }
    output
}

fn http_mapping(annotation: &RawFrameworkAnnotationFact) -> Option<Mapping> {
    let (operations, rule) = match terminal(annotation) {
        "HttpDelete" => (vec!["DELETE".to_owned()], "aspnet-http-attribute"),
        "HttpGet" => (vec!["GET".to_owned()], "aspnet-http-attribute"),
        "HttpHead" => (vec!["HEAD".to_owned()], "aspnet-http-attribute"),
        "HttpOptions" => (vec!["OPTIONS".to_owned()], "aspnet-http-attribute"),
        "HttpPatch" => (vec!["PATCH".to_owned()], "aspnet-http-attribute"),
        "HttpPost" => (vec!["POST".to_owned()], "aspnet-http-attribute"),
        "HttpPut" => (vec!["PUT".to_owned()], "aspnet-http-attribute"),
        "AcceptVerbs" => (
            accept_verbs(annotation)
                .into_iter()
                .flat_map(|value| {
                    value
                        .split(',')
                        .map(|part| part.trim().trim_matches('"').to_ascii_uppercase())
                        .collect::<Vec<_>>()
                })
                .filter(|value| !value.is_empty())
                .collect(),
            "aspnet-accept-verbs-attribute",
        ),
        _ => return None,
    };
    let paths = if terminal(annotation) == "AcceptVerbs" {
        annotation
            .arguments
            .get("Route")
            .or_else(|| annotation.arguments.get("Template"))
            .and_then(Value::as_str)
            .map(|value| vec![value.to_owned()])
            .unwrap_or_default()
    } else {
        argument_strings(annotation, "0", "Template")
    };
    Some(Mapping {
        operations,
        paths: if paths.is_empty() {
            vec![String::new()]
        } else {
            paths
        },
        rule,
    })
}

fn accept_verbs(annotation: &RawFrameworkAnnotationFact) -> Vec<String> {
    let mut positional = annotation
        .arguments
        .iter()
        .filter_map(|(key, value)| key.parse::<u32>().ok().map(|position| (position, value)))
        .collect::<Vec<_>>();
    positional.sort_unstable_by_key(|(position, _)| *position);
    let mut output = positional
        .into_iter()
        .flat_map(|(_, value)| match value {
            Value::Array(values) => values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Value::String(value) => vec![value.clone()],
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    if let Some(value) = annotation.arguments.get("HttpMethods") {
        output.extend(match value {
            Value::Array(values) => values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            Value::String(value) => vec![value.clone()],
            _ => Vec::new(),
        });
    }
    output
}

fn argument_strings(
    annotation: &RawFrameworkAnnotationFact,
    positional: &str,
    named: &str,
) -> Vec<String> {
    annotation
        .arguments
        .get(named)
        .or_else(|| annotation.arguments.get(positional))
        .map(|value| match value {
            Value::Array(values) => values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            Value::String(value) => vec![value.clone()],
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

fn expand_tokens(template: &str, owner_type: &str, method: &str) -> String {
    let controller = owner_type
        .rsplit('.')
        .next()
        .unwrap_or(owner_type)
        .trim_end_matches("Controller");
    let action = method.rsplit("::").next().unwrap_or(method);
    template
        .replace("[controller]", controller)
        .replace("[action]", action)
}

fn compose_route(prefix: &str, action: &str) -> String {
    if let Some(absolute) = action.strip_prefix("~/") {
        return normalize(absolute);
    }
    if action.starts_with('/') || prefix.is_empty() {
        return normalize(action);
    }
    normalize(&format!("{prefix}/{action}"))
}

fn normalize(value: &str) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(1));
    output.push('/');
    let mut slash = true;
    for character in value.trim().trim_matches('/').chars() {
        if character == '/' {
            if !slash {
                output.push('/');
            }
            slash = true;
        } else {
            output.push(character);
            slash = false;
        }
    }
    if output.len() > 1 && output.ends_with('/') {
        output.pop();
    }
    output
}

fn terminal(annotation: &RawFrameworkAnnotationFact) -> &str {
    annotation.annotation_name.trim_end_matches("Attribute")
}
