use std::collections::BTreeSet;

use compass_model::code_graph::{EdgeKind, EdgeRecord, GraphDocument, NodeRecord};
use compass_model::provenance::effective_confidence;
use compass_model::validate_code_graph;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::PROJECTION_SCHEMA_V1;

/// Finite work limits applied before activation and before materializing reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionLimits {
    max_nodes: usize,
    max_relations: usize,
    max_projected_bytes: u64,
}

impl ProjectionLimits {
    pub fn new(
        max_nodes: usize,
        max_relations: usize,
        max_projected_bytes: u64,
    ) -> Result<Self, ProjectionError> {
        if max_nodes == 0 || max_relations == 0 || max_projected_bytes == 0 {
            return Err(ProjectionError::InvalidPlan(
                "projection limits must all be greater than zero".to_owned(),
            ));
        }
        Ok(Self {
            max_nodes,
            max_relations,
            max_projected_bytes,
        })
    }

    #[must_use]
    pub const fn max_nodes(self) -> usize {
        self.max_nodes
    }

    #[must_use]
    pub const fn max_relations(self) -> usize {
        self.max_relations
    }

    #[must_use]
    pub const fn max_projected_bytes(self) -> u64 {
        self.max_projected_bytes
    }

    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    pub(crate) fn validate_plan(self, plan: &ProjectionPlan) -> Result<u64, ProjectionError> {
        enforce_limit("nodes", plan.nodes.len(), self.max_nodes)?;
        enforce_limit("relations", plan.relations.len(), self.max_relations)?;
        let projected_bytes = plan.projected_bytes()?;
        if projected_bytes > self.max_projected_bytes {
            return Err(ProjectionError::LimitExceeded {
                resource: "projected bytes",
                actual: projected_bytes,
                limit: self.max_projected_bytes,
            });
        }
        Ok(projected_bytes)
    }
}

impl Default for ProjectionLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_relations: 2_500_000,
            max_projected_bytes: compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES,
        }
    }
}

/// A closed set of schemafull relation tables.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationFamily {
    Structural,
    Dependency,
    Execution,
    DataFlow,
    Evidence,
}

impl RelationFamily {
    pub const ALL: [Self; 5] = [
        Self::Structural,
        Self::Dependency,
        Self::Execution,
        Self::DataFlow,
        Self::Evidence,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structural => "structural_relation",
            Self::Dependency => "dependency_relation",
            Self::Execution => "execution_relation",
            Self::DataFlow => "data_flow_relation",
            Self::Evidence => "evidence_relation",
        }
    }
}

