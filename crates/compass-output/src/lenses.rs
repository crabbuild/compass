use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use compass_graph::Communities;
use compass_model::{Graph, GraphDocument};
use serde::Serialize;

use crate::{GraphViewModel, HtmlOptions, OutputError, graph_view_model};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactLens {
    Dependencies,
    Routes,
    Data,
    Messaging,
    Tests,
    Provenance,
}

impl ArtifactLens {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::Routes => "routes",
            Self::Data => "data",
            Self::Messaging => "messaging",
            Self::Tests => "tests",
            Self::Provenance => "provenance",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dependencies => "Dependencies",
            Self::Routes => "Routes and handlers",
            Self::Data => "Data access",
            Self::Messaging => "Messaging and jobs",
            Self::Tests => "Tests and verification",
            Self::Provenance => "Aliases and provenance",
        }
    }

    #[must_use]
    pub fn relations(self) -> Vec<String> {
        let relations: &[&str] = match self {
            Self::Dependencies => &[
                "imports",
                "imports_from",
                "re_exports",
                "depends_on",
                "uses",
                "references",
            ],
            Self::Routes => &["routes_to", "handles", "registers", "decorates"],
            Self::Data => &["reads", "writes", "maps_to"],
            Self::Messaging => &[
                "publishes",
                "subscribes",
                "produces",
                "consumes",
                "schedules",
                "triggers",
            ],
            Self::Tests => &["tests"],
            Self::Provenance => &["aliases", "exports", "re_exports", "overrides"],
        };
        relations
            .iter()
            .map(|relation| (*relation).to_owned())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct LensProjection {
    pub model: GraphViewModel,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct AffectedLensOptions<'a> {
    pub relations: &'a [String],
    pub depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
}

pub fn artifact_lens_view_model(
    document: &GraphDocument,
    communities: &Communities,
    labels: Option<&BTreeMap<usize, String>>,
    lens: ArtifactLens,
    max_nodes: usize,
    max_edges: usize,
) -> LensProjection {
    let relations = lens.relations();
    let mut projection = relation_projection(
        document,
        communities,
        labels,
        lens.label(),
        &relations,
        max_nodes,
        max_edges,
    );
    if lens == ArtifactLens::Routes {
        prepare_route_lens(document, &mut projection.model);
    }
    projection
}

pub fn affected_lens_view_model(
    document: &GraphDocument,
    communities: &Communities,
    labels: Option<&BTreeMap<usize, String>>,
    root: &str,
    options: AffectedLensOptions<'_>,
) -> Result<LensProjection, OutputError> {
    let graph = Graph::from_document(document.clone())
        .map_err(|_| OutputError::AffectedRoot(root.to_owned()))?;
    let seed = compass_query::resolve_seed(&graph, root)
        .ok_or_else(|| OutputError::AffectedRoot(root.to_owned()))?;
    let seed_id = graph.node(seed).id.clone();
    let relation_set = normalized_relations(options.relations);
    let mut incoming = HashMap::<String, Vec<usize>>::new();
    let mut outgoing = HashMap::<String, Vec<usize>>::new();
    for (index, edge) in document.links.iter().enumerate() {
        incoming.entry(edge.target.clone()).or_default().push(index);
        outgoing.entry(edge.source.clone()).or_default().push(index);
    }
    let mut distance = BTreeMap::from([(seed_id.clone(), 0_usize)]);
    let mut queue = VecDeque::from([(seed_id.clone(), 0_usize)]);
    let mut selected_edges = BTreeSet::new();
    let mut truncated = false;
    for edge_index in outgoing.get(&seed_id).into_iter().flatten() {
        let edge = &document.links[*edge_index];
        if !matches!(normalized_relation(edge).as_str(), "method" | "contains")
            || distance.contains_key(&edge.target)
        {
            continue;
        }
        if distance.len() >= options.max_nodes || selected_edges.len() >= options.max_edges {
            truncated = true;
        } else {
            distance.insert(edge.target.clone(), 0);
            queue.push_back((edge.target.clone(), 0));
            selected_edges.insert(*edge_index);
        }
    }
    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= options.depth {
            continue;
        }
        let mut candidates = incoming.get(&current).cloned().unwrap_or_default();
        candidates.sort_by(|left, right| {
            edge_key(&document.links[*left]).cmp(&edge_key(&document.links[*right]))
        });
        for edge_index in candidates {
            let edge = &document.links[edge_index];
            if !relation_set.contains(&normalized_relation(edge)) {
                continue;
            }
            if selected_edges.len() >= options.max_edges {
                truncated = true;
                break;
            }
            if !distance.contains_key(&edge.source) {
                if distance.len() >= options.max_nodes {
                    truncated = true;
                    break;
                }
                distance.insert(edge.source.clone(), current_depth + 1);
                queue.push_back((edge.source.clone(), current_depth + 1));
            }
            selected_edges.insert(edge_index);
        }
        if truncated {
            break;
        }
    }
    let ids = distance.keys().cloned().collect::<BTreeSet<_>>();
    let filtered = filtered_document(document, &ids, &selected_edges);
    let filtered_communities = filtered_communities(communities, &ids);
    let mut model = graph_view_model(
        &filtered,
        &filtered_communities,
        format!("Affected by {root}"),
        &HtmlOptions {
            community_labels: labels,
            ..HtmlOptions::default()
        },
        false,
    );
    for node in &mut model.nodes {
        node.depth = distance.get(&node.id).copied();
        node.root = Some(node.id == seed_id);
    }
    Ok(LensProjection { model, truncated })
}

