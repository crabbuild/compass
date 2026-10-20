//! Relationship postings recall identifiers, not behavioral assertions.
//! Only a witnessed outgoing call/reference inside a callable contributes its
//! target's terminal name. Namespace, path and neighboring prose are excluded.
use super::*;
use compass_model::code_graph::EdgeRecord;

const MAX_POSTING_PROBES: usize = 8;
const MAX_OWNERS: usize = 64;
const MAX_OUTGOING: usize = 32;
const MAX_READS: u64 = 6400;
const MAX_PROBES: u64 = 96;

#[derive(Default)]
pub(super) struct RelationshipRecall {
    pub nodes: BTreeMap<String, NodeRecord>,
    pub terms: BTreeMap<String, BTreeSet<String>>,
    pub sources: BTreeMap<String, BTreeSet<SourceAnchor>>,
    pub witnesses: BTreeMap<String, EdgeRecord>,
    pub targets: BTreeMap<String, NodeRecord>,
    pub reads: u64,
    pub probes: u64,
    pub bounded: bool,
    pub legacy: bool,
}

pub(super) fn encloses(owner: &SourceAnchor, site: &SourceAnchor) -> bool {
    owner.is_valid()
        && site.is_valid()
        && site.start_byte < site.end_byte
        && owner.file == site.file
        && owner.start_byte <= site.start_byte
        && site.end_byte <= owner.end_byte
        && (owner.start_line, owner.start_column) <= (site.start_line, site.start_column)
        && (site.end_line, site.end_column) <= (owner.end_line, owner.end_column)
}

pub(super) fn exact_ast(node: &NodeRecord) -> bool {
    let Some(source) = &node.source else {
        return false;
    };
    !node.evidence.is_empty()
        && node.evidence.iter().all(|proof| {
            proof.origin != EvidenceOrigin::Heuristic
                && proof.confidence == EvidenceConfidence::Exact
                && proof.validate().is_ok()
        })
        && node.evidence.iter().any(|proof| {
            proof.origin == EvidenceOrigin::Ast
                && proof.anchors.iter().any(|anchor| encloses(source, anchor))
        })
}

pub(super) fn verified_site<'a>(
    owner: &NodeRecord,
    edge: &'a EdgeRecord,
) -> Option<&'a SourceAnchor> {
    let (Some(source), Some(site)) = (&owner.source, &edge.relationship_site) else {
        return None;
    };
    (edge.source == owner.id
        && matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)
        && encloses(source, site)
        && !edge.evidence.is_empty()
        && edge.evidence.iter().all(|proof| {
            proof.origin != EvidenceOrigin::Heuristic
                && proof.confidence == EvidenceConfidence::Exact
                && proof.validate().is_ok()
        })
        && edge
            .evidence
            .iter()
            .any(|proof| proof.origin == EvidenceOrigin::Ast && proof.anchors.contains(site)))
    .then_some(site)
}

pub(super) fn recall(
    backend: &PinnedDiscoveryBackend<'_>,
    groups: &[(String, Vec<String>, f64, u64)],
    scopes: &[DiscoveryScope],
    check: &dyn Fn() -> Result<(), QueryError>,
    admit: &dyn Fn(&NodeRecord) -> bool,
) -> Result<RelationshipRecall, QueryError> {
    let mut result = RelationshipRecall::default();
    if let PinnedDiscoveryBackend::Store(reader) = backend {
        result.probes += 1;
        result.legacy = !reader
            .supports_relationship_search_terms()
            .map_err(crate::code_query::snapshot_error)?;
        result.bounded |= result.legacy;
    }
    let mut sources = BTreeMap::<String, f64>::new();
    // One variant per concept before alternative spellings; a single expanded
    // concept must not consume every independent recall quota.
    let mut queries = Vec::new();
    for offset in 0..groups.iter().map(|group| group.1.len()).max().unwrap_or(0) {
        for (_, variants, idf, _) in groups {
            if let Some(variant) = variants.get(offset) {
                queries.push((variant, *idf));
            }
        }
    }
    result.bounded |= queries.len() > MAX_POSTING_PROBES;
    for (variant, idf) in queries.into_iter().take(MAX_POSTING_PROBES) {
        check()?;
        if result.reads.saturating_add(POSTING_QUOTA as u64 * 4) > MAX_READS
            || result.probes.saturating_add(8) > MAX_PROBES
        {
            result.bounded = true;
            break;
        }
        // The older discovery posting intentionally contains direct calls
        // only. Use the separately versioned broad relationship posting.
        let read = backend.relationship_sources_for_term(variant, POSTING_QUOTA)?;
        result.reads = result.reads.saturating_add(read.node_ids_decoded);
        // Store reads also check the capability marker before choosing their
        // broad or legacy posting. Count that logical lookup, too.
        result.probes = result.probes.saturating_add(
            1 + read.chunks_decoded
                + u64::from(matches!(backend, PinnedDiscoveryBackend::Store(_))),
        );
        result.bounded |= read.truncated;
        for id in read.source_ids {
            *sources.entry(id).or_default() += idf;
        }
    }
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result.bounded |= sources.len() > MAX_OWNERS;
    for (id, _) in sources.into_iter().take(MAX_OWNERS) {
        check()?;
        if result.reads.saturating_add((MAX_OUTGOING * 2 + 2) as u64) > MAX_READS
            || result.probes >= MAX_PROBES
        {
            result.bounded = true;
            break;
        }
        result.reads += 1;
        let Some(owner) = backend.node_by_id(&id)? else {
            continue;
        };
        if !owner.kind.is_callable()
            || !exact_ast(&owner)
            || !discovery_scope_matches(&owner, scopes)
            || !admit(&owner)
        {
            continue;
        }
        result.probes += 1;
        let (edges, truncated, examined) = backend.matching_bounded_counted(
            &id,
            false,
            &[EdgeKind::Calls, EdgeKind::References],
            true,
            MAX_OUTGOING,
        )?;
        result.reads = result.reads.saturating_add(examined as u64);
        result.bounded |= truncated;
        for edge in edges {
            check()?;
            let Some(site) = verified_site(&owner, &edge) else {
                continue;
            };
            result.reads += 1;
            let Some(target) = backend.node_by_id(&edge.target)? else {
                continue;
            };
            if !exact_ast(&target) {
                continue;
            }
            // Shared projections enforce byte/token bounds even on adversarial
            // terminal names. Do not bypass those bounds with raw tokenization.
            let document = search_document(&target);
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
            result.bounded |= document.truncated;
            if !groups
                .iter()
                .any(|(_, variants, _, _)| variants.iter().any(|term| terms.contains(term)))
            {
                continue;
            }
            let retained = result.terms.entry(id.clone()).or_default();
            retained.extend(terms);
            if retained.len() > compass_model::search::SEARCH_FIELD_MAX_TERMS {
                result.bounded = true;
                *retained = retained
                    .iter()
                    .take(compass_model::search::SEARCH_FIELD_MAX_TERMS)
                    .cloned()
                    .collect();
            }
            result
                .sources
                .entry(id.clone())
                .or_default()
                .insert(site.clone());
            result.targets.insert(target.id.clone(), target);
            result.witnesses.insert(edge.id.clone(), edge);
        }
        if result.terms.contains_key(&id) {
            result.nodes.insert(id, owner);
        }
    }
    Ok(result)
}
