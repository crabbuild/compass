//! Bounded collection resolution and low-inference evidence compaction.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use ahash::AHashSet;
use compass_languages::{
    CandidateRelation, EvidenceLimits, HierarchyConstraint, RawEdgeRecord as EdgeRecord,
    RawNodeRecord as NodeRecord, SemanticEvidenceBatch, validate_evidence,
};

use crate::ResolutionAdmission;

use super::{
    UniversalResolutionCounts, UniversalResolutionIndex, UniversalResolutionLimits,
    UniversalResolutionReport,
};

const LOW_DETAIL_DECLARATION_KINDS: &[&str] = &[
    "const_parameter",
    "lifetime_parameter",
    "parameter",
    "property",
    "type_parameter",
];

pub(crate) fn materialize_bounded_owned(
    mut batches: Vec<SemanticEvidenceBatch>,
    project_edges: &[EdgeRecord],
    root: &Path,
    limits: UniversalResolutionLimits,
    admission: ResolutionAdmission,
    prevalidated: bool,
    output: (&mut Vec<NodeRecord>, &mut Vec<EdgeRecord>),
) -> UniversalResolutionReport {
    let (nodes, edges) = output;
    let inventory_node_count = nodes.len();
    batches.sort_by(|left, right| batch_source_key(left).cmp(&batch_source_key(right)));
    let input = evidence_counts(&batches);
    if !prevalidated && let Err(reason) = validate_batches_for_projection(&batches) {
        return UniversalResolutionReport {
            degraded: true,
            failed_partitions: 1,
            omitted_candidates: input.candidates,
            input,
            retained: input,
            reason: Some(reason),
            ..UniversalResolutionReport::default()
        };
    }
    let compacted_declarations = if admission == ResolutionAdmission::Low {
        compact_low_inference_evidence(&mut batches)
    } else {
        0
    };
    let retained = evidence_counts(&batches);

    // Declaration projection is independent from project-wide target selection.
    // Do it before constructing any large indexes so an index limit or corrupt
    // secondary lookup can never collapse a repository to file scaffolding.
    let graph_ids = super::projection::project_declaration_batches(&batches, nodes);
    for declaration in batches.iter_mut().flat_map(|batch| &mut batch.declarations) {
        if let Some(graph_node_id) = graph_ids.get(&declaration.id) {
            declaration.graph_node_id.clone_from(graph_node_id);
        }
    }
    drop(graph_ids);

    if retained.fits(limits) {
        let candidate_count = retained.candidates;
        let index = if prevalidated {
            UniversalResolutionIndex::new_with_prevalidated_project_inventory_owned_at_inference(
                batches,
                &nodes[..inventory_node_count],
                project_edges,
                root,
                limits,
                admission,
            )
        } else {
            UniversalResolutionIndex::new_with_project_inventory_owned(
                batches,
                &nodes[..inventory_node_count],
                project_edges,
                root,
                limits,
            )
        };
        return match index {
            Ok(index) => {
                index.materialize_relationships_at_inference(nodes, edges, admission);
                UniversalResolutionReport {
                    partitions: 1,
                    compacted_declarations,
                    input,
                    retained,
                    ..UniversalResolutionReport::default()
                }
            }
            Err(error) => UniversalResolutionReport {
                degraded: true,
                partitions: 1,
                failed_partitions: 1,
                compacted_declarations,
                omitted_candidates: candidate_count,
                input,
                retained,
                reason: Some(error),
                ..UniversalResolutionReport::default()
            },
        };
    }

    let mut omitted_candidates = 0_usize;
    for batch in &mut batches {
        let before = batch.candidates.len();
        batch
            .candidates
            .retain(|candidate| candidate.constraints.exact_target_declaration_id.is_some());
        omitted_candidates =
            omitted_candidates.saturating_add(before.saturating_sub(batch.candidates.len()));
        let retained_occurrences = batch
            .candidates
            .iter()
            .filter_map(|candidate| candidate.occurrence_id.clone())
            .collect::<AHashSet<_>>();
        batch
            .occurrences
            .retain(|occurrence| retained_occurrences.contains(occurrence.id.as_str()));
        // Exact-target resolution does not consult lexical scopes or bindings.
        // Declaration definition anchors were already projected above.
        batch.scopes.clear();
        batch.bindings.clear();
    }
    let partition_limits = partition_target(limits);
    let mut queue = VecDeque::from(pack_partitions(batches, partition_limits));
    let mut report = UniversalResolutionReport {
        partitioned: true,
        degraded: true,
        compacted_declarations,
        omitted_candidates,
        input,
        retained,
        reason: Some(format!(
            "aggregate universal evidence exceeds one bounded resolver: declarations {}>{}, bindings {}>{}, occurrences {}>{}, candidates {}>{}, scopes {}>{}",
            retained.declarations,
            limits.declarations,
            retained.bindings,
            limits.bindings,
            retained.occurrences,
            limits.occurrences,
            retained.candidates,
            limits.candidates,
            retained.scopes,
            limits.candidates,
        )),
        ..UniversalResolutionReport::default()
    };

    while let Some(mut partition) = queue.pop_front() {
        let declaration_ids = partition
            .iter()
            .flat_map(|batch| batch.declarations.iter())
            .map(|declaration| declaration.id.clone())
            .collect::<BTreeSet<_>>();
        let mut omitted = 0_usize;
        for batch in &mut partition {
            let before = batch.candidates.len();
            batch.candidates.retain(|candidate| {
                declaration_ids.contains(candidate.source_declaration_id.as_str())
                    && candidate
                        .constraints
                        .exact_target_declaration_id
                        .as_deref()
                        .is_some_and(|target| declaration_ids.contains(target))
            });
            omitted = omitted.saturating_add(before.saturating_sub(batch.candidates.len()));
        }
        report.omitted_candidates = report.omitted_candidates.saturating_add(omitted);

        let retained_candidates = partition
            .iter()
            .map(|batch| batch.candidates.len())
            .sum::<usize>();
        // An aggregate-overflow partition has only local visibility. Force the
        // conservative admission profile even when the caller requested a
        // richer one, because closed-world inference would be unsound inside
        // a partial repository view. Every retained candidate carries an
        // producer-proven exact target ID.
        let partition_admission = ResolutionAdmission::Low;
        let index =
            UniversalResolutionIndex::new_with_prevalidated_project_inventory_owned_at_inference(
                partition,
                &nodes[..inventory_node_count],
                &[],
                root,
                limits,
                partition_admission,
            );
        report.partitions = report.partitions.saturating_add(1);
        match index {
            Ok(index) => {
                index.materialize_relationships_at_inference(nodes, edges, partition_admission);
            }
            Err(error) => {
                report.failed_partitions = report.failed_partitions.saturating_add(1);
                report.omitted_candidates = report
                    .omitted_candidates
                    .saturating_add(retained_candidates);
                if report.reason.as_deref().is_none_or(str::is_empty) {
                    report.reason = Some(error);
                }
            }
        }
    }
    report
}

