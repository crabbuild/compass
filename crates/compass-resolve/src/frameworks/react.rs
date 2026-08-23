//! Resolver-side React projections.
//!
//! The language pack owns syntax detection and emits exact UI-role facts. This
//! module only projects already-resolved JSX references into the framework
//! relation vocabulary. It intentionally keeps the underlying `references`
//! edge and never upgrades an unresolved or ambiguous candidate.

use std::collections::BTreeSet;

use compass_languages::{Extraction, RawEdgeRecord, RawFrameworkFact};
use serde_json::{Value, json};

const MAX_RENDER_EDGES: usize = 100_000;

/// React's per-file facts already contain complete syntax evidence. The
/// adapter exists so the descriptor participates in the same universal pack
/// lifecycle as every other production descriptor without adding a second
/// project-wide parser or mutating unrelated facts.
pub(super) fn expand(_extraction: &mut Extraction) -> Result<(), super::FrameworkResolutionError> {
    Ok(())
}

pub(crate) fn project_render_relations(extraction: &mut Extraction) {
    let component_ids = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact)
                if fact.framework == "react"
                    && fact.kind == "ui_role"
                    && fact.detail.get("role").and_then(Value::as_str) == Some("ui_component") =>
            {
                fact.detail
                    .get("source_reference")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            RawFrameworkFact::Role(fact)
                if fact.framework == "react" && fact.role == "ui_component" =>
            {
                fact.subject_reference.clone()
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let client_component_ids = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact)
                if fact.framework == "react"
                    && fact.kind == "ui_role"
                    && fact.detail.get("role").and_then(Value::as_str)
                        == Some("client_component") =>
            {
                fact.detail
                    .get("source_reference")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            RawFrameworkFact::Role(fact)
                if fact.framework == "react" && fact.role == "client_component" =>
            {
                fact.subject_reference.clone()
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let node_kinds = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.string("symbol_kind")))
        .collect::<std::collections::BTreeMap<_, _>>();
    // JSX is itself a direct semantic use of the target.  A function/class
    // target may not have received a separate `ui_component` role (for
    // example when syntax-role detection was intentionally conservative), so
    // retain exact local callable targets as a fallback.  Variables and
    // properties are deliberately excluded here: mutable aliases such as
    // `let Tag; Tag = Link;` and context values created by `createContext`
    // are exact references but are not component identities.  Treating them
    // as renders creates a misleading graph edge and makes `<Context.Provider>`
    // look like a component render.
    let exact_jsx_target_ids = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "references"
                && edge.string("context") == "jsx"
                && !edge.attributes.contains_key("external")
                && edge.attributes.get("deferred") != Some(&Value::Bool(true))
                && !edge.string("resolution_rule").contains("external")
                && !edge.string("resolution_rule").contains("deferred")
                && node_kinds.get(edge.target.as_str()).is_some_and(|kind| {
                    matches!(kind.as_str(), "function" | "method" | "class" | "component")
                })
        })
        .map(|edge| edge.target.clone())
        .collect::<BTreeSet<_>>();
    let mut factory_calls = extraction
        .edges
        .iter()
        .filter_map(|edge| {
            (edge.string("relation") == "calls")
                .then(|| react_factory_kind(edge))
                .flatten()
                .map(|kind| {
                    (
                        edge.source.clone(),
                        edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX),
                        edge.string("end_byte").parse::<u64>().unwrap_or(u64::MAX),
                        kind,
                    )
                })
        })
        .collect::<Vec<_>>();
    factory_calls.sort_by(|left, right| {
        (left.0.as_str(), left.1, left.2, left.3.as_str()).cmp(&(
            right.0.as_str(),
            right.1,
            right.2,
            right.3.as_str(),
        ))
    });
    if component_ids.is_empty() && factory_calls.is_empty() {
        return;
    }
    let mut existing = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "renders")
        .map(render_key)
        .collect::<BTreeSet<_>>();
    let mut additions = Vec::new();

    for edge in &extraction.edges {
        if additions.len() >= MAX_RENDER_EDGES
            || edge.string("relation") != "references"
            || edge.string("context") != "jsx"
            || (!component_ids.contains(&edge.target)
                && !exact_jsx_target_ids.contains(&edge.target))
            || edge.source == edge.target
            || !node_kinds.get(edge.source.as_str()).is_some_and(|kind| {
                matches!(
                    kind.as_str(),
                    "file" | "module" | "function" | "closure" | "method" | "class" | "component"
                )
            })
            || !node_kinds.get(edge.target.as_str()).is_some_and(|kind| {
                matches!(
                    kind.as_str(),
                    "function" | "method" | "class" | "component" | "variable" | "property"
                )
            })
        {
            continue;
        }
        // Universal projection only emits an edge after an exact resolution
        // decision. Reject external/deferred placeholders defensively so a
        // future resolver path cannot manufacture a render relation here.
        if edge.attributes.get("deferred") == Some(&Value::Bool(true))
            || edge.attributes.get("external") == Some(&Value::Bool(true))
            || edge.string("resolution_rule").contains("external")
            || edge.string("resolution_rule").contains("deferred")
        {
            continue;
        }
        let key = render_key(edge);
        if !existing.insert(key) {
            continue;
        }
        let mut attributes = edge.attributes.clone();
        attributes.insert("relation".to_owned(), Value::String("renders".to_owned()));
        attributes.insert("render_kind".to_owned(), Value::String("jsx".to_owned()));
        attributes.insert(
            "rule".to_owned(),
            Value::String("react-jsx-render".to_owned()),
        );
        attributes.insert("framework".to_owned(), Value::String("react".to_owned()));
        if client_component_ids.contains(&edge.source)
            || client_component_ids.contains(&edge.target)
        {
            attributes.insert("boundary".to_owned(), Value::String("client".to_owned()));
        }
        additions.push(RawEdgeRecord {
            source: edge.source.clone(),
            target: edge.target.clone(),
            attributes,
        });
    }
    for (index, (source, _start_byte, end_byte, kind)) in factory_calls.iter().enumerate() {
        if additions.len() >= MAX_RENDER_EDGES {
            break;
        }
        let next_start = factory_calls
            .iter()
            .skip(index.saturating_add(1))
            .find(|(candidate_source, _, _, _)| candidate_source == source)
            .map(|(_, candidate_start, _, _)| *candidate_start);
        let expected_context = if *kind == "create_element" {
            "value"
        } else {
            "jsx"
        };
        let Some(reference) = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == *source
                    && edge.string("relation") == "references"
                    && edge.string("context") == expected_context
                    && edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX) > *end_byte
                    && next_start.is_none_or(|next| {
                        edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX) < next
                    })
                    && node_kinds
                        .get(edge.target.as_str())
                        .is_some_and(|kind| render_target_kind(kind))
                    && !edge.attributes.contains_key("external")
                    && !edge.string("resolution_rule").contains("external")
            })
            .min_by_key(|edge| edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX))
        else {
            continue;
        };
        if !node_kinds
            .get(reference.source.as_str())
            .is_some_and(|kind| {
                matches!(
                    kind.as_str(),
                    "file" | "module" | "function" | "closure" | "method" | "class" | "component"
                )
            })
        {
            continue;
        }
        let mut attributes = reference.attributes.clone();
        attributes.insert("relation".to_owned(), Value::String("renders".to_owned()));
        attributes.insert("render_kind".to_owned(), Value::String(kind.clone()));
        attributes.insert(
            "rule".to_owned(),
            Value::String(format!("react-{kind}-render")),
        );
        attributes.insert("framework".to_owned(), Value::String("react".to_owned()));
        if client_component_ids.contains(&reference.source)
            || client_component_ids.contains(&reference.target)
        {
            attributes.insert("boundary".to_owned(), Value::String("client".to_owned()));
        }
        let key = render_key(&RawEdgeRecord {
            source: reference.source.clone(),
            target: reference.target.clone(),
            attributes: attributes.clone(),
        });
        if existing.insert(key) {
            additions.push(RawEdgeRecord {
                source: reference.source.clone(),
                target: reference.target.clone(),
                attributes,
            });
        }
    }
    project_lazy_render_relations(extraction, &node_kinds, &mut existing, &mut additions);
    if additions.len() >= MAX_RENDER_EDGES {
        extraction.extensions.insert(
            "react_render_diagnostics".to_owned(),
            json!([{
                "kind": "render_edge_limit",
                "maximum": MAX_RENDER_EDGES,
                "observed": additions.len().saturating_add(1),
            }]),
        );
    }
    extraction.edges.extend(additions);
}

