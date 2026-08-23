use std::collections::BTreeMap;
use std::path::{Component, Path};

use compass_model::code_graph::{EdgeRecord, GraphDocument, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use serde::{Deserialize, Serialize};

use crate::contract::validate_bounded_text;
use crate::{
    AgentFactDraft, AgentGraphDiagnostic, AgentGraphError, AgentGraphErrorCode, AgentGraphLimits,
    AssertionDigest, AssertionDraft, AssertionId, BaseEdgeRef, BaseFactRef, BaseGenerationId,
    BaseNodeRef, Digest, GroundingCertificateDigest, OverlayRevisionId, canonical_digest,
};

pub const GROUNDING_SCHEMA_V1: &str = "compass.agent-graph.grounding/1";
pub const DEFAULT_GROUNDING_POLICY_ID: &str = "compass.agent-graph.topology-source-span";
pub const DEFAULT_GROUNDING_POLICY_VERSION: &str = "1";
const SOURCE_SPAN_VERIFIER: &str = "source-span/1";
const BASE_FACT_VERIFIER: &str = "base-fact/1";
const BASE_PATH_VERIFIER: &str = "base-path/1";
const PRIOR_ASSERTION_VERIFIER: &str = "prior-assertion/1";
const SNAPSHOT_ARTIFACT_VERIFIER: &str = "snapshot-artifact/1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroundingSubmission {
    pub schema: String,
    pub policy_id: String,
    pub evidence: Vec<GroundingEvidence>,
}