fn relation_projection(
    document: &GraphDocument,
    communities: &Communities,
    labels: Option<&BTreeMap<usize, String>>,
    title: &str,
    relations: &[String],
    max_nodes: usize,
    max_edges: usize,
) -> LensProjection {
    let relation_set = normalized_relations(relations);
    let mut matching = document
        .links
        .iter()
        .enumerate()
        .filter(|(_, edge)| relation_set.contains(&normalized_relation(edge)))
        .collect::<Vec<_>>();
    matching.sort_by_key(|(_, edge)| edge_key(edge));
    let total_edges = matching.len();
    matching.truncate(max_edges);
    let mut ids = matching
        .iter()
        .flat_map(|(_, edge)| [edge.source.clone(), edge.target.clone()])
        .collect::<BTreeSet<_>>();
    let total_nodes = ids.len();
    if ids.len() > max_nodes {
        ids = ids.into_iter().take(max_nodes).collect();
    }
    let selected_edges = matching
        .into_iter()
        .filter(|(_, edge)| ids.contains(&edge.source) && ids.contains(&edge.target))
        .map(|(index, _)| index)
        .collect::<BTreeSet<_>>();
    let filtered = filtered_document(document, &ids, &selected_edges);
    let filtered_communities = filtered_communities(communities, &ids);
    let model = graph_view_model(
        &filtered,
        &filtered_communities,
        title,
        &HtmlOptions {
            community_labels: labels,
            ..HtmlOptions::default()
        },
        false,
    );
    LensProjection {
        model,
        truncated: total_nodes > max_nodes || total_edges > max_edges,
    }
}