/// Map every closed Compass edge kind to exactly one relation family.
#[must_use]
pub const fn relation_family(kind: EdgeKind) -> RelationFamily {
    match kind {
        EdgeKind::Contains
        | EdgeKind::Embeds
        | EdgeKind::Extends
        | EdgeKind::Implements
        | EdgeKind::MixesIn
        | EdgeKind::TypeOf
        | EdgeKind::Overrides
        | EdgeKind::Decorates
        | EdgeKind::Aliases => RelationFamily::Structural,
        EdgeKind::Imports | EdgeKind::Exports | EdgeKind::References | EdgeKind::DependsOn => {
            RelationFamily::Dependency
        }
        EdgeKind::Calls
        | EdgeKind::Instantiates
        | EdgeKind::RoutesTo
        | EdgeKind::Registers
        | EdgeKind::Handles
        | EdgeKind::Schedules
        | EdgeKind::Triggers
        | EdgeKind::Tests
        | EdgeKind::Renders => RelationFamily::Execution,
        EdgeKind::Returns
        | EdgeKind::Reads
        | EdgeKind::Writes
        | EdgeKind::Publishes
        | EdgeKind::Subscribes
        | EdgeKind::Produces
        | EdgeKind::Consumes
        | EdgeKind::MapsTo => RelationFamily::DataFlow,
        EdgeKind::Documents => RelationFamily::Evidence,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedNode {
    pub record_key: String,
    pub repository_id: String,
    pub generation_id: String,
    pub schema_version: String,
    pub compass_node_id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub normalized_names: Vec<String>,
    pub language: Option<String>,
    pub source_path: Option<String>,
    pub confidence: String,
    pub heuristic: bool,
    pub payload_json: String,
}

impl ProjectedNode {
    pub fn decode(&self) -> Result<NodeRecord, ProjectionError> {
        Ok(serde_json::from_str(&self.payload_json)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedRelation {
    pub record_key: String,
    pub repository_id: String,
    pub generation_id: String,
    pub schema_version: String,
    pub compass_edge_id: String,
    pub family: RelationFamily,
    pub kind: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub source_record_key: String,
    pub target_record_key: String,
    pub source_path: Option<String>,
    pub confidence: String,
    pub heuristic: bool,
    pub payload_json: String,
}

impl ProjectedRelation {
    pub fn decode(&self) -> Result<EdgeRecord, ProjectionError> {
        Ok(serde_json::from_str(&self.payload_json)?)
    }
}

/// Deterministic database-independent plan for one immutable generation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionPlan {
    pub repository_id: String,
    pub generation_id: String,
    pub schema_version: String,
    pub source_tree_digest: String,
    pub schema_fingerprint: String,
    pub projection_fingerprint: String,
    pub nodes: Vec<ProjectedNode>,
    pub relations: Vec<ProjectedRelation>,
}

impl ProjectionPlan {
    pub fn from_graph(
        repository_id: impl Into<String>,
        graph: &GraphDocument,
    ) -> Result<Self, ProjectionError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(ProjectionError::EmptyRepositoryId);
        }
        validate_code_graph(graph)?;
        let generation_id = graph.graph.build.generation_id.clone();
        let mut nodes = graph
            .nodes
            .iter()
            .map(|node| project_node(&repository_id, &generation_id, node))
            .collect::<Result<Vec<_>, _>>()?;
        nodes.sort_by(|left, right| left.compass_node_id.cmp(&right.compass_node_id));
        let node_ids = nodes
            .iter()
            .map(|node| node.compass_node_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut relations = graph
            .links
            .iter()
            .map(|edge| {
                for endpoint in [&edge.source, &edge.target] {
                    if !node_ids.contains(endpoint.as_str()) {
                        return Err(ProjectionError::MissingEndpoint {
                            edge_id: edge.id.clone(),
                            endpoint: endpoint.clone(),
                        });
                    }
                }
                project_relation(&repository_id, &generation_id, edge)
            })
            .collect::<Result<Vec<_>, _>>()?;
        relations.sort_by(|left, right| left.compass_edge_id.cmp(&right.compass_edge_id));
        let projection_fingerprint =
            fingerprint(&repository_id, &generation_id, &nodes, &relations);
        let plan = Self {
            repository_id,
            generation_id,
            schema_version: PROJECTION_SCHEMA_V1.to_owned(),
            source_tree_digest: graph.graph.build.source_tree_digest.clone(),
            schema_fingerprint: graph.graph.build.schema_fingerprint.clone(),
            projection_fingerprint,
            nodes,
            relations,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), ProjectionError> {
        if self.schema_version != PROJECTION_SCHEMA_V1 {
            return Err(ProjectionError::UnsupportedProjectionSchema(
                self.schema_version.clone(),
            ));
        }
        if self.repository_id.trim().is_empty() {
            return Err(ProjectionError::EmptyRepositoryId);
        }
        if self.generation_id.trim().is_empty() {
            return Err(ProjectionError::EmptyGenerationId);
        }
        ensure_strict_order(
            self.nodes.iter().map(|node| node.compass_node_id.as_str()),
            "node",
        )?;
        ensure_strict_order(
            self.relations
                .iter()
                .map(|relation| relation.compass_edge_id.as_str()),
            "relation",
        )?;
        let node_ids = self
            .nodes
            .iter()
            .map(|node| node.compass_node_id.as_str())
            .collect::<BTreeSet<_>>();
        for relation in &self.relations {
            for endpoint in [&relation.source_node_id, &relation.target_node_id] {
                if !node_ids.contains(endpoint.as_str()) {
                    return Err(ProjectionError::MissingEndpoint {
                        edge_id: relation.compass_edge_id.clone(),
                        endpoint: endpoint.clone(),
                    });
                }
            }
        }
        if fingerprint(
            &self.repository_id,
            &self.generation_id,
            &self.nodes,
            &self.relations,
        ) != self.projection_fingerprint
        {
            return Err(ProjectionError::FingerprintMismatch);
        }
        Ok(())
    }

    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    pub(crate) fn projected_bytes(&self) -> Result<u64, ProjectionError> {
        self.nodes
            .iter()
            .map(serialized_len)
            .chain(self.relations.iter().map(serialized_len))
            .try_fold(0_u64, |total, length| {
                total
                    .checked_add(length?)
                    .ok_or(ProjectionError::LimitExceeded {
                        resource: "projected bytes",
                        actual: u64::MAX,
                        limit: u64::MAX - 1,
                    })
            })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("repository identity must not be empty")]
    EmptyRepositoryId,
    #[error("generation identity must not be empty")]
    EmptyGenerationId,
    #[error("unsupported projection schema {0}")]
    UnsupportedProjectionSchema(String),
    #[error("canonical graph validation failed: {0}")]
    GraphValidation(#[from] compass_model::CodeGraphValidationError),
    #[error("projection serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("edge {edge_id} references missing endpoint {endpoint}")]
    MissingEndpoint { edge_id: String, endpoint: String },
    #[error("projection {record_class} identities are not strictly ordered and unique")]
    NonDeterministicOrder { record_class: &'static str },
    #[error("projection fingerprint does not match its records")]
    FingerprintMismatch,
    #[error("projection {resource} limit exceeded: actual {actual}, limit {limit}")]
    LimitExceeded {
        resource: &'static str,
        actual: u64,
        limit: u64,
    },
    #[error("invalid projection plan: {0}")]
    InvalidPlan(String),
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("SurrealDB {stage} failed: {message}")]
    Database {
        stage: &'static str,
        message: String,
    },
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("projection staging failed ({cause}); transaction cancellation also failed: {message}")]
    Cancellation {
        cause: Box<ProjectionError>,
        message: String,
    },
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("projection was interrupted after {completed_mutations} mutations")]
    Interrupted { completed_mutations: usize },
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("repository {repository_id:?} has no complete active Surreal generation")]
    ActiveGenerationUnavailable { repository_id: String },
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("repository {repository_id:?} changed active generation during the query")]
    ActiveGenerationChanged { repository_id: String },
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("invalid native-query cursor: {0}")]
    InvalidCursor(String),
    #[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
    #[error("invalid native query: {0}")]
    InvalidQuery(String),
}

