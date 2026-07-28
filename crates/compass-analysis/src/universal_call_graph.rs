use std::collections::{BTreeMap, BTreeSet, VecDeque};

use compass_model::{GraphDocument, NodeRecord};
use serde::Serialize;

use crate::{
    AnalysisBundle, CallGraphDirection, CallGraphRequest, CallGraphRoot, CallResolution,
    build_call_graph,
};

pub const UNIVERSAL_CALL_GRAPH_SCHEMA: &str = "compass.call_graph/1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UniversalCallGraphRoot {
    Symbol { symbol: String },
    SourcePosition { file: String, byte: u64, line: u64 },
}

#[derive(Clone, Debug)]
pub struct UniversalCallGraphRequest {
    pub root: UniversalCallGraphRoot,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallGraphResponse {
    pub schema: &'static str,
    pub root_symbol: String,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub nodes: Vec<UniversalCallNode>,
    pub edges: Vec<UniversalCallEdge>,
    pub truncated: bool,
    pub continuations: Vec<UniversalCallContinuation>,
    pub coverage: UniversalCallCoverage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallNode {
    pub id: String,
    pub symbol: Option<String>,
    pub name: String,
    pub file: Option<String>,
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
    pub start_byte: Option<u64>,
    pub end_byte: Option<u64>,
    pub graph_node_id: Option<String>,
    pub unresolved: bool,
    pub evidence_layer: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub callee: String,
    pub resolution: CallResolution,
    pub confidence: Option<String>,
    pub call_sites: Vec<UniversalCallSite>,
    pub evidence_layer: &'static str,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallSite {
    pub source_file: Option<String>,
    pub line: Option<u64>,
    pub start_byte: Option<u64>,
    pub end_byte: Option<u64>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallContinuation {
    pub symbol: String,
    pub direction: CallGraphDirection,
    pub next_depth: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCallCoverage {
    pub resolved: usize,
    pub inferred: usize,
    pub ambiguous: usize,
    pub unresolved: usize,
    pub evidence_layer: &'static str,
    pub partial: bool,
    pub limitations: Vec<String>,
    pub warning: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UniversalCallGraphError {
    #[error("call graph depth and bounds must be positive")]
    InvalidBounds,
    #[error("no callable graph node matches {0}")]
    MissingRoot(String),
    #[error("multiple callable graph nodes match {0}")]
    AmbiguousRoot(String),
}

pub fn build_universal_call_graph(
    graph: &GraphDocument,
    analysis: Option<&AnalysisBundle>,
    request: &UniversalCallGraphRequest,
) -> Result<UniversalCallGraphResponse, UniversalCallGraphError> {
    if request.depth == 0 || request.max_nodes == 0 || request.max_edges == 0 {
        return Err(UniversalCallGraphError::InvalidBounds);
    }

    let call_endpoints = graph
        .links
        .iter()
        .filter(|edge| edge.string("relation").eq_ignore_ascii_case("calls"))
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<BTreeSet<_>>();
    let mut all_nodes = graph
        .nodes
        .iter()
        .filter(|node| callable(node, &call_endpoints))
        .map(|node| (node.id.clone(), structural_node(node)))
        .collect::<BTreeMap<_, _>>();
    attach_program_symbols(&mut all_nodes, analysis);
    let (root, approximate_root_range) = resolve_structural_root(&all_nodes, &request.root)?;

    let mut all_edges = graph
        .links
        .iter()
        .filter(|edge| {
            edge.string("relation").eq_ignore_ascii_case("calls")
                && all_nodes.contains_key(&edge.source)
                && all_nodes.contains_key(&edge.target)
        })
        .map(|edge| {
            let confidence = edge.string("confidence");
            let resolution = if confidence.eq_ignore_ascii_case("inferred") {
                CallResolution::Inferred
            } else if confidence.eq_ignore_ascii_case("ambiguous") {
                CallResolution::Ambiguous
            } else {
                CallResolution::Resolved
            };
            let source_file = nonempty(edge.string("source_file"));
            let source_location = edge.string("source_location");
            let line = source_line(&source_location);
            let callee = all_nodes
                .get(&edge.target)
                .map(|node| node.name.clone())
                .unwrap_or_else(|| edge.target.clone());
            UniversalCallEdge {
                id: format!(
                    "{}:{}:{}",
                    edge.source,
                    edge.target,
                    source_location.to_ascii_lowercase()
                ),
                source: edge.source.clone(),
                target: edge.target.clone(),
                callee,
                resolution,
                confidence: nonempty(confidence),
                call_sites: vec![UniversalCallSite {
                    source_file,
                    line,
                    start_byte: None,
                    end_byte: None,
                    evidence: Vec::new(),
                }],
                evidence_layer: "structural_graph",
            }
        })
        .collect::<Vec<_>>();
    all_edges.sort_by(|left, right| left.id.cmp(&right.id));
    let enriched = enrich_from_program_ir(
        graph,
        analysis,
        &root,
        request,
        &mut all_nodes,
        &mut all_edges,
    );

    let (selected, mut continuations) = traverse(&root, &all_nodes, &all_edges, request);
    let mut truncated = false;
    let mut selected = if selected.len() > request.max_nodes {
        truncated = true;
        let mut retained = BTreeSet::from([root.clone()]);
        for symbol in selected.iter().filter(|symbol| **symbol != root) {
            if retained.len() >= request.max_nodes {
                continuations.insert((*symbol).clone());
                continue;
            }
            retained.insert((*symbol).clone());
        }
        retained
    } else {
        selected
    };
    selected.insert(root.clone());

    let mut edges = all_edges
        .into_iter()
        .filter(|edge| selected.contains(&edge.source) && selected.contains(&edge.target))
        .collect::<Vec<_>>();
    if edges.len() > request.max_edges {
        for edge in &edges[request.max_edges..] {
            insert_continuation(&mut continuations, edge.source.clone(), request.max_nodes);
            insert_continuation(&mut continuations, edge.target.clone(), request.max_nodes);
        }
        edges.truncate(request.max_edges);
        truncated = true;
    }
    let nodes = selected
        .iter()
        .filter_map(|id| all_nodes.remove(id))
        .collect::<Vec<_>>();
    let continuation_records = continuations
        .into_iter()
        .take(request.max_nodes)
        .filter(|symbol| symbol != &root)
        .map(|symbol| UniversalCallContinuation {
            symbol,
            direction: request.direction,
            next_depth: request.depth.saturating_add(1),
        })
        .collect::<Vec<_>>();
    truncated |= !continuation_records.is_empty();
    let coverage = coverage(&edges, enriched, approximate_root_range);

    Ok(UniversalCallGraphResponse {
        schema: UNIVERSAL_CALL_GRAPH_SCHEMA,
        root_symbol: root,
        direction: request.direction,
        depth: request.depth,
        nodes,
        edges,
        truncated,
        continuations: continuation_records,
        coverage,
    })
}

fn attach_program_symbols(
    nodes: &mut BTreeMap<String, UniversalCallNode>,
    analysis: Option<&AnalysisBundle>,
) {
    for function in analysis
        .into_iter()
        .flat_map(|analysis| &analysis.program.modules)
        .flat_map(|module| &module.functions)
    {
        let Some(graph_node_id) = &function.graph_node_id else {
            continue;
        };
        let Some(node) = nodes.get_mut(graph_node_id) else {
            continue;
        };
        node.symbol = Some(function.symbol_id.clone());
        node.start_byte = Some(function.anchor.start_byte);
        node.end_byte = Some(function.anchor.end_byte);
        node.evidence_layer = "combined";
    }
}

fn enrich_from_program_ir(
    graph: &GraphDocument,
    analysis: Option<&AnalysisBundle>,
    root: &str,
    request: &UniversalCallGraphRequest,
    nodes: &mut BTreeMap<String, UniversalCallNode>,
    edges: &mut Vec<UniversalCallEdge>,
) -> bool {
    let Some(analysis) = analysis else {
        return false;
    };
    let Some(root_symbol) = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .find(|function| function.graph_node_id.as_deref() == Some(root))
        .map(|function| function.symbol_id.clone())
    else {
        return false;
    };
    let Ok(program) = build_call_graph(
        analysis,
        Some(graph),
        &CallGraphRequest {
            root: CallGraphRoot::Symbol {
                symbol: root_symbol,
            },
            direction: request.direction,
            depth: request.depth,
            max_nodes: request.max_nodes,
            max_edges: request.max_edges,
        },
    ) else {
        return false;
    };
    let mut canonical = BTreeMap::new();
    for program_node in program.nodes {
        let id = program_node
            .graph_node_id
            .as_ref()
            .filter(|id| nodes.contains_key(*id))
            .cloned()
            .unwrap_or_else(|| program_node.id.clone());
        canonical.insert(program_node.id.clone(), id.clone());
        if let Some(node) = nodes.get_mut(&id) {
            node.symbol = program_node.symbol;
            if let Some(anchor) = program_node.anchor {
                node.start_byte = Some(anchor.start_byte);
                node.end_byte = Some(anchor.end_byte);
            }
            node.evidence_layer = "combined";
            continue;
        }
        let anchor = program_node.anchor;
        nodes.insert(
            id.clone(),
            UniversalCallNode {
                id,
                symbol: program_node.symbol,
                name: program_node.name,
                file: program_node.file,
                start_line: None,
                end_line: None,
                start_byte: anchor.as_ref().map(|anchor| anchor.start_byte),
                end_byte: anchor.as_ref().map(|anchor| anchor.end_byte),
                graph_node_id: program_node.graph_node_id,
                unresolved: program_node.unresolved,
                evidence_layer: "program_ir",
            },
        );
    }
    for program_edge in program.edges {
        let Some(source) = canonical.get(&program_edge.source).cloned() else {
            continue;
        };
        let Some(target) = canonical.get(&program_edge.target).cloned() else {
            continue;
        };
        let sites = program_edge
            .call_sites
            .into_iter()
            .map(|site| UniversalCallSite {
                source_file: Some(site.anchor.source_file),
                line: None,
                start_byte: Some(site.anchor.start_byte),
                end_byte: Some(site.anchor.end_byte),
                evidence: site.evidence,
            })
            .collect::<Vec<_>>();
        if let Some(edge) = edges
            .iter_mut()
            .find(|edge| edge.source == source && edge.target == target)
        {
            edge.resolution = stronger_resolution(edge.resolution, program_edge.resolution);
            edge.call_sites.extend(sites);
            edge.call_sites.sort();
            edge.call_sites.dedup();
            edge.evidence_layer = "combined";
            continue;
        }
        edges.push(UniversalCallEdge {
            id: format!("program:{}:{}:{}", source, target, program_edge.id),
            source,
            target,
            callee: program_edge.callee,
            resolution: program_edge.resolution,
            confidence: None,
            call_sites: sites,
            evidence_layer: "program_ir",
        });
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    true
}

fn stronger_resolution(left: CallResolution, right: CallResolution) -> CallResolution {
    if left >= right { left } else { right }
}

fn callable(node: &NodeRecord, call_endpoints: &BTreeSet<&String>) -> bool {
    let kind = node.string("symbol_kind").to_ascii_lowercase();
    let known_kind = matches!(
        kind.as_str(),
        "function" | "method" | "constructor" | "procedure" | "subroutine"
    );
    let has_location = line(node, "line_start").is_some();
    let is_file = kind == "file";
    has_location && !is_file && (known_kind || call_endpoints.contains(&node.id))
}

fn structural_node(node: &NodeRecord) -> UniversalCallNode {
    UniversalCallNode {
        id: node.id.clone(),
        symbol: Some(node.id.clone()),
        name: node.label().to_owned(),
        file: nonempty(node.string("source_file")),
        start_line: line(node, "line_start"),
        end_line: line(node, "line_end"),
        start_byte: None,
        end_byte: None,
        graph_node_id: Some(node.id.clone()),
        unresolved: false,
        evidence_layer: "structural_graph",
    }
}

fn resolve_structural_root(
    nodes: &BTreeMap<String, UniversalCallNode>,
    root: &UniversalCallGraphRoot,
) -> Result<(String, bool), UniversalCallGraphError> {
    match root {
        UniversalCallGraphRoot::Symbol { symbol } => {
            if nodes.contains_key(symbol) {
                return Ok((symbol.clone(), false));
            }
            let matches = nodes
                .values()
                .filter(|node| node.symbol.as_deref() == Some(symbol.as_str()))
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [id] => Ok((id.clone(), false)),
                [] => Err(UniversalCallGraphError::MissingRoot(symbol.clone())),
                _ => Err(UniversalCallGraphError::AmbiguousRoot(symbol.clone())),
            }
        }
        UniversalCallGraphRoot::SourcePosition {
            file, byte, line, ..
        } => {
            let normalized = normalize_path(file);
            let exact = nodes
                .values()
                .filter(|node| {
                    node.file
                        .as_deref()
                        .is_some_and(|candidate| normalize_path(candidate) == normalized)
                        && node.start_line.is_some_and(|start| start <= *line)
                        && node.end_line.is_some_and(|end| *line <= end)
                })
                .min_by_key(|node| {
                    (
                        node.end_line
                            .unwrap_or(u64::MAX)
                            .saturating_sub(node.start_line.unwrap_or_default()),
                        node.id.as_str(),
                    )
                })
                .map(|node| node.id.clone());
            if let Some(id) = exact {
                return Ok((id, false));
            }
            // Older/source-driven extractors may only have a declaration line.
            // Select the closest preceding callable and disclose the approximate
            // range instead of making those languages cursor-incompatible.
            nodes
                .values()
                .filter(|node| {
                    node.file
                        .as_deref()
                        .is_some_and(|candidate| normalize_path(candidate) == normalized)
                        && node.start_line.is_some_and(|start| start <= *line)
                })
                .max_by_key(|node| (node.start_line.unwrap_or_default(), node.id.as_str()))
                .map(|node| (node.id.clone(), true))
                .ok_or_else(|| {
                    UniversalCallGraphError::MissingRoot(format!("{file}:{byte} (line {line})"))
                })
        }
    }
}

fn traverse(
    root: &str,
    nodes: &BTreeMap<String, UniversalCallNode>,
    edges: &[UniversalCallEdge],
    request: &UniversalCallGraphRequest,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut adjacency = BTreeMap::<&str, Vec<usize>>::new();
    for (index, edge) in edges.iter().enumerate() {
        match request.direction {
            CallGraphDirection::Callers => {
                adjacency.entry(&edge.target).or_default().push(index);
            }
            CallGraphDirection::Callees => {
                adjacency.entry(&edge.source).or_default().push(index);
            }
            CallGraphDirection::Both => {
                adjacency.entry(&edge.source).or_default().push(index);
                if edge.target != edge.source {
                    adjacency.entry(&edge.target).or_default().push(index);
                }
            }
        }
    }

    let mut selected = BTreeSet::from([root.to_owned()]);
    let mut queue = VecDeque::from([(root.to_owned(), 0_u32)]);
    let mut continuations = BTreeSet::new();
    while let Some((symbol, level)) = queue.pop_front() {
        for edge in adjacency
            .get(symbol.as_str())
            .into_iter()
            .flatten()
            .map(|index| &edges[*index])
        {
            let next = if edge.source == symbol {
                &edge.target
            } else {
                &edge.source
            };
            if level >= request.depth {
                if nodes.contains_key(next) {
                    insert_continuation(&mut continuations, next.clone(), request.max_nodes);
                }
                continue;
            }
            if selected.contains(next) {
                continue;
            }
            if selected.len() >= request.max_nodes {
                insert_continuation(&mut continuations, next.clone(), request.max_nodes);
                continue;
            }
            if selected.insert(next.clone()) {
                queue.push_back((next.clone(), level + 1));
            }
        }
    }
    (selected, continuations)
}

fn insert_continuation(continuations: &mut BTreeSet<String>, symbol: String, limit: usize) {
    if continuations.len() < limit {
        continuations.insert(symbol);
    }
}

fn coverage(
    edges: &[UniversalCallEdge],
    enriched: bool,
    approximate_root_range: bool,
) -> UniversalCallCoverage {
    let count = |resolution| {
        edges
            .iter()
            .filter(|edge| edge.resolution == resolution)
            .count()
    };
    let mut limitations = if enriched {
        vec!["structural_graph_baseline".to_owned()]
    } else {
        vec!["program_ir_unavailable".to_owned()]
    };
    if approximate_root_range {
        limitations.push("approximate_callable_range".to_owned());
    }
    UniversalCallCoverage {
        resolved: count(CallResolution::Resolved),
        inferred: count(CallResolution::Inferred),
        ambiguous: count(CallResolution::Ambiguous),
        unresolved: count(CallResolution::Unresolved),
        evidence_layer: if enriched {
            "combined"
        } else {
            "structural_graph"
        },
        partial: true,
        limitations,
        warning: if approximate_root_range {
            "Compass approximated this callable range from declaration lines; move the cursor to the declaration if the selected root is incorrect."
                .to_owned()
        } else if enriched {
            "Structural call evidence is enriched with available Program IR; language coverage may still be partial."
                .to_owned()
        } else {
            "Structural call evidence is available; unresolved calls may be omitted without Program IR."
                .to_owned()
        },
    }
}

fn line(node: &NodeRecord, key: &str) -> Option<u64> {
    node.unsigned(key)
}

fn source_line(location: &str) -> Option<u64> {
    location
        .strip_prefix('L')
        .and_then(|value| value.split(['-', ':']).next())
        .and_then(|value| value.parse().ok())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_owned()
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
