use std::collections::{BTreeMap, HashMap};

use compass_graph::Communities;
use compass_history::{GraphRecordSink, HistoryError, RealizationReader};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde_json::{Map, Value};

use crate::{
    GraphViewModel, HtmlOptions, OutputError, graph_community_view_model_document,
    graph_view_model_document,
};

/// Builds the public historical viewer model from graph-only history roots.
pub fn historical_view_model(
    reader: &RealizationReader<'_>,
    title: impl Into<String>,
    node_limit: isize,
    community: Option<usize>,
) -> Result<GraphViewModel, HistoricalViewError> {
    let mode = if community.is_some() {
        ProjectionMode::Community(community.unwrap_or_default())
    } else if reader.version().version.node_count > node_limit.max(0) as u64 {
        ProjectionMode::Aggregate
    } else {
        ProjectionMode::Exact
    };
    let mut builder = HistoricalViewBuilder::new(mode);
    reader.scan_graph(&mut builder)?;
    builder.project(title.into(), node_limit)
}

/// Reconstruct only the node-link fields required by graph queries.
pub fn historical_graph_document(
    reader: &RealizationReader<'_>,
) -> Result<GraphDocument, HistoricalViewError> {
    historical_graph_document_with_labels(reader).map(|(document, _)| document)
}

#[derive(Debug, thiserror::Error)]
pub enum HistoricalViewError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("historical graph has no renderable overview")]
    Empty,
}

#[derive(Clone, Copy)]
enum ProjectionMode {
    Exact,
    Community(usize),
    Aggregate,
}

struct HistoricalViewBuilder {
    mode: ProjectionMode,
    attributes: BTreeMap<String, Map<String, Value>>,
    labels: Option<Value>,
    nodes: Vec<NodeRecord>,
    edges: Vec<EdgeRecord>,
    membership: HashMap<String, usize>,
    member_counts: BTreeMap<usize, usize>,
    cross_edges: BTreeMap<(usize, usize), usize>,
}

impl HistoricalViewBuilder {
    fn new(mode: ProjectionMode) -> Self {
        Self {
            mode,
            attributes: BTreeMap::new(),
            labels: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            membership: HashMap::new(),
            member_counts: BTreeMap::new(),
            cross_edges: BTreeMap::new(),
        }
    }
}

impl GraphRecordSink for HistoricalViewBuilder {
    fn node_attribute(
        &mut self,
        node_id: String,
        field: String,
        value: Value,
    ) -> Result<(), HistoryError> {
        self.attributes
            .entry(node_id)
            .or_default()
            .insert(field, value);
        Ok(())
    }

    fn labels(&mut self, labels: Value) -> Result<(), HistoryError> {
        self.labels = Some(labels);
        Ok(())
    }

    fn node(&mut self, mut node: NodeRecord) -> Result<(), HistoryError> {
        if let Some(attributes) = self.attributes.remove(&node.id) {
            node.attributes.extend(attributes);
        }
        let community = node
            .attributes
            .get("community")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        self.membership.insert(node.id.clone(), community);
        *self.member_counts.entry(community).or_default() += 1;
        if matches!(self.mode, ProjectionMode::Exact)
            || matches!(self.mode, ProjectionMode::Community(selected) if selected == community)
        {
            self.nodes.push(node);
        }
        Ok(())
    }

    fn edge(&mut self, edge: EdgeRecord) -> Result<(), HistoryError> {
        let source = self.membership.get(&edge.source).copied();
        let target = self.membership.get(&edge.target).copied();
        match self.mode {
            ProjectionMode::Exact => self.edges.push(edge),
            ProjectionMode::Community(selected)
                if source == Some(selected) && target == Some(selected) =>
            {
                self.edges.push(edge);
            }
            ProjectionMode::Aggregate => {
                if let (Some(source), Some(target)) = (source, target)
                    && source != target
                {
                    *self
                        .cross_edges
                        .entry((source.min(target), source.max(target)))
                        .or_default() += 1;
                }
            }
            ProjectionMode::Community(_) => {}
        }
        Ok(())
    }
}

