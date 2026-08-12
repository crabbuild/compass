use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    ChangeRequest, FindingType, Omission, PrIntelligenceError, PullRequestReport, RevisionSet,
    VerificationState, report_digest,
};

pub const PR_READINESS_SCHEMA: &str = "compass.pr-readiness/1";
pub const DOC_DRIFT_RULE_VERSION: u32 = 1;
const MAX_OWNERSHIP_RECORDS: usize = 1_000;
const MAX_READINESS_ITEMS: usize = 100_000;
const MAX_READINESS_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetState {
    Confirmed,
    Advisory,
    Unknown,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalOwnership {
    pub path: String,
    pub contributor: String,
    pub commits: u32,
    pub evidence_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessExtractionFingerprints {
    pub base: String,
    pub comparison: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignatureBodyFacet {
    pub state: FacetState,
    pub signature_finding_fingerprints: Vec<String>,
    pub body_finding_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactFacet {
    pub state: FacetState,
    pub direct_entities: Vec<String>,
    pub transitive_entities: Vec<String>,
    pub finding_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestReadinessFacet {
    pub state: FacetState,
    pub verification_states: Vec<VerificationState>,
    pub exact_tests: Vec<String>,
    pub recommended_tests: Vec<String>,
    pub gap_finding_fingerprints: Vec<String>,
    pub statement: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocDriftFacet {
    pub state: FacetState,
    pub rule_version: u32,
    pub changed_code_paths: Vec<String>,
    pub changed_documentation_paths: Vec<String>,
    pub linked_documentation_entities: Vec<String>,
    pub statement: String,
    pub advisory_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipFacet {
    pub state: FacetState,
    pub records: Vec<LocalOwnership>,
    pub statement: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessFacets {
    pub signature_body: SignatureBodyFacet,
    pub impact: ImpactFacet,
    pub tests: TestReadinessFacet,
    pub documentation_drift: DocDriftFacet,
    pub local_ownership: OwnershipFacet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PullRequestReadiness {
    pub schema: String,
    pub report_digest: String,
    pub revisions: RevisionSet,
    pub graph_schema: String,
    pub extractor_version: String,
    pub configuration_digest: String,
    pub extraction_fingerprints: ReadinessExtractionFingerprints,
    pub evidence_manifest_digest: String,
    pub facets: ReadinessFacets,
    pub missing_evidence: Vec<Omission>,
    pub readiness_digest: String,
}

impl PullRequestReadiness {
    pub fn from_json(bytes: &[u8]) -> Result<Self, PrIntelligenceError> {
        if bytes.len() > MAX_READINESS_BYTES {
            return Err(PrIntelligenceError::Limit(format!(
                "readiness input exceeds {MAX_READINESS_BYTES} bytes"
            )));
        }
        let readiness: Self = serde_json::from_slice(bytes)?;
        readiness.validate()?;
        Ok(readiness)
    }

    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        if self.schema != PR_READINESS_SCHEMA {
            return Err(PrIntelligenceError::UnsupportedSchema(self.schema.clone()));
        }
        validate_digest("referenced report", &self.report_digest)?;
        validate_digest(
            "readiness evidence manifest",
            &self.evidence_manifest_digest,
        )?;
        validate_digest("readiness", &self.readiness_digest)?;
        for (name, object_id) in [
            ("readiness merge base", &self.revisions.merge_base),
            (
                "readiness pull-request head",
                &self.revisions.pull_request_head,
            ),
            ("readiness target head", &self.revisions.target_head),
        ] {
            validate_object_id(name, object_id)?;
        }
        if let Some(object_id) = self.revisions.merge_result.object_id() {
            validate_object_id("readiness merge result", object_id)?;
        }
        for (name, value) in [
            ("readiness graph schema", &self.graph_schema),
            ("readiness extractor version", &self.extractor_version),
        ] {
            if value.is_empty() || value.len() > crate::MAX_STRING_BYTES {
                return Err(PrIntelligenceError::InvalidEvidence(format!(
                    "{name} is empty or exceeds the string bound"
                )));
            }
        }
        if self.configuration_digest.len() != 64
            || !self.configuration_digest.bytes().all(is_lower_hex)
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "readiness configuration digest is not lowercase SHA-256".to_owned(),
            ));
        }
        for (name, fingerprint) in [
            ("base", &self.extraction_fingerprints.base),
            ("comparison", &self.extraction_fingerprints.comparison),
        ] {
            if fingerprint.len() != 64 || !fingerprint.bytes().all(is_lower_hex) {
                return Err(PrIntelligenceError::InvalidEvidence(format!(
                    "readiness {name} extraction fingerprint is not lowercase SHA-256"
                )));
            }
        }
        if self.facets.documentation_drift.rule_version != DOC_DRIFT_RULE_VERSION
            || !self.facets.documentation_drift.advisory_only
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "documentation drift must use the advisory-only versioned rule".to_owned(),
            ));
        }
        if self.facets.local_ownership.records.len() > MAX_OWNERSHIP_RECORDS {
            return Err(PrIntelligenceError::Limit(format!(
                "local ownership has more than {MAX_OWNERSHIP_RECORDS} records"
            )));
        }
        for (name, values) in [
            (
                "signature findings",
                self.facets
                    .signature_body
                    .signature_finding_fingerprints
                    .as_slice(),
            ),
            (
                "body findings",
                self.facets
                    .signature_body
                    .body_finding_fingerprints
                    .as_slice(),
            ),
            (
                "direct impact",
                self.facets.impact.direct_entities.as_slice(),
            ),
            (
                "transitive impact",
                self.facets.impact.transitive_entities.as_slice(),
            ),
            (
                "impact findings",
                self.facets.impact.finding_fingerprints.as_slice(),
            ),
            ("exact tests", self.facets.tests.exact_tests.as_slice()),
            (
                "recommended tests",
                self.facets.tests.recommended_tests.as_slice(),
            ),
            (
                "test gaps",
                self.facets.tests.gap_finding_fingerprints.as_slice(),
            ),
            (
                "changed code paths",
                self.facets
                    .documentation_drift
                    .changed_code_paths
                    .as_slice(),
            ),
            (
                "changed documentation paths",
                self.facets
                    .documentation_drift
                    .changed_documentation_paths
                    .as_slice(),
            ),
            (
                "linked documentation entities",
                self.facets
                    .documentation_drift
                    .linked_documentation_entities
                    .as_slice(),
            ),
        ] {
            validate_bounded_strings(name, values)?;
        }
        for (name, statement) in [
            ("test statement", self.facets.tests.statement.as_str()),
            (
                "documentation drift statement",
                self.facets.documentation_drift.statement.as_str(),
            ),
            (
                "ownership statement",
                self.facets.local_ownership.statement.as_str(),
            ),
        ] {
            validate_bounded_string(name, statement)?;
        }
        if self.missing_evidence.len() > MAX_READINESS_ITEMS {
            return Err(PrIntelligenceError::Limit(format!(
                "readiness has more than {MAX_READINESS_ITEMS} missing-evidence records"
            )));
        }
        for omission in &self.missing_evidence {
            validate_bounded_string("omission category", &omission.category)?;
            validate_bounded_string("omission reason", &omission.reason)?;
        }
        for record in &self.facets.local_ownership.records {
            if record.path.is_empty()
                || record.contributor.is_empty()
                || record.commits == 0
                || record.path.len() > crate::MAX_STRING_BYTES
                || record.contributor.len() > crate::MAX_STRING_BYTES
            {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "local ownership contains an empty or oversized field".to_owned(),
                ));
            }
            validate_object_id("ownership evidence revision", &record.evidence_revision)?;
        }
        if !strictly_ordered(&self.facets.signature_body.signature_finding_fingerprints)
            || !strictly_ordered(&self.facets.signature_body.body_finding_fingerprints)
            || !strictly_ordered(&self.facets.impact.direct_entities)
            || !strictly_ordered(&self.facets.impact.transitive_entities)
            || !strictly_ordered(&self.facets.impact.finding_fingerprints)
            || !strictly_ordered(&self.facets.tests.exact_tests)
            || !strictly_ordered(&self.facets.tests.recommended_tests)
            || !strictly_ordered(&self.facets.tests.gap_finding_fingerprints)
            || !strictly_ordered(&self.facets.documentation_drift.changed_code_paths)
            || !strictly_ordered(&self.facets.documentation_drift.changed_documentation_paths)
            || !strictly_ordered(
                &self
                    .facets
                    .documentation_drift
                    .linked_documentation_entities,
            )
            || !strictly_ordered(&self.facets.local_ownership.records)
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "readiness evidence must be strictly ordered and deduplicated".to_owned(),
            ));
        }
        let expected = readiness_digest(self)?;
        if expected != self.readiness_digest {
            return Err(PrIntelligenceError::InvalidEvidence(format!(
                "readiness digest mismatch: expected {expected}, found {}",
                self.readiness_digest
            )));
        }
        Ok(())
    }
}

