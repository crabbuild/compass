//! Local declaration names are inspection evidence for their unique callable
//! owner. Containment does not establish behavior or resolve a reference.
use super::*;
use compass_model::code_graph::EdgeRecord;

const MAX_CHILDREN: usize = 32;
const MAX_PARENTS: usize = 4;
pub(super) const READ_RESERVE: u64 = (MAX_CHILDREN * (MAX_PARENTS * 2 + 1)) as u64;
pub(super) const PROBE_RESERVE: u64 = MAX_CHILDREN as u64;

pub(super) fn is_local(node: &NodeRecord) -> bool {
    matches!(
        node.kind,
        NodeKind::Parameter
            | NodeKind::Variable
            | NodeKind::Field
            | NodeKind::Property
            | NodeKind::Constant
    )
}

#[derive(Default)]
pub(super) struct OwnershipRecall {
    pub nodes: BTreeMap<String, NodeRecord>,
    pub terms: BTreeMap<String, BTreeSet<String>>,
    pub sources: BTreeMap<String, BTreeSet<SourceAnchor>>,
    pub witnesses: BTreeMap<String, EdgeRecord>,
    pub children: BTreeMap<String, NodeRecord>,
    pub reads: u64,
    pub probes: u64,
    pub bounded: bool,
}

pub(super) fn recall(
    backend: &PinnedDiscoveryBackend<'_>,
    candidates: &BTreeMap<String, NodeRecord>,
    groups: &[(String, Vec<String>, f64, u64)],
    scopes: &[DiscoveryScope],
    check: &dyn Fn() -> Result<(), QueryError>,
    admit: &dyn Fn(&NodeRecord) -> bool,
) -> Result<OwnershipRecall, QueryError> {
    let mut result = OwnershipRecall::default();
    let mut children = Vec::new();
    for child in candidates.values() {
        check()?;
        if !is_local(child) || !relationships::exact_ast(child) {
            continue;
        }
        let document = search_document(child);
        result.bounded |= document.truncated;
        let terms = document
            .fields
            .get("name")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|term| {
                groups
                    .iter()
                    .any(|(_, variants, _, _)| variants.contains(term))
            })
            .collect::<BTreeSet<_>>();
        if terms.is_empty() {
            continue;
        }
        let weight = groups
            .iter()
            .filter(|(_, variants, _, _)| variants.iter().any(|term| terms.contains(term)))
            .map(|(_, _, idf, _)| idf)
            .sum::<f64>();
        children.push((child, terms, weight));
    }
    children.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.id.cmp(&b.0.id)));
    result.bounded |= children.len() > MAX_CHILDREN;
    for (child, terms, _) in children.into_iter().take(MAX_CHILDREN) {
        check()?;
        result.probes += 1;
        let (edges, truncated, examined) = backend.matching_bounded_counted(
            &child.id,
            true,
            &[EdgeKind::Contains],
            true,
            MAX_PARENTS,
        )?;
        result.reads += examined as u64;
        // A hidden parent can invalidate uniqueness. Never choose from a
        // truncated prefix, even when every visible parent agrees.
        if truncated {
            result.bounded = true;
            continue;
        }
        let mut owners = BTreeMap::<String, (NodeRecord, Vec<EdgeRecord>)>::new();
        for edge in edges {
            check()?;
            if edge.target != child.id
                || !compass_model::search::is_exact_nonheuristic_relationship(&edge)
                || edge.evidence.iter().any(|proof| proof.validate().is_err())
            {
                continue;
            }
            let (Some(child_source), Some(site)) = (&child.source, &edge.relationship_site) else {
                continue;
            };
            if !relationships::encloses(child_source, site)
                || !edge.evidence.iter().any(|proof| {
                    proof.origin == EvidenceOrigin::Ast && proof.anchors.contains(site)
                })
            {
                continue;
            }
            result.reads += 1;
            let Some(owner) = backend.node_by_id(&edge.source)? else {
                continue;
            };
            if !owner.kind.is_callable()
                || !relationships::exact_ast(&owner)
                || !owner
                    .source
                    .as_ref()
                    .is_some_and(|source| relationships::encloses(source, child_source))
            {
                continue;
            }
            owners
                .entry(owner.id.clone())
                .or_insert_with(|| (owner, Vec::new()))
                .1
                .push(edge);
        }
        // Scope is applied after uniqueness: excluding one ambiguous parent
        // must never manufacture unique ownership of the other.
        if owners.len() != 1 {
            continue;
        }
        let Some((owner, edges)) = owners.into_values().next() else {
            continue;
        };
        if !discovery_scope_matches(&owner, scopes) || !admit(&owner) {
            continue;
        }
        let retained = result.terms.entry(owner.id.clone()).or_default();
        retained.extend(terms);
        if retained.len() > compass_model::search::SEARCH_FIELD_MAX_TERMS {
            result.bounded = true;
            *retained = retained
                .iter()
                .take(compass_model::search::SEARCH_FIELD_MAX_TERMS)
                .cloned()
                .collect();
        }
        for edge in edges {
            if let Some(site) = &edge.relationship_site {
                result
                    .sources
                    .entry(owner.id.clone())
                    .or_default()
                    .insert(site.clone());
            }
            result.witnesses.insert(edge.id.clone(), edge);
        }
        result.children.insert(child.id.clone(), child.clone());
        result.nodes.insert(owner.id.clone(), owner);
    }
    Ok(result)
}
