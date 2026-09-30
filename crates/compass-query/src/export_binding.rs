//! Name lookup may expose both a public export binding and its declaration.
//! Only explicit, coincident export evidence lets lookup remove the redundant
//! binding candidate. Exact-ID lookup bypasses this rule entirely.

use std::collections::BTreeSet;

use compass_model::code_graph::{EdgeRecord, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, SourceAnchor, effective_confidence};

pub(crate) const MAX_EXPORT_PROOF_EDGES: usize = 1024;
pub(crate) const MAX_EXPORT_PROOF_CANDIDATES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceRange {
    file: String,
    start: (u32, u32),
    end: (u32, u32),
}

impl SourceRange {
    fn from_anchor(anchor: &SourceAnchor) -> Option<Self> {
        Self::new(
            anchor.file.clone(),
            (anchor.start_line, anchor.start_column),
            (anchor.end_line, anchor.end_column),
        )
    }

    fn new(file: String, start: (u32, u32), end: (u32, u32)) -> Option<Self> {
        (!file.is_empty() && start.0 > 0 && start < end).then_some(Self { file, start, end })
    }

    fn from_location(file: String, location: &str) -> Option<Self> {
        let (start, end) = location.strip_prefix('L')?.split_once("-L")?;
        let position = |value: &str| {
            let (line, column) = value.split_once(':')?;
            Some((line.parse::<u32>().ok()?, column.parse::<u32>().ok()?))
        };
        Self::new(file, position(start)?, position(end)?)
    }
}

pub(crate) struct BindingNode {
    pub(crate) id: String,
    kind: String,
    name: String,
    qualified_name: String,
    site: Option<SourceRange>,
}

impl BindingNode {
    pub(crate) fn typed(node: &NodeRecord) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind.as_str().to_owned(),
            name: crate::normalize_code_query_symbol(&node.name),
            qualified_name: node.qualified_name.clone(),
            site: node.source.as_ref().and_then(SourceRange::from_anchor),
        }
    }

    pub(crate) fn legacy(node: &compass_model::NodeRecord) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind_name().to_owned(),
            name: crate::normalize_code_query_symbol(node.label()),
            qualified_name: node
                .logical_property("qualified_name")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            site: SourceRange::from_location(
                node.string("source_file"),
                &node.string("source_location"),
            ),
        }
    }

    fn represents(&self, target: &Self) -> bool {
        self.kind == "export"
            && !matches!(
                target.kind.as_str(),
                "export" | "import" | "module" | "file"
            )
            && !self.qualified_name.is_empty()
            && self.qualified_name == target.qualified_name
            && self.name == target.name
            && matches!((&self.site, &target.site), (Some(left), Some(right)) if left.file == right.file)
    }
}

pub(crate) struct BindingEdge {
    source: String,
    target: String,
    site: Option<SourceRange>,
    exact: bool,
}

impl BindingEdge {
    pub(crate) fn typed(edge: &EdgeRecord) -> Self {
        Self {
            source: edge.source.clone(),
            target: edge.target.clone(),
            site: edge
                .relationship_site
                .as_ref()
                .and_then(SourceRange::from_anchor),
            exact: !edge.deferred
                && effective_confidence(&edge.evidence) == Some(EvidenceConfidence::Exact),
        }
    }

    pub(crate) fn legacy(edge: &compass_model::EdgeRecord) -> Self {
        // Do not inherit the permissive compatibility default for an unknown
        // confidence spelling, or select the first item of mixed evidence.
        let mut confidence_seen = false;
        let mut exact = edge
            .attributes
            .get("deferred")
            .is_none_or(|value| value.as_bool() == Some(false));
        if let Some(value) = edge.attributes.get("confidence") {
            confidence_seen = true;
            exact &= matches!(value.as_str(), Some("EXTRACTED" | "exact"));
        }
        if let Some(evidence) = edge.attributes.get("evidence") {
            if let Some(items) = evidence.as_array() {
                for item in items {
                    confidence_seen = true;
                    exact &=
                        item.get("confidence").and_then(serde_json::Value::as_str) == Some("exact");
                }
            } else {
                exact = false;
            }
        }
        Self {
            source: edge.source.clone(),
            target: edge.target.clone(),
            site: SourceRange::from_location(
                edge.string("source_file"),
                &edge.string("source_location"),
            ),
            exact: exact && confidence_seen,
        }
    }
}

pub(crate) struct ProofRead {
    pub(crate) edges: Vec<BindingEdge>,
    pub(crate) examined: usize,
    pub(crate) truncated: bool,
}

