use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path};

use compass_analysis::FunctionSummary;
use compass_ir::{Capability, Coverage, CoverageState, FunctionIr, ParameterIr};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::SemanticDiffError;
use crate::model::{
    AffectedConsumer, CLASSIFIER_VERSION, ChangeDirection, ChangedEntity, CollapsedGroup,
    Comparison, Compatibility, Completeness, Confidence, DependencyDelta, EntitySnapshot,
    EvidenceRef, FeatureGroup, FindingOrigin, FindingType, MAX_DIRECT_ENTITIES,
    MAX_EVIDENCE_PER_FINDING, MAX_FINDINGS, MAX_IMPACT_DEPTH, MAX_TRAVERSED_CALL_EDGES,
    REPORT_SCHEMA, SemanticDiffInput, SemanticDiffReport, SemanticFinding, SnapshotSide,
    TestEvidence, Verification, VerificationState, WitnessHop, WitnessPath,
};

pub fn compare(input: SemanticDiffInput<'_>) -> Result<SemanticDiffReport, SemanticDiffError> {
    if input.old.fingerprint != input.new.fingerprint {
        return Err(SemanticDiffError::InvalidInput(
            "realizations have different extraction profiles; rebuild NEW with --profile-from OLD"
                .to_owned(),
        ));
    }
    let mut limitations = Vec::new();
    let entities = collect_changed_entities(&input, &mut limitations)?;
    let mut findings = Vec::new();
    let mut matched_graph_nodes = BTreeSet::new();
    for entity in &entities {
        if let Some(id) = entity
            .old
            .as_ref()
            .and_then(|snapshot| snapshot.function.graph_node_id.as_ref())
        {
            matched_graph_nodes.insert(id.clone());
        }
        if let Some(id) = entity
            .new
            .as_ref()
            .and_then(|snapshot| snapshot.function.graph_node_id.as_ref())
        {
            matched_graph_nodes.insert(id.clone());
        }
        classify_entity(&input, entity, &mut findings, &mut limitations)?;
    }
    classify_graph_fallbacks(
        &input,
        &matched_graph_nodes,
        &mut findings,
        &mut limitations,
    )?;
    for dependency in input.dependency_deltas {
        if validate_logical_identity(&dependency.source).is_err()
            || validate_logical_identity(&dependency.target).is_err()
        {
            limitations.push(format!(
                "excluded unstable dependency identity {} -> {}",
                dependency.source, dependency.target
            ));
            continue;
        }
        findings.push(dependency_finding(&input, dependency)?);
    }

    for finding in &mut findings {
        limitations.extend(attach_impact(&input, finding)?);
        apply_verification(input.test_evidence.tests_for(&finding.subject), finding);
    }
    add_verification_findings(&mut findings);
    finalize_report(
        Comparison {
            old_commit: input.old.commit,
            new_commit: input.new.commit,
            fingerprint: input.old.fingerprint,
        },
        findings,
        limitations,
    )
}

