use std::collections::{BTreeMap, BTreeSet, VecDeque};

use compass_ir::{FunctionIr, OperationKind, SourceAnchor};
use compass_model::GraphDocument;
use serde::{Deserialize, Serialize};

use crate::AnalysisBundle;

pub const CALL_GRAPH_SCHEMA: &str = "compass.program.call_graph/1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallGraphRoot {
    Symbol { symbol: String },
    SourceByte { file: String, byte: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallGraphDirection {
    Callers,
    Callees,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallResolution {
    Resolved,
    Inferred,
    Ambiguous,
    Unresolved,
}

#[derive(Clone, Debug)]
pub struct CallGraphRequest {
    pub root: CallGraphRoot,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallGraphResponse {
    pub schema: &'static str,
    pub root_symbol: String,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub nodes: Vec<CallNode>,
    pub edges: Vec<CallEdge>,
    pub truncated: bool,
    pub continuations: Vec<CallContinuation>,
    pub coverage: CallCoverage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallNode {
    pub id: String,
    pub symbol: Option<String>,
    pub name: String,
    pub file: Option<String>,
    pub anchor: Option<SourceAnchor>,
    pub graph_node_id: Option<String>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub callee: String,
    pub resolution: CallResolution,
    pub call_sites: Vec<CallSite>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSite {
    pub anchor: SourceAnchor,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallContinuation {
    pub symbol: String,
    pub direction: CallGraphDirection,
    pub next_depth: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallCoverage {
    pub resolved: usize,
    pub inferred: usize,
    pub ambiguous: usize,
    pub unresolved: usize,
    pub warning: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum CallGraphError {
    #[error("call graph depth and bounds must be positive")]
    InvalidBounds,
    #[error("no Program IR function matches {0}")]
    MissingRoot(String),
    #[error("multiple Program IR functions match {0}")]
    AmbiguousRoot(String),
}

pub fn build_call_graph(
    analysis: &AnalysisBundle,
    graph: Option<&GraphDocument>,
    request: &CallGraphRequest,
) -> Result<CallGraphResponse, CallGraphError> {
    if request.depth == 0 || request.max_nodes == 0 || request.max_edges == 0 {
        return Err(CallGraphError::InvalidBounds);
    }
    let functions = analysis
        .program
        .modules
        .iter()
        .flat_map(|module| {
            module
                .functions
                .iter()
                .map(move |function| (function.symbol_id.clone(), (module, function)))
        })
        .collect::<BTreeMap<_, _>>();
    let root = resolve_root(&functions, &request.root)?;
    let inferred = inferred_calls(graph);
    let mut all_nodes = functions
        .iter()
        .map(|(symbol, (module, function))| {
            (
                symbol.clone(),
                CallNode {
                    id: symbol.clone(),
                    symbol: Some(symbol.clone()),
                    name: function.name.clone(),
                    file: Some(module.source_file.clone()),
                    anchor: Some(function.anchor.clone()),
                    graph_node_id: function.graph_node_id.clone(),
                    unresolved: false,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut all_edges = Vec::new();
    for (caller, (_, function)) in &functions {
        for block in &function.blocks {
            for operation in &block.operations {
                let OperationKind::Call {
                    callee,
                    resolved_symbols,
                    ..
                } = &operation.kind
                else {
                    continue;
                };
                let call_site = CallSite {
                    anchor: operation.anchor.clone(),
                    evidence: operation.evidence.clone(),
                };
                if resolved_symbols.is_empty() {
                    let target = format!("unresolved:{caller}:{}", operation.ordinal);
                    all_nodes.entry(target.clone()).or_insert_with(|| CallNode {
                        id: target.clone(),
                        symbol: None,
                        name: callee.clone(),
                        file: Some(operation.anchor.source_file.clone()),
                        anchor: Some(operation.anchor.clone()),
                        graph_node_id: None,
                        unresolved: true,
                    });
                    all_edges.push(edge(
                        caller,
                        &target,
                        callee,
                        CallResolution::Unresolved,
                        call_site,
                    ));
                    continue;
                }
                let resolution = if resolved_symbols.len() > 1 {
                    CallResolution::Ambiguous
                } else if is_inferred(function, &resolved_symbols[0], &functions, &inferred) {
                    CallResolution::Inferred
                } else {
                    CallResolution::Resolved
                };
                for target in resolved_symbols {
                    all_edges.push(edge(caller, target, callee, resolution, call_site.clone()));
                }
            }
        }
    }
    all_edges.sort_by(|left, right| left.id.cmp(&right.id));

    let mut selected = BTreeSet::from([root.clone()]);
    let mut queue = VecDeque::from([(root.clone(), 0_u32)]);
    let mut continuations = BTreeSet::new();
    while let Some((symbol, level)) = queue.pop_front() {
        let related = all_edges.iter().filter(|edge| match request.direction {
            CallGraphDirection::Callers => edge.target == symbol,
            CallGraphDirection::Callees => edge.source == symbol,
            CallGraphDirection::Both => edge.source == symbol || edge.target == symbol,
        });
        for edge in related {
            let next = if edge.source == symbol {
                &edge.target
            } else {
                &edge.source
            };
            if level >= request.depth {
                if all_nodes.get(next).is_some_and(|node| !node.unresolved) {
                    continuations.insert(next.clone());
                }
                continue;
            }
            if selected.insert(next.clone()) {
                queue.push_back((next.clone(), level + 1));
            }
        }
    }
    let mut truncated = false;
    if selected.len() > request.max_nodes {
        let retained = selected
            .iter()
            .take(request.max_nodes)
            .cloned()
            .collect::<BTreeSet<_>>();
        continuations.extend(selected.difference(&retained).cloned());
        selected = retained;
        selected.insert(root.clone());
        truncated = true;
    }
    let mut edges = all_edges
        .into_iter()
        .filter(|edge| selected.contains(&edge.source) && selected.contains(&edge.target))
        .collect::<Vec<_>>();
    if edges.len() > request.max_edges {
        for edge in &edges[request.max_edges..] {
            continuations.insert(edge.source.clone());
            continuations.insert(edge.target.clone());
        }
        edges.truncate(request.max_edges);
        truncated = true;
    }
    let nodes = selected
        .iter()
        .filter_map(|id| all_nodes.remove(id))
        .collect::<Vec<_>>();
    let coverage = CallCoverage {
        resolved: edges
            .iter()
            .filter(|edge| edge.resolution == CallResolution::Resolved)
            .count(),
        inferred: edges
            .iter()
            .filter(|edge| edge.resolution == CallResolution::Inferred)
            .count(),
        ambiguous: edges
            .iter()
            .filter(|edge| edge.resolution == CallResolution::Ambiguous)
            .count(),
        unresolved: edges
            .iter()
            .filter(|edge| edge.resolution == CallResolution::Unresolved)
            .count(),
        warning: "Unresolved calls are retained and never prove absence.",
    };
    let continuations = continuations
        .into_iter()
        .map(|symbol| CallContinuation {
            symbol,
            direction: request.direction,
            next_depth: request.depth.saturating_add(1),
        })
        .collect::<Vec<_>>();
    truncated |= !continuations.is_empty();
    Ok(CallGraphResponse {
        schema: CALL_GRAPH_SCHEMA,
        root_symbol: root,
        direction: request.direction,
        depth: request.depth,
        nodes,
        edges,
        truncated,
        continuations,
        coverage,
    })
}

fn resolve_root(
    functions: &BTreeMap<String, (&compass_ir::ModuleIr, &FunctionIr)>,
    root: &CallGraphRoot,
) -> Result<String, CallGraphError> {
    match root {
        CallGraphRoot::Symbol { symbol } => {
            if functions.contains_key(symbol) {
                return Ok(symbol.clone());
            }
            let matches = functions
                .iter()
                .filter(|(_, (_, function))| function.name == *symbol)
                .map(|(symbol, _)| symbol.clone())
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [symbol] => Ok(symbol.clone()),
                [] => Err(CallGraphError::MissingRoot(symbol.clone())),
                _ => Err(CallGraphError::AmbiguousRoot(symbol.clone())),
            }
        }
        CallGraphRoot::SourceByte { file, byte } => functions
            .iter()
            .filter(|(_, (module, function))| {
                module.source_file == *file
                    && function.anchor.start_byte <= *byte
                    && *byte <= function.anchor.end_byte
            })
            .min_by_key(|(symbol, (_, function))| {
                (
                    function.anchor.end_byte - function.anchor.start_byte,
                    symbol.as_str(),
                )
            })
            .map(|(symbol, _)| symbol.clone())
            .ok_or_else(|| CallGraphError::MissingRoot(format!("{file}:{byte}"))),
    }
}

fn edge(
    source: &str,
    target: &str,
    callee: &str,
    resolution: CallResolution,
    call_site: CallSite,
) -> CallEdge {
    CallEdge {
        id: format!(
            "{source}:{}:{target}:{}",
            call_site.anchor.start_byte, call_site.anchor.end_byte
        ),
        source: source.to_owned(),
        target: target.to_owned(),
        callee: callee.to_owned(),
        resolution,
        call_sites: vec![call_site],
    }
}

fn inferred_calls(graph: Option<&GraphDocument>) -> BTreeSet<(String, String)> {
    graph
        .into_iter()
        .flat_map(|graph| &graph.links)
        .filter(|edge| {
            edge.string("relation").eq_ignore_ascii_case("calls")
                && edge.string("confidence").eq_ignore_ascii_case("inferred")
        })
        .map(|edge| (edge.source.clone(), edge.target.clone()))
        .collect()
}

fn is_inferred(
    caller: &FunctionIr,
    target: &str,
    functions: &BTreeMap<String, (&compass_ir::ModuleIr, &FunctionIr)>,
    inferred: &BTreeSet<(String, String)>,
) -> bool {
    caller.graph_node_id.as_ref().is_some_and(|source| {
        functions
            .get(target)
            .and_then(|(_, function)| function.graph_node_id.as_ref())
            .is_some_and(|target| inferred.contains(&(source.clone(), target.clone())))
    })
}
