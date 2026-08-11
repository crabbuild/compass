use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    MAX_CHANGE_HUNKS, MAX_ENTITIES_PER_FINDING, MAX_EVIDENCE_REPOSITORIES, MAX_EVIDENCE_SOURCES,
    MAX_FINDINGS, MAX_LOCATIONS_PER_FINDING, MAX_STRING_BYTES, MAX_TESTS_PER_FINDING,
    MAX_WITNESS_HOPS, PrIntelligenceError,
};

pub const REPORT_SCHEMA: &str = "compass.pr_intelligence.report/1";
pub const RUBRIC_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryIdentity {
    pub forge: String,
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepositoryIdentity {
    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        for (name, value) in [
            ("repository forge", &self.forge),
            ("repository host", &self.host),
            ("repository owner", &self.owner),
            ("repository name", &self.name),
        ] {
            validate_string(name, value)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn canonical_name(&self) -> String {
        format!(
            "{}://{}/{}/{}",
            self.forge, self.host, self.owner, self.name
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum MergeOutcome {
    Clean { object_id: String },
    Conflicted { evidence_digest: String },
    Unavailable { reason: String },
}

impl MergeOutcome {
    #[must_use]
    pub fn object_id(&self) -> Option<&str> {
        match self {
            Self::Clean { object_id } => Some(object_id),
            Self::Conflicted { .. } | Self::Unavailable { .. } => None,
        }
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean { .. })
    }

    fn validate(&self) -> Result<(), PrIntelligenceError> {
        match self {
            Self::Clean { object_id } => validate_object_id("merge result", object_id),
            Self::Conflicted { evidence_digest } => {
                validate_digest("merge conflict evidence", evidence_digest)
            }
            Self::Unavailable { reason } => validate_string("merge unavailable reason", reason),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionSet {
    pub merge_base: String,
    pub pull_request_head: String,
    pub target_head: String,
    pub merge_result: MergeOutcome,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRange {
    pub start_line: u64,
    pub line_count: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeHunk {
    pub old_path: String,
    pub new_path: String,
    pub status: String,
    pub old: SourceRange,
    pub new: SourceRange,
    pub patch_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequest {
    pub repository: RepositoryIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request_number: Option<u64>,
    pub revisions: RevisionSet,
    pub hunks: Vec<ChangeHunk>,
}

impl ChangeRequest {
    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        self.repository.validate()?;
        if self.pull_request_number == Some(0) {
            return Err(PrIntelligenceError::InvalidEvidence(
                "pull request number must be positive".to_owned(),
            ));
        }
        validate_object_id("merge base", &self.revisions.merge_base)?;
        validate_object_id("pull request head", &self.revisions.pull_request_head)?;
        validate_object_id("target head", &self.revisions.target_head)?;
        self.revisions.merge_result.validate()?;
        validate_count("change hunks", self.hunks.len(), MAX_CHANGE_HUNKS)?;
        validate_strict_order("change hunks", &self.hunks)?;
        for hunk in &self.hunks {
            validate_string("old hunk path", &hunk.old_path)?;
            validate_string("new hunk path", &hunk.new_path)?;
            if !matches!(
                hunk.status.as_str(),
                "added" | "deleted" | "modified" | "renamed"
            ) {
                return Err(PrIntelligenceError::InvalidEvidence(format!(
                    "unsupported hunk status {:?}",
                    hunk.status
                )));
            }
            validate_digest("hunk patch", &hunk.patch_digest)?;
            hunk.old.validate("old hunk range")?;
            hunk.new.validate("new hunk range")?;
        }
        Ok(())
    }
}

impl SourceRange {
    fn validate(&self, name: &str) -> Result<(), PrIntelligenceError> {
        self.start_line
            .checked_add(self.line_count)
            .ok_or_else(|| {
                PrIntelligenceError::InvalidEvidence(format!(
                    "{name} exceeds the line-number range"
                ))
            })?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    LocalExact,
    DownstreamComplete,
    DownstreamPartial,
    DownstreamUnavailable,
}

impl Completeness {
    #[must_use]
    pub fn incomplete(self) -> bool {
        matches!(self, Self::DownstreamPartial | Self::DownstreamUnavailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    ExactHead,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRepository {
    pub repository: RepositoryIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_revision: Option<String>,
    pub observed_head: String,
    pub freshness: Freshness,
    pub authorized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSource {
    pub kind: String,
    pub identity: String,
    pub digest: String,
    pub completeness: Completeness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceManifest {
    pub digest: String,
    pub graph_schema: String,
    pub extractor_version: String,
    pub configuration_digest: String,
    pub policy_pack_digest: String,
    pub completeness: Completeness,
    pub repositories: Vec<EvidenceRepository>,
    pub sources: Vec<EvidenceSource>,
}

impl EvidenceManifest {
    pub fn seal(mut self) -> Result<Self, PrIntelligenceError> {
        self.repositories.sort();
        self.sources.sort();
        self.digest = crate::evidence_manifest_digest(&self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        validate_count(
            "evidence repositories",
            self.repositories.len(),
            MAX_EVIDENCE_REPOSITORIES,
        )?;
        validate_count("evidence sources", self.sources.len(), MAX_EVIDENCE_SOURCES)?;
        validate_strict_order("evidence repositories", &self.repositories)?;
        validate_strict_order("evidence sources", &self.sources)?;
        for (name, value) in [
            ("graph schema", &self.graph_schema),
            ("extractor version", &self.extractor_version),
        ] {
            validate_string(name, value)?;
        }
        validate_lower_hex("configuration digest", &self.configuration_digest, 64)?;
        validate_digest("policy pack", &self.policy_pack_digest)?;
        for repository in &self.repositories {
            repository.repository.validate()?;
            validate_object_id("observed repository head", &repository.observed_head)?;
            if let Some(revision) = &repository.graph_revision {
                validate_object_id("repository graph revision", revision)?;
            }
            if let Some(failure) = &repository.failure {
                validate_string("repository evidence failure", failure)?;
            }
        }
        for source in &self.sources {
            validate_string("evidence source kind", &source.kind)?;
            validate_string("evidence source identity", &source.identity)?;
            validate_digest("evidence source", &source.digest)?;
        }
        validate_digest("evidence manifest", &self.digest)?;
        let expected = crate::evidence_manifest_digest(self)?;
        if expected != self.digest {
            return Err(PrIntelligenceError::InvalidEvidence(format!(
                "evidence manifest digest mismatch: expected {expected}, found {}",
                self.digest
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshot {
    pub revision: String,
    pub realization: String,
    pub graph_schema: String,
    pub extractor_version: String,
    pub configuration_digest: String,
}

impl GraphSnapshot {
    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        validate_object_id("graph snapshot revision", &self.revision)?;
        for (name, value) in [
            ("graph realization", &self.realization),
            ("graph schema", &self.graph_schema),
            ("extractor version", &self.extractor_version),
        ] {
            validate_string(name, value)?;
        }
        validate_lower_hex("graph configuration digest", &self.configuration_digest, 64)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Probable,
    Inferred,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingType {
    ArchitectureDelta,
    ContractChange,
    Impact,
    VerificationGap,
    DependencyChange,
    StructuralChange,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessHop {
    pub source: String,
    pub relation: String,
    pub target: String,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPlan {
    pub state: VerificationState,
    pub exact_tests: Vec<String>,
    pub recommended_tests: Vec<String>,
    pub gap: bool,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    Unknown,
    Covered,
    Gap,
    Partial,
    Stale,
    Failing,
    NotRun,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub fingerprint: String,
    pub finding_type: FindingType,
    pub classifier_version: u32,
    pub statement: String,
    pub source_entities: Vec<String>,
    pub target_entities: Vec<String>,
    pub witness: Vec<WitnessHop>,
    pub locations: Vec<Location>,
    pub verification: VerificationPlan,
    pub source_revision: String,
    pub evidence_source: String,
    pub evidence_digest: String,
    pub confidence: Confidence,
    pub completeness: Completeness,
    pub freshness: Freshness,
    pub remediation: String,
    pub deterministic: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskFactorKind {
    PublicContractChange,
    AffectedConsumer,
    CrossBoundaryImpact,
    Cycle,
    WeakConfidenceWitness,
    VerificationGap,
    IncompleteEvidence,
    MergeConflict,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskFactor {
    pub kind: RiskFactorKind,
    pub points: u16,
    pub explanation: String,
    pub finding_fingerprints: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskBand {
    Low,
    Moderate,
    High,
    Critical,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryRisk {
    pub rubric_version: u32,
    pub score: Option<u16>,
    pub band: RiskBand,
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    Pass,
    Fail,
    Indeterminate,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateResult {
    pub id: String,
    pub rule_version: u32,
    pub state: GateState,
    pub statement: String,
    pub finding_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Omission {
    pub category: String,
    pub count: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportIdentity {
    pub repository: RepositoryIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request_number: Option<u64>,
    pub revisions: RevisionSet,
    pub graph_schema: String,
    pub extractor_version: String,
    pub configuration_digest: String,
    pub policy_pack_digest: String,
    pub evidence_manifest_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestReport {
    pub schema: String,
    pub identity: ReportIdentity,
    pub completeness: Completeness,
    pub findings: Vec<Finding>,
    pub risk_factors: Vec<RiskFactor>,
    pub advisory_risk: AdvisoryRisk,
    pub gates: Vec<GateResult>,
    pub omissions: Vec<Omission>,
    pub report_digest: String,
}

impl PullRequestReport {
    pub fn from_json(bytes: &[u8]) -> Result<Self, PrIntelligenceError> {
        let report: Self = serde_json::from_slice(bytes)?;
        if report.schema != REPORT_SCHEMA {
            return Err(PrIntelligenceError::UnsupportedSchema(report.schema));
        }
        report.validate()?;
        let expected = crate::report_digest(&report)?;
        if expected != report.report_digest {
            return Err(PrIntelligenceError::InvalidEvidence(format!(
                "report digest mismatch: expected {expected}, found {}",
                report.report_digest
            )));
        }
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PrIntelligenceError> {
        if self.schema != REPORT_SCHEMA {
            return Err(PrIntelligenceError::UnsupportedSchema(self.schema.clone()));
        }
        self.identity.repository.validate()?;
        if self.identity.pull_request_number == Some(0) {
            return Err(PrIntelligenceError::InvalidEvidence(
                "pull request number must be positive".to_owned(),
            ));
        }
        validate_object_id("report merge base", &self.identity.revisions.merge_base)?;
        validate_object_id(
            "report pull request head",
            &self.identity.revisions.pull_request_head,
        )?;
        validate_object_id("report target head", &self.identity.revisions.target_head)?;
        self.identity.revisions.merge_result.validate()?;
        for (name, value) in [
            ("report graph schema", &self.identity.graph_schema),
            ("report extractor version", &self.identity.extractor_version),
        ] {
            validate_string(name, value)?;
        }
        validate_lower_hex(
            "report configuration digest",
            &self.identity.configuration_digest,
            64,
        )?;
        validate_digest("report policy pack", &self.identity.policy_pack_digest)?;
        validate_digest(
            "report evidence manifest",
            &self.identity.evidence_manifest_digest,
        )?;
        validate_digest("report", &self.report_digest)?;
        validate_count("findings", self.findings.len(), MAX_FINDINGS)?;
        if self
            .findings
            .windows(2)
            .any(|pair| pair[0].fingerprint >= pair[1].fingerprint)
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "findings must be strictly ordered by fingerprint".to_owned(),
            ));
        }
        let fingerprints = self
            .findings
            .iter()
            .map(|finding| finding.fingerprint.as_str())
            .collect::<BTreeSet<_>>();
        let expected_source_revision = self
            .identity
            .revisions
            .merge_result
            .object_id()
            .unwrap_or(&self.identity.revisions.pull_request_head);
        for finding in &self.findings {
            validate_finding(finding)?;
            if finding.completeness != self.completeness
                || finding.evidence_digest != self.identity.evidence_manifest_digest
                || finding.source_revision != expected_source_revision
            {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "finding evidence identity or completeness contradicts the report".to_owned(),
                ));
            }
            if finding.deterministic
                && (finding.confidence != Confidence::Exact
                    || finding.freshness != Freshness::ExactHead
                    || finding.completeness.incomplete()
                    || !self.identity.revisions.merge_result.is_clean())
            {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "deterministic finding lacks exact, fresh, complete merge evidence".to_owned(),
                ));
            }
        }
        if self
            .risk_factors
            .windows(2)
            .any(|pair| pair[0].kind >= pair[1].kind)
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "risk factors must be strictly ordered by kind".to_owned(),
            ));
        }
        for factor in &self.risk_factors {
            let (points, cap) = factor_points(factor.kind);
            let count = u16::try_from(factor.finding_fingerprints.len())
                .unwrap_or(u16::MAX)
                .max(1);
            let expected_points = points.checked_mul(count).unwrap_or(cap).min(cap);
            if factor.points != expected_points {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "risk factor points contradict rubric version 1".to_owned(),
                ));
            }
            if factor.finding_fingerprints.is_empty()
                && !matches!(
                    factor.kind,
                    RiskFactorKind::IncompleteEvidence | RiskFactorKind::MergeConflict
                )
            {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "risk factor must cite finding evidence".to_owned(),
                ));
            }
            validate_string("risk factor explanation", &factor.explanation)?;
            validate_fingerprint_references(
                "risk factor",
                &factor.finding_fingerprints,
                &fingerprints,
            )?;
        }
        validate_advisory_risk(self)?;
        validate_string("advisory risk explanation", &self.advisory_risk.explanation)?;
        if self.gates.windows(2).any(|pair| pair[0].id >= pair[1].id) {
            return Err(PrIntelligenceError::InvalidEvidence(
                "gates must be strictly ordered by ID".to_owned(),
            ));
        }
        for gate in &self.gates {
            validate_string("gate id", &gate.id)?;
            validate_string("gate statement", &gate.statement)?;
            if gate.rule_version == 0 {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "gate rule versions must be positive".to_owned(),
                ));
            }
            validate_fingerprint_references("gate", &gate.finding_fingerprints, &fingerprints)?;
            if gate.state == GateState::Fail && gate.finding_fingerprints.is_empty() {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "a failing gate must cite at least one finding".to_owned(),
                ));
            }
        }
        validate_initial_gate(self)?;
        let mut omission_categories = BTreeSet::new();
        for omission in &self.omissions {
            validate_string("omission category", &omission.category)?;
            validate_string("omission reason", &omission.reason)?;
            if omission.count == 0 || !omission_categories.insert(omission.category.as_str()) {
                return Err(PrIntelligenceError::InvalidEvidence(
                    "omission categories must be unique and counts must be positive".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_finding(finding: &Finding) -> Result<(), PrIntelligenceError> {
    validate_finding_fingerprint(&finding.fingerprint)?;
    if finding.classifier_version == 0 {
        return Err(PrIntelligenceError::InvalidEvidence(
            "finding classifier version must be positive".to_owned(),
        ));
    }
    validate_string("finding statement", &finding.statement)?;
    validate_count(
        "finding source entities",
        finding.source_entities.len(),
        MAX_ENTITIES_PER_FINDING,
    )?;
    validate_count(
        "finding target entities",
        finding.target_entities.len(),
        MAX_ENTITIES_PER_FINDING,
    )?;
    if finding.source_entities.is_empty() {
        return Err(PrIntelligenceError::InvalidEvidence(
            "finding must identify at least one source entity".to_owned(),
        ));
    }
    for entity in finding
        .source_entities
        .iter()
        .chain(&finding.target_entities)
    {
        validate_string("finding entity", entity)?;
    }
    validate_strict_order("finding source entities", &finding.source_entities)?;
    validate_strict_order("finding target entities", &finding.target_entities)?;
    validate_count("witness hops", finding.witness.len(), MAX_WITNESS_HOPS)?;
    for hop in &finding.witness {
        validate_string("witness source", &hop.source)?;
        validate_string("witness relation", &hop.relation)?;
        validate_string("witness target", &hop.target)?;
    }
    if finding
        .witness
        .windows(2)
        .any(|pair| pair[0].target != pair[1].source)
    {
        return Err(PrIntelligenceError::InvalidEvidence(
            "witness hops do not form a continuous path".to_owned(),
        ));
    }
    validate_count(
        "finding locations",
        finding.locations.len(),
        MAX_LOCATIONS_PER_FINDING,
    )?;
    for location in &finding.locations {
        validate_string("finding location path", &location.path)?;
        if let (Some(start), Some(end)) = (location.start_byte, location.end_byte)
            && end < start
        {
            return Err(PrIntelligenceError::InvalidEvidence(
                "finding location end precedes its start".to_owned(),
            ));
        }
    }
    validate_strict_order("finding locations", &finding.locations)?;
    validate_count(
        "exact tests",
        finding.verification.exact_tests.len(),
        MAX_TESTS_PER_FINDING,
    )?;
    validate_count(
        "recommended tests",
        finding.verification.recommended_tests.len(),
        MAX_TESTS_PER_FINDING,
    )?;
    for test in finding
        .verification
        .exact_tests
        .iter()
        .chain(&finding.verification.recommended_tests)
    {
        validate_string("verification test", test)?;
    }
    validate_strict_order("exact tests", &finding.verification.exact_tests)?;
    validate_strict_order("recommended tests", &finding.verification.recommended_tests)?;
    let expected_gap = matches!(
        finding.verification.state,
        VerificationState::Gap
            | VerificationState::Partial
            | VerificationState::Failing
            | VerificationState::NotRun
    );
    if finding.verification.gap != expected_gap {
        return Err(PrIntelligenceError::InvalidEvidence(
            "verification gap contradicts verification state".to_owned(),
        ));
    }
    validate_string("verification reason", &finding.verification.reason)?;
    validate_object_id("finding source revision", &finding.source_revision)?;
    validate_string("finding evidence source", &finding.evidence_source)?;
    validate_digest("finding evidence", &finding.evidence_digest)?;
    validate_optional_string("finding remediation", &finding.remediation)?;
    Ok(())
}

const fn factor_points(kind: RiskFactorKind) -> (u16, u16) {
    match kind {
        RiskFactorKind::PublicContractChange => (20, 40),
        RiskFactorKind::AffectedConsumer => (4, 24),
        RiskFactorKind::CrossBoundaryImpact => (10, 20),
        RiskFactorKind::Cycle => (20, 20),
        RiskFactorKind::WeakConfidenceWitness => (4, 16),
        RiskFactorKind::VerificationGap => (12, 36),
        RiskFactorKind::IncompleteEvidence => (20, 20),
        RiskFactorKind::MergeConflict => (30, 30),
    }
}

fn validate_advisory_risk(report: &PullRequestReport) -> Result<(), PrIntelligenceError> {
    if report.advisory_risk.rubric_version != RUBRIC_VERSION {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "unsupported advisory rubric version {}",
            report.advisory_risk.rubric_version
        )));
    }
    let expected_score = if report.identity.revisions.merge_result.is_clean() {
        Some(
            report
                .risk_factors
                .iter()
                .fold(0_u16, |total, factor| total.saturating_add(factor.points))
                .min(100),
        )
    } else {
        None
    };
    let expected_band = expected_score.map_or(RiskBand::Unavailable, |score| match score {
        0..=19 => RiskBand::Low,
        20..=44 => RiskBand::Moderate,
        45..=69 => RiskBand::High,
        _ => RiskBand::Critical,
    });
    if report.advisory_risk.score != expected_score || report.advisory_risk.band != expected_band {
        return Err(PrIntelligenceError::InvalidEvidence(
            "advisory risk contradicts the versioned factors or merge state".to_owned(),
        ));
    }
    Ok(())
}

fn validate_initial_gate(report: &PullRequestReport) -> Result<(), PrIntelligenceError> {
    let gate = report
        .gates
        .iter()
        .find(|gate| gate.id == "proven-contract-break")
        .ok_or_else(|| {
            PrIntelligenceError::InvalidEvidence(
                "report has no proven-contract-break gate".to_owned(),
            )
        })?;
    if gate.rule_version != 1 {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "unsupported proven-contract-break rule version {}",
            gate.rule_version
        )));
    }
    let expected_findings = report
        .findings
        .iter()
        .filter(|finding| {
            finding.finding_type == FindingType::ContractChange && finding.deterministic
        })
        .map(|finding| finding.fingerprint.clone())
        .collect::<Vec<_>>();
    let expected_state =
        if !report.identity.revisions.merge_result.is_clean() || report.completeness.incomplete() {
            GateState::Indeterminate
        } else if expected_findings.is_empty() {
            GateState::Pass
        } else {
            GateState::Fail
        };
    if gate.state != expected_state || gate.finding_fingerprints != expected_findings {
        return Err(PrIntelligenceError::InvalidEvidence(
            "proven-contract-break gate contradicts its deterministic findings".to_owned(),
        ));
    }
    Ok(())
}

fn validate_fingerprint_references(
    name: &str,
    references: &[String],
    findings: &BTreeSet<&str>,
) -> Result<(), PrIntelligenceError> {
    if references.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} finding fingerprints must be strictly ordered"
        )));
    }
    for fingerprint in references {
        validate_finding_fingerprint(fingerprint)?;
        if !findings.contains(fingerprint.as_str()) {
            return Err(PrIntelligenceError::InvalidEvidence(format!(
                "{name} references unknown finding {fingerprint}"
            )));
        }
    }
    Ok(())
}

fn validate_finding_fingerprint(value: &str) -> Result<(), PrIntelligenceError> {
    let Some(digest) = value.strip_prefix("cmpprv1:") else {
        return Err(PrIntelligenceError::InvalidEvidence(
            "finding fingerprint must use cmpprv1".to_owned(),
        ));
    };
    validate_lower_hex("finding fingerprint", digest, 64)
}

fn validate_digest(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} digest must use sha256"
        )));
    };
    validate_lower_hex(name, digest, 64)
}

fn validate_object_id(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    if !matches!(value.len(), 40 | 64) {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} must be a full Git object ID"
        )));
    }
    validate_lower_hex(name, value, value.len())
}

fn validate_lower_hex(
    name: &str,
    value: &str,
    expected_len: usize,
) -> Result<(), PrIntelligenceError> {
    if value.len() != expected_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} must contain {expected_len} lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_count(name: &str, count: usize, maximum: usize) -> Result<(), PrIntelligenceError> {
    if count > maximum {
        return Err(PrIntelligenceError::Limit(format!(
            "{name} count {count} exceeds {maximum}"
        )));
    }
    Ok(())
}

fn validate_strict_order<T: Ord>(name: &str, values: &[T]) -> Result<(), PrIntelligenceError> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} must be strictly ordered"
        )));
    }
    Ok(())
}

pub(crate) fn validate_string(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    if value.is_empty() {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} must not be empty"
        )));
    }
    if value.len() > MAX_STRING_BYTES {
        return Err(PrIntelligenceError::Limit(format!(
            "{name} exceeds {MAX_STRING_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(PrIntelligenceError::InvalidEvidence(format!(
            "{name} contains control characters"
        )));
    }
    Ok(())
}

fn validate_optional_string(name: &str, value: &str) -> Result<(), PrIntelligenceError> {
    if value.is_empty() {
        Ok(())
    } else {
        validate_string(name, value)
    }
}