impl GroundingSubmission {
    pub fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        if self.schema != GROUNDING_SCHEMA_V1 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnsupportedSchema,
                format!(
                    "grounding schema must be {GROUNDING_SCHEMA_V1}; got {}",
                    self.schema
                ),
            ));
        }
        validate_bounded_text("policyId", &self.policy_id, 256, true)?;
        if self.evidence.is_empty() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::GroundingFailed,
                "grounding evidence must not be empty",
            ));
        }
        if self.evidence.len() > limits.max_citations_per_assertion {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "grounding has {} citations; maximum is {}",
                    self.evidence.len(),
                    limits.max_citations_per_assertion
                ),
            ));
        }
        let bytes = crate::canonical_bytes(&self.evidence)?;
        if bytes.len() > limits.max_evidence_bytes {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "grounding evidence is {} bytes; maximum is {}",
                    bytes.len(),
                    limits.max_evidence_bytes
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "evidenceType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GroundingEvidence {
    SourceSpan {
        file: String,
        anchor: SourceAnchor,
        file_digest: Digest,
        excerpt_digest: Digest,
    },
    BaseFact {
        fact: BaseFactRef,
        record_digest: Digest,
    },
    BasePath {
        nodes: Vec<BaseNodeRef>,
        edges: Vec<BaseEdgeRef>,
        path_digest: Digest,
    },
    PriorAssertion {
        assertion: AssertionId,
        revision: OverlayRevisionId,
        assertion_digest: AssertionDigest,
    },
    SnapshotArtifact {
        artifact: String,
        artifact_digest: Digest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        json_pointer: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum GroundingStatus {
    #[serde(rename = "GROUNDED")]
    Grounded,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundedEffect {
    ProjectTopology,
    FlagBaseFact,
    MaskBaseFact,
}

/// Proof that Compass verified one exact claim and its citations.
///
/// The fields are intentionally private and no mutation draft accepts this
/// type. Deserialization exists only so repository state can be reopened; the
/// repository revalidates its digest and citations before trusting it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingCertificate {
    status: GroundingStatus,
    claim_digest: Digest,
    evidence_digest: Digest,
    base_generation: BaseGenerationId,
    policy_id: String,
    policy_version: String,
    verifier_versions: Vec<String>,
    permitted_effects: Vec<GroundedEffect>,
}

impl GroundingCertificate {
    #[must_use]
    pub const fn status(&self) -> GroundingStatus {
        self.status
    }

    #[must_use]
    pub fn claim_digest(&self) -> &Digest {
        &self.claim_digest
    }

    #[must_use]
    pub fn evidence_digest(&self) -> &Digest {
        &self.evidence_digest
    }

    #[must_use]
    pub fn base_generation(&self) -> &BaseGenerationId {
        &self.base_generation
    }

    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    #[must_use]
    pub fn permitted_effects(&self) -> &[GroundedEffect] {
        &self.permitted_effects
    }

    pub fn digest(&self) -> Result<GroundingCertificateDigest, AgentGraphError> {
        canonical_digest("compass.agent-graph.certificate/1", self).map(GroundingCertificateDigest)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundedAssertion {
    pub assertion_digest: AssertionDigest,
    pub certificate_digest: GroundingCertificateDigest,
    pub certificate: GroundingCertificate,
    pub fact: AgentFactDraft,
    pub summary: String,
    pub grounding: GroundingSubmission,
    pub projection_provenance: Provenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundedChallenge {
    pub certificate_digest: GroundingCertificateDigest,
    pub certificate: GroundingCertificate,
    pub grounding: GroundingSubmission,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRecord {
    bytes: Vec<u8>,
}

impl ArtifactRecord {
    pub fn new(bytes: Vec<u8>) -> Result<Self, AgentGraphError> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "snapshot artifact exceeds the 16 MiB verification ceiling",
            ));
        }
        Ok(Self { bytes })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriorAssertionRecord {
    pub digest: AssertionDigest,
}

pub trait BaseGenerationView: Send + Sync {
    fn identity(&self) -> &BaseGenerationId;
    fn graph(&self) -> &GraphDocument;
    fn source_bytes(&self, repository_path: &str) -> Result<Option<Vec<u8>>, AgentGraphError>;
    fn artifact(&self, artifact: &str) -> Result<Option<ArtifactRecord>, AgentGraphError>;
    fn prior_assertion(
        &self,
        revision: &OverlayRevisionId,
        assertion: &AssertionId,
    ) -> Result<Option<PriorAssertionRecord>, AgentGraphError>;
}

#[derive(Clone, Debug)]
pub struct InMemoryBaseGeneration {
    identity: BaseGenerationId,
    graph: GraphDocument,
    source_files: BTreeMap<String, Vec<u8>>,
    artifacts: BTreeMap<String, ArtifactRecord>,
    prior_assertions: BTreeMap<(OverlayRevisionId, AssertionId), PriorAssertionRecord>,
}

impl InMemoryBaseGeneration {
    pub fn new(
        identity: BaseGenerationId,
        graph: GraphDocument,
        source_files: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, AgentGraphError> {
        identity.validate()?;
        if graph.graph.build.generation_id != identity.generation_id {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "graph build generation does not match the selected Base Generation",
            ));
        }
        let graph_digest = Digest::raw_bytes(&crate::canonical_bytes(&graph)?);
        if graph_digest != identity.graph_digest {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "canonical Base Graph digest does not match the selected identity",
            ));
        }
        compass_model::validate_code_graph(&graph).map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                format!("selected Base Generation is invalid: {error}"),
            )
        })?;
        Ok(Self {
            identity,
            graph,
            source_files,
            artifacts: BTreeMap::new(),
            prior_assertions: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn with_artifact(mut self, name: String, artifact: ArtifactRecord) -> Self {
        self.artifacts.insert(name, artifact);
        self
    }

    #[must_use]
    pub fn with_prior_assertion(
        mut self,
        revision: OverlayRevisionId,
        assertion: AssertionId,
        record: PriorAssertionRecord,
    ) -> Self {
        self.prior_assertions.insert((revision, assertion), record);
        self
    }
}

impl BaseGenerationView for InMemoryBaseGeneration {
    fn identity(&self) -> &BaseGenerationId {
        &self.identity
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn source_bytes(&self, repository_path: &str) -> Result<Option<Vec<u8>>, AgentGraphError> {
        Ok(self.source_files.get(repository_path).cloned())
    }

    fn artifact(&self, artifact: &str) -> Result<Option<ArtifactRecord>, AgentGraphError> {
        Ok(self.artifacts.get(artifact).cloned())
    }

    fn prior_assertion(
        &self,
        revision: &OverlayRevisionId,
        assertion: &AssertionId,
    ) -> Result<Option<PriorAssertionRecord>, AgentGraphError> {
        Ok(self
            .prior_assertions
            .get(&(revision.clone(), assertion.clone()))
            .cloned())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroundingPolicy {
    require_source_span: bool,
    permit_mask: bool,
}

impl Default for GroundingPolicy {
    fn default() -> Self {
        Self {
            require_source_span: true,
            permit_mask: false,
        }
    }
}

impl GroundingPolicy {
    #[must_use]
    pub const fn allowing_masks() -> Self {
        Self {
            require_source_span: true,
            permit_mask: true,
        }
    }
}

pub fn ground_assertion(
    draft: &AssertionDraft,
    base: &dyn BaseGenerationView,
    policy: GroundingPolicy,
    limits: AgentGraphLimits,
) -> Result<GroundedAssertion, AgentGraphError> {
    draft.validate(limits)?;
    if draft.grounding.policy_id != DEFAULT_GROUNDING_POLICY_ID {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::GroundingPolicyUnsupported,
            format!("unsupported Grounding policy {}", draft.grounding.policy_id),
        ));
    }
    let mut verified = Vec::with_capacity(draft.grounding.evidence.len());
    let mut source_anchor = None;
    for (index, evidence) in draft.grounding.evidence.iter().enumerate() {
        match verify_evidence(evidence, base) {
            Ok(verification) => {
                if source_anchor.is_none() {
                    source_anchor = verification.anchor;
                }
                verified.push((verification.canonical_digest, verification.verifier));
            }
            Err(error) => {
                return Err(error.with_diagnostic(AgentGraphDiagnostic {
                    code: "citation_verification_failed".to_owned(),
                    field: format!("grounding.evidence[{index}]"),
                    message: "citation did not match the selected Base Generation".to_owned(),
                    related_ids: Vec::new(),
                    omitted_count: 0,
                }));
            }
        }
    }
    if policy.require_source_span && source_anchor.is_none() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::GroundingFailed,
            "topology assertions require at least one verified source span",
        ));
    }
    verified.sort();
    if verified.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            "grounding evidence must not contain duplicate citations",
        ));
    }
    let evidence_digest = canonical_digest("compass.agent-graph.verified-evidence/1", &verified)?;
    let claim_digest = canonical_digest("compass.agent-graph.claim/1", &draft.fact)?;
    let mut verifier_versions = verified
        .iter()
        .map(|(_, verifier)| verifier.clone())
        .collect::<Vec<_>>();
    verifier_versions.sort();
    verifier_versions.dedup();
    let permitted_effects = vec![
        GroundedEffect::ProjectTopology,
        GroundedEffect::FlagBaseFact,
    ];
    let certificate = GroundingCertificate {
        status: GroundingStatus::Grounded,
        claim_digest,
        evidence_digest,
        base_generation: base.identity().clone(),
        policy_id: draft.grounding.policy_id.clone(),
        policy_version: DEFAULT_GROUNDING_POLICY_VERSION.to_owned(),
        verifier_versions,
        permitted_effects,
    };
    let certificate_digest = certificate.digest()?;
    let assertion_digest = AssertionDigest(canonical_digest(
        "compass.agent-graph.assertion-version/1",
        &(
            &draft.fact,
            &draft.summary,
            &certificate_digest,
            base.identity(),
        ),
    )?);
    let Some(anchor) = source_anchor else {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::GroundingFailed,
            "Grounding policy did not produce a projection source anchor",
        ));
    };
    let projection_provenance = Provenance::direct(
        EvidenceOrigin::Artifact,
        "compass.agent-graph",
        EvidenceConfidence::Inferred,
        anchor,
    )
    .map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::GroundingFailed,
            format!("verified evidence cannot be projected as provenance: {error}"),
        )
    })?;
    Ok(GroundedAssertion {
        assertion_digest,
        certificate_digest,
        certificate,
        fact: draft.fact.clone(),
        summary: draft.summary.clone(),
        grounding: draft.grounding.clone(),
        projection_provenance,
    })
}

