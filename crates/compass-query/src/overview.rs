//! Bounded source scopes and evidence-labelled topology summaries.
use std::collections::{BTreeMap, BTreeSet};

use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde::Serialize;
use thiserror::Error;

pub const MAX_OVERVIEW_NODES: usize = 1_000_000;
pub const MAX_OVERVIEW_EDGES: usize = 2_000_000;
pub const MAX_HOTSPOTS: usize = 100;

#[derive(Debug, Error)]
pub enum OverviewError {
    #[error("invalid scope: {0}; use a relative path or module:NAME")]
    Scope(String),
    #[error(
        "overview exceeds its graph work limit; build a smaller graph with compass ensure PATH"
    )]
    WorkLimit,
    #[error("hotspot limit must be between 1 and 100")]
    Limit,
    #[error("overview label exceeds 4096 bytes; use a graph with bounded symbol metadata")]
    MetadataLimit,
    #[error(
        "overview graph has duplicate identities or dangling relationships; run compass ensure to publish a valid graph"
    )]
    InvalidGraph,
    #[error(
        "unsupported overview graph schema {0}; run compass update to publish a supported graph"
    )]
    UnsupportedSchema(String),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewScope {
    pub filters: Vec<String>,
}

impl OverviewScope {
    pub fn new(filters: Vec<String>) -> Result<Self, OverviewError> {
        if filters.len() > 16 {
            return Err(OverviewError::Scope(
                "at most 16 filters are allowed".into(),
            ));
        }
        let mut normalized = BTreeSet::new();
        for filter in filters {
            let filter = filter.trim().replace('\\', "/");
            let value = filter
                .strip_prefix("module:")
                .or_else(|| filter.strip_prefix("path:"))
                .unwrap_or(&filter);
            if value.is_empty()
                || filter.len() > 4096
                || value.starts_with('/')
                || value.split('/').any(|part| part == "..")
                || value.chars().any(char::is_control)
                || (!filter.starts_with("module:") && value.contains(':'))
            {
                return Err(OverviewError::Scope(filter));
            }
            let normalized_filter = if filter.starts_with("module:") {
                format!("module:{}", value.trim_end_matches('/'))
            } else {
                let path = value.trim_start_matches("./").trim_end_matches('/');
                if path.is_empty() || path == "." {
                    return Err(OverviewError::Scope(filter));
                }
                path.to_owned()
            };
            normalized.insert(normalized_filter);
        }
        Ok(Self {
            filters: normalized.into_iter().collect(),
        })
    }