fn validate_batches_for_projection(batches: &[SemanticEvidenceBatch]) -> Result<(), String> {
    let mut fact_ids = AHashSet::new();
    for batch in batches {
        validate_evidence(batch, EvidenceLimits::default()).map_err(|error| error.to_string())?;
        for id in batch
            .declarations
            .iter()
            .map(|fact| &fact.id)
            .chain(batch.scopes.iter().map(|fact| &fact.id))
            .chain(batch.bindings.iter().map(|fact| &fact.id))
            .chain(batch.occurrences.iter().map(|fact| &fact.id))
            .chain(batch.candidates.iter().map(|fact| &fact.id))
        {
            if !fact_ids.insert(id.as_str()) {
                return Err(format!("duplicate universal fact id `{id}` across batches"));
            }
        }
    }
    Ok(())
}

fn compact_low_inference_evidence(batches: &mut [SemanticEvidenceBatch]) -> usize {
    let mut required_details = AHashSet::<String>::new();
    let mut used_bindings = AHashSet::<String>::new();
    for candidate in batches.iter().flat_map(|batch| &batch.candidates) {
        if !matches!(
            candidate.relation,
            CandidateRelation::Contains | CandidateRelation::Owns
        ) {
            required_details.insert(candidate.source_declaration_id.clone());
            if let Some(target) = &candidate.constraints.exact_target_declaration_id {
                required_details.insert(target.clone());
            }
            if let Some(binding) = &candidate.binding_id {
                used_bindings.insert(binding.clone());
            }
            if let Some(HierarchyConstraint::RustAssociatedType {
                receiver_declaration_id,
                ..
            }) = &candidate.constraints.hierarchy
            {
                required_details.insert(receiver_declaration_id.clone());
            }
        }
    }
    for binding in batches
        .iter()
        .flat_map(|batch| &batch.bindings)
        .filter(|binding| used_bindings.contains(&binding.id))
    {
        if let Some(target) = &binding.target_declaration_id {
            required_details.insert(target.clone());
        }
    }
    // A scope owner is part of the lexical evidence contract. Preserve detail
    // declarations that own scopes so compaction cannot leave dangling scope
    // references in an otherwise valid batch.
    required_details.extend(
        batches
            .iter()
            .flat_map(|batch| &batch.scopes)
            .filter_map(|scope| scope.owner_declaration_id.clone()),
    );
    // Some producers intentionally leave lexical targets for the collection
    // resolver instead of stamping an exact declaration ID. Preserve a leaf
    // declaration whenever its spelling is used in the same source batch;
    // otherwise compaction could erase a parameter/property before lexical
    // resolution has the opportunity to prove it.
    for batch in batches.iter() {
        let referenced_spellings = batch
            .occurrences
            .iter()
            .map(|occurrence| occurrence.spelling.as_str())
            .collect::<BTreeSet<_>>();
        required_details.extend(
            batch
                .declarations
                .iter()
                .filter(|declaration| {
                    LOW_DETAIL_DECLARATION_KINDS.contains(&declaration.kind.as_str())
                        && referenced_spellings.contains(declaration.name.as_str())
                })
                .map(|declaration| declaration.id.clone()),
        );
    }

    let mut removed_ids = AHashSet::<String>::new();
    for declaration in batches.iter().flat_map(|batch| &batch.declarations) {
        if LOW_DETAIL_DECLARATION_KINDS.contains(&declaration.kind.as_str())
            && !required_details.contains(&declaration.id)
        {
            removed_ids.insert(declaration.id.clone());
        }
    }
    if removed_ids.is_empty() {
        return 0;
    }

    let mut removed_binding_ids = AHashSet::new();
    for batch in batches.iter_mut() {
        batch
            .declarations
            .retain(|declaration| !removed_ids.contains(&declaration.id));
        batch
            .occurrences
            .retain(|occurrence| !removed_ids.contains(&occurrence.owner_declaration_id));
        batch.bindings.retain(|binding| {
            let keep = binding
                .target_declaration_id
                .as_ref()
                .is_none_or(|target| !removed_ids.contains(target));
            if !keep {
                removed_binding_ids.insert(binding.id.clone());
            }
            keep
        });
    }
    for batch in batches {
        batch.candidates.retain(|candidate| {
            !removed_ids.contains(&candidate.source_declaration_id)
                && candidate
                    .constraints
                    .exact_target_declaration_id
                    .as_ref()
                    .is_none_or(|target| !removed_ids.contains(target))
                && candidate
                    .binding_id
                    .as_ref()
                    .is_none_or(|binding| !removed_binding_ids.contains(binding))
        });
    }
    removed_ids.len()
}

