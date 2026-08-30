use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use compass_semantic_diff as semantic;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::ReportIdentity;
use crate::{
    AdvisoryRisk, ChangeRequest, Confidence, EvidenceManifest, Finding, FindingType, Freshness,
    GateResult, GateState, GraphSnapshot, Location, MAX_FINDINGS, MAX_WITNESS_HOPS, MergeOutcome,
    Omission, PrIntelligenceError, PullRequestReport, REPORT_SCHEMA, RUBRIC_VERSION, RiskBand,
    RiskFactor, RiskFactorKind, VerificationPlan, VerificationState, WitnessHop,
    canonical_json_bytes, report_digest,
};

/// Build one canonical report from frozen evidence.
pub fn analyze(
    request: &ChangeRequest,
    base: &GraphSnapshot,
    result: Option<&GraphSnapshot>,
    manifest: &EvidenceManifest,
    semantic_diff: &semantic::SemanticDiffReport,
) -> Result<PullRequestReport, PrIntelligenceError> {
    validate_inputs(request, base, result, manifest, semantic_diff)?;
    let mut omissions = Vec::new();
    let converted = semantic_diff
        .findings
        .iter()
        .take(MAX_FINDINGS)
        .map(|finding| {
            convert_finding(
                request,
                manifest,
                finding,
                &semantic_diff.entity_display_names,
            )
            .map(|converted| (finding, converted))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if semantic_diff.findings.len() > MAX_FINDINGS {
        omissions.push(Omission {
            category: "findings".to_owned(),
            count: semantic_diff.findings.len() - MAX_FINDINGS,
            reason: format!("canonical finding limit is {MAX_FINDINGS}"),
        });
    }
    let factors = risk_factors(request, manifest, &converted)?;
    let mut findings = converted
        .into_iter()
        .map(|(_, finding)| finding)
        .collect::<Vec<_>>();
    findings.sort_by(|left, right| {
        left.fingerprint
            .as_bytes()
            .cmp(right.fingerprint.as_bytes())
    });
    findings.dedup_by(|left, right| left.fingerprint == right.fingerprint);

    let advisory_risk = advisory_risk(request, &factors);
    let gates = gates(request, manifest, &findings);
    let mut report = PullRequestReport {
        schema: REPORT_SCHEMA.to_owned(),
        identity: ReportIdentity {
            repository: request.repository.clone(),
            pull_request_number: request.pull_request_number,
            revisions: request.revisions.clone(),
            graph_schema: manifest.graph_schema.clone(),
            extractor_version: manifest.extractor_version.clone(),
            configuration_digest: manifest.configuration_digest.clone(),
            policy_pack_digest: manifest.policy_pack_digest.clone(),
            evidence_manifest_digest: manifest.digest.clone(),
        },
        completeness: manifest.completeness,
        findings,
        risk_factors: factors,
        advisory_risk,
        gates,
        omissions,
        report_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_owned(),
    };
    report.report_digest = report_digest(&report)?;
    report.validate()?;
    Ok(report)
}

fn validate_inputs(
    request: &ChangeRequest,
    base: &GraphSnapshot,
    result: Option<&GraphSnapshot>,
    manifest: &EvidenceManifest,
    semantic_diff: &semantic::SemanticDiffReport,
) -> Result<(), PrIntelligenceError> {
    request.validate()?;
    base.validate()?;
    if let Some(result) = result {
        result.validate()?;
    }
    manifest.validate()?;
    if base.revision != request.revisions.target_head {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "base graph revision {} does not match target head {}",
            base.revision, request.revisions.target_head
        )));
    }
    match (&request.revisions.merge_result, result) {
        (MergeOutcome::Clean { object_id }, Some(result)) if result.revision == *object_id => {}
        (MergeOutcome::Clean { object_id }, Some(result)) => {
            return Err(PrIntelligenceError::InvalidEvidence(format!(
                "result graph revision {} does not match merge result {object_id}",
                result.revision
            )));
        }
        (MergeOutcome::Clean { .. }, None) => {
            return Err(PrIntelligenceError::InvalidEvidence(
                "clean merge requires a result graph".to_owned(),
            ));
        }
        (MergeOutcome::Conflicted { .. } | MergeOutcome::Unavailable { .. }, Some(_)) => {
            return Err(PrIntelligenceError::InvalidEvidence(
                "non-clean merge must not provide a merge-result graph".to_owned(),
            ));
        }
        (MergeOutcome::Conflicted { .. } | MergeOutcome::Unavailable { .. }, None) => {}
    }
    if let Some(result) = result
        && (base.graph_schema != result.graph_schema
            || base.extractor_version != result.extractor_version
            || base.configuration_digest != result.configuration_digest)
    {
        return Err(PrIntelligenceError::InvalidEvidence(
            "base and result graph profiles are not comparable".to_owned(),
        ));
    }
    if base.graph_schema != manifest.graph_schema
        || base.extractor_version != manifest.extractor_version
        || base.configuration_digest != manifest.configuration_digest
    {
        return Err(PrIntelligenceError::InvalidEvidence(
            "evidence manifest does not match graph profile".to_owned(),
        ));
    }
    if semantic_diff.schema != semantic::REPORT_SCHEMA {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "unsupported semantic diff schema {:?}",
            semantic_diff.schema
        )));
    }
    let expected_old = &request.revisions.target_head;
    let expected_new = request
        .revisions
        .merge_result
        .object_id()
        .unwrap_or(&request.revisions.pull_request_head);
    if semantic_diff.comparison.old_commit != *expected_old
        || semantic_diff.comparison.new_commit != expected_new
    {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "semantic diff compares {}..{}, expected {}..{}",
            semantic_diff.comparison.old_commit,
            semantic_diff.comparison.new_commit,
            expected_old,
            expected_new
        )));
    }
    Ok(())
}

