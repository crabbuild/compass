mod support;

use std::error::Error;

use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_model::identity::edge_id;
use compass_model::query_contract::{CodeQueryLimits, NodeTrailRequest};
use compass_query::{
    HopPathResult, open_with_document, render_shortest_path_with_limit, shortest_hop_path,
};

const PAIRS: [(usize, usize); 6] = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
type Matrix = [[Option<u32>; 4]; 4];

// Independent enumeration of all simple paths. Positive weights mean a cycle
// cannot improve either cost or hop count. This oracle does not reuse the
// production queue, dominance rules, relation-weight mapping, or predecessor map.
fn oracle(matrix: &Matrix, max_depth: usize) -> Option<(u32, usize)> {
    fn visit(matrix: &Matrix, node: usize, remaining: usize, visited: u8) -> Option<(u32, usize)> {
        if node == 3 {
            return Some((0, 0));
        }
        if remaining == 0 {
            return None;
        }
        (0..4)
            .filter(|next| visited & (1 << next) == 0)
            .filter_map(|next| {
                let edge = matrix[node][next]?;
                let (cost, hops) = visit(matrix, next, remaining - 1, visited | (1 << next))?;
                Some((edge + cost, 1 + hops))
            })
            .min()
    }
    visit(matrix, 0, max_depth, 1)
}

fn hop_oracle(matrix: &Matrix, max_depth: usize) -> Option<(usize, u32)> {
    fn visit(matrix: &Matrix, node: usize, remaining: usize, visited: u8) -> Option<(usize, u32)> {
        if node == 3 {
            return Some((0, 0));
        }
        if remaining == 0 {
            return None;
        }
        (0..4)
            .filter(|next| visited & (1 << next) == 0)
            .filter_map(|next| {
                let edge = matrix[node][next]?;
                let (hops, cost) = visit(matrix, next, remaining - 1, visited | (1 << next))?;
                Some((hops + 1, cost + edge))
            })
            .min()
    }
    visit(matrix, 0, max_depth, 1)
}