pub fn ground_challenge(
    target: &BaseFactRef,
    summary: &str,
    submission: &GroundingSubmission,
    base: &dyn BaseGenerationView,
    policy: GroundingPolicy,
    limits: AgentGraphLimits,
) -> Result<GroundedChallenge, AgentGraphError> {
    submission.validate(limits)?;
    validate_bounded_text("summary", summary, limits.max_text_bytes, false)?;
    if submission.policy_id != DEFAULT_GROUNDING_POLICY_ID {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::GroundingPolicyUnsupported,
            format!("unsupported Grounding policy {}", submission.policy_id),
        ));
    }
    let mut verified = Vec::with_capacity(submission.evidence.len());
    let mut has_source_span = false;
    let mut cites_target = false;
    for (index, evidence) in submission.evidence.iter().enumerate() {
        let verification = verify_evidence(evidence, base).map_err(|error| {
            error.with_diagnostic(AgentGraphDiagnostic {
                code: "citation_verification_failed".to_owned(),
                field: format!("grounding.evidence[{index}]"),
                message: "citation did not match the selected Base Generation".to_owned(),
                related_ids: Vec::new(),
                omitted_count: 0,
            })
        })?;
        has_source_span |= matches!(evidence, GroundingEvidence::SourceSpan { .. });
        cites_target |=
            matches!(evidence, GroundingEvidence::BaseFact { fact, .. } if fact == target);
        verified.push((verification.canonical_digest, verification.verifier));
    }
    if !has_source_span || !cites_target {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::GroundingFailed,
            "a Challenge requires a verified source span and an exact citation of its target",
        ));
    }
    verified.sort();
    if verified.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            "grounding evidence must not contain duplicate citations",
        ));
    }
    let evidence_digest = canonical_digest("compass.agent-graph.verified-evidence/1", &verified)?;
    let claim_digest =
        canonical_digest("compass.agent-graph.challenge-claim/1", &(target, summary))?;
    let mut verifier_versions = verified
        .iter()
        .map(|(_, verifier)| verifier.clone())
        .collect::<Vec<_>>();
    verifier_versions.sort();
    verifier_versions.dedup();
    let mut permitted_effects = vec![GroundedEffect::FlagBaseFact];
    if policy.permit_mask {
        permitted_effects.push(GroundedEffect::MaskBaseFact);
    }
    let certificate = GroundingCertificate {
        status: GroundingStatus::Grounded,
        claim_digest,
        evidence_digest,
        base_generation: base.identity().clone(),
        policy_id: submission.policy_id.clone(),
        policy_version: DEFAULT_GROUNDING_POLICY_VERSION.to_owned(),
        verifier_versions,
        permitted_effects,
    };
    Ok(GroundedChallenge {
        certificate_digest: certificate.digest()?,
        certificate,
        grounding: submission.clone(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Verification {
    canonical_digest: Digest,
    verifier: String,
    #[serde(skip)]
    anchor: Option<SourceAnchor>,
}

fn verify_evidence(
    evidence: &GroundingEvidence,
    base: &dyn BaseGenerationView,
) -> Result<Verification, AgentGraphError> {
    match evidence {
        GroundingEvidence::SourceSpan {
            file,
            anchor,
            file_digest,
            excerpt_digest,
        } => verify_source_span(file, anchor, file_digest, excerpt_digest, base),
        GroundingEvidence::BaseFact {
            fact,
            record_digest,
        } => verify_base_fact(fact, record_digest, base),
        GroundingEvidence::BasePath {
            nodes,
            edges,
            path_digest,
        } => verify_base_path(nodes, edges, path_digest, base),
        GroundingEvidence::PriorAssertion {
            assertion,
            revision,
            assertion_digest,
        } => {
            let Some(record) = base.prior_assertion(revision, assertion)? else {
                return invalid_citation("prior assertion does not exist at the exact revision");
            };
            if record.digest != *assertion_digest {
                return invalid_citation("prior assertion digest does not match");
            }
            Ok(Verification {
                canonical_digest: canonical_digest(
                    "compass.agent-graph.prior-assertion-citation/1",
                    &(revision, assertion, assertion_digest),
                )?,
                verifier: PRIOR_ASSERTION_VERIFIER.to_owned(),
                anchor: None,
            })
        }
        GroundingEvidence::SnapshotArtifact {
            artifact,
            artifact_digest,
            json_pointer,
        } => {
            validate_bounded_text("artifact", artifact, 1_024, false)?;
            if let Some(pointer) = json_pointer {
                validate_json_pointer(pointer)?;
            }
            let Some(record) = base.artifact(artifact)? else {
                return invalid_citation("snapshot artifact does not exist");
            };
            if Digest::raw_bytes(record.bytes()) != *artifact_digest {
                return invalid_citation("snapshot artifact digest does not match");
            }
            if let Some(pointer) = json_pointer {
                let document = serde_json::from_slice::<serde_json::Value>(record.bytes())
                    .map_err(|_| {
                        AgentGraphError::new(
                            AgentGraphErrorCode::InvalidCitation,
                            "snapshot artifact with a JSON pointer is not valid JSON",
                        )
                    })?;
                if document.pointer(pointer).is_none() {
                    return invalid_citation("snapshot artifact JSON pointer does not resolve");
                }
            }
            Ok(Verification {
                canonical_digest: canonical_digest(
                    "compass.agent-graph.snapshot-artifact-citation/1",
                    &(artifact, artifact_digest, json_pointer),
                )?,
                verifier: SNAPSHOT_ARTIFACT_VERIFIER.to_owned(),
                anchor: None,
            })
        }
    }
}

fn verify_source_span(
    file: &str,
    anchor: &SourceAnchor,
    file_digest: &Digest,
    excerpt_digest: &Digest,
    base: &dyn BaseGenerationView,
) -> Result<Verification, AgentGraphError> {
    if file != anchor.file || !anchor.is_valid() || anchor.start_byte == anchor.end_byte {
        return invalid_citation("source span path or range is invalid");
    }
    if Path::new(file)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return invalid_citation("source span path must be a confined repository-relative path");
    }
    let Some(inventory) = base
        .graph()
        .graph
        .files
        .iter()
        .find(|entry| entry.path == file)
    else {
        return invalid_citation(
            "source span file is not present in the Base Generation inventory",
        );
    };
    let Some(bytes) = base.source_bytes(file)? else {
        return invalid_citation("source span file bytes are unavailable");
    };
    let actual_file_digest = Digest::raw_bytes(&bytes);
    if actual_file_digest != *file_digest
        || inventory.content_digest != file_digest.as_str()
        || inventory.byte_size != bytes.len() as u64
    {
        return invalid_citation("source file digest does not match bytes and inventory");
    }
    let start = usize::try_from(anchor.start_byte).map_err(|_| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            "source start is out of range",
        )
    })?;
    let end = usize::try_from(anchor.end_byte).map_err(|_| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            "source end is out of range",
        )
    })?;
    let Some(excerpt) = bytes.get(start..end) else {
        return invalid_citation("source span lies outside the verified file bytes");
    };
    if Digest::raw_bytes(excerpt) != *excerpt_digest {
        return invalid_citation("source excerpt digest does not match");
    }
    let (start_line, start_column) = byte_line_column(&bytes, start);
    let (end_line, end_column) = byte_line_column(&bytes, end);
    if (anchor.start_line, anchor.start_column) != (start_line, start_column)
        || (anchor.end_line, anchor.end_column) != (end_line, end_column)
    {
        return invalid_citation("source span line and column do not match its byte range");
    }
    Ok(Verification {
        canonical_digest: canonical_digest(
            "compass.agent-graph.source-span-citation/1",
            &(file, anchor, file_digest, excerpt_digest),
        )?,
        verifier: SOURCE_SPAN_VERIFIER.to_owned(),
        anchor: Some(anchor.clone()),
    })
}