fn prepare_route_lens(document: &GraphDocument, model: &mut GraphViewModel) {
    let route_ids = model
        .edges
        .iter()
        .map(|edge| edge.source.as_str())
        .collect::<BTreeSet<_>>();
    let handler_ids = model
        .edges
        .iter()
        .map(|edge| edge.target.as_str())
        .filter(|id| !route_ids.contains(id))
        .collect::<BTreeSet<_>>();
    for node in &mut model.nodes {
        let route = route_ids.contains(node.id.as_str());
        node.depth = Some(usize::from(!route));
        node.root = Some(route);
    }

    let source_nodes = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut duplicate_labels = BTreeMap::<String, Vec<String>>::new();
    for node in &model.nodes {
        if handler_ids.contains(node.id.as_str()) {
            duplicate_labels
                .entry(node.label.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
    for ids in duplicate_labels.values().filter(|ids| ids.len() > 1) {
        let qualified = ids
            .iter()
            .map(|id| {
                source_nodes
                    .get(id.as_str())
                    .and_then(|node| node.logical_property("qualified_name"))
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let labels = minimal_qualified_labels(&qualified);
        for (id, label) in ids.iter().zip(labels) {
            if label.is_empty() {
                continue;
            }
            if let Some(node) = model.nodes.iter_mut().find(|node| node.id == *id) {
                if node.label.ends_with("()") && !label.ends_with("()") {
                    node.label = format!("{label}()");
                } else {
                    node.label = label;
                }
            }
        }
    }
}

fn minimal_qualified_labels(values: &[String]) -> Vec<String> {
    let parts = values
        .iter()
        .map(|value| qualified_parts(value))
        .collect::<Vec<_>>();
    parts
        .iter()
        .enumerate()
        .map(|(index, segments)| {
            if segments.is_empty() {
                return String::new();
            }
            for length in 2..=segments.len() {
                let suffix = &segments[segments.len() - length..];
                let unique = parts.iter().enumerate().all(|(other_index, other)| {
                    other_index == index
                        || other.len() < length
                        || other[other.len() - length..] != *suffix
                });
                if unique {
                    return suffix.join("::");
                }
            }
            segments.join("::")
        })
        .collect()
}

fn qualified_parts(value: &str) -> Vec<String> {
    value
        .replace("::", ".")
        .split(['.', '/', '\\', '#'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn filtered_document(
    document: &GraphDocument,
    ids: &BTreeSet<String>,
    edge_indexes: &BTreeSet<usize>,
) -> GraphDocument {
    GraphDocument {
        directed: true,
        multigraph: document.multigraph,
        graph: document.graph.clone(),
        nodes: document
            .nodes
            .iter()
            .filter(|node| ids.contains(&node.id))
            .cloned()
            .collect(),
        links: document
            .links
            .iter()
            .enumerate()
            .filter(|(index, edge)| {
                edge_indexes.contains(index)
                    && ids.contains(&edge.source)
                    && ids.contains(&edge.target)
            })
            .map(|(_, edge)| edge.clone())
            .collect(),
        extras: document.extras.clone(),
    }
}

fn filtered_communities(communities: &Communities, ids: &BTreeSet<String>) -> Communities {
    communities
        .iter()
        .filter_map(|(community, members)| {
            let members = members
                .iter()
                .filter(|member| ids.contains(*member))
                .cloned()
                .collect::<Vec<_>>();
            (!members.is_empty()).then_some((*community, members))
        })
        .collect()
}

fn normalized_relations(relations: &[String]) -> HashSet<String> {
    relations
        .iter()
        .map(|relation| relation.trim().to_ascii_lowercase())
        .collect()
}

fn normalized_relation(edge: &compass_model::EdgeRecord) -> String {
    edge.string("relation").trim().to_ascii_lowercase()
}

fn edge_key(edge: &compass_model::EdgeRecord) -> (String, String, String, String) {
    (
        edge.source.clone(),
        edge.target.clone(),
        normalized_relation(edge),
        edge.string("source_location"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Result<GraphDocument, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"api","label":"api","kind":"function","community":0},
                {"id":"service","label":"service","kind":"function","community":0},
                {"id":"database","label":"database","kind":"resource","community":1},
                {"id":"test","label":"test","kind":"function","community":2}
            ],
            "links": [
                {"source":"api","target":"service","relation":"calls"},
                {"source":"service","target":"database","relation":"writes"},
                {"source":"test","target":"api","relation":"tests"}
            ]
        }))
    }

    fn communities() -> Communities {
        BTreeMap::from([
            (0, vec!["api".to_owned(), "service".to_owned()]),
            (1, vec!["database".to_owned()]),
            (2, vec!["test".to_owned()]),
        ])
    }

    #[test]
    fn artifact_lenses_publish_only_matching_relations() -> Result<(), serde_json::Error> {
        let projection = artifact_lens_view_model(
            &fixture()?,
            &communities(),
            None,
            ArtifactLens::Data,
            20,
            20,
        );
        assert_eq!(projection.model.stats.nodes, 2);
        assert_eq!(projection.model.stats.edges, 1);
        assert_eq!(projection.model.edges[0].relation, "writes");
        Ok(())
    }

    #[test]
    fn affected_lens_walks_inbound_and_marks_depth() -> Result<(), Box<dyn std::error::Error>> {
        let relations = vec!["calls".to_owned(), "tests".to_owned()];
        let projection = affected_lens_view_model(
            &fixture()?,
            &communities(),
            None,
            "service",
            AffectedLensOptions {
                relations: &relations,
                depth: 2,
                max_nodes: 20,
                max_edges: 20,
            },
        )?;
        let depths = projection
            .model
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.depth, node.root))
            .collect::<BTreeSet<_>>();
        assert!(depths.contains(&("service", Some(0), Some(true))));
        assert!(depths.contains(&("api", Some(1), Some(false))));
        assert!(depths.contains(&("test", Some(2), Some(false))));
        Ok(())
    }

    #[test]
    fn bounded_lens_reports_truncation_deterministically() -> Result<(), serde_json::Error> {
        let first = artifact_lens_view_model(
            &fixture()?,
            &communities(),
            None,
            ArtifactLens::Tests,
            1,
            20,
        );
        let second = artifact_lens_view_model(
            &fixture()?,
            &communities(),
            None,
            ArtifactLens::Tests,
            1,
            20,
        );
        assert!(first.truncated);
        assert_eq!(
            serde_json::to_value(&first.model)?,
            serde_json::to_value(&second.model)?
        );
        Ok(())
    }

    #[test]
    fn route_lens_separates_routes_and_disambiguates_handlers() -> Result<(), serde_json::Error> {
        let document = serde_json::from_value(serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"get-route","label":"GET /items","kind":"route","community":0},
                {"id":"post-route","label":"POST /items","kind":"route","community":0},
                {
                    "id":"get-handler",
                    "label":"handler()",
                    "qualifiedName":"app::items::get::handler",
                    "kind":"function",
                    "community":0
                },
                {
                    "id":"post-handler",
                    "label":"handler()",
                    "qualifiedName":"app::items::post::handler",
                    "kind":"function",
                    "community":0
                }
            ],
            "links": [
                {"source":"get-route","target":"get-handler","relation":"routes_to"},
                {"source":"post-route","target":"post-handler","relation":"routes_to"}
            ]
        }))?;
        let projection = artifact_lens_view_model(
            &document,
            &BTreeMap::from([(
                0,
                vec![
                    "get-route".to_owned(),
                    "post-route".to_owned(),
                    "get-handler".to_owned(),
                    "post-handler".to_owned(),
                ],
            )]),
            None,
            ArtifactLens::Routes,
            20,
            20,
        );
        let nodes = projection
            .model
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.label.as_str(), node.depth, node.root))
            .collect::<BTreeSet<_>>();
        assert!(nodes.contains(&("get-route", "GET /items", Some(0), Some(true))));
        assert!(nodes.contains(&("post-route", "POST /items", Some(0), Some(true))));
        assert!(nodes.contains(&("get-handler", "get::handler()", Some(1), Some(false))));
        assert!(nodes.contains(&("post-handler", "post::handler()", Some(1), Some(false))));
        Ok(())
    }
}
