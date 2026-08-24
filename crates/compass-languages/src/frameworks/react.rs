//! Conservative React UI-role evidence over the prepared TypeScript/JavaScript
//! tree. This module deliberately emits project-neutral facts; target identity
//! and graph publication remain resolver responsibilities.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawFrameworkRoleFact,
};

use super::UniversalDetectionContext;
use super::typescript_syntax::TypeScriptSyntax;

const MAX_HOOK_CLOSURE: usize = 128;
const REACT_BUILTIN_HOOKS: &[&str] = &[
    "use",
    "useActionState",
    "useCallback",
    "useContext",
    "useDebugValue",
    "useDeferredValue",
    "useEffect",
    "useEffectEvent",
    "useId",
    "useImperativeHandle",
    "useInsertionEffect",
    "useLayoutEffect",
    "useMemo",
    "useOptimistic",
    "useReducer",
    "useRef",
    "useState",
    "useSyncExternalStore",
    "useTransition",
];

pub(super) fn detect(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let syntax = TypeScriptSyntax::new(context.root, context.source);
    let has_project_runtime = context.project.is_some_and(|project| {
        let jsx_runtime_is_react = project.jsx_import_sources().is_empty()
            || project
                .jsx_import_sources()
                .iter()
                .all(|source| react_jsx_runtime(project, source));
        jsx_runtime_is_react
            && project.has_any_dependency(&[
                "react",
                "react-dom",
                "react-router",
                "react-router-dom",
                "@remix-run/react",
                "remix",
                "@tanstack/react-router",
                "@vitejs/plugin-react",
                "@vitejs/plugin-react-swc",
            ])
    });
    let has_source_runtime = context.evidence.bindings.iter().any(|binding| {
        let module = binding
            .qualified_target
            .split_once("::")
            .map_or(binding.qualified_target.as_str(), |(module, _)| module);
        let import_name = module
            .split_once('/')
            .map_or(module, |(package, _)| package);
        let import_resolves_to_react = context.project.is_none_or(|project| {
            project
                .dependency_aliases()
                .get(import_name)
                .is_none_or(|target| target == import_name)
        });
        import_resolves_to_react
            && matches!(
                module,
                "react"
                    | "react-dom"
                    | "react/jsx-runtime"
                    | "react/jsx-dev-runtime"
                    | "react-dom/client"
            )
    });
    if !has_project_runtime && !has_source_runtime {
        return Vec::new();
    }

    let factory_component_targets = react_factory_component_targets(context);

    let has_client_directive = syntax.top_level_directive("use client");
    let has_server_directive = syntax.top_level_directive("use server");
    let qualified_hooks = qualifying_hook_declarations(context);
    let mut facts = Vec::new();
    let mut seen = BTreeSet::new();

    for declaration in &context.evidence.declarations {
        let node = smallest_covering_node(
            context.root,
            declaration.range.start_byte as usize,
            declaration.range.end_byte as usize,
        )
        .map(covering_declaration_node);
        let has_jsx = node.is_some_and(|node| declaration_contains_jsx(&syntax, node));
        let is_component = declaration
            .name
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
            && is_component_declaration(&declaration.kind)
            && (factory_component_targets.contains(&declaration.id) || has_jsx);
        let is_hook = qualified_hooks.contains(&declaration.id);

        let mut roles = BTreeSet::new();
        if is_component {
            roles.insert("ui_component");
        }
        if is_hook {
            roles.insert("hook");
        }
        // A Next/React client module marks every component declaration in the
        // module as client code, including private helpers that are rendered
        // by an exported component.  Requiring a direct `export` ancestor
        // misses the common `export { Component }` form as well as local
        // components such as option rows in a client-only file.
        if has_client_directive && is_component {
            roles.insert("client_boundary");
            roles.insert("client_component");
        }
        let has_local_server_directive = node
            .and_then(|node| node.child_by_field_name("body"))
            .is_some_and(|body| {
                TypeScriptSyntax::new(body, context.source).top_level_directive("use server")
            });
        if (has_server_directive || has_local_server_directive)
            && declaration.kind == "function"
            && node.is_some_and(|node| is_exported_declaration(node))
        {
            roles.insert("server_function");
        }
        if roles.is_empty() {
            continue;
        }

        for role in roles {
            let key = (declaration.graph_node_id.clone(), role.to_owned());
            if !seen.insert(key) {
                continue;
            }
            let mut detail = Map::new();
            detail.insert("pack_id".to_owned(), Value::String("react-ui".to_owned()));
            detail.insert(
                "source_reference".to_owned(),
                Value::String(declaration.graph_node_id.clone()),
            );
            detail.insert("role".to_owned(), Value::String(role.to_owned()));
            detail.insert(
                "declaration_id".to_owned(),
                Value::String(declaration.id.clone()),
            );
            let anchor = RawFrameworkAnchor {
                source_file: declaration.range.source_file.clone(),
                start_byte: declaration.range.start_byte,
                end_byte: declaration.range.end_byte,
                start_line: declaration.range.start_line,
                start_column: declaration.range.start_column,
                end_line: declaration.range.end_line,
                end_column: declaration.range.end_column,
            };
            facts.push(RawFrameworkFact::Domain(RawDomainFact {
                framework: "react".to_owned(),
                kind: "ui_role".to_owned(),
                name: declaration.name.clone(),
                declaring_scope: declaration.scope_id.clone().unwrap_or_default(),
                anchor: anchor.clone(),
                origin: RawFrameworkOrigin::Ast,
                detail,
            }));
            facts.push(RawFrameworkFact::Role(RawFrameworkRoleFact {
                pack_id: "react-ui".to_owned(),
                framework: "react".to_owned(),
                role: role.to_owned(),
                subject_reference: Some(declaration.graph_node_id.clone()),
                context: declaration.scope_id.clone(),
                anchor,
                origin: RawFrameworkOrigin::Ast,
                evidence_class: "exact".to_owned(),
                detail: Map::from_iter([(
                    "declaration_id".to_owned(),
                    Value::String(declaration.id.clone()),
                )]),
            }));
        }
    }

    facts.sort_by_key(fact_key);
    facts
}