fn verify_base_fact(
    fact: &BaseFactRef,
    submitted_record_digest: &Digest,
    base: &dyn BaseGenerationView,
) -> Result<Verification, AgentGraphError> {
    let actual = match fact {
        BaseFactRef::Node(reference) => {
            verify_generation(&reference.base_generation, base.identity())?;
            if &reference.record_digest != submitted_record_digest {
                return invalid_citation("Base Fact contains conflicting record digests");
            }
            let Some(record) = find_node(base.graph(), &reference.id) else {
                return invalid_citation("base node does not exist");
            };
            if record.kind != reference.kind {
                return invalid_citation("base node kind does not match");
            }
            canonical_digest("compass.agent-graph.base-node-record/1", record)?
        }
        BaseFactRef::Edge(reference) => {
            verify_generation(&reference.base_generation, base.identity())?;
            if &reference.record_digest != submitted_record_digest {
                return invalid_citation("Base Fact contains conflicting record digests");
            }
            let Some(record) = find_edge(base.graph(), &reference.id) else {
                return invalid_citation("base edge does not exist");
            };
            if record.kind != reference.kind
                || record.source != reference.source
                || record.target != reference.target
            {
                return invalid_citation("base edge kind, direction, or endpoints do not match");
            }
            canonical_digest("compass.agent-graph.base-edge-record/1", record)?
        }
    };
    if &actual != submitted_record_digest {
        return invalid_citation("Base Fact record digest does not match canonical record bytes");
    }
    Ok(Verification {
        canonical_digest: canonical_digest("compass.agent-graph.base-fact-citation/1", fact)?,
        verifier: BASE_FACT_VERIFIER.to_owned(),
        anchor: None,
    })
}