#[derive(Default)]
pub(crate) struct ExportProof {
    pub(crate) redundant: BTreeSet<String>,
    pub(crate) examined: usize,
    pub(crate) truncated: bool,
}

pub(crate) fn redundant_export_bindings<E>(
    candidates: &[BindingNode],
    mut read: impl FnMut(&str, bool, usize) -> Result<ProofRead, E>,
) -> Result<ExportProof, E> {
    let mut proof = ExportProof::default();
    if candidates.len() > MAX_EXPORT_PROOF_CANDIDATES {
        return Ok(proof);
    }
    for binding in candidates.iter().filter(|n| n.kind == "export") {
        if !candidates.iter().any(|target| binding.represents(target)) {
            continue;
        }
        let Some(site) = binding.site.as_ref() else {
            continue;
        };
        let remaining = MAX_EXPORT_PROOF_EDGES.saturating_sub(proof.examined);
        if remaining == 0 {
            proof.truncated = true;
            break;
        }
        let incoming = read(&binding.id, true, remaining)?;
        proof.examined = proof.examined.saturating_add(incoming.examined);
        if incoming.truncated {
            proof.truncated = true;
            break;
        }
        let [contains] = incoming.edges.as_slice() else {
            continue;
        };
        if contains.target != binding.id || !contains.exact || contains.site.as_ref() != Some(site)
        {
            continue;
        }
        let remaining = MAX_EXPORT_PROOF_EDGES.saturating_sub(proof.examined);
        if remaining == 0 {
            proof.truncated = true;
            break;
        }
        let exports = read(&contains.source, false, remaining)?;
        proof.examined = proof.examined.saturating_add(exports.examined);
        if exports.truncated {
            proof.truncated = true;
            break;
        }
        // Missing occurrence evidence cannot rule out another target at this
        // binding. Do not decide uniqueness from a partial source projection.
        if exports.edges.iter().any(|edge| edge.site.is_none()) {
            continue;
        }
        let selected = exports
            .edges
            .iter()
            .filter(|edge| edge.site.as_ref() == Some(site))
            .collect::<Vec<_>>();
        if selected.is_empty()
            || selected
                .iter()
                .any(|edge| !edge.exact || edge.source != contains.source)
        {
            continue;
        }
        let targets = selected
            .iter()
            .map(|edge| edge.target.as_str())
            .collect::<BTreeSet<_>>();
        if targets.len() != 1 {
            continue;
        }
        if candidates
            .iter()
            .any(|target| targets.contains(target.id.as_str()) && binding.represents(target))
        {
            proof.redundant.insert(binding.id.clone());
        }
    }
    // Avoid a partially canonicalized result if any required proof exhausts
    // its budget. The caller retains ambiguity, never an empty/no-match claim.
    if proof.truncated {
        proof.redundant.clear();
    }
    Ok(proof)
}

pub(crate) fn legacy_candidates(
    graph: &compass_model::Graph,
    mut matches: Vec<compass_model::NodeIndex>,
) -> Vec<compass_model::NodeIndex> {
    if matches.len() < 2
        || matches.len() > MAX_EXPORT_PROOF_CANDIDATES
        || !matches
            .iter()
            .any(|index| graph.node(*index).kind_name() == "export")
    {
        return matches;
    }
    let candidates = matches
        .iter()
        .map(|index| BindingNode::legacy(graph.node(*index)))
        .collect::<Vec<_>>();
    let result =
        redundant_export_bindings::<std::convert::Infallible>(&candidates, |id, inbound, limit| {
            let Some(index) = graph.node_index(id) else {
                return Ok(ProofRead {
                    edges: Vec::new(),
                    examined: 0,
                    truncated: false,
                });
            };
            let mut edges = Vec::new();
            let mut examined = 0;
            let mut truncated = false;
            let indices: Box<dyn Iterator<Item = _>> = if inbound {
                Box::new(graph.incoming_edges(index))
            } else {
                Box::new(graph.outgoing_edges(index))
            };
            for edge_index in indices.take(limit.saturating_add(1)) {
                examined += 1;
                if examined > limit {
                    truncated = true;
                    break;
                }
                let edge = graph.edge(edge_index);
                if edge.relation() == if inbound { "contains" } else { "exports" } {
                    edges.push(BindingEdge::legacy(edge));
                }
            }
            Ok(ProofRead {
                edges,
                examined,
                truncated,
            })
        });
    let proof = match result {
        Ok(proof) => proof,
        Err(never) => match never {},
    };
    matches.retain(|index| !proof.redundant.contains(&graph.node(*index).id));
    matches
}