#[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
fn serialized_len<T: Serialize>(value: &T) -> Result<u64, ProjectionError> {
    u64::try_from(serde_json::to_vec(value)?.len()).map_err(|_| ProjectionError::LimitExceeded {
        resource: "projected bytes",
        actual: u64::MAX,
        limit: u64::MAX - 1,
    })
}

#[cfg(any(feature = "mem", feature = "surrealkv", feature = "rocksdb"))]
fn enforce_limit(
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), ProjectionError> {
    if actual > limit {
        return Err(ProjectionError::LimitExceeded {
            resource,
            actual: u64::try_from(actual).unwrap_or(u64::MAX),
            limit: u64::try_from(limit).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

fn project_node(
    repository_id: &str,
    generation_id: &str,
    node: &NodeRecord,
) -> Result<ProjectedNode, ProjectionError> {
    let mut normalized_names = [&node.name, &node.qualified_name]
        .into_iter()
        .map(|name| compass_model::query_contract::normalize_query_symbol(name))
        .collect::<Vec<_>>();
    normalized_names.sort();
    normalized_names.dedup();
    Ok(ProjectedNode {
        record_key: record_key("node", &[repository_id, generation_id, &node.id]),
        repository_id: repository_id.to_owned(),
        generation_id: generation_id.to_owned(),
        schema_version: PROJECTION_SCHEMA_V1.to_owned(),
        compass_node_id: node.id.clone(),
        kind: node.kind.as_str().to_owned(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        normalized_names,
        language: node.language.clone(),
        source_path: node.source.as_ref().map(|source| source.file.clone()),
        confidence: confidence(&node.evidence),
        heuristic: node.evidence.iter().any(|evidence| {
            evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
        }),
        payload_json: serde_json::to_string(node)?,
    })
}

fn project_relation(
    repository_id: &str,
    generation_id: &str,
    edge: &EdgeRecord,
) -> Result<ProjectedRelation, ProjectionError> {
    Ok(ProjectedRelation {
        record_key: record_key("edge", &[repository_id, generation_id, &edge.id]),
        repository_id: repository_id.to_owned(),
        generation_id: generation_id.to_owned(),
        schema_version: PROJECTION_SCHEMA_V1.to_owned(),
        compass_edge_id: edge.id.clone(),
        family: relation_family(edge.kind),
        kind: edge.kind.as_str().to_owned(),
        source_node_id: edge.source.clone(),
        target_node_id: edge.target.clone(),
        source_record_key: record_key("node", &[repository_id, generation_id, &edge.source]),
        target_record_key: record_key("node", &[repository_id, generation_id, &edge.target]),
        source_path: edge
            .relationship_site
            .as_ref()
            .map(|source| source.file.clone()),
        confidence: confidence(&edge.evidence),
        heuristic: edge.evidence.iter().any(|evidence| {
            evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
        }),
        payload_json: serde_json::to_string(edge)?,
    })
}

fn confidence(evidence: &[compass_model::provenance::Provenance]) -> String {
    effective_confidence(evidence)
        .map_or("unknown", |value| value.as_str())
        .to_owned()
}

pub(crate) fn record_key(class: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    write_part(&mut hasher, PROJECTION_SCHEMA_V1.as_bytes());
    write_part(&mut hasher, class.as_bytes());
    for part in parts {
        write_part(&mut hasher, part.as_bytes());
    }
    hex_digest(hasher.finalize())
}

fn fingerprint(
    repository_id: &str,
    generation_id: &str,
    nodes: &[ProjectedNode],
    relations: &[ProjectedRelation],
) -> String {
    let mut hasher = Sha256::new();
    for value in [PROJECTION_SCHEMA_V1, repository_id, generation_id] {
        write_part(&mut hasher, value.as_bytes());
    }
    for node in nodes {
        write_part(&mut hasher, node.record_key.as_bytes());
        write_part(&mut hasher, node.payload_json.as_bytes());
    }
    for relation in relations {
        write_part(&mut hasher, relation.family.as_str().as_bytes());
        write_part(&mut hasher, relation.record_key.as_bytes());
        write_part(&mut hasher, relation.payload_json.as_bytes());
    }
    format!("sha256:{}", hex_digest(hasher.finalize()))
}

fn write_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    let mut encoded = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn ensure_strict_order<'a>(
    values: impl Iterator<Item = &'a str>,
    record_class: &'static str,
) -> Result<(), ProjectionError> {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|previous| previous >= value) {
            return Err(ProjectionError::NonDeterministicOrder { record_class });
        }
        previous = Some(value);
    }
    Ok(())
}