fn react_jsx_runtime(project: &crate::ProjectEvidence, source: &str) -> bool {
    if source == "react" || source.ends_with("/react") {
        return true;
    }
    // Remix's documented `remix/ui` automatic JSX runtime is a React-backed
    // runtime. It is only accepted when the owning package also proves Remix
    // activation; a package name or JSX-shaped source alone is insufficient.
    if matches!(source, "remix/ui" | "@remix-run/react")
        && project.has_any_dependency(&[
            "remix",
            "@remix-run/dev",
            "@remix-run/node",
            "@remix-run/react",
            "@remix-run/router",
            "@remix-run/serve",
        ])
    {
        return true;
    }
    false
}

fn react_factory_component_targets(
    context: &UniversalDetectionContext<'_, '_>,
) -> BTreeSet<String> {
    let occurrences = context
        .evidence
        .occurrences
        .iter()
        .map(|occurrence| (occurrence.id.as_str(), occurrence))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut targets = BTreeSet::new();
    for call in context.evidence.candidates.iter().filter(|candidate| {
        candidate.relation == crate::CandidateRelation::Calls
            && candidate.target_spelling == "createElement"
            && candidate
                .constraints
                .module_or_package
                .as_deref()
                .is_some_and(|module| module == "react" || module.starts_with("react/"))
    }) {
        let Some(occurrence) = call
            .occurrence_id
            .as_deref()
            .and_then(|id| occurrences.get(id))
        else {
            continue;
        };
        let Some(target) = context
            .evidence
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == crate::CandidateRelation::References
                    && candidate.source_declaration_id == call.source_declaration_id
                    && candidate.constraints.exact_target_declaration_id.is_some()
                    && candidate
                        .occurrence_id
                        .as_deref()
                        .and_then(|id| occurrences.get(id))
                        .is_some_and(|reference| {
                            reference.context.as_deref() == Some("value")
                                && reference.range.start_byte > occurrence.range.end_byte
                        })
            })
            .min_by_key(|candidate| {
                candidate
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| occurrences.get(id))
                    .map_or(u64::MAX, |reference| reference.range.start_byte)
            })
            .and_then(|candidate| candidate.constraints.exact_target_declaration_id.clone())
        else {
            continue;
        };
        targets.insert(target);
        targets.insert(call.source_declaration_id.clone());
    }
    targets
}