fn convert_finding(
    request: &ChangeRequest,
    manifest: &EvidenceManifest,
    finding: &semantic::SemanticFinding,
    entity_display_names: &BTreeMap<String, String>,
) -> Result<Finding, PrIntelligenceError> {
    let finding_type = match finding.finding_type {
        semantic::FindingType::ContractChange => FindingType::ContractChange,
        semantic::FindingType::BehaviorChange => FindingType::ArchitectureDelta,
        semantic::FindingType::DependencyChange => FindingType::DependencyChange,
        semantic::FindingType::ImpactChange => FindingType::Impact,
        semantic::FindingType::VerificationGap => FindingType::VerificationGap,
        semantic::FindingType::StructuralChange => FindingType::StructuralChange,
    };
    let confidence = map_confidence(finding.confidence);
    let witness_path = finding.witness_paths.iter().min_by(|left, right| {
        left.hops
            .len()
            .cmp(&right.hops.len())
            .then_with(|| left.cmp(right))
    });
    if witness_path.is_some_and(|path| path.hops.len() > MAX_WITNESS_HOPS) {
        return Err(PrIntelligenceError::Limit(format!(
            "shortest witness has more than {MAX_WITNESS_HOPS} hops"
        )));
    }
    let witness = witness_path
        .map(|path| {
            path.hops
                .iter()
                .map(|hop| WitnessHop {
                    source: hop.source.clone(),
                    relation: hop.relation.clone(),
                    target: hop.target.clone(),
                    confidence: map_confidence(hop.confidence),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut targets = finding
        .affected_consumers
        .iter()
        .map(|consumer| consumer.symbol_id.clone())
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    let fingerprint = finding_fingerprint(finding_type, finding, &witness, &targets)?;
    let deterministic = confidence == Confidence::Exact
        && finding.compatibility == semantic::Compatibility::ProvenBreak
        && request.revisions.merge_result.is_clean()
        && !manifest.completeness.incomplete();
    let mut locations = finding
        .evidence
        .iter()
        .filter(|evidence| !evidence.source_file.is_empty())
        .map(|evidence| Location {
            path: evidence.source_file.clone(),
            start_byte: evidence.start_byte,
            end_byte: evidence.end_byte,
        })
        .chain(
            finding
                .affected_consumers
                .iter()
                .filter(|consumer| !consumer.source_file.is_empty())
                .map(|consumer| Location {
                    path: consumer.source_file.clone(),
                    start_byte: None,
                    end_byte: None,
                }),
        )
        .collect::<Vec<_>>();
    locations.sort();
    locations.dedup();
    let verification_gap = matches!(
        finding.verification.state,
        semantic::VerificationState::Gap
            | semantic::VerificationState::Partial
            | semantic::VerificationState::Failing
            | semantic::VerificationState::NotRun
    );
    let mut exact_tests = finding.verification.exact_tests.clone();
    exact_tests.sort();
    exact_tests.dedup();
    let mut recommended_tests = finding.verification.recommended_tests.clone();
    recommended_tests.sort();
    recommended_tests.dedup();
    Ok(Finding {
        fingerprint,
        finding_type,
        classifier_version: semantic::CLASSIFIER_VERSION,
        statement: humanize_text(&finding.headline, entity_display_names),
        source_entities: vec![finding.subject.clone()],
        target_entities: targets,
        witness,
        locations,
        verification: VerificationPlan {
            state: map_verification_state(finding.verification.state),
            exact_tests,
            recommended_tests,
            gap: verification_gap,
            reason: humanize_text(&finding.verification.reason, entity_display_names),
        },
        source_revision: request
            .revisions
            .merge_result
            .object_id()
            .unwrap_or(&request.revisions.pull_request_head)
            .to_owned(),
        evidence_source: "compass-semantic-diff".to_owned(),
        evidence_digest: manifest.digest.clone(),
        confidence,
        completeness: manifest.completeness,
        freshness: Freshness::ExactHead,
        remediation: humanize_text(&finding.reviewer_action, entity_display_names),
        deterministic,
    })
}

fn humanize_text(value: &str, entity_display_names: &BTreeMap<String, String>) -> String {
    let mut replacements = entity_display_names
        .iter()
        .filter(|(identity, display_name)| identity.as_str() != display_name.as_str())
        .map(|(identity, display_name)| (identity.as_str(), display_name.as_str()))
        .collect::<Vec<_>>();
    replacements.sort_by_key(|(identity, _)| Reverse(identity.len()));

    let mut humanized = value.to_owned();
    for (identity, display_name) in replacements {
        if humanized.contains(identity) {
            humanized = humanized.replace(identity, display_name);
        }
    }
    humanized
}

fn map_verification_state(value: semantic::VerificationState) -> VerificationState {
    match value {
        semantic::VerificationState::Unknown => VerificationState::Unknown,
        semantic::VerificationState::Covered => VerificationState::Covered,
        semantic::VerificationState::Gap => VerificationState::Gap,
        semantic::VerificationState::Partial => VerificationState::Partial,
        semantic::VerificationState::Stale => VerificationState::Stale,
        semantic::VerificationState::Failing => VerificationState::Failing,
        semantic::VerificationState::NotRun => VerificationState::NotRun,
    }
}

fn finding_fingerprint(
    finding_type: FindingType,
    finding: &semantic::SemanticFinding,
    witness: &[WitnessHop],
    targets: &[String],
) -> Result<String, PrIntelligenceError> {
    let relationships = witness
        .iter()
        .map(|hop| (&hop.source, &hop.relation, &hop.target))
        .collect::<Vec<_>>();
    let scalar_evidence = stable_scalar_evidence(finding);
    let identity = json!({
        "fingerprint_schema": crate::FINDING_FINGERPRINT_SCHEMA,
        "finding_type": finding_type,
        "classifier_version": semantic::CLASSIFIER_VERSION,
        "source_entities": [&finding.subject],
        "target_entities": targets,
        "relationships": relationships,
        "scalar_evidence": scalar_evidence,
    });
    Ok(format!(
        "{}:{:x}",
        crate::FINDING_FINGERPRINT_SCHEMA,
        Sha256::digest(canonical_json_bytes(&identity)?)
    ))
}

fn stable_scalar_evidence(finding: &semantic::SemanticFinding) -> BTreeMap<&'static str, Value> {
    let mut evidence = BTreeMap::new();
    evidence.insert("compatibility", json!(finding.compatibility));
    evidence.insert("confidence", json!(finding.confidence));
    evidence.insert("public_surface", json!(finding.public_surface));
    evidence.insert("verification_state", json!(finding.verification.state));
    if let Some(before) = &finding.before {
        evidence.insert("before", scrub_locations(before.clone()));
    }
    if let Some(after) = &finding.after {
        evidence.insert("after", scrub_locations(after.clone()));
    }
    evidence
}

fn scrub_locations(mut value: Value) -> Value {
    match &mut value {
        Value::Object(object) => {
            for key in [
                "line",
                "column",
                "start_line",
                "end_line",
                "start_byte",
                "end_byte",
                "source_file",
                "timestamp",
                "generated_at",
                "observed_at",
                "duration",
                "duration_ms",
                "elapsed_ms",
            ] {
                object.remove(key);
            }
            for child in object.values_mut() {
                *child = scrub_locations(std::mem::take(child));
            }
        }
        Value::Array(values) => {
            for child in values {
                *child = scrub_locations(std::mem::take(child));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    value
}

fn map_confidence(value: semantic::Confidence) -> Confidence {
    match value {
        semantic::Confidence::Exact => Confidence::Exact,
        semantic::Confidence::Probable => Confidence::Probable,
        semantic::Confidence::Inferred => Confidence::Inferred,
        semantic::Confidence::Unknown => Confidence::Unknown,
    }
}

fn risk_factors(
    request: &ChangeRequest,
    manifest: &EvidenceManifest,
    findings: &[(&semantic::SemanticFinding, Finding)],
) -> Result<Vec<RiskFactor>, PrIntelligenceError> {
    let mut grouped = BTreeMap::<RiskFactorKind, BTreeSet<String>>::new();
    for (finding, converted) in findings {
        let fingerprint = converted.fingerprint.clone();
        if finding.public_surface && finding.finding_type == semantic::FindingType::ContractChange {
            grouped
                .entry(RiskFactorKind::PublicContractChange)
                .or_default()
                .insert(fingerprint.clone());
        }
        if !finding.affected_consumers.is_empty() {
            grouped
                .entry(RiskFactorKind::AffectedConsumer)
                .or_default()
                .insert(fingerprint.clone());
        }
        if finding
            .dependency_topology
            .as_ref()
            .is_some_and(|topology| {
                matches!(
                    (topology.source_community, topology.target_community),
                    (Some(source), Some(target)) if source != target
                )
            })
        {
            grouped
                .entry(RiskFactorKind::CrossBoundaryImpact)
                .or_default()
                .insert(fingerprint.clone());
        }
        if finding
            .dependency_topology
            .as_ref()
            .and_then(|topology| topology.participates_in_cycle)
            == Some(true)
        {
            grouped
                .entry(RiskFactorKind::Cycle)
                .or_default()
                .insert(fingerprint.clone());
        }
        if finding.confidence != semantic::Confidence::Exact {
            grouped
                .entry(RiskFactorKind::WeakConfidenceWitness)
                .or_default()
                .insert(fingerprint.clone());
        }
        if matches!(
            finding.verification.state,
            semantic::VerificationState::Gap
                | semantic::VerificationState::Partial
                | semantic::VerificationState::Failing
                | semantic::VerificationState::NotRun
        ) {
            grouped
                .entry(RiskFactorKind::VerificationGap)
                .or_default()
                .insert(fingerprint);
        }
    }
    if manifest.completeness.incomplete() {
        grouped
            .entry(RiskFactorKind::IncompleteEvidence)
            .or_default();
    }
    if matches!(
        request.revisions.merge_result,
        MergeOutcome::Conflicted { .. }
    ) {
        grouped.entry(RiskFactorKind::MergeConflict).or_default();
    }
    let mut factors = grouped
        .into_iter()
        .map(|(kind, fingerprints)| {
            let count = u16::try_from(fingerprints.len()).unwrap_or(u16::MAX).max(1);
            let (per, cap, explanation) = factor_rubric(kind);
            RiskFactor {
                kind,
                points: per.checked_mul(count).unwrap_or(cap).min(cap),
                explanation: explanation.to_owned(),
                finding_fingerprints: fingerprints.into_iter().collect(),
            }
        })
        .collect::<Vec<_>>();
    factors.sort_by_key(|factor| factor.kind);
    Ok(factors)
}

fn factor_rubric(kind: RiskFactorKind) -> (u16, u16, &'static str) {
    match kind {
        RiskFactorKind::PublicContractChange => (20, 40, "Public contracts changed"),
        RiskFactorKind::AffectedConsumer => (4, 24, "Callers or consumers are affected"),
        RiskFactorKind::CrossBoundaryImpact => (10, 20, "Dependencies cross a boundary"),
        RiskFactorKind::Cycle => (20, 20, "A dependency cycle changed"),
        RiskFactorKind::WeakConfidenceWitness => (4, 16, "Some witness evidence is not exact"),
        RiskFactorKind::VerificationGap => (12, 36, "Affected behavior lacks verification"),
        RiskFactorKind::IncompleteEvidence => (20, 20, "Evidence is incomplete"),
        RiskFactorKind::MergeConflict => (30, 30, "The synthetic merge conflicts"),
    }
}

fn advisory_risk(request: &ChangeRequest, factors: &[RiskFactor]) -> AdvisoryRisk {
    if !request.revisions.merge_result.is_clean() {
        return AdvisoryRisk {
            rubric_version: RUBRIC_VERSION,
            score: None,
            band: RiskBand::Unavailable,
            explanation: "Advisory risk is unavailable without an exact synthetic merge result"
                .to_owned(),
        };
    }
    let score = factors
        .iter()
        .fold(0_u16, |total, factor| total.saturating_add(factor.points))
        .min(100);
    let band = match score {
        0..=19 => RiskBand::Low,
        20..=44 => RiskBand::Moderate,
        45..=69 => RiskBand::High,
        _ => RiskBand::Critical,
    };
    AdvisoryRisk {
        rubric_version: RUBRIC_VERSION,
        score: Some(score),
        band,
        explanation: format!("Version {RUBRIC_VERSION} integer rubric; advisory only"),
    }
}

fn gates(
    request: &ChangeRequest,
    manifest: &EvidenceManifest,
    findings: &[Finding],
) -> Vec<GateResult> {
    let proven_breaks = findings
        .iter()
        .filter(|finding| {
            finding.finding_type == FindingType::ContractChange && finding.deterministic
        })
        .map(|finding| finding.fingerprint.clone())
        .collect::<Vec<_>>();
    let (state, statement) = if !request.revisions.merge_result.is_clean() {
        (
            GateState::Indeterminate,
            "Contract gate is indeterminate without an exact merge result",
        )
    } else if manifest.completeness.incomplete() {
        (
            GateState::Indeterminate,
            "Contract gate is indeterminate because required evidence is incomplete",
        )
    } else if proven_breaks.is_empty() {
        (
            GateState::Pass,
            "No classifier-proven exact contract break was found",
        )
    } else {
        (
            GateState::Fail,
            "Classifier-proven exact contract breaks require remediation",
        )
    };
    vec![GateResult {
        id: "proven-contract-break".to_owned(),
        rule_version: 1,
        state,
        statement: statement.to_owned(),
        finding_fingerprints: proven_breaks,
    }]
}