fn pack_partitions(
    batches: Vec<SemanticEvidenceBatch>,
    limits: UniversalResolutionLimits,
) -> Vec<Vec<SemanticEvidenceBatch>> {
    let mut partitions = Vec::new();
    let mut current = Vec::new();
    let mut counts = UniversalResolutionCounts::default();
    for batch in batches {
        let batch_counts = evidence_counts(std::slice::from_ref(&batch));
        let next = add_counts(counts, batch_counts);
        if !current.is_empty() && !next.fits(limits) {
            partitions.push(std::mem::take(&mut current));
            counts = UniversalResolutionCounts::default();
        }
        counts = add_counts(counts, batch_counts);
        current.push(batch);
    }
    if !current.is_empty() {
        partitions.push(current);
    }
    partitions
}

fn partition_target(limits: UniversalResolutionLimits) -> UniversalResolutionLimits {
    UniversalResolutionLimits {
        declarations: (limits.declarations / 2).max(1),
        bindings: (limits.bindings / 2).max(1),
        occurrences: (limits.occurrences / 2).max(1),
        candidates: (limits.candidates / 2).max(1),
        candidates_per_lookup: limits.candidates_per_lookup,
    }
}

fn evidence_counts(batches: &[SemanticEvidenceBatch]) -> UniversalResolutionCounts {
    batches
        .iter()
        .fold(UniversalResolutionCounts::default(), |counts, batch| {
            add_counts(
                counts,
                UniversalResolutionCounts {
                    declarations: batch.declarations.len(),
                    bindings: batch.bindings.len(),
                    occurrences: batch.occurrences.len(),
                    candidates: batch.candidates.len(),
                    scopes: batch.scopes.len(),
                },
            )
        })
}