pub fn build_readiness(
    report: &PullRequestReport,
    request: &ChangeRequest,
    extraction_fingerprints: ReadinessExtractionFingerprints,
    ownership: Vec<LocalOwnership>,
    ownership_omission: Option<String>,
) -> Result<PullRequestReadiness, PrIntelligenceError> {
    report.validate()?;
    request.validate()?;
    let expected_report_digest = report_digest(report)?;
    if expected_report_digest != report.report_digest {
        return Err(PrIntelligenceError::InvalidEvidence(
            "readiness cannot reference an unsealed PR report".to_owned(),
        ));
    }
    if report.identity.revisions != request.revisions {
        return Err(PrIntelligenceError::InvalidEvidence(
            "readiness change request revisions differ from the PR report".to_owned(),
        ));
    }

    let mut signature = BTreeSet::new();
    let mut body = BTreeSet::new();
    let mut direct = BTreeSet::new();
    let mut transitive = BTreeSet::new();
    let mut impact_findings = BTreeSet::new();
    let mut verification_states = BTreeSet::new();
    let mut exact_tests = BTreeSet::new();
    let mut recommended_tests = BTreeSet::new();
    let mut gaps = BTreeSet::new();
    let mut linked_documentation = BTreeSet::new();
    for finding in &report.findings {
        match finding.finding_type {
            FindingType::ContractChange => {
                signature.insert(finding.fingerprint.clone());
            }
            FindingType::ArchitectureDelta
            | FindingType::DependencyChange
            | FindingType::StructuralChange => {
                body.insert(finding.fingerprint.clone());
            }
            FindingType::Impact | FindingType::VerificationGap => {}
        }
        direct.extend(finding.source_entities.iter().cloned());
        transitive.extend(finding.target_entities.iter().cloned());
        transitive.extend(finding.witness.iter().map(|hop| hop.target.clone()));
        if !finding.target_entities.is_empty() || !finding.witness.is_empty() {
            impact_findings.insert(finding.fingerprint.clone());
        }
        verification_states.insert(finding.verification.state);
        exact_tests.extend(finding.verification.exact_tests.iter().cloned());
        recommended_tests.extend(finding.verification.recommended_tests.iter().cloned());
        if finding.verification.gap {
            gaps.insert(finding.fingerprint.clone());
        }
        for hop in &finding.witness {
            if hop.relation == "documents" {
                linked_documentation.insert(hop.source.clone());
                linked_documentation.insert(hop.target.clone());
            }
        }
        if [
            signature.len(),
            body.len(),
            direct.len(),
            transitive.len(),
            impact_findings.len(),
            exact_tests.len(),
            recommended_tests.len(),
            gaps.len(),
            linked_documentation.len(),
        ]
        .into_iter()
        .any(|count| count > MAX_READINESS_ITEMS)
        {
            return Err(PrIntelligenceError::Limit(format!(
                "readiness evidence exceeds {MAX_READINESS_ITEMS} items in one facet"
            )));
        }
    }
    for entity in &direct {
        transitive.remove(entity);
    }

    let mut code_paths = BTreeSet::new();
    let mut doc_paths = BTreeSet::new();
    for hunk in &request.hunks {
        let path = if hunk.status == "deleted" {
            &hunk.old_path
        } else {
            &hunk.new_path
        };
        if is_documentation(path) {
            doc_paths.insert(path.clone());
        } else {
            code_paths.insert(path.clone());
        }
    }
    let (doc_state, doc_statement) = if code_paths.is_empty() {
        (
            FacetState::Confirmed,
            "no code path changed, so documentation drift is not indicated",
        )
    } else if doc_paths.is_empty() && linked_documentation.is_empty() {
        (
            FacetState::Advisory,
            "code changed without a changed documentation path; review for possible documentation drift",
        )
    } else if doc_paths.is_empty() {
        (
            FacetState::Advisory,
            "code changed without a changed documentation path and exact graph evidence links documentation entities; review those claims for drift",
        )
    } else {
        (
            FacetState::Confirmed,
            "the change request includes code and documentation paths",
        )
    };

    let mut ownership = ownership;
    ownership.sort();
    ownership.dedup();
    if ownership.len() > MAX_OWNERSHIP_RECORDS {
        return Err(PrIntelligenceError::Limit(format!(
            "local ownership has more than {MAX_OWNERSHIP_RECORDS} records"
        )));
    }
    let mut missing_evidence = report.omissions.clone();
    let ownership_state = if ownership.is_empty() {
        FacetState::Unknown
    } else {
        FacetState::Confirmed
    };
    let ownership_statement = ownership_omission.clone().unwrap_or_else(|| {
        if ownership.is_empty() {
            "local ownership evidence was unavailable".to_owned()
        } else {
            "ownership was derived from bounded local Git history at the exact pull-request head"
                .to_owned()
        }
    });
    if let Some(reason) = ownership_omission {
        missing_evidence.push(Omission {
            category: "local_ownership".to_owned(),
            count: code_paths.len().max(1),
            reason,
        });
    } else if ownership_state == FacetState::Unknown {
        missing_evidence.push(Omission {
            category: "local_ownership".to_owned(),
            count: code_paths.len().max(1),
            reason: ownership_statement.clone(),
        });
    }
    missing_evidence.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.reason.cmp(&right.reason))
    });
    missing_evidence
        .dedup_by(|left, right| left.category == right.category && left.reason == right.reason);

    let tests_state = if !gaps.is_empty() {
        FacetState::Advisory
    } else if exact_tests.is_empty() {
        FacetState::Unknown
    } else {
        FacetState::Confirmed
    };
    let tests_statement = match tests_state {
        FacetState::Confirmed => {
            "exact static evidence maps related tests; execution status is not inferred"
        }
        FacetState::Advisory => {
            "verification findings identify a gap or partial evidence; this does not claim tests were run"
        }
        FacetState::Unknown => {
            "no exact test mapping is available; readiness remains unknown rather than untested"
        }
    };
    let mut readiness = PullRequestReadiness {
        schema: PR_READINESS_SCHEMA.to_owned(),
        report_digest: report.report_digest.clone(),
        revisions: report.identity.revisions.clone(),
        graph_schema: report.identity.graph_schema.clone(),
        extractor_version: report.identity.extractor_version.clone(),
        configuration_digest: report.identity.configuration_digest.clone(),
        extraction_fingerprints,
        evidence_manifest_digest: report.identity.evidence_manifest_digest.clone(),
        facets: ReadinessFacets {
            signature_body: SignatureBodyFacet {
                state: if signature.is_empty() && body.is_empty() {
                    FacetState::Unknown
                } else {
                    FacetState::Confirmed
                },
                signature_finding_fingerprints: signature.into_iter().collect(),
                body_finding_fingerprints: body.into_iter().collect(),
            },
            impact: ImpactFacet {
                state: if impact_findings.is_empty() {
                    FacetState::Unknown
                } else {
                    FacetState::Confirmed
                },
                direct_entities: direct.into_iter().collect(),
                transitive_entities: transitive.into_iter().collect(),
                finding_fingerprints: impact_findings.into_iter().collect(),
            },
            tests: TestReadinessFacet {
                state: tests_state,
                verification_states: verification_states.into_iter().collect(),
                exact_tests: exact_tests.into_iter().collect(),
                recommended_tests: recommended_tests.into_iter().collect(),
                gap_finding_fingerprints: gaps.into_iter().collect(),
                statement: tests_statement.to_owned(),
            },
            documentation_drift: DocDriftFacet {
                state: doc_state,
                rule_version: DOC_DRIFT_RULE_VERSION,
                changed_code_paths: code_paths.into_iter().collect(),
                changed_documentation_paths: doc_paths.into_iter().collect(),
                linked_documentation_entities: linked_documentation.into_iter().collect(),
                statement: doc_statement.to_owned(),
                advisory_only: true,
            },
            local_ownership: OwnershipFacet {
                state: ownership_state,
                records: ownership,
                statement: ownership_statement,
            },
        },
        missing_evidence,
        readiness_digest: String::new(),
    };
    readiness.readiness_digest = readiness_digest(&readiness)?;
    readiness.validate()?;
    Ok(readiness)
}