fn react_factory_kind(edge: &RawEdgeRecord) -> Option<String> {
    if edge.string("resolution_rule") != "qualified-external" {
        return None;
    }
    let target = edge.target.to_ascii_lowercase();
    if !target.starts_with("external_") || !target.contains("react") {
        return None;
    }
    if target.contains("createelement") {
        Some("create_element".to_owned())
    } else if target.contains("createroot") {
        Some("root".to_owned())
    } else if target.contains("react") && target.contains("lazy") {
        Some("lazy".to_owned())
    } else if target.contains("next") && target.contains("dynamic") {
        Some("dynamic".to_owned())
    } else {
        None
    }
}

fn project_lazy_render_relations(
    extraction: &Extraction,
    node_kinds: &std::collections::BTreeMap<&str, String>,
    existing: &mut BTreeSet<(String, String, String, String, String)>,
    additions: &mut Vec<RawEdgeRecord>,
) {
    let lazy_calls = extraction
        .edges
        .iter()
        .filter_map(|edge| {
            (edge.string("relation") == "calls")
                .then(|| react_factory_kind(edge))
                .flatten()
                .filter(|kind| kind == "lazy" || kind == "dynamic")
                .map(|kind| (edge.source.clone(), edge, kind))
        })
        .collect::<Vec<_>>();
    for (source, call, kind) in lazy_calls {
        if additions.len() >= MAX_RENDER_EDGES {
            break;
        }
        let call_start = call.string("start_byte").parse::<u64>().unwrap_or(u64::MAX);
        let Some(owner) = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == source
                    && edge.string("relation") == "contains"
                    && node_kinds
                        .get(edge.target.as_str())
                        .is_some_and(|node_kind| node_kind == "variable")
                    && edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX) < call_start
            })
            .max_by_key(|edge| edge.string("start_byte").parse::<u64>().unwrap_or(0))
            .map(|edge| edge.target.clone())
        else {
            continue;
        };
        if !node_kinds.get(owner.as_str()).is_some_and(|kind| {
            matches!(
                kind.as_str(),
                "file" | "module" | "function" | "closure" | "method" | "class" | "component"
            )
        }) {
            continue;
        }
        let Some(import) = extraction.edges.iter().find(|edge| {
            edge.source == source
                && edge.string("relation") == "imports_from"
                && edge.string("context") == "dynamic_import"
                && edge.string("start_byte").parse::<u64>().unwrap_or(u64::MAX) > call_start
        }) else {
            continue;
        };
        let mut targets = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == import.target
                    && edge.string("relation") == "re_exports"
                    && node_kinds
                        .get(edge.target.as_str())
                        .is_some_and(|node_kind| render_target_kind(node_kind))
            })
            .map(|edge| edge.target.clone())
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        let Some(target) = (targets.len() == 1).then(|| targets.remove(0)) else {
            continue;
        };
        let mut attributes = import.attributes.clone();
        attributes.insert("relation".to_owned(), Value::String("renders".to_owned()));
        attributes.insert("render_kind".to_owned(), Value::String(kind.clone()));
        attributes.insert(
            "rule".to_owned(),
            Value::String(format!("react-{kind}-render")),
        );
        attributes.insert("framework".to_owned(), Value::String("react".to_owned()));
        let candidate = RawEdgeRecord {
            source: owner,
            target,
            attributes,
        };
        if existing.insert(render_key(&candidate)) {
            additions.push(candidate);
        }
    }
}