fn add_counts(
    left: UniversalResolutionCounts,
    right: UniversalResolutionCounts,
) -> UniversalResolutionCounts {
    UniversalResolutionCounts {
        declarations: left.declarations.saturating_add(right.declarations),
        bindings: left.bindings.saturating_add(right.bindings),
        occurrences: left.occurrences.saturating_add(right.occurrences),
        candidates: left.candidates.saturating_add(right.candidates),
        scopes: left.scopes.saturating_add(right.scopes),
    }
}

fn batch_source_key(batch: &SemanticEvidenceBatch) -> (&str, &str) {
    let source = batch
        .declarations
        .first()
        .map(|fact| fact.range.source_file.as_str())
        .or_else(|| {
            batch
                .scopes
                .first()
                .map(|fact| fact.range.source_file.as_str())
        })
        .or_else(|| {
            batch
                .bindings
                .first()
                .map(|fact| fact.range.source_file.as_str())
        })
        .or_else(|| {
            batch
                .occurrences
                .first()
                .map(|fact| fact.range.source_file.as_str())
        })
        .unwrap_or_default();
    (source, batch.pipeline.language.as_str())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use compass_languages::Engine;
    use serde_json::{Map, Value};

    use super::*;

    #[test]
    fn low_compaction_drops_unreferenced_detail_declarations()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut extraction = Engine::default().extract_source(
            Path::new("src/example.ts"),
            b"export function run(unused: string): void { return }",
        )?;
        let mut batches = vec![
            extraction
                .semantic_evidence
                .take()
                .ok_or("missing evidence")?,
        ];
        let before = batches[0].declarations.len();

        let removed = compact_low_inference_evidence(&mut batches);

        assert!(removed > 0);
        assert!(batches[0].declarations.len() < before);
        assert!(batches[0].declarations.iter().all(|declaration| {
            declaration.kind != "parameter" || declaration.name != "unused"
        }));
        validate_evidence(&batches[0], EvidenceLimits::default())?;
        Ok(())
    }

    #[test]
    fn low_compaction_preserves_referenced_detail_declarations()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut extraction = Engine::default().extract_source(
            Path::new("src/example.ts"),
            b"export function run(used: () => void): void { used() }",
        )?;
        let mut batches = vec![
            extraction
                .semantic_evidence
                .take()
                .ok_or("missing evidence")?,
        ];

        compact_low_inference_evidence(&mut batches);

        assert!(
            batches[0].declarations.iter().any(|declaration| {
                declaration.kind == "parameter" && declaration.name == "used"
            }),
            "retained declarations: {:?}; occurrences: {:?}",
            batches[0]
                .declarations
                .iter()
                .map(|declaration| (&declaration.kind, &declaration.name))
                .collect::<Vec<_>>(),
            batches[0]
                .occurrences
                .iter()
                .map(|occurrence| &occurrence.spelling)
                .collect::<Vec<_>>()
        );
        validate_evidence(&batches[0], EvidenceLimits::default())?;
        Ok(())
    }

    #[test]
    fn partition_packing_is_source_ordered_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let mut engine = Engine::default();
        let right = engine
            .extract_source(Path::new("z.py"), b"def right():\n    pass\n")?
            .semantic_evidence
            .ok_or("missing right evidence")?;
        let left = engine
            .extract_source(Path::new("a.py"), b"def left():\n    pass\n")?
            .semantic_evidence
            .ok_or("missing left evidence")?;
        let per_batch = evidence_counts(std::slice::from_ref(&left));
        let limits = UniversalResolutionLimits {
            declarations: per_batch.declarations,
            bindings: per_batch.bindings.max(1),
            occurrences: per_batch.occurrences.max(1),
            candidates: per_batch.candidates.max(per_batch.scopes).max(1),
            candidates_per_lookup: 16,
        };
        let mut batches = vec![right, left];
        batches.sort_by(|a, b| batch_source_key(a).cmp(&batch_source_key(b)));

        let partitions = pack_partitions(batches, limits);

        assert_eq!(partitions.len(), 2);
        assert_eq!(batch_source_key(&partitions[0][0]).0, "a.py");
        assert_eq!(batch_source_key(&partitions[1][0]).0, "z.py");
        assert!(
            partitions
                .iter()
                .all(|partition| evidence_counts(partition).fits(limits))
        );
        Ok(())
    }

    #[test]
    fn aggregate_overflow_projects_all_declarations_deterministically()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut engine = Engine::default();
        let left = engine
            .extract_source(
                Path::new("a.py"),
                b"def left():\n    return 1\n\ndef call_left():\n    return left()\n",
            )?
            .semantic_evidence
            .ok_or("missing left evidence")?;
        let right = engine
            .extract_source(
                Path::new("z.py"),
                b"def right():\n    return 2\n\ndef call_right():\n    return right()\n",
            )?
            .semantic_evidence
            .ok_or("missing right evidence")?;
        let left_counts = evidence_counts(std::slice::from_ref(&left));
        let right_counts = evidence_counts(std::slice::from_ref(&right));
        let limits = UniversalResolutionLimits {
            declarations: left_counts.declarations.max(right_counts.declarations),
            bindings: left_counts.bindings.max(right_counts.bindings).max(1),
            occurrences: left_counts.occurrences.max(right_counts.occurrences).max(1),
            candidates: left_counts
                .candidates
                .max(right_counts.candidates)
                .max(left_counts.scopes)
                .max(right_counts.scopes)
                .max(1),
            candidates_per_lookup: 32,
        };
        let materialize = |batches| {
            let mut nodes = Vec::new();
            let mut edges = Vec::new();
            let report = materialize_bounded_owned(
                batches,
                &[],
                Path::new("."),
                limits,
                ResolutionAdmission::Max,
                true,
                (&mut nodes, &mut edges),
            );
            (report, nodes, edges)
        };

        let forward = materialize(vec![left.clone(), right.clone()]);
        let reverse = materialize(vec![right, left]);

        assert!(forward.0.partitioned);
        assert!(forward.0.degraded);
        assert_eq!(forward.0.partitions, 2);
        assert!(forward.1.len() >= forward.0.retained.declarations);
        assert_eq!(forward, reverse);
        Ok(())
    }

    #[test]
    fn bounded_single_envelope_matches_direct_materialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let batch = Engine::default()
            .extract_source(
                Path::new("src/example.py"),
                b"class Service:\n    def run(self):\n        return 1\n\ndef invoke():\n    service = Service()\n    return service.run()\n",
            )?
            .semantic_evidence
            .ok_or("missing evidence")?;
        let limits = UniversalResolutionLimits::default();
        let inventory_id = batch
            .declarations
            .iter()
            .find(|declaration| declaration.kind == "file")
            .map(|declaration| declaration.graph_node_id.clone())
            .filter(|id| !id.is_empty())
            .ok_or("missing file graph node id")?;
        let inventory = vec![NodeRecord {
            id: inventory_id,
            attributes: Map::from_iter([("inventory_marker".to_owned(), Value::Bool(true))]),
        }];
        let direct = UniversalResolutionIndex::new_with_project_inventory_owned(
            vec![batch.clone()],
            &inventory,
            &[],
            Path::new("."),
            limits,
        )?;
        let mut direct_nodes = inventory.clone();
        let mut direct_edges = Vec::new();
        direct.materialize(&mut direct_nodes, &mut direct_edges);
        let mut bounded_nodes = inventory;
        let mut bounded_edges = Vec::new();

        let report = materialize_bounded_owned(
            vec![batch],
            &[],
            Path::new("."),
            limits,
            ResolutionAdmission::Max,
            false,
            (&mut bounded_nodes, &mut bounded_edges),
        );

        assert!(!report.degraded);
        assert_eq!(bounded_nodes, direct_nodes);
        assert_eq!(bounded_edges, direct_edges);
        Ok(())
    }

    #[test]
    fn unvalidated_evidence_is_rejected_before_declaration_projection()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut batch = Engine::default()
            .extract_source(Path::new("src/example.py"), b"def run():\n    return 1\n")?
            .semantic_evidence
            .ok_or("missing evidence")?;
        batch.pipeline.language.clear();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        let report = materialize_bounded_owned(
            vec![batch],
            &[],
            Path::new("."),
            UniversalResolutionLimits::default(),
            ResolutionAdmission::Max,
            false,
            (&mut nodes, &mut edges),
        );

        assert!(report.degraded);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
        Ok(())
    }
}