#[test]
fn all_path_engines_match_exhaustive_four_node_oracles() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut template = GraphDocument::load(&graph_path)?;
    let node_pattern = regex::Regex::new(r"Node([0-3])")?;
    let edge_pattern = regex::Regex::new(
        r"--(calls|references)(?: \[[^\]]+\])?-->|<--(calls|references)(?: \[[^\]]+\])?--",
    )?;
    let edge_template = template
        .links
        .first()
        .cloned()
        .ok_or("missing edge template")?;
    template.nodes = (0..4)
        .map(|i| {
            support::node(
                &format!("n:{i}"),
                NodeKind::Function,
                &format!("Node{i}"),
                &format!("Node{i}"),
            )
        })
        .collect();

    // Every one of the six forward pairs is absent, a cost-1 call, or a cost-4
    // reference: 729 graphs. The typed engine sees DAGs; the undirected path
    // engine also sees cycles. Every graph is queried at all three hop bounds.
    for encoding in 0_u32..729 {
        let mut graph = template.clone();
        graph.links.clear();
        let mut directed: Matrix = [[None; 4]; 4];
        let mut undirected: Matrix = [[None; 4]; 4];
        let mut digits = encoding;
        for (source, target) in PAIRS {
            let digit = digits % 3;
            digits /= 3;
            let (kind, weight) = match digit {
                1 => (EdgeKind::Calls, 1),
                2 => (EdgeKind::References, 4),
                _ => continue,
            };
            directed[source][target] = Some(weight);
            undirected[source][target] = Some(weight);
            undirected[target][source] = Some(weight);
            let mut edge = edge_template.clone();
            edge.source = format!("n:{source}");
            edge.target = format!("n:{target}");
            edge.kind = kind;
            edge.occurrence_rule = None;
            edge.id = edge_id(
                &edge.source,
                kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key.clone_from(&edge.id);
            graph.links.push(edge);
        }
        let legacy = compass_model::Graph::from_document(serde_json::from_value(
            serde_json::to_value(&graph)?,
        )?)?;
        // Drop each index before the next graph so this test's disk use stays
        // bounded by one tiny graph rather than accumulating 729 databases.
        let cache = tempfile::tempdir_in(directory.path())?;
        let engine = open_with_document(graph.clone(), &graph_path, None, cache.path())?;
        for depth in 1..=3 {
            let context = format!("graph={encoding}, depth={depth}");
            let start = legacy.node_index("n:0").ok_or("start")?;
            let end = legacy.node_index("n:3").ok_or("end")?;
            match (
                hop_oracle(&undirected, depth),
                shortest_hop_path(&legacy, start, end, depth)?,
            ) {
                (Some((hops, cost)), HopPathResult::Found { nodes, edges }) => {
                    assert_eq!(nodes.first(), Some(&start), "{context}");
                    assert_eq!(nodes.last(), Some(&end), "{context}");
                    assert_eq!(nodes.len(), hops + 1, "{context}");
                    assert_eq!(edges.len(), hops, "{context}");
                    let mut actual_cost = 0;
                    for (pair, edge) in nodes.windows(2).zip(edges) {
                        let edge = legacy.edge(edge);
                        let weight = match edge.string("relation").as_str() {
                            "calls" => 1,
                            "references" => 4,
                            _ => return Err("unknown path relation".into()),
                        };
                        let source = edge
                            .source
                            .strip_prefix("n:")
                            .ok_or("edge source")?
                            .parse::<usize>()?;
                        let target = edge
                            .target
                            .strip_prefix("n:")
                            .ok_or("edge target")?
                            .parse::<usize>()?;
                        assert_eq!(directed[source][target], Some(weight), "{context}");
                        let endpoints = (&legacy.node(pair[0]).id, &legacy.node(pair[1]).id);
                        assert!(
                            endpoints == (&edge.source, &edge.target)
                                || endpoints == (&edge.target, &edge.source),
                            "{context}"
                        );
                        actual_cost += weight;
                    }
                    assert_eq!(actual_cost, cost, "{context}");
                }
                (None, HopPathResult::NoPath { depth_limited, .. }) => {
                    if !depth_limited {
                        assert!(hop_oracle(&undirected, 3).is_none(), "{context}");
                    }
                }
                (expected, actual) => {
                    return Err(format!("{context}: expected {expected:?}, got {actual:?}").into());
                }
            }
            let output = render_shortest_path_with_limit(&legacy, "n:0", "n:3", depth)?;
            if let Some((cost, hops)) = oracle(&undirected, depth) {
                assert!(
                    output.contains(&format!(
                        "Best path (weighted, {hops} hops, weight {cost}):"
                    )),
                    "{context}: {output}"
                );
                let body = output
                    .lines()
                    .find(|line| line.trim_start().starts_with("Node0"))
                    .ok_or("missing rendered path body")?;
                let nodes = node_pattern
                    .captures_iter(body)
                    .map(|capture| capture[1].parse::<usize>())
                    .collect::<Result<Vec<_>, _>>()?;
                let edges = edge_pattern.captures_iter(body).collect::<Vec<_>>();
                assert_eq!(nodes.first(), Some(&0), "{context}");
                assert_eq!(nodes.last(), Some(&3), "{context}");
                assert_eq!(nodes.len(), hops + 1, "{context}");
                assert_eq!(edges.len(), hops, "{context}");
                let mut rendered_cost = 0;
                for (pair, edge) in nodes.windows(2).zip(edges) {
                    let (source, target, relation) = if let Some(relation) = edge.get(1) {
                        (pair[0], pair[1], relation.as_str())
                    } else {
                        (
                            pair[1],
                            pair[0],
                            edge.get(2).ok_or("missing relation")?.as_str(),
                        )
                    };
                    let weight = if relation == "calls" { 1 } else { 4 };
                    assert_eq!(directed[source][target], Some(weight), "{context}: {body}");
                    rendered_cost += weight;
                }
                assert_eq!(rendered_cost, cost, "{context}");
            } else {
                assert!(output.contains("NO PATH FOUND"), "{context}: {output}");
            }
            let response = engine.node_trail(NodeTrailRequest {
                source: "n:0".to_owned(),
                target: "n:3".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits {
                    max_depth: depth as u32,
                    ..CodeQueryLimits::default()
                },
            })?;
            assert!(!response.truncated, "{context}");
            if let Some((expected_cost, expected_hops)) = oracle(&directed, depth) {
                assert_eq!(response.paths.len(), 1, "{context}");
                let path = &response.paths[0];
                assert_eq!(
                    path.node_ids.first().map(String::as_str),
                    Some("n:0"),
                    "{context}"
                );
                assert_eq!(
                    path.node_ids.last().map(String::as_str),
                    Some("n:3"),
                    "{context}"
                );
                assert_eq!(path.edge_ids.len(), expected_hops, "{context}");
                assert_eq!(path.node_ids.len(), expected_hops + 1, "{context}");
                let mut cost = 0;
                for (pair, edge_id) in path.node_ids.windows(2).zip(&path.edge_ids) {
                    let edge = graph
                        .links
                        .iter()
                        .find(|edge| edge.id == *edge_id)
                        .ok_or("path invented an edge")?;
                    assert_eq!(
                        (&edge.source, &edge.target),
                        (&pair[0], &pair[1]),
                        "{context}"
                    );
                    cost += match edge.kind {
                        EdgeKind::Calls => 1,
                        EdgeKind::References => 4,
                        _ => return Err("unexpected edge kind".into()),
                    };
                }
                assert_eq!(cost, expected_cost, "{context}");
            } else {
                assert!(response.paths.is_empty(), "{context}");
            }
        }
    }
    Ok(())
}