fn render_target_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function" | "method" | "class" | "component" | "variable" | "property"
    )
}

fn render_key(edge: &RawEdgeRecord) -> (String, String, String, String, String) {
    let render_kind = if edge.string("relation") == "renders" {
        edge.string("render_kind")
    } else {
        "jsx".to_owned()
    };
    (
        edge.source.clone(),
        edge.target.clone(),
        edge.string("source_file"),
        edge.string("_occurrence_rule"),
        format!(
            "{}:{}:{}:{}:{}",
            edge.string("start_byte"),
            edge.string("end_byte"),
            edge.string("line_start"),
            edge.string("column_start"),
            render_kind,
        ),
    )
}

#[cfg(test)]
mod tests {
    use compass_languages::{
        Extraction, RawEdgeRecord, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin,
        RawNodeRecord,
    };
    use serde_json::{Map, Value, json};

    use super::project_render_relations;

    fn node(id: &str, kind: &str) -> RawNodeRecord {
        RawNodeRecord {
            id: id.to_owned(),
            attributes: Map::from_iter([
                ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
                ("label".to_owned(), Value::String(id.to_owned())),
            ]),
        }
    }

    #[test]
    fn projects_only_exact_jsx_component_references_and_preserves_occurrences() {
        let mut extraction = Extraction {
            nodes: vec![node("owner", "function"), node("component", "function")],
            edges: vec![
                RawEdgeRecord {
                    source: "owner".to_owned(),
                    target: "component".to_owned(),
                    attributes: Map::from_iter([
                        ("relation".to_owned(), json!("references")),
                        ("context".to_owned(), json!("jsx")),
                        ("source_file".to_owned(), json!("src/App.tsx")),
                        ("_occurrence_rule".to_owned(), json!("jsx:1")),
                        ("start_byte".to_owned(), json!(10)),
                        ("end_byte".to_owned(), json!(20)),
                        (
                            "resolution_rule".to_owned(),
                            json!("exact-source-declaration"),
                        ),
                    ]),
                },
                RawEdgeRecord {
                    source: "owner".to_owned(),
                    target: "component".to_owned(),
                    attributes: Map::from_iter([
                        ("relation".to_owned(), json!("references")),
                        ("context".to_owned(), json!("jsx")),
                        ("source_file".to_owned(), json!("src/App.tsx")),
                        ("_occurrence_rule".to_owned(), json!("jsx:2")),
                        ("start_byte".to_owned(), json!(30)),
                        ("end_byte".to_owned(), json!(40)),
                        (
                            "resolution_rule".to_owned(),
                            json!("exact-source-declaration"),
                        ),
                    ]),
                },
            ],
            framework_facts: vec![RawFrameworkFact::Domain(compass_languages::RawDomainFact {
                framework: "react".to_owned(),
                kind: "ui_role".to_owned(),
                name: "Component".to_owned(),
                declaring_scope: String::new(),
                anchor: RawFrameworkAnchor {
                    source_file: "src/Component.tsx".to_owned(),
                    start_byte: 0,
                    end_byte: 1,
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    end_column: 1,
                },
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([
                    ("source_reference".to_owned(), json!("component")),
                    ("role".to_owned(), json!("ui_component")),
                ]),
            })],
            ..Extraction::default()
        };

        project_render_relations(&mut extraction);
        let renders = extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "renders")
            .collect::<Vec<_>>();
        assert_eq!(renders.len(), 2);
        assert!(
            renders
                .iter()
                .all(|edge| edge.string("render_kind") == "jsx")
        );
    }

    #[test]
    fn does_not_project_context_values_or_mutable_aliases_as_components() {
        let mut extraction = Extraction {
            nodes: vec![node("owner", "function"), node("context", "variable")],
            edges: vec![RawEdgeRecord {
                source: "owner".to_owned(),
                target: "context".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("references")),
                    ("context".to_owned(), json!("jsx")),
                    ("source_file".to_owned(), json!("src/App.tsx")),
                    ("_occurrence_rule".to_owned(), json!("jsx:context")),
                    ("start_byte".to_owned(), json!(10)),
                    ("end_byte".to_owned(), json!(17)),
                    (
                        "resolution_rule".to_owned(),
                        json!("exact-source-declaration"),
                    ),
                ]),
            }],
            ..Extraction::default()
        };

        project_render_relations(&mut extraction);

        assert!(
            !extraction
                .edges
                .iter()
                .any(|edge| edge.string("relation") == "renders")
        );
    }
}