fn collect_changed_entities(
    input: &SemanticDiffInput<'_>,
    limitations: &mut Vec<String>,
) -> Result<Vec<ChangedEntity>, SemanticDiffError> {
    let mut output = Vec::new();
    for delta in input.source_deltas {
        let old = load_module(
            input,
            SnapshotSide::Old,
            delta.old_path.as_deref(),
            limitations,
        )?;
        let new = load_module(
            input,
            SnapshotSide::New,
            delta.new_path.as_deref(),
            limitations,
        )?;
        let mut old_functions = old
            .map(|module| {
                module
                    .functions
                    .into_iter()
                    .map(|function| EntitySnapshot {
                        language: module.language.clone(),
                        source_file: module.source_file.clone(),
                        function,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut new_functions = new
            .map(|module| {
                module
                    .functions
                    .into_iter()
                    .map(|function| EntitySnapshot {
                        language: module.language.clone(),
                        source_file: module.source_file.clone(),
                        function,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        old_functions.sort_by(|left, right| {
            left.function
                .symbol_id
                .as_bytes()
                .cmp(right.function.symbol_id.as_bytes())
        });
        new_functions.sort_by(|left, right| {
            left.function
                .symbol_id
                .as_bytes()
                .cmp(right.function.symbol_id.as_bytes())
        });

        let mut used_new = BTreeSet::new();
        for old in old_functions {
            let exact = new_functions
                .iter()
                .position(|new| new.function.symbol_id == old.function.symbol_id);
            let named = || {
                let matches = new_functions
                    .iter()
                    .enumerate()
                    .filter(|(index, new)| {
                        !used_new.contains(index)
                            && new.language == old.language
                            && new.function.name == old.function.name
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                (matches.len() == 1).then(|| matches[0])
            };
            let matched = exact.or_else(named);
            let new = matched.map(|index| {
                used_new.insert(index);
                new_functions[index].clone()
            });
            output.push(ChangedEntity {
                old: Some(old),
                new,
                hunks: delta.hunks.clone(),
            });
        }
        for (index, new) in new_functions.into_iter().enumerate() {
            if !used_new.contains(&index) {
                output.push(ChangedEntity {
                    old: None,
                    new: Some(new),
                    hunks: delta.hunks.clone(),
                });
            }
        }
        if output.len() > MAX_DIRECT_ENTITIES {
            return Err(SemanticDiffError::LimitExceeded {
                resource: "direct_entities",
                limit: MAX_DIRECT_ENTITIES,
            });
        }
    }
    Ok(output)
}

fn load_module(
    input: &SemanticDiffInput<'_>,
    side: SnapshotSide,
    path: Option<&str>,
    limitations: &mut Vec<String>,
) -> Result<Option<compass_ir::ModuleIr>, SemanticDiffError> {
    let Some(path) = path else {
        return Ok(None);
    };
    if let Err(reason) = validate_logical_identity(path) {
        limitations.push(format!(
            "excluded unstable source identity {path}: {reason}"
        ));
        return Ok(None);
    }
    input.snapshots.module(side, path)
}

fn classify_entity(
    input: &SemanticDiffInput<'_>,
    entity: &ChangedEntity,
    findings: &mut Vec<SemanticFinding>,
    limitations: &mut Vec<String>,
) -> Result<(), SemanticDiffError> {
    match (&entity.old, &entity.new) {
        (None, Some(new)) => {
            findings.push(added_function_finding(new, &entity.hunks));
        }
        (Some(old), None) => {
            findings.push(removed_function_finding(old, &entity.hunks));
        }
        (Some(old), Some(new)) => {
            if let Some(finding) = contract_finding(old, new, &entity.hunks) {
                findings.push(finding);
            }
            let old_summary = input
                .snapshots
                .summary(SnapshotSide::Old, &old.function.symbol_id)?;
            let new_summary = input
                .snapshots
                .summary(SnapshotSide::New, &new.function.symbol_id)?;
            if let Some(finding) =
                behavior_finding(old, new, old_summary.as_ref(), new_summary.as_ref())
            {
                findings.push(finding);
            }
            if old.function.symbol_id != new.function.symbol_id
                && old.function.name != new.function.name
            {
                limitations.push(format!(
                    "probable entity alignment {} -> {}",
                    old.function.name, new.function.name
                ));
            }
        }
        (None, None) => {}
    }
    Ok(())
}

fn added_function_finding(
    snapshot: &EntitySnapshot,
    _hunks: &[compass_history::SourceHunk],
) -> SemanticFinding {
    let public = function_is_public_surface(snapshot);
    let mut finding = base_finding(
        FindingType::StructuralChange,
        &snapshot.function.symbol_id,
        FindingOrigin::Direct,
        format!("{} was added", snapshot.function.name),
        if public {
            "A new public callable is available."
        } else {
            "A new internal callable was added."
        },
        Compatibility::Compatible,
        Confidence::Exact,
        None,
        Some(contract_value(&snapshot.function)),
        snapshot_evidence(snapshot, "contracts"),
        if public {
            "Confirm the new public surface is intentional."
        } else {
            "No reviewer action is required unless this internal addition is unexpected."
        },
    );
    finding.public_surface = public;
    finding
}

fn removed_function_finding(
    snapshot: &EntitySnapshot,
    _hunks: &[compass_history::SourceHunk],
) -> SemanticFinding {
    let public = function_is_public_surface(snapshot);
    let mut finding = base_finding(
        FindingType::ContractChange,
        &snapshot.function.symbol_id,
        FindingOrigin::Direct,
        format!("{} was removed", snapshot.function.name),
        if public {
            "A public callable no longer exists and exact consumers may fail."
        } else {
            "An internal callable no longer exists."
        },
        if public {
            Compatibility::ProvenBreak
        } else {
            Compatibility::PossibleBreak
        },
        Confidence::Exact,
        Some(contract_value(&snapshot.function)),
        None,
        snapshot_evidence(snapshot, "contracts"),
        "Review affected callers and update or remove them.",
    );
    finding.public_surface = public;
    finding
}

fn contract_finding(
    old: &EntitySnapshot,
    new: &EntitySnapshot,
    _hunks: &[compass_history::SourceHunk],
) -> Option<SemanticFinding> {
    let mut changes = Vec::new();
    let mut compatibility = Compatibility::Compatible;
    let complete =
        contracts_complete(&old.function.coverage) && contracts_complete(&new.function.coverage);
    let mut confidence = if old.function.symbol_id == new.function.symbol_id {
        Confidence::Exact
    } else {
        Confidence::Probable
    };
    if !complete {
        confidence = Confidence::Unknown;
    }

    if visibility_rank(new.function.visibility) < visibility_rank(old.function.visibility) {
        changes.push("visibility was narrowed".to_owned());
        compatibility = strongest(compatibility, Compatibility::ProvenBreak);
    }
    if old.function.execution_mode != new.function.execution_mode {
        changes.push(format!(
            "execution mode changed from {:?} to {:?}",
            old.function.execution_mode, new.function.execution_mode
        ));
        compatibility = strongest(compatibility, Compatibility::ProvenBreak);
    }
    compare_parameters(
        &old.language,
        &old.function.parameters,
        &new.function.parameters,
        &mut changes,
        &mut compatibility,
    );
    let old_return = old
        .function
        .return_type
        .as_ref()
        .map(|value| value.spelling.as_str());
    let new_return = new
        .function
        .return_type
        .as_ref()
        .map(|value| value.spelling.as_str());
    if old_return != new_return {
        changes.push(format!(
            "return type changed from {} to {}",
            old_return.unwrap_or("<inferred>"),
            new_return.unwrap_or("<inferred>")
        ));
        compatibility = strongest(compatibility, Compatibility::PossibleBreak);
    }
    if changes.is_empty() {
        return None;
    }
    if !complete && matches!(compatibility, Compatibility::ProvenBreak) {
        compatibility = Compatibility::PossibleBreak;
    }
    let mut finding = base_finding(
        FindingType::ContractChange,
        &new.function.symbol_id,
        FindingOrigin::Direct,
        format!("{} contract changed", new.function.name),
        changes.join("; "),
        compatibility,
        confidence,
        Some(contract_value(&old.function)),
        Some(contract_value(&new.function)),
        merge_evidence(
            snapshot_evidence(old, "contracts"),
            snapshot_evidence(new, "contracts"),
        ),
        "Update affected callers for the new callable contract.",
    );
    finding.public_surface = function_is_public_surface(old) || function_is_public_surface(new);
    Some(finding)
}

fn compare_parameters(
    language: &str,
    old: &[ParameterIr],
    new: &[ParameterIr],
    changes: &mut Vec<String>,
    compatibility: &mut Compatibility,
) {
    for (index, old_parameter) in old.iter().enumerate() {
        let Some(new_parameter) = new.get(index) else {
            changes.push(format!("parameter {} was removed", old_parameter.name));
            *compatibility = strongest(*compatibility, Compatibility::ProvenBreak);
            continue;
        };
        if old_parameter.kind != new_parameter.kind {
            changes.push(format!("parameter {} changed kind", old_parameter.name));
            *compatibility = strongest(*compatibility, Compatibility::ProvenBreak);
        }
        if old_parameter.name != new_parameter.name {
            changes.push(format!(
                "parameter {} was renamed to {}",
                old_parameter.name, new_parameter.name
            ));
            if language == "python" {
                *compatibility = strongest(*compatibility, Compatibility::PossibleBreak);
            }
        }
        if old_parameter.default_digest != new_parameter.default_digest {
            changes.push(format!("default for {} changed", new_parameter.name));
            *compatibility = strongest(*compatibility, Compatibility::Behavioral);
        }
        if !old_parameter.required && new_parameter.required {
            changes.push(format!("parameter {} became required", new_parameter.name));
            *compatibility = strongest(*compatibility, Compatibility::ProvenBreak);
        }
    }
    for parameter in new.iter().skip(old.len()) {
        if parameter.required {
            changes.push(format!("required parameter {} was added", parameter.name));
            *compatibility = strongest(*compatibility, Compatibility::ProvenBreak);
        } else {
            changes.push(format!("optional parameter {} was added", parameter.name));
        }
    }
}

fn behavior_finding(
    old: &EntitySnapshot,
    new: &EntitySnapshot,
    old_summary: Option<&FunctionSummary>,
    new_summary: Option<&FunctionSummary>,
) -> Option<SemanticFinding> {
    if old.function.body_digest == new.function.body_digest {
        return None;
    }
    let (Some(old_summary), Some(new_summary)) = (old_summary, new_summary) else {
        return Some(base_finding(
            FindingType::BehaviorChange,
            &new.function.symbol_id,
            FindingOrigin::Direct,
            format!("{} implementation changed", new.function.name),
            "The body changed, but available Program IR cannot explain the behavior difference.",
            Compatibility::Indeterminate,
            Confidence::Unknown,
            Some(json!({"body_digest": old.function.body_digest})),
            Some(json!({"body_digest": new.function.body_digest})),
            merge_evidence(
                snapshot_evidence(old, "implementation"),
                snapshot_evidence(new, "implementation"),
            ),
            "Inspect the implementation and add stronger program evidence if this change is material.",
        ));
    };
    let before = summary_value(old_summary);
    let after = summary_value(new_summary);
    if before == after {
        return Some(base_finding(
            FindingType::BehaviorChange,
            &new.function.symbol_id,
            FindingOrigin::Direct,
            format!("{} implementation changed", new.function.name),
            "The body changed without a supported semantic-summary delta.",
            Compatibility::Indeterminate,
            Confidence::Unknown,
            Some(json!({"body_digest": old.function.body_digest})),
            Some(json!({"body_digest": new.function.body_digest})),
            snapshot_evidence(new, "implementation"),
            "Inspect the body-only change because semantic coverage is incomplete.",
        ));
    }
    let changes = describe_summary_changes(old_summary, new_summary);
    let unresolved_delta = old_summary.unresolved_calls != new_summary.unresolved_calls;
    let call_change = old_summary.resolved_calls != new_summary.resolved_calls || unresolved_delta;
    let call_resolution_complete =
        capability_complete(&old_summary.coverage, Capability::CallResolution)
            && capability_complete(&new_summary.coverage, Capability::CallResolution);
    let mut finding = base_finding(
        FindingType::BehaviorChange,
        &new.function.symbol_id,
        FindingOrigin::Direct,
        format!("{} behavior changed", new.function.name),
        changes.join("; "),
        Compatibility::Behavioral,
        if !call_change {
            Confidence::Exact
        } else if unresolved_delta {
            Confidence::Unknown
        } else if call_resolution_complete {
            Confidence::Exact
        } else {
            Confidence::Inferred
        },
        Some(before),
        Some(after),
        snapshot_evidence(new, "behavior"),
        "Review the changed calls, effects, errors, and state access.",
    );
    if call_change {
        finding.completeness.insert(
            "call_resolution".to_owned(),
            if call_resolution_complete
                && old_summary.unresolved_calls.is_empty()
                && new_summary.unresolved_calls.is_empty()
            {
                Completeness::Complete
            } else {
                Completeness::Partial
            },
        );
    }
    Some(finding)
}

fn describe_summary_changes(old: &FunctionSummary, new: &FunctionSummary) -> Vec<String> {
    let mut changes = Vec::new();
    describe_set(
        "resolved calls",
        &old.resolved_calls,
        &new.resolved_calls,
        &mut changes,
    );
    describe_set(
        "unresolved calls",
        &old.unresolved_calls,
        &new.unresolved_calls,
        &mut changes,
    );
    describe_set("reads", &old.reads, &new.reads, &mut changes);
    describe_set("writes", &old.writes, &new.writes, &mut changes);
    describe_set("effects", &old.effects, &new.effects, &mut changes);
    let old_exceptions = old
        .exceptions
        .iter()
        .map(compass_ir::ExceptionEffect::display_name)
        .collect::<Vec<_>>();
    let new_exceptions = new
        .exceptions
        .iter()
        .map(compass_ir::ExceptionEffect::display_name)
        .collect::<Vec<_>>();
    describe_set("exceptions", &old_exceptions, &new_exceptions, &mut changes);
    changes
}

fn describe_set(label: &str, old: &[String], new: &[String], output: &mut Vec<String>) {
    let old = old.iter().collect::<BTreeSet<_>>();
    let new = new.iter().collect::<BTreeSet<_>>();
    let added = new
        .difference(&old)
        .map(|value| value.as_str())
        .collect::<Vec<_>>();
    let removed = old
        .difference(&new)
        .map(|value| value.as_str())
        .collect::<Vec<_>>();
    if !added.is_empty() {
        output.push(format!("added {label}: {}", added.join(", ")));
    }
    if !removed.is_empty() {
        output.push(format!("removed {label}: {}", removed.join(", ")));
    }
}

fn dependency_finding(
    input: &SemanticDiffInput<'_>,
    delta: &DependencyDelta,
) -> Result<SemanticFinding, SemanticDiffError> {
    let direction = match delta.change {
        ChangeDirection::Added => "added",
        ChangeDirection::Removed => "removed",
    };
    let side = if delta.change == ChangeDirection::Added {
        SnapshotSide::New
    } else {
        SnapshotSide::Old
    };
    let mut evidence = delta.evidence.clone();
    let source = input.snapshots.node(side, &delta.source)?;
    let target = input.snapshots.node(side, &delta.target)?;
    evidence.extend(node_evidence(
        source.as_ref(),
        &delta.source,
        "dependencies",
    ));
    evidence.extend(node_evidence(
        target.as_ref(),
        &delta.target,
        "dependencies",
    ));
    let source_file = source
        .as_ref()
        .and_then(|node| node.attributes.get("source_file"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target_file = target
        .as_ref()
        .and_then(|node| node.attributes.get("source_file"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let routine = evidence
        .iter()
        .any(|item| is_test_source(&item.source_file))
        || matches!(delta.relation.as_str(), "uses" | "references")
        || (!source_file.is_empty() && source_file == target_file);
    let mut finding = base_finding(
        FindingType::DependencyChange,
        &format!("{}:{}:{}", delta.source, delta.relation, delta.target),
        FindingOrigin::Direct,
        format!(
            "{} dependency {} {} {}",
            delta.source, direction, delta.relation, delta.target
        ),
        format!(
            "{} {} the {} dependency on {}.",
            delta.source, direction, delta.relation, delta.target
        ),
        Compatibility::Behavioral,
        Confidence::Exact,
        (delta.change == ChangeDirection::Removed).then(
            || json!({"source": delta.source, "relation": delta.relation, "target": delta.target}),
        ),
        (delta.change == ChangeDirection::Added).then(
            || json!({"source": delta.source, "relation": delta.relation, "target": delta.target}),
        ),
        evidence,
        "Confirm the new dependency direction and affected module boundary are intentional.",
    );
    finding.routine = routine;
    Ok(finding)
}

fn classify_graph_fallbacks(
    input: &SemanticDiffInput<'_>,
    matched: &BTreeSet<String>,
    findings: &mut Vec<SemanticFinding>,
    limitations: &mut Vec<String>,
) -> Result<(), SemanticDiffError> {
    for node_id in input.changed_node_ids {
        if matched.contains(node_id) {
            continue;
        }
        if validate_logical_identity(node_id).is_err() {
            limitations.push(format!("excluded unstable graph identity {node_id}"));
            continue;
        }
        let old = input.snapshots.node(SnapshotSide::Old, node_id)?;
        let new = input.snapshots.node(SnapshotSide::New, node_id)?;
        match (&old, &new) {
            (None, Some(node)) => {
                findings.push(added_graph_node_finding(input, node, node_id));
                continue;
            }
            (Some(node), None) => {
                findings.push(removed_graph_node_finding(input, node, node_id));
                continue;
            }
            (None, None) => continue,
            (Some(_), Some(_)) => {}
        }
        let old_signature = old
            .as_ref()
            .and_then(|node| digest_attribute(node, "signature_hash"));
        let new_signature = new
            .as_ref()
            .and_then(|node| digest_attribute(node, "signature_hash"));
        let old_body = old
            .as_ref()
            .and_then(|node| digest_attribute(node, "implementation_hash"));
        let new_body = new
            .as_ref()
            .and_then(|node| digest_attribute(node, "implementation_hash"));
        if old_signature != new_signature && (old_signature.is_some() || new_signature.is_some()) {
            findings.push(base_finding(
                FindingType::ContractChange,
                node_id,
                FindingOrigin::Direct,
                format!("{node_id} signature changed"),
                "A signature digest changed without enough Program IR to prove compatibility.",
                Compatibility::Indeterminate,
                Confidence::Unknown,
                old_signature.map(|value| json!({"signature_digest": value})),
                new_signature.map(|value| json!({"signature_digest": value})),
                node_evidence(old.as_ref().or(new.as_ref()), node_id, "contracts"),
                "Inspect the source-level signature change.",
            ));
        } else if old_body != new_body && (old_body.is_some() || new_body.is_some()) {
            findings.push(base_finding(
                FindingType::BehaviorChange,
                node_id,
                FindingOrigin::Direct,
                format!("{node_id} implementation changed"),
                "An implementation digest changed without supported semantic evidence.",
                Compatibility::Indeterminate,
                Confidence::Unknown,
                old_body.map(|value| json!({"body_digest": value})),
                new_body.map(|value| json!({"body_digest": value})),
                node_evidence(old.as_ref().or(new.as_ref()), node_id, "implementation"),
                "Inspect the body-only change.",
            ));
        }
    }
    Ok(())
}

fn added_graph_node_finding(
    input: &SemanticDiffInput<'_>,
    node: &compass_model::NodeRecord,
    node_id: &str,
) -> SemanticFinding {
    let label = graph_label(node, node_id);
    let kind = graph_kind(node);
    let public = graph_public_surface_candidate(input, node, node_id);
    let mut finding = base_finding(
        FindingType::StructuralChange,
        node_id,
        FindingOrigin::Direct,
        if public {
            format!("Public-surface {kind} {label} was added")
        } else {
            format!("{kind} {label} was added")
        },
        if public {
            "A new callable or type is available from the changed source surface."
        } else {
            "A new internal graph entity was added."
        },
        Compatibility::Compatible,
        if public {
            Confidence::Probable
        } else {
            Confidence::Exact
        },
        None,
        Some(json!({"id": node_id, "label": label, "kind": kind})),
        node_evidence(Some(node), node_id, "contracts"),
        if public {
            "Confirm the new public surface and its documentation are intentional."
        } else {
            "No reviewer action is required unless this addition is unexpected."
        },
    );
    finding.public_surface = public;
    finding
}

fn removed_graph_node_finding(
    input: &SemanticDiffInput<'_>,
    node: &compass_model::NodeRecord,
    node_id: &str,
) -> SemanticFinding {
    let label = graph_label(node, node_id);
    let kind = graph_kind(node);
    let public = graph_public_surface_candidate(input, node, node_id);
    let mut finding = base_finding(
        if public {
            FindingType::ContractChange
        } else {
            FindingType::StructuralChange
        },
        node_id,
        FindingOrigin::Direct,
        if public {
            format!("Public-surface {kind} {label} was removed")
        } else {
            format!("{kind} {label} was removed")
        },
        if public {
            "A callable or type disappeared from the source surface."
        } else {
            "An internal graph entity was removed."
        },
        if public {
            Compatibility::PossibleBreak
        } else {
            Compatibility::Compatible
        },
        if public {
            Confidence::Probable
        } else {
            Confidence::Exact
        },
        Some(json!({"id": node_id, "label": label, "kind": kind})),
        None,
        node_evidence(Some(node), node_id, "contracts"),
        if public {
            "Review external callers before accepting the removed surface."
        } else {
            "No reviewer action is required unless this removal is unexpected."
        },
    );
    finding.public_surface = public;
    finding
}

fn graph_label<'a>(node: &'a compass_model::NodeRecord, fallback: &'a str) -> &'a str {
    node.attributes
        .get("label")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
}

fn graph_kind(node: &compass_model::NodeRecord) -> &str {
    node.attributes
        .get("symbol_kind")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("symbol")
}

fn graph_public_surface_candidate(
    input: &SemanticDiffInput<'_>,
    node: &compass_model::NodeRecord,
    node_id: &str,
) -> bool {
    let kind = graph_kind(node);
    if !matches!(kind, "class" | "function" | "method" | "interface" | "type") {
        return false;
    }
    let label = graph_label(node, "");
    if label.is_empty() || label.starts_with('_') {
        return false;
    }
    let source = node
        .attributes
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let non_test = !source.starts_with("tests/")
        && !source.contains("/tests/")
        && !source
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("test_") || name.ends_with("_test.py"));
    if !non_test {
        return false;
    }
    !is_internal_source(&source)
        || input.dependency_deltas.iter().any(|dependency| {
            dependency.target == node_id
                && matches!(dependency.relation.as_str(), "imports" | "imports_from")
                && is_public_facade_identity(&dependency.source)
        })
}

fn function_is_public_surface(snapshot: &EntitySnapshot) -> bool {
    if snapshot.function.visibility != compass_ir::Visibility::Public
        || snapshot.function.is_test
        || is_test_source(&snapshot.source_file)
        || is_test_source(&snapshot.function.anchor.source_file)
        || is_internal_source(&snapshot.source_file)
        || is_internal_source(&snapshot.function.anchor.source_file)
    {
        return false;
    }
    snapshot
        .function
        .name
        .split(['.', ':', '#'])
        .filter(|component| !component.is_empty())
        .all(|component| !component.starts_with('_'))
}

fn is_internal_source(source: &str) -> bool {
    source
        .replace('\\', "/")
        .split('/')
        .any(|component| component.starts_with('_') && component != "__init__.py")
}

fn is_public_facade_identity(identity: &str) -> bool {
    let identity = identity.to_ascii_lowercase();
    ["_api", "__init__", "_lib", "_prelude"]
        .iter()
        .any(|suffix| identity.ends_with(suffix))
}

fn attach_impact(
    input: &SemanticDiffInput<'_>,
    finding: &mut SemanticFinding,
) -> Result<Vec<String>, SemanticDiffError> {
    if !matches!(
        finding.compatibility,
        Compatibility::ProvenBreak
            | Compatibility::PossibleBreak
            | Compatibility::Behavioral
            | Compatibility::Indeterminate
    ) || finding.finding_type == FindingType::DependencyChange
    {
        return Ok(Vec::new());
    }
    let mut limitations = Vec::new();
    let side = if finding.after.is_some() {
        SnapshotSide::New
    } else {
        SnapshotSide::Old
    };
    let mut queue = VecDeque::from([(finding.subject.clone(), 0_u8, Vec::<WitnessHop>::new())]);
    let mut visited = BTreeSet::from([finding.subject.clone()]);
    let mut traversed = 0_usize;
    while let Some((symbol, distance, path)) = queue.pop_front() {
        let callers = input.snapshots.reverse_callers(side, &symbol)?;
        if distance >= MAX_IMPACT_DEPTH {
            if !callers.is_empty() {
                limitations.push(format!(
                    "affected-consumer mapping for {} was truncated at depth {}",
                    finding.subject, MAX_IMPACT_DEPTH
                ));
            }
            continue;
        }
        for caller in callers {
            traversed += 1;
            if traversed > MAX_TRAVERSED_CALL_EDGES {
                limitations.push(format!(
                    "affected-consumer mapping for {} was truncated after {} call edges",
                    finding.subject, MAX_TRAVERSED_CALL_EDGES
                ));
                return Ok(limitations);
            }
            if !visited.insert(caller.clone()) {
                continue;
            }
            let mut witness = path.clone();
            witness.push(WitnessHop {
                source: caller.clone(),
                relation: "calls".to_owned(),
                target: symbol.clone(),
                confidence: Confidence::Exact,
            });
            let next_distance = distance.saturating_add(1);
            let caller_function = input.snapshots.function(side, &caller)?;
            finding.affected_consumers.push(AffectedConsumer {
                symbol_id: caller.clone(),
                display_name: caller_function
                    .as_ref()
                    .map(|function| function.name.clone())
                    .unwrap_or_else(|| caller.clone()),
                source_file: caller_function
                    .as_ref()
                    .map(|function| function.anchor.source_file.clone())
                    .unwrap_or_default(),
                distance: next_distance,
            });
            finding.witness_paths.push(WitnessPath {
                consumer: caller.clone(),
                confidence: Confidence::Exact,
                hops: witness.clone(),
            });
            queue.push_back((caller, next_distance, witness));
        }
    }
    Ok(limitations)
}

fn apply_verification(evidence: TestEvidence, finding: &mut SemanticFinding) {
    finding
        .completeness
        .insert("test_mapping".to_owned(), evidence.completeness);
    finding.verification = match evidence.completeness {
        Completeness::Complete if evidence.exact_tests.is_empty() => Verification {
            state: VerificationState::Gap,
            exact_tests: Vec::new(),
            recommended_tests: evidence.suggested_tests,
            reason: "complete test evidence maps no test to this change".to_owned(),
        },
        Completeness::Complete => Verification {
            state: VerificationState::Covered,
            exact_tests: evidence.exact_tests,
            recommended_tests: evidence.suggested_tests,
            reason: "exact current tests cover this subject".to_owned(),
        },
        Completeness::Partial => Verification {
            state: VerificationState::Partial,
            exact_tests: evidence.exact_tests,
            recommended_tests: evidence.suggested_tests,
            reason: "test mapping is incomplete and cannot prove a gap".to_owned(),
        },
        Completeness::Unavailable => Verification {
            state: VerificationState::Unknown,
            exact_tests: evidence.exact_tests,
            recommended_tests: evidence.suggested_tests,
            reason: "test evidence is unavailable".to_owned(),
        },
    };
}

fn add_verification_findings(findings: &mut Vec<SemanticFinding>) {
    let gaps = findings
        .iter()
        .filter(|finding| finding.verification.state == VerificationState::Gap)
        .map(|finding| {
            base_finding(
                FindingType::VerificationGap,
                &finding.subject,
                FindingOrigin::Derived,
                format!("{} has no mapped test", finding.headline),
                "Complete test evidence maps no test to this semantic change.",
                Compatibility::NotApplicable,
                Confidence::Exact,
                None,
                None,
                finding.evidence.clone(),
                "Add a focused test for the changed behavior or contract.",
            )
        })
        .collect::<Vec<_>>();
    findings.extend(gaps);
}

fn finalize_report(
    comparison: Comparison,
    mut findings: Vec<SemanticFinding>,
    mut limitations: Vec<String>,
) -> Result<SemanticDiffReport, SemanticDiffError> {
    if findings.len() > MAX_FINDINGS {
        return Err(SemanticDiffError::LimitExceeded {
            resource: "findings",
            limit: MAX_FINDINGS,
        });
    }
    for finding in &mut findings {
        finding.evidence.sort();
        finding.evidence.dedup();
        if finding.evidence.len() > MAX_EVIDENCE_PER_FINDING {
            return Err(SemanticDiffError::LimitExceeded {
                resource: "evidence_per_finding",
                limit: MAX_EVIDENCE_PER_FINDING,
            });
        }
        finding.affected_consumers.sort();
        finding.affected_consumers.dedup();
        finding.witness_paths.sort();
        finding.witness_paths.dedup();
        finding.review_priority = review_priority(finding);
        let relationships = finding
            .witness_paths
            .iter()
            .flat_map(|path| &path.hops)
            .map(|hop| format!("{}:{}:{}", hop.source, hop.relation, hop.target))
            .collect::<Vec<_>>();
        finding.id = finding_id(
            finding.finding_type,
            &finding.subject,
            finding.before.as_ref(),
            finding.after.as_ref(),
            CLASSIFIER_VERSION,
            &relationships,
        )?;
    }
    findings.sort_by(|left, right| {
        Reverse(left.review_priority)
            .cmp(&Reverse(right.review_priority))
            .then_with(|| confidence_rank(left.confidence).cmp(&confidence_rank(right.confidence)))
            .then_with(|| {
                Reverse(left.affected_consumers.len()).cmp(&Reverse(right.affected_consumers.len()))
            })
            .then_with(|| left.subject.as_bytes().cmp(right.subject.as_bytes()))
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });
    let feature_groups = build_feature_groups(&findings)?;
    let routine_symbol_ids = findings
        .iter()
        .filter(|finding| {
            finding.finding_type == FindingType::StructuralChange
                && finding.compatibility == Compatibility::Compatible
                && !finding.public_surface
                && finding.affected_consumers.is_empty()
        })
        .map(|finding| finding.id.clone())
        .collect::<Vec<_>>();
    let routine_test_ids = findings
        .iter()
        .filter(|finding| {
            finding.routine
                && finding
                    .evidence
                    .iter()
                    .any(|evidence| is_test_source(&evidence.source_file))
        })
        .map(|finding| finding.id.clone())
        .collect::<Vec<_>>();
    let routine_dependency_ids = findings
        .iter()
        .filter(|finding| {
            finding.routine
                && finding.finding_type == FindingType::DependencyChange
                && !routine_test_ids.contains(&finding.id)
        })
        .map(|finding| finding.id.clone())
        .collect::<Vec<_>>();
    let mut collapsed_groups = Vec::new();
    if !routine_symbol_ids.is_empty() {
        collapsed_groups.push(CollapsedGroup {
            label: "routine symbol churn".to_owned(),
            count: routine_symbol_ids.len(),
            finding_ids: routine_symbol_ids,
        });
    }
    if !routine_dependency_ids.is_empty() {
        collapsed_groups.push(CollapsedGroup {
            label: "internal dependency churn".to_owned(),
            count: routine_dependency_ids.len(),
            finding_ids: routine_dependency_ids,
        });
    }
    if !routine_test_ids.is_empty() {
        collapsed_groups.push(CollapsedGroup {
            label: "test dependency churn".to_owned(),
            count: routine_test_ids.len(),
            finding_ids: routine_test_ids,
        });
    }
    limitations.sort();
    limitations.dedup();
    let test_mapping = findings
        .iter()
        .map(|finding| {
            finding
                .completeness
                .get("test_mapping")
                .copied()
                .unwrap_or(Completeness::Unavailable)
        })
        .reduce(least_complete)
        .unwrap_or(Completeness::Unavailable);
    let call_resolution = findings
        .iter()
        .map(|finding| {
            finding
                .completeness
                .get("call_resolution")
                .copied()
                .unwrap_or(Completeness::Unavailable)
        })
        .reduce(least_complete)
        .unwrap_or(Completeness::Unavailable);
    Ok(SemanticDiffReport {
        schema: REPORT_SCHEMA.to_owned(),
        comparison,
        findings,
        feature_groups,
        collapsed_groups,
        completeness: BTreeMap::from([
            ("identity".to_owned(), Completeness::Complete),
            ("source_delta".to_owned(), Completeness::Complete),
            ("call_resolution".to_owned(), call_resolution),
            ("test_mapping".to_owned(), test_mapping),
        ]),
        limitations,
    })
}

fn least_complete(left: Completeness, right: Completeness) -> Completeness {
    match (left, right) {
        (Completeness::Unavailable, _) | (_, Completeness::Unavailable) => {
            Completeness::Unavailable
        }
        (Completeness::Partial, _) | (_, Completeness::Partial) => Completeness::Partial,
        (Completeness::Complete, Completeness::Complete) => Completeness::Complete,
    }
}

pub fn finding_id(
    finding_type: FindingType,
    subject: &str,
    before: Option<&Value>,
    after: Option<&Value>,
    classifier_version: u32,
    relationship_identities: &[String],
) -> Result<String, SemanticDiffError> {
    let mut relationships = relationship_identities.to_vec();
    relationships.sort();
    relationships.dedup();
    let bytes = serde_json::to_vec(&(
        REPORT_SCHEMA,
        classifier_version,
        finding_type,
        subject,
        before,
        after,
        relationships,
    ))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sd1-{}", hex(&digest[..12])))
}

fn build_feature_groups(
    findings: &[SemanticFinding],
) -> Result<Vec<FeatureGroup>, SemanticDiffError> {
    let mut parent = (0..findings.len()).collect::<Vec<_>>();
    let mut owner = BTreeMap::<String, usize>::new();
    for (index, finding) in findings.iter().enumerate() {
        let files = finding
            .evidence
            .iter()
            .map(|evidence| evidence.source_file.replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .collect::<BTreeSet<_>>();
        for file in files {
            if let Some(other) = owner.get(&file).copied() {
                union_groups(&mut parent, index, other);
            } else {
                owner.insert(file, index);
            }
        }
    }
    let mut groups = BTreeMap::<usize, Vec<&SemanticFinding>>::new();
    for (index, finding) in findings.iter().enumerate() {
        let root = find_group(&mut parent, index);
        groups.entry(root).or_default().push(finding);
    }
    let mut output = Vec::new();
    for mut members in groups.into_values() {
        members.sort_by(|left, right| {
            Reverse(left.review_priority)
                .cmp(&Reverse(right.review_priority))
                .then_with(|| left.id.cmp(&right.id))
        });
        let finding_ids = members
            .iter()
            .map(|finding| finding.id.clone())
            .collect::<Vec<_>>();
        let source_files = members
            .iter()
            .flat_map(|finding| &finding.evidence)
            .map(|evidence| evidence.source_file.replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let public_surface_changes = members
            .iter()
            .filter(|finding| finding.public_surface)
            .count();
        let behavior_changes = members
            .iter()
            .filter(|finding| finding.finding_type == FindingType::BehaviorChange)
            .count();
        let dependency_changes = members
            .iter()
            .filter(|finding| finding.finding_type == FindingType::DependencyChange)
            .count();
        let test_changes = members
            .iter()
            .filter(|finding| {
                finding
                    .evidence
                    .iter()
                    .any(|evidence| is_test_source(&evidence.source_file))
            })
            .count();
        let anchor = members
            .iter()
            .find(|finding| finding.public_surface)
            .copied()
            .unwrap_or(members[0]);
        let headline = if members.len() == 1 {
            anchor.headline.clone()
        } else {
            format!("{} and related changes", anchor.headline)
        };
        let summary = format!(
            "{public_surface_changes} public-surface, {behavior_changes} behavior, \
             {dependency_changes} dependency, and {test_changes} test-related changes across {} files.",
            source_files.len()
        );
        let digest = Sha256::digest(serde_json::to_vec(&finding_ids)?);
        output.push(FeatureGroup {
            id: format!("sg1-{}", hex(&digest[..12])),
            headline,
            summary,
            finding_ids,
            source_files,
            public_surface_changes,
            behavior_changes,
            dependency_changes,
            test_changes,
        });
    }
    output.sort_by(|left, right| {
        Reverse(left.public_surface_changes)
            .cmp(&Reverse(right.public_surface_changes))
            .then_with(|| Reverse(left.behavior_changes).cmp(&Reverse(right.behavior_changes)))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(output)
}

fn find_group(parent: &mut [usize], index: usize) -> usize {
    if parent[index] != index {
        parent[index] = find_group(parent, parent[index]);
    }
    parent[index]
}

fn union_groups(parent: &mut [usize], left: usize, right: usize) {
    let left = find_group(parent, left);
    let right = find_group(parent, right);
    if left != right {
        let (first, second) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        parent[second] = first;
    }
}

fn is_test_source(source: &str) -> bool {
    let path = source.replace('\\', "/").to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    path.starts_with("tests/")
        || path.contains("/tests/")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
}

#[allow(clippy::too_many_arguments)]
fn base_finding(
    finding_type: FindingType,
    subject: &str,
    origin: FindingOrigin,
    headline: impl Into<String>,
    explanation: impl Into<String>,
    compatibility: Compatibility,
    confidence: Confidence,
    before: Option<Value>,
    after: Option<Value>,
    evidence: Vec<EvidenceRef>,
    reviewer_action: impl Into<String>,
) -> SemanticFinding {
    SemanticFinding {
        id: String::new(),
        finding_type,
        subject: subject.to_owned(),
        origin,
        headline: headline.into(),
        explanation: explanation.into(),
        compatibility,
        confidence,
        review_priority: 0,
        public_surface: false,
        routine: false,
        before,
        after,
        affected_consumers: Vec::new(),
        witness_paths: Vec::new(),
        verification: Verification {
            state: VerificationState::Unknown,
            exact_tests: Vec::new(),
            recommended_tests: Vec::new(),
            reason: "verification has not been evaluated".to_owned(),
        },
        reviewer_action: reviewer_action.into(),
        evidence,
        completeness: BTreeMap::from([
            ("signature".to_owned(), Completeness::Complete),
            ("implementation".to_owned(), Completeness::Partial),
            ("call_resolution".to_owned(), Completeness::Partial),
            ("effects".to_owned(), Completeness::Partial),
            ("test_mapping".to_owned(), Completeness::Unavailable),
        ]),
    }
}

fn snapshot_evidence(snapshot: &EntitySnapshot, capability: &str) -> Vec<EvidenceRef> {
    vec![EvidenceRef {
        source_file: snapshot.source_file.clone(),
        start_byte: Some(snapshot.function.anchor.start_byte),
        end_byte: Some(snapshot.function.anchor.end_byte),
        record_key: Some(snapshot.function.symbol_id.clone()),
        capability: capability.to_owned(),
    }]
}

fn node_evidence(
    node: Option<&compass_model::NodeRecord>,
    node_id: &str,
    capability: &str,
) -> Vec<EvidenceRef> {
    let source_file = node
        .and_then(|node| node.attributes.get("source_file"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    vec![EvidenceRef {
        source_file,
        start_byte: None,
        end_byte: None,
        record_key: Some(node_id.to_owned()),
        capability: capability.to_owned(),
    }]
}

fn merge_evidence(mut first: Vec<EvidenceRef>, second: Vec<EvidenceRef>) -> Vec<EvidenceRef> {
    first.extend(second);
    first
}

fn contract_value(function: &FunctionIr) -> Value {
    json!({
        "name": function.name,
        "visibility": function.visibility,
        "execution_mode": function.execution_mode,
        "parameters": function.parameters,
        "return_type": function.return_type,
    })
}

fn summary_value(summary: &FunctionSummary) -> Value {
    json!({
        "resolved_calls": summary.resolved_calls,
        "unresolved_calls": summary.unresolved_calls,
        "reads": summary.reads,
        "writes": summary.writes,
        "effects": summary.effects,
        "exceptions": summary.exceptions,
    })
}

fn contracts_complete(coverage: &Coverage) -> bool {
    capability_complete(coverage, Capability::Contracts)
}

fn capability_complete(coverage: &Coverage, capability: Capability) -> bool {
    matches!(coverage.get(&capability), Some(CoverageState::Complete))
}

fn visibility_rank(visibility: compass_ir::Visibility) -> u8 {
    match visibility {
        compass_ir::Visibility::Public => 4,
        compass_ir::Visibility::Protected => 3,
        compass_ir::Visibility::Internal => 2,
        compass_ir::Visibility::Private => 1,
        compass_ir::Visibility::Unknown => 0,
    }
}

fn strongest(left: Compatibility, right: Compatibility) -> Compatibility {
    if compatibility_rank(right) > compatibility_rank(left) {
        right
    } else {
        left
    }
}

fn compatibility_rank(compatibility: Compatibility) -> u8 {
    match compatibility {
        Compatibility::ProvenBreak => 6,
        Compatibility::PossibleBreak => 5,
        Compatibility::Indeterminate => 4,
        Compatibility::Behavioral => 3,
        Compatibility::Compatible => 2,
        Compatibility::NotApplicable => 1,
    }
}

fn review_priority(finding: &SemanticFinding) -> u16 {
    let base = match finding.compatibility {
        Compatibility::ProvenBreak => 1_000,
        Compatibility::PossibleBreak => 900,
        Compatibility::Indeterminate => 750,
        Compatibility::Behavioral => 700,
        Compatibility::Compatible => 300,
        Compatibility::NotApplicable => 200,
    };
    let consumers = u16::try_from(finding.affected_consumers.len())
        .unwrap_or(u16::MAX)
        .min(100);
    base + consumers
}

fn confidence_rank(confidence: Confidence) -> u8 {
    match confidence {
        Confidence::Exact => 0,
        Confidence::Probable => 1,
        Confidence::Inferred => 2,
        Confidence::Unknown => 3,
    }
}

fn digest_attribute<'a>(node: &'a compass_model::NodeRecord, key: &str) -> Option<&'a str> {
    node.attributes.get(key).and_then(Value::as_str)
}

fn validate_logical_identity(value: &str) -> Result<(), &'static str> {
    if value.contains("git_compass_tmp_worktree_")
        || value.contains(".git/compass/tmp")
        || value.contains("worktree-")
    {
        return Err("temporary worktree path");
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("non-logical path");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
