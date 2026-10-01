//! Independent direct-adjacency audit and deterministic impact grouping.
use super::*;
use compass_model::query_contract::{ImpactFileGroup, ImpactSummary};

impl CodeQueryEngine {
    pub(super) fn summarize_impact(
        &self,
        response: &mut CodeQueryResponse,
        seed: &str,
        relations: &[EdgeKind],
        include_heuristic: bool,
    ) -> Result<(), QueryError> {
        let limit = usize::try_from(response.limits.max_edges).unwrap_or(usize::MAX);
        let (incident, limited) = self.backend.incident_bounded(seed, true, limit)?;
        let mut contacts = BTreeSet::new();
        let mut selected = BTreeSet::new();
        let mut outgoing = BTreeSet::new();
        let mut excluded = BTreeSet::new();
        for edge in incident {
            self.check_deadline()?;
            if edge.target == seed {
                let contact = (false, edge.source.clone());
                contacts.insert(contact.clone());
                if relations.contains(&edge.kind) && (include_heuristic || !is_heuristic(&edge)) {
                    selected.insert(edge.source.clone());
                } else {
                    excluded.insert(contact);
                }
            }
            if edge.source == seed {
                contacts.insert((true, edge.target.clone()));
                outgoing.insert(edge.target);
            }
        }
        // A neighbor with several relation kinds is covered if any selected
        // incoming relation reaches it. Do not count parallel edges as nodes.
        excluded.retain(|(outbound, id)| *outbound || !selected.contains(id));
        // A self-reference is a recorded contact, but the seed is not its
        // own affected dependent. Account for it explicitly as an exclusion.
        if selected.remove(seed) {
            excluded.insert((false, seed.to_owned()));
        }
        let returned = response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let missing = selected
            .iter()
            .filter(|id| !returned.contains(id.as_str()))
            .count();
        if missing > 0 && !response.truncated && !limited {
            return Err(QueryError::new(
                QueryErrorKind::GraphInvariant,
                "impact_direct_coverage_mismatch",
                format!(
                    "impact omitted {missing} selected direct dependents; retry compass impact {seed} --format json and report impact_direct_coverage_mismatch"
                ),
            ));
        }
        let direct = selected
            .into_iter()
            .filter(|id| id != seed && returned.contains(id.as_str()))
            .collect::<BTreeSet<_>>();
        let affected = response
            .edges
            .iter()
            .filter(|edge| relations.contains(&edge.kind))
            .map(|edge| edge.source.clone())
            .filter(|id| id != seed && returned.contains(id.as_str()))
            .collect::<BTreeSet<_>>();
        let transitive = affected
            .difference(&direct)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut groups = BTreeMap::<(Option<String>, String), ImpactFileGroup>::new();
        for node in &response.nodes {
            if !direct.contains(&node.id) && !transitive.contains(&node.id) {
                continue;
            }
            let file = node.source.as_ref().map(|source| source.file.clone());
            let module = file
                .as_deref()
                .and_then(|file| file.rsplit_once('/').map(|(parent, _)| parent))
                .unwrap_or("<root>")
                .to_owned();
            let group = groups
                .entry((file.clone(), module.clone()))
                .or_insert_with(|| ImpactFileGroup {
                    file,
                    module,
                    direct: Vec::new(),
                    transitive: Vec::new(),
                });
            if direct.contains(&node.id) {
                group.direct.push(node.id.clone());
            } else {
                group.transitive.push(node.id.clone());
            }
        }
        let covered = direct.len() + outgoing.len() + excluded.len();
        if missing == 0 && covered != contacts.len() {
            return Err(QueryError::new(
                QueryErrorKind::GraphInvariant,
                "impact_connection_accounting_mismatch",
                "impact direct-connection accounting is inconsistent",
            ));
        }
        let mut relations = relations.to_vec();
        relations.sort_by_key(|kind| kind.as_str());
        relations.dedup();
        response.impact_summary = Some(ImpactSummary {
            relations,
            observed_direct_connections: u64::try_from(contacts.len()).unwrap_or(u64::MAX),
            direct_dependents: direct.into_iter().collect(),
            transitive_dependents: transitive.into_iter().collect(),
            outgoing_context: outgoing.into_iter().collect(),
            excluded_direct_connections: u64::try_from(excluded.len()).unwrap_or(u64::MAX),
            // This audits published adjacency. Repository-wide omissions stay
            // in IncompleteCoverage diagnostics and never become a completeness
            // claim about the source tree.
            direct_coverage_complete: !limited && missing == 0,
            groups: groups.into_values().collect(),
        });
        enforce_response_size(response)
    }
}