pub fn readiness_digest(readiness: &PullRequestReadiness) -> Result<String, PrIntelligenceError> {
    let mut value = serde_json::to_value(readiness)?;
    let object = value.as_object_mut().ok_or_else(|| {
        PrIntelligenceError::InvalidEvidence("readiness must encode as an object".to_owned())
    })?;
    object.remove("readinessDigest");
    let bytes = canonical_value_bytes(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn canonical_value_bytes(mut value: Value) -> Result<Vec<u8>, PrIntelligenceError> {
    sort_value(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let old = std::mem::take(object);
            let mut entries = old.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut child) in entries {
                sort_value(&mut child);
                object.insert(key, child);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(sort_value),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_documentation(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("docs/")
        || lower.contains("/docs/")
        || lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".rst")
        || lower.ends_with(".adoc")
}

fn strictly_ordered<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_bounded_strings(name: &str, values: &[String]) -> Result<(), PrIntelligenceError> {
    if values.len() > MAX_READINESS_ITEMS {
        return Err(PrIntelligenceError::Limit(format!(
            "{name} has more than {MAX_READINESS_ITEMS} items"
        )));
    }
    for value in values {
        validate_bounded_string(name, value)?;
    }
    Ok(())
}

fn validate_bounded_string(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    if value.is_empty()
        || value.len() > crate::MAX_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} contains an empty, oversized, or control-bearing value"
        )));
    }
    Ok(())
}

fn validate_digest(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} digest has no sha256 prefix"
        )));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} digest is not lowercase SHA-256"
        )));
    }
    Ok(())
}

fn validate_object_id(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(is_lower_hex) {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} is not a full lowercase Git object ID"
        )));
    }
    Ok(())
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}