fn verify_base_path(
    nodes: &[BaseNodeRef],
    edges: &[BaseEdgeRef],
    path_digest: &Digest,
    base: &dyn BaseGenerationView,
) -> Result<Verification, AgentGraphError> {
    if nodes.len() < 2 || edges.len() + 1 != nodes.len() || nodes.len() > 1_001 {
        return invalid_citation("Base Path must contain 2..=1001 nodes and one fewer edges");
    }
    for node in nodes {
        verify_base_fact(&BaseFactRef::Node(node.clone()), &node.record_digest, base)?;
    }
    for (index, edge) in edges.iter().enumerate() {
        verify_base_fact(&BaseFactRef::Edge(edge.clone()), &edge.record_digest, base)?;
        if edge.source != nodes[index].id || edge.target != nodes[index + 1].id {
            return invalid_citation("Base Path edge sequence does not preserve direction");
        }
    }
    let actual = canonical_digest("compass.agent-graph.base-path/1", &(nodes, edges))?;
    if &actual != path_digest {
        return invalid_citation("Base Path digest does not match the exact ordered path");
    }
    Ok(Verification {
        canonical_digest: actual,
        verifier: BASE_PATH_VERIFIER.to_owned(),
        anchor: None,
    })
}

fn verify_generation(
    submitted: &BaseGenerationId,
    selected: &BaseGenerationId,
) -> Result<(), AgentGraphError> {
    if submitted != selected {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::UnknownBaseGeneration,
            "citation names a different Base Generation",
        ));
    }
    Ok(())
}

fn validate_json_pointer(pointer: &str) -> Result<(), AgentGraphError> {
    if pointer.len() > 4_096 || (!pointer.is_empty() && !pointer.starts_with('/')) {
        return invalid_citation("JSON pointer is invalid or exceeds 4096 bytes");
    }
    Ok(())
}

fn byte_line_column(bytes: &[u8], offset: usize) -> (u32, u32) {
    let prefix = bytes.get(..offset).unwrap_or(bytes);
    let line = prefix
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        .saturating_add(1);
    let column = prefix
        .iter()
        .rev()
        .take_while(|byte| **byte != b'\n')
        .count();
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

fn find_node<'a>(graph: &'a GraphDocument, id: &str) -> Option<&'a NodeRecord> {
    graph.nodes.iter().find(|node| node.id == id)
}

fn find_edge<'a>(graph: &'a GraphDocument, id: &str) -> Option<&'a EdgeRecord> {
    graph.links.iter().find(|edge| edge.id == id)
}

fn invalid_citation<T>(message: impl Into<String>) -> Result<T, AgentGraphError> {
    Err(AgentGraphError::new(
        AgentGraphErrorCode::InvalidCitation,
        message,
    ))
}