    pub fn matches(&self, node: &NodeRecord) -> bool {
        self.filters.is_empty()
            || self.filters.iter().any(|filter| {
                if let Some(module) = filter.strip_prefix("module:") {
                    ["module", "package", "qualified_name", "qualifiedName"]
                        .iter()
                        .any(|key| {
                            let value = node.string(key);
                            value == module
                                || value.strip_prefix(module).is_some_and(|suffix| {
                                    suffix.starts_with("::")
                                        || suffix.starts_with('.')
                                        || suffix.starts_with('/')
                                })
                        })
                } else {
                    let prefix = filter.strip_prefix("path:").unwrap_or(filter);
                    let path = node.source_file().unwrap_or_default().replace('\\', "/");
                    let path = path.trim_start_matches("./");
                    path == prefix
                        || path
                            .strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                }
            })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSelection {
    pub scope: OverviewScope,
    pub input_nodes: usize,
    pub selected_nodes: usize,
    pub omitted_nodes: usize,
    pub omitted_relationships: usize,
    pub boundary_relationships: usize,
}

pub fn scoped_document(
    document: &GraphDocument,
    scope: &OverviewScope,
) -> Result<(GraphDocument, ScopeSelection), OverviewError> {
    check_work(document)?;
    let ids = document
        .nodes
        .iter()
        .filter(|node| scope.matches(node))
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let nodes = document
        .nodes
        .iter()
        .filter(|node| ids.contains(node.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let links = document
        .links
        .iter()
        .filter(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let selection = ScopeSelection {
        scope: scope.clone(),
        input_nodes: document.nodes.len(),
        selected_nodes: nodes.len(),
        omitted_nodes: document.nodes.len() - nodes.len(),
        omitted_relationships: document.links.len() - links.len(),
        boundary_relationships: document
            .links
            .iter()
            .filter(|edge| ids.contains(edge.source.as_str()) != ids.contains(edge.target.as_str()))
            .count(),
    };
    let scoped = GraphDocument {
        directed: document.directed,
        multigraph: document.multigraph,
        graph: document.graph.clone(),
        extras: document.extras.clone(),
        nodes,
        links,
    };
    Ok((scoped, selection))
}

fn check_work(document: &GraphDocument) -> Result<(), OverviewError> {
    if let Some(schema) = document
        .graph
        .get("schema")
        .and_then(serde_json::Value::as_str)
        && schema.starts_with("compass.graph/")
        && schema != compass_model::code_graph::CODE_GRAPH_SCHEMA_V1
    {
        return Err(OverviewError::UnsupportedSchema(crate::sanitize_label(
            &schema.chars().take(64).collect::<String>(),
        )));
    }
    if document.nodes.len() > MAX_OVERVIEW_NODES || document.links.len() > MAX_OVERVIEW_EDGES {
        return Err(OverviewError::WorkLimit);
    }
    let mut ids = BTreeSet::new();
    if document
        .nodes
        .iter()
        .any(|node| !ids.insert(node.id.as_str()))
        || document
            .links
            .iter()
            .any(|edge| !ids.contains(edge.source.as_str()) || !ids.contains(edge.target.as_str()))
    {
        return Err(OverviewError::InvalidGraph);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionLayers {
    pub inferred: bool,
    pub documents: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionLayer {
    Structural,
    Inferred,
    Document,
}

fn document_node(node: &NodeRecord) -> bool {
    node.string("file_type") == "document"
        || matches!(
            node.kind_name(),
            "document" | "document_section" | "doc_section" | "rationale"
        )
        || node
            .source_file()
            .is_some_and(|path| path.ends_with(".md") || path.ends_with(".mdx"))
}

fn layer(
    edge: &EdgeRecord,
    source: &NodeRecord,
    target: &NodeRecord,
) -> Option<(ConnectionLayer, bool)> {
    let confidence = edge.string("confidence").to_ascii_uppercase();
    if matches!(confidence.as_str(), "AMBIGUOUS" | "UNRESOLVED") {
        return None;
    }
    let inferred = matches!(confidence.as_str(), "INFERRED" | "HEURISTIC");
    if document_node(source)
        || document_node(target)
        || matches!(
            edge.relation(),
            "documents" | "mentions" | "document_mentions"
        )
    {
        Some((ConnectionLayer::Document, inferred))
    } else if inferred {
        Some((ConnectionLayer::Inferred, true))
    } else {
        Some((ConnectionLayer::Structural, false))
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub source_file: Option<String>,
    pub source_location: String,
    pub connections: usize,
    pub dependents: usize,
    pub relationship_records: usize,
    /// Per-layer distinct directional contacts. A contact present in two layers
    /// appears in both layer counts but only once in the total.
    pub layers: BTreeMap<ConnectionLayer, usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotspotReport {
    pub schema: &'static str,
    pub scope: OverviewScope,
    pub included_layers: ConnectionLayers,
    pub count_policy: &'static str,
    pub directed: bool,
    pub selected_symbols: usize,
    pub omitted_symbols: usize,
    pub excluded_relationship_records: usize,
    pub most_connected: Vec<Hotspot>,
    pub most_depended_on: Vec<Hotspot>,
}

#[derive(Default)]
struct Contacts<'a> {
    total: BTreeSet<(u8, &'a str)>,
    dependents: BTreeSet<&'a str>,
    layers: BTreeMap<ConnectionLayer, BTreeSet<(u8, &'a str)>>,
    records: usize,
}

/// Rank source symbols within the scope, retaining contacts from outside it.
/// Self-loops and parallel occurrences do not inflate distinct contacts.
/// Counts describe only published evidence; no missing relationships are invented.
pub fn hotspots(
    document: &GraphDocument,
    scope: &OverviewScope,
    layers: ConnectionLayers,
    limit: usize,
) -> Result<HotspotReport, OverviewError> {
    check_work(document)?;
    if !(1..=MAX_HOTSPOTS).contains(&limit) {
        return Err(OverviewError::Limit);
    }
    let nodes = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut contacts = document
        .nodes
        .iter()
        .filter(|node| {
            scope.matches(node)
                && !document_node(node)
                && !matches!(
                    node.kind_name(),
                    "file" | "module" | "package" | "directory" | "external"
                )
                && node.source_file().is_some_and(|path| !path.is_empty())
        })
        .map(|node| (node.id.as_str(), Contacts::default()))
        .collect::<BTreeMap<_, _>>();
    let mut excluded = 0;
    for edge in &document.links {
        let (Some(source), Some(target)) = (
            nodes.get(edge.source.as_str()),
            nodes.get(edge.target.as_str()),
        ) else {
            continue;
        };
        let Some((layer, inferred)) = layer(edge, source, target) else {
            excluded += usize::from(
                contacts.contains_key(edge.source.as_str())
                    || contacts.contains_key(edge.target.as_str()),
            );
            continue;
        };
        if (inferred && !layers.inferred)
            || (layer == ConnectionLayer::Document && !layers.documents)
        {
            excluded += usize::from(
                contacts.contains_key(edge.source.as_str())
                    || contacts.contains_key(edge.target.as_str()),
            );
            continue;
        }
        if edge.source == edge.target {
            continue;
        }
        for (id, other, direction) in [
            (edge.source.as_str(), edge.target.as_str(), 0),
            (edge.target.as_str(), edge.source.as_str(), 1),
        ] {
            if let Some(contact) = contacts.get_mut(id) {
                let contact_key = (if document.directed { direction } else { 0 }, other);
                contact.total.insert(contact_key);
                contact.layers.entry(layer).or_default().insert(contact_key);
                if inferred && layer == ConnectionLayer::Document {
                    contact
                        .layers
                        .entry(ConnectionLayer::Inferred)
                        .or_default()
                        .insert(contact_key);
                }
                contact.records += 1;
                if document.directed
                    && direction == 1
                    && !matches!(edge.relation(), "contains" | "declares" | "member_of")
                {
                    contact.dependents.insert(other);
                }
            }
        }
    }
    for id in contacts.keys() {
        let node = nodes[id];
        if [
            node.id.as_str(),
            &node.display_label(),
            node.kind_name(),
            node.source_file().unwrap_or_default(),
            &node.string("source_location"),
        ]
        .iter()
        .any(|value| value.len() > 4096)
        {
            return Err(OverviewError::MetadataLimit);
        }
    }
    let mut connected = contacts
        .into_iter()
        .map(|(id, counts)| {
            let node = nodes[id];
            Hotspot {
                id: id.to_owned(),
                name: node.display_label(),
                kind: node.kind_name().to_owned(),
                source_file: node.source_file().map(str::to_owned),
                source_location: node.string("source_location"),
                connections: counts.total.len(),
                dependents: counts.dependents.len(),
                relationship_records: counts.records,
                layers: counts
                    .layers
                    .into_iter()
                    .map(|(layer, values)| (layer, values.len()))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    let selected_symbols = connected.len();
    let mut depended = connected.clone();
    connected.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.id.cmp(&b.id))
    });
    depended.sort_by(|a, b| {
        b.dependents
            .cmp(&a.dependents)
            .then_with(|| b.connections.cmp(&a.connections))
            .then_with(|| a.id.cmp(&b.id))
    });
    connected.truncate(limit);
    if document.directed {
        depended.truncate(limit);
    } else {
        depended.clear();
    }
    Ok(HotspotReport {
        schema: "compass.hotspots/1",
        scope: scope.clone(),
        included_layers: layers,
        count_policy: "distinct-direction-neighbor; dependents=distinct-incoming-noncontainment; scoped-symbols-global-contacts",
        directed: document.directed,
        selected_symbols,
        omitted_symbols: selected_symbols.saturating_sub(limit),
        excluded_relationship_records: excluded,
        most_connected: connected,
        most_depended_on: depended,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn scopes_layers_limits_and_direction_do_not_invent_contacts()
    -> Result<(), Box<dyn std::error::Error>> {
        for invalid in [
            ".",
            "./",
            "path:./",
            "module:",
            "/absolute",
            "C:\\absolute",
            "a/../b",
        ] {
            assert!(
                OverviewScope::new(vec![invalid.into()]).is_err(),
                "{invalid}"
            );
        }
        let scope = OverviewScope::new(vec!["path:./src/".into()])?;
        assert_eq!(scope.filters, vec!["src"]);
        let mut document: GraphDocument = serde_json::from_value(serde_json::json!({
            "directed":false,"nodes":[{"id":"a","kind":"function","source_file":"src/a.rs"},
                {"id":"b","kind":"function","source_file":"src/b.rs"}],
            "links":[{"source":"a","target":"b","relation":"uses","confidence":"AMBIGUOUS"}]
        }))?;
        let layers = ConnectionLayers {
            inferred: true,
            documents: true,
        };
        let ambiguous = hotspots(&document, &scope, layers, 10)?;
        assert_eq!(ambiguous.most_connected[0].connections, 0);
        assert_eq!(ambiguous.excluded_relationship_records, 1);
        document.links[0].attributes.remove("confidence");
        let undirected = hotspots(&document, &scope, layers, 1)?;
        assert_eq!(undirected.most_connected[0].id, "a");
        assert_eq!(undirected.most_connected[0].connections, 1);
        assert!(undirected.most_depended_on.is_empty());
        assert_eq!(undirected.omitted_symbols, 1);
        document.nodes[0].id = "x".repeat(4097);
        document.links[0].source = document.nodes[0].id.clone();
        assert!(matches!(
            hotspots(&document, &scope, layers, 10),
            Err(OverviewError::MetadataLimit)
        ));
        document
            .graph
            .insert("schema".into(), serde_json::json!("compass.graph/99"));
        assert!(matches!(
            hotspots(&document, &scope, layers, 10),
            Err(OverviewError::UnsupportedSchema(_))
        ));
        Ok(())
    }

    use super::*;

    #[test]
    fn inferred_document_contacts_require_both_layers_and_keep_both_labels()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = serde_json::from_value(serde_json::json!({"directed":true,"nodes":[
            {"id":"a","kind":"class","source_file":"app/a.py"},
            {"id":"doc","kind":"document_section","source_file":"README.md"}],
            "links":[{"source":"doc","target":"a","relation":"mentions","confidence":"INFERRED"}]}))?;
        let scope = OverviewScope::new(Vec::new())?;
        let only_docs = hotspots(
            &document,
            &scope,
            ConnectionLayers {
                inferred: false,
                documents: true,
            },
            10,
        )?;
        assert_eq!(only_docs.most_connected[0].connections, 0);
        assert_eq!(only_docs.excluded_relationship_records, 1);
        let both = hotspots(
            &document,
            &scope,
            ConnectionLayers {
                inferred: true,
                documents: true,
            },
            10,
        )?;
        assert_eq!(both.most_connected[0].connections, 1);
        assert_eq!(both.most_connected[0].layers[&ConnectionLayer::Inferred], 1);
        assert_eq!(both.most_connected[0].layers[&ConnectionLayer::Document], 1);
        Ok(())
    }

    #[test]
    fn scopes_use_component_boundaries_and_preserve_only_internal_edges()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(serde_json::json!({"nodes":[
            {"id":"a","source_file":"app/services/a.py","qualified_name":"app.services.A"},
            {"id":"b","source_file":"app/services_extra/b.py","qualified_name":"app.services_extra.B"}],
            "links":[{"source":"b","target":"a","relation":"calls"}]}))?;
        for filter in ["app/services", "module:app.services"] {
            let (scoped, selection) =
                scoped_document(&document, &OverviewScope::new(vec![filter.into()])?)?;
            assert_eq!(scoped.nodes.len(), 1);
            assert_eq!(scoped.nodes[0].id, "a");
            assert!(scoped.links.is_empty());
            assert_eq!(selection.boundary_relationships, 1);
        }
        assert!(OverviewScope::new(vec!["../app".into()]).is_err());
        Ok(())
    }

    #[test]
    fn hotspot_layers_keep_occurrences_distinct_from_contacts_and_scope_from_contacts()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(
            serde_json::json!({"directed":true,"multigraph":true,"nodes":[
            {"id":"a","kind":"class","source_file":"app/a.py"}, {"id":"b","kind":"function","source_file":"other/b.py"},
            {"id":"c","kind":"function","source_file":"other/c.py"}, {"id":"d","source_file":"README.md"}],
            "links":[{"source":"b","target":"a","relation":"calls","confidence":"EXTRACTED"},
            {"source":"b","target":"a","relation":"calls","confidence":"EXTRACTED"},
            {"source":"c","target":"a","relation":"uses","confidence":"INFERRED"},
            {"source":"d","target":"a","relation":"mentions","confidence":"EXTRACTED"},
            {"source":"a","target":"a","relation":"calls","confidence":"EXTRACTED"}]}),
        )?;
        let scope = OverviewScope::new(vec!["app".into()])?;
        let base = hotspots(&document, &scope, ConnectionLayers::default(), 10)?;
        assert_eq!(base.most_connected[0].connections, 1);
        assert_eq!(base.most_connected[0].relationship_records, 2);
        assert_eq!(base.excluded_relationship_records, 2);
        let all = hotspots(
            &document,
            &scope,
            ConnectionLayers {
                inferred: true,
                documents: true,
            },
            10,
        )?;
        assert_eq!(all.most_connected[0].connections, 3);
        assert_eq!(all.most_connected[0].dependents, 3);
        assert_eq!(all.most_connected[0].layers.len(), 3);
        assert!(hotspots(&document, &scope, ConnectionLayers::default(), 0).is_err());
        Ok(())
    }
}