fn fact_key(fact: &RawFrameworkFact) -> (String, String, String) {
    match fact {
        RawFrameworkFact::Domain(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            fact.detail
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        RawFrameworkFact::Route(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            "route".to_owned(),
        ),
        RawFrameworkFact::Annotation(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            "annotation".to_owned(),
        ),
        RawFrameworkFact::Role(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            fact.role.clone(),
        ),
        RawFrameworkFact::Relation(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            fact.relation.clone(),
        ),
        RawFrameworkFact::Configuration(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            fact.field.clone(),
        ),
        RawFrameworkFact::FileSet(fact) => (
            fact.anchor.source_file.clone(),
            fact.anchor.start_byte.to_string(),
            fact.owner_reference.clone(),
        ),
    }
}

fn is_custom_hook(kind: &str, name: &str) -> bool {
    matches!(kind, "function" | "variable" | "property")
        && name
            .strip_prefix("use")
            .is_some_and(|suffix| suffix.chars().next().is_some_and(char::is_uppercase))
}

fn is_component_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "closure" | "function" | "method" | "variable"
    )
}

fn declaration_contains_jsx(syntax: &TypeScriptSyntax<'_, '_>, node: Node<'_>) -> bool {
    let has_jsx = |node: Node<'_>| {
        ["jsx_element", "jsx_self_closing_element", "jsx_fragment"]
            .iter()
            .any(|kind| syntax.contains_kind(node, kind))
    };
    match node.kind() {
        "variable_declarator" => node
            .child_by_field_name("value")
            .is_some_and(|value| match value.kind() {
                "arrow_function" | "function_expression" => has_jsx(value),
                "call_expression" => value
                    .named_children(&mut value.walk())
                    .find(|child| child.kind() == "arguments")
                    .is_some_and(has_jsx),
                _ => false,
            }),
        "function_declaration" | "function_expression" | "arrow_function" | "method_definition" => {
            node.child_by_field_name("body").is_some_and(has_jsx)
        }
        "class_declaration" | "class_expression" => has_jsx(node),
        _ => false,
    }
}

fn qualifying_hook_declarations(context: &UniversalDetectionContext<'_, '_>) -> BTreeSet<String> {
    let callable_ids = context
        .evidence
        .declarations
        .iter()
        .filter(|declaration| is_custom_hook(&declaration.kind, &declaration.name))
        .map(|declaration| declaration.id.clone())
        .collect::<BTreeSet<_>>();
    let mut qualified = BTreeSet::new();
    for _ in 0..MAX_HOOK_CLOSURE {
        let mut changed = false;
        for candidate in &context.evidence.candidates {
            if candidate.relation != crate::CandidateRelation::Calls
                || !callable_ids.contains(&candidate.source_declaration_id)
            {
                continue;
            }
            let builtin = candidate.constraints.module_or_package.as_deref() == Some("react")
                && REACT_BUILTIN_HOOKS.contains(&candidate.target_spelling.as_str());
            let custom = candidate
                .constraints
                .exact_target_declaration_id
                .as_ref()
                .is_some_and(|target| qualified.contains(target));
            if (builtin || custom) && qualified.insert(candidate.source_declaration_id.clone()) {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    qualified
}

fn smallest_covering_node<'tree>(
    node: Node<'tree>,
    start: usize,
    end: usize,
) -> Option<Node<'tree>> {
    if node.start_byte() > start || node.end_byte() < end {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = smallest_covering_node(child, start, end) {
            return Some(found);
        }
    }
    Some(node)
}

fn covering_declaration_node<'tree>(node: Node<'tree>) -> Node<'tree> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if matches!(
            parent.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "class_declaration"
                | "class_expression"
                | "method_definition"
                | "variable_declarator"
                | "public_field_definition"
        ) {
            return parent;
        }
        current = parent;
    }
    node
}

fn is_exported_declaration(node: Node<'_>) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "export_statement" {
            return true;
        }
        if matches!(
            parent.kind(),
            "program" | "statement_block" | "class_body" | "object"
        ) {
            return false;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_custom_hook;

    #[test]
    fn custom_hook_classification_requires_use_and_an_uppercase_suffix() {
        assert!(is_custom_hook("function", "useOrders"));
        assert!(!is_custom_hook("function", "use"));
        assert!(!is_custom_hook("function", "user"));
        assert!(!is_custom_hook("class", "useOrders"));
    }
}