fn historical_graph_document_with_labels(
    reader: &RealizationReader<'_>,
) -> Result<(GraphDocument, Option<Value>), HistoricalViewError> {
    let mut builder = HistoricalViewBuilder::new(ProjectionMode::Exact);
    reader.scan_graph(&mut builder)?;
    builder.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    builder.edges.sort_by(|left, right| {
        (&left.source, &left.target, left.string("relation")).cmp(&(
            &right.source,
            &right.target,
            right.string("relation"),
        ))
    });
    Ok((
        GraphDocument {
            directed: true,
            multigraph: false,
            graph: Map::new(),
            nodes: builder.nodes,
            links: builder.edges,
            extras: BTreeMap::new(),
        },
        builder.labels,
    ))
}

impl HistoricalViewBuilder {
    fn project(
        mut self,
        title: String,
        node_limit: isize,
    ) -> Result<GraphViewModel, HistoricalViewError> {
        if matches!(self.mode, ProjectionMode::Aggregate) {
            return self.aggregate(title);
        }
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| {
            (&left.source, &left.target, left.string("relation")).cmp(&(
                &right.source,
                &right.target,
                right.string("relation"),
            ))
        });
        let selected = match self.mode {
            ProjectionMode::Community(selected) => Some(selected),
            _ => None,
        };
        let document = GraphDocument {
            directed: true,
            multigraph: false,
            graph: Map::new(),
            nodes: self.nodes,
            links: self.edges,
            extras: BTreeMap::new(),
        };
        project_exact(document, self.labels, title, node_limit, selected)
    }

    fn aggregate(self, title: String) -> Result<GraphViewModel, HistoricalViewError> {
        if self.member_counts.len() <= 1 {
            return Err(HistoricalViewError::Empty);
        }
        let label_map = labels(self.labels.as_ref());
        let nodes = self
            .member_counts
            .keys()
            .map(|community| NodeRecord {
                id: community.to_string(),
                attributes: Map::from_iter([
                    (
                        "label".to_owned(),
                        Value::String(
                            label_map
                                .get(community)
                                .cloned()
                                .unwrap_or_else(|| format!("Community {community}")),
                        ),
                    ),
                    (
                        "symbol_kind".to_owned(),
                        Value::String("community".to_owned()),
                    ),
                ]),
            })
            .collect();
        let links = self
            .cross_edges
            .into_iter()
            .map(|((source, target), count)| EdgeRecord {
                source: source.to_string(),
                target: target.to_string(),
                attributes: Map::from_iter([
                    ("weight".to_owned(), Value::from(count)),
                    (
                        "relation".to_owned(),
                        Value::String(format!("{count} cross-community edges")),
                    ),
                    (
                        "confidence".to_owned(),
                        Value::String("AGGREGATED".to_owned()),
                    ),
                ]),
            })
            .collect();
        let document = GraphDocument {
            directed: false,
            multigraph: false,
            graph: Map::new(),
            nodes,
            links,
            extras: BTreeMap::new(),
        };
        let communities = self
            .member_counts
            .keys()
            .map(|community| (*community, vec![community.to_string()]))
            .collect();
        Ok(crate::viewer_model::graph_view_model(
            &document,
            &communities,
            title,
            &HtmlOptions {
                community_labels: (!label_map.is_empty()).then_some(&label_map),
                member_counts: Some(&self.member_counts),
                node_limit: None,
                learning_overlay: None,
            },
            true,
        ))
    }
}

fn project_exact(
    document: GraphDocument,
    label_value: Option<Value>,
    title: String,
    node_limit: isize,
    community: Option<usize>,
) -> Result<GraphViewModel, HistoricalViewError> {
    let mut communities = Communities::new();
    for node in &document.nodes {
        let community = node
            .attributes
            .get("community")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        communities
            .entry(community)
            .or_default()
            .push(node.id.clone());
    }
    let labels = labels(label_value.as_ref());
    let options = HtmlOptions {
        community_labels: (!labels.is_empty()).then_some(&labels),
        member_counts: None,
        node_limit: Some(node_limit),
        learning_overlay: None,
    };
    match community {
        Some(community) => Ok(graph_community_view_model_document(
            &document,
            &communities,
            title,
            &options,
            community,
        )?),
        None => graph_view_model_document(&document, &communities, title, &options)?
            .ok_or(HistoricalViewError::Empty),
    }
}

fn labels(value: Option<&Value>) -> BTreeMap<usize, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|labels| labels.iter())
        .filter_map(|(community, label)| {
            Some((community.parse().ok()?, label.as_str()?.to_owned()))
        })
        .collect()
}
