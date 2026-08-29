//! Feature-gated SurrealDB execution layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::engine::local::Db;
use surrealdb::types::{RecordId, SurrealValue as _, Value as DatabaseValue};
use surrealdb::{IndexedResults, Surreal};

use crate::projection::record_key;
use crate::{
    PROJECTION_SCHEMA_V1, ProjectedNode, ProjectedRelation, ProjectionError, ProjectionLimits,
    ProjectionPlan, RelationFamily,
};

mod query;

pub use query::{NATIVE_RELATION_PAGE_SCHEMA_V1, RelationPage, RelationPageRequest};

const SCHEMA: &str = r#"
DEFINE TABLE OVERWRITE generation_manifest SCHEMAFULL;
DEFINE FIELD OVERWRITE repositoryId ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE generationId ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE projectionFingerprint ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE sourceTreeDigest ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE schemaFingerprint ON generation_manifest TYPE string;
DEFINE FIELD OVERWRITE nodeCount ON generation_manifest TYPE int;
DEFINE FIELD OVERWRITE relationCount ON generation_manifest TYPE int;
DEFINE FIELD OVERWRITE projectedBytes ON generation_manifest TYPE int;
DEFINE FIELD OVERWRITE complete ON generation_manifest TYPE bool;

DEFINE TABLE OVERWRITE repository_pointer SCHEMAFULL;
DEFINE FIELD OVERWRITE repositoryId ON repository_pointer TYPE string;
DEFINE FIELD OVERWRITE generationId ON repository_pointer TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON repository_pointer TYPE string;
DEFINE FIELD OVERWRITE projectionFingerprint ON repository_pointer TYPE string;

DEFINE TABLE OVERWRITE code_node SCHEMAFULL;
DEFINE FIELD OVERWRITE recordKey ON code_node TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON code_node TYPE string;
DEFINE FIELD OVERWRITE generationId ON code_node TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON code_node TYPE string;
DEFINE FIELD OVERWRITE compassNodeId ON code_node TYPE string;
DEFINE FIELD OVERWRITE kind ON code_node TYPE string;
DEFINE FIELD OVERWRITE name ON code_node TYPE string;
DEFINE FIELD OVERWRITE qualifiedName ON code_node TYPE string;
DEFINE FIELD OVERWRITE normalizedNames ON code_node TYPE array<string>;
DEFINE FIELD OVERWRITE language ON code_node TYPE option<string>;
DEFINE FIELD OVERWRITE sourcePath ON code_node TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON code_node TYPE string;
DEFINE FIELD OVERWRITE heuristic ON code_node TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON code_node TYPE string;
DEFINE INDEX OVERWRITE code_node_projection ON TABLE code_node FIELDS repositoryId, generationId, compassNodeId;
DEFINE INDEX OVERWRITE code_node_normalized_names ON TABLE code_node FIELDS repositoryId, generationId, normalizedNames[*];

DEFINE TABLE OVERWRITE structural_relation TYPE RELATION IN code_node OUT code_node SCHEMAFULL;
DEFINE TABLE OVERWRITE dependency_relation TYPE RELATION IN code_node OUT code_node SCHEMAFULL;
DEFINE TABLE OVERWRITE execution_relation TYPE RELATION IN code_node OUT code_node SCHEMAFULL;
DEFINE TABLE OVERWRITE data_flow_relation TYPE RELATION IN code_node OUT code_node SCHEMAFULL;
DEFINE TABLE OVERWRITE evidence_relation TYPE RELATION IN code_node OUT code_node SCHEMAFULL;

DEFINE FIELD OVERWRITE recordKey ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE generationId ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE compassEdgeId ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE family ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE kind ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE sourceNodeId ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE targetNodeId ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE sourceRecordKey ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE targetRecordKey ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE sourcePath ON structural_relation TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON structural_relation TYPE string;
DEFINE FIELD OVERWRITE heuristic ON structural_relation TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON structural_relation TYPE string;

DEFINE FIELD OVERWRITE recordKey ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE generationId ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE compassEdgeId ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE family ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE kind ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE sourceNodeId ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE targetNodeId ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE sourceRecordKey ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE targetRecordKey ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE sourcePath ON dependency_relation TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON dependency_relation TYPE string;
DEFINE FIELD OVERWRITE heuristic ON dependency_relation TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON dependency_relation TYPE string;

DEFINE FIELD OVERWRITE recordKey ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE generationId ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE compassEdgeId ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE family ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE kind ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE sourceNodeId ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE targetNodeId ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE sourceRecordKey ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE targetRecordKey ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE sourcePath ON execution_relation TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON execution_relation TYPE string;
DEFINE FIELD OVERWRITE heuristic ON execution_relation TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON execution_relation TYPE string;

DEFINE FIELD OVERWRITE recordKey ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE generationId ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE compassEdgeId ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE family ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE kind ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE sourceNodeId ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE targetNodeId ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE sourceRecordKey ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE targetRecordKey ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE sourcePath ON data_flow_relation TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON data_flow_relation TYPE string;
DEFINE FIELD OVERWRITE heuristic ON data_flow_relation TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON data_flow_relation TYPE string;

DEFINE FIELD OVERWRITE recordKey ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE repositoryId ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE generationId ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE schemaVersion ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE compassEdgeId ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE family ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE kind ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE sourceNodeId ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE targetNodeId ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE sourceRecordKey ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE targetRecordKey ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE sourcePath ON evidence_relation TYPE option<string>;
DEFINE FIELD OVERWRITE confidence ON evidence_relation TYPE string;
DEFINE FIELD OVERWRITE heuristic ON evidence_relation TYPE bool;
DEFINE FIELD OVERWRITE payloadJson ON evidence_relation TYPE string;

DEFINE INDEX OVERWRITE structural_relation_projection ON TABLE structural_relation FIELDS repositoryId, generationId, compassEdgeId;
DEFINE INDEX OVERWRITE dependency_relation_projection ON TABLE dependency_relation FIELDS repositoryId, generationId, compassEdgeId;
DEFINE INDEX OVERWRITE execution_relation_projection ON TABLE execution_relation FIELDS repositoryId, generationId, compassEdgeId;
DEFINE INDEX OVERWRITE data_flow_relation_projection ON TABLE data_flow_relation FIELDS repositoryId, generationId, compassEdgeId;
DEFINE INDEX OVERWRITE evidence_relation_projection ON TABLE evidence_relation FIELDS repositoryId, generationId, compassEdgeId;
"#;

const UPSERT_RECORD: &str = "UPSERT $record CONTENT $payload";
const CLAIM_GENERATION: &str = "INSERT INTO generation_manifest $payload ON DUPLICATE KEY UPDATE repositoryId = repositoryId RETURN NONE";
const INSERT_NODE_BATCH: &str = r#"INSERT INTO code_node $payloads ON DUPLICATE KEY UPDATE
recordKey = $input.recordKey, repositoryId = $input.repositoryId,
generationId = $input.generationId, schemaVersion = $input.schemaVersion,
compassNodeId = $input.compassNodeId, kind = $input.kind, name = $input.name,
qualifiedName = $input.qualifiedName, normalizedNames = $input.normalizedNames,
language = $input.language, sourcePath = $input.sourcePath,
confidence = $input.confidence, heuristic = $input.heuristic,
payloadJson = $input.payloadJson RETURN NONE"#;

macro_rules! relation_insert_statement {
    ($table:literal) => {
        concat!(
            "INSERT RELATION INTO ",
            $table,
            " $payloads ON DUPLICATE KEY UPDATE ",
            "in = $input.in, out = $input.out, recordKey = $input.recordKey, ",
            "repositoryId = $input.repositoryId, generationId = $input.generationId, ",
            "schemaVersion = $input.schemaVersion, compassEdgeId = $input.compassEdgeId, ",
            "family = $input.family, kind = $input.kind, sourceNodeId = $input.sourceNodeId, ",
            "targetNodeId = $input.targetNodeId, sourceRecordKey = $input.sourceRecordKey, ",
            "targetRecordKey = $input.targetRecordKey, sourcePath = $input.sourcePath, ",
            "confidence = $input.confidence, heuristic = $input.heuristic, ",
            "payloadJson = $input.payloadJson RETURN NONE"
        )
    };
}

const INSERT_STRUCTURAL_RELATION_BATCH: &str = relation_insert_statement!("structural_relation");
const INSERT_DEPENDENCY_RELATION_BATCH: &str = relation_insert_statement!("dependency_relation");
const INSERT_EXECUTION_RELATION_BATCH: &str = relation_insert_statement!("execution_relation");
const INSERT_DATA_FLOW_RELATION_BATCH: &str = relation_insert_statement!("data_flow_relation");
const INSERT_EVIDENCE_RELATION_BATCH: &str = relation_insert_statement!("evidence_relation");
const PROJECTION_WRITE_BATCH: usize = 512;
const SELECT_NODE_IDS: &str = "SELECT VALUE compassNodeId FROM code_node WHERE repositoryId = $repository AND generationId = $generation ORDER BY compassNodeId LIMIT $limit";
const SELECT_RELATION_IDS: &str = "SELECT VALUE compassEdgeId FROM type::table($table) WHERE repositoryId = $repository AND generationId = $generation ORDER BY compassEdgeId LIMIT $limit";
const SELECT_NODES: &str = "SELECT * OMIT id FROM code_node WHERE repositoryId = $repository AND generationId = $generation ORDER BY compassNodeId LIMIT $limit";
const SELECT_RELATIONS: &str = "SELECT * OMIT id, in, out FROM type::table($table) WHERE repositoryId = $repository AND generationId = $generation ORDER BY compassEdgeId LIMIT $limit";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationOutcome {
    pub generation_id: String,
    pub nodes: usize,
    pub relations: usize,
    pub already_present: bool,
}

/// Test fault-injection point counted across node and relation mutations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptAfter(pub usize);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerationManifest {
    repository_id: String,
    generation_id: String,
    schema_version: String,
    projection_fingerprint: String,
    source_tree_digest: String,
    schema_fingerprint: String,
    node_count: usize,
    relation_count: usize,
    projected_bytes: u64,
    complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivePointer {
    repository_id: String,
    generation_id: String,
    schema_version: String,
    projection_fingerprint: String,
}

/// A local SurrealDB projection client with a closed operation surface.
pub struct SurrealProjection {
    database: Surreal<Db>,
    limits: ProjectionLimits,
}

impl SurrealProjection {
    #[cfg(feature = "mem")]
    pub async fn memory(namespace: &str, database: &str) -> Result<Self, ProjectionError> {
        Self::memory_with_limits(namespace, database, ProjectionLimits::default()).await
    }

    #[cfg(feature = "mem")]
    pub async fn memory_with_limits(
        namespace: &str,
        database: &str,
        limits: ProjectionLimits,
    ) -> Result<Self, ProjectionError> {
        use surrealdb::engine::local::Mem;

        let client = Surreal::new::<Mem>(())
            .await
            .map_err(|error| database_error("connect_mem", error))?;
        Self::initialize(client, namespace, database, limits).await
    }

    #[cfg(feature = "surrealkv")]
    pub async fn surrealkv(
        path: &str,
        namespace: &str,
        database: &str,
    ) -> Result<Self, ProjectionError> {
        Self::surrealkv_with_limits(path, namespace, database, ProjectionLimits::default()).await
    }

    #[cfg(feature = "surrealkv")]
    pub async fn surrealkv_with_limits(
        path: &str,
        namespace: &str,
        database: &str,
        limits: ProjectionLimits,
    ) -> Result<Self, ProjectionError> {
        use surrealdb::engine::local::SurrealKv;

        let client = Surreal::new::<SurrealKv>(path)
            .await
            .map_err(|error| database_error("connect_surrealkv", error))?;
        Self::initialize(client, namespace, database, limits).await
    }

    #[cfg(feature = "rocksdb")]
    pub async fn rocksdb(
        path: &str,
        namespace: &str,
        database: &str,
    ) -> Result<Self, ProjectionError> {
        Self::rocksdb_with_limits(path, namespace, database, ProjectionLimits::default()).await
    }

    #[cfg(feature = "rocksdb")]
    pub async fn rocksdb_with_limits(
        path: &str,
        namespace: &str,
        database: &str,
        limits: ProjectionLimits,
    ) -> Result<Self, ProjectionError> {
        use surrealdb::engine::local::RocksDb;

        let client = Surreal::new::<RocksDb>(path)
            .await
            .map_err(|error| database_error("connect_rocksdb", error))?;
        Self::initialize(client, namespace, database, limits).await
    }

    async fn initialize(
        client: Surreal<Db>,
        namespace: &str,
        database: &str,
        limits: ProjectionLimits,
    ) -> Result<Self, ProjectionError> {
        if namespace.trim().is_empty() || database.trim().is_empty() {
            return Err(ProjectionError::InvalidPlan(
                "namespace and database must not be empty".to_owned(),
            ));
        }
        client
            .use_ns(namespace)
            .use_db(database)
            .await
            .map_err(|error| database_error("select_namespace_database", error))?;
        client
            .query(SCHEMA)
            .await
            .and_then(IndexedResults::check)
            .map_err(|error| database_error("define_schema", error))?;
        Ok(Self {
            database: client,
            limits,
        })
    }

    pub async fn activate(
        &self,
        plan: &ProjectionPlan,
    ) -> Result<ActivationOutcome, ProjectionError> {
        self.activate_with_interrupt(plan, None).await
    }

    pub async fn activate_with_interrupt(
        &self,
        plan: &ProjectionPlan,
        interrupt: Option<InterruptAfter>,
    ) -> Result<ActivationOutcome, ProjectionError> {
        plan.validate()?;
        let projected_bytes = self.limits.validate_plan(plan)?;
        stage_generation(&self.database, plan, projected_bytes, interrupt).await
    }

    pub async fn active_generation(
        &self,
        repository_id: &str,
    ) -> Result<Option<String>, ProjectionError> {
        let pointer = select_record::<ActivePointer>(
            &self.database,
            RecordId::new("repository_pointer", pointer_key(repository_id)),
            "read_active_pointer",
        )
        .await?;
        Ok(pointer.map(|pointer| pointer.generation_id))
    }

    pub async fn read_active_projection(
        &self,
        repository_id: &str,
    ) -> Result<Option<ProjectionPlan>, ProjectionError> {
        let Some(pointer) = select_record::<ActivePointer>(
            &self.database,
            RecordId::new("repository_pointer", pointer_key(repository_id)),
            "read_active_pointer",
        )
        .await?
        else {
            return Ok(None);
        };
        let manifest = select_record::<GenerationManifest>(
            &self.database,
            RecordId::new(
                "generation_manifest",
                manifest_key(repository_id, &pointer.generation_id),
            ),
            "read_generation_manifest",
        )
        .await?
        .ok_or_else(|| {
            ProjectionError::InvalidPlan("active generation manifest is missing".to_owned())
        })?;
        if !manifest.complete || manifest.schema_version != PROJECTION_SCHEMA_V1 {
            return Err(ProjectionError::InvalidPlan(
                "active generation manifest is incomplete or unsupported".to_owned(),
            ));
        }
        validate_manifest_limits(&manifest, self.limits)?;
        let nodes = read_nodes(
            &self.database,
            repository_id,
            &pointer.generation_id,
            self.limits.max_nodes(),
        )
        .await?;
        let mut relations = Vec::new();
        for family in RelationFamily::ALL {
            let remaining = self.limits.max_relations().saturating_sub(relations.len());
            relations.extend(
                read_relations(
                    &self.database,
                    repository_id,
                    &pointer.generation_id,
                    family,
                    remaining,
                )
                .await?,
            );
        }
        relations.sort_by(|left, right| left.compass_edge_id.cmp(&right.compass_edge_id));
        let plan = ProjectionPlan {
            repository_id: repository_id.to_owned(),
            generation_id: pointer.generation_id,
            schema_version: manifest.schema_version,
            source_tree_digest: manifest.source_tree_digest,
            schema_fingerprint: manifest.schema_fingerprint,
            projection_fingerprint: manifest.projection_fingerprint,
            nodes,
            relations,
        };
        plan.validate()?;
        let projected_bytes = self.limits.validate_plan(&plan)?;
        if plan.nodes.len() != manifest.node_count
            || plan.relations.len() != manifest.relation_count
            || projected_bytes != manifest.projected_bytes
        {
            return Err(ProjectionError::InvalidPlan(
                "active generation records do not match the manifest".to_owned(),
            ));
        }
        Ok(Some(plan))
    }
}

async fn stage_generation(
    database: &Surreal<Db>,
    plan: &ProjectionPlan,
    projected_bytes: u64,
    interrupt: Option<InterruptAfter>,
) -> Result<ActivationOutcome, ProjectionError> {
    let manifest_id = RecordId::new(
        "generation_manifest",
        manifest_key(&plan.repository_id, &plan.generation_id),
    );
    let incomplete_manifest = GenerationManifest {
        repository_id: plan.repository_id.clone(),
        generation_id: plan.generation_id.clone(),
        schema_version: plan.schema_version.clone(),
        projection_fingerprint: plan.projection_fingerprint.clone(),
        source_tree_digest: plan.source_tree_digest.clone(),
        schema_fingerprint: plan.schema_fingerprint.clone(),
        node_count: plan.nodes.len(),
        relation_count: plan.relations.len(),
        projected_bytes,
        complete: false,
    };
    claim_generation(database, manifest_id.clone(), &incomplete_manifest).await?;
    let existing = select_record::<GenerationManifest>(
        database,
        manifest_id.clone(),
        "read_candidate_manifest",
    )
    .await?
    .ok_or_else(|| ProjectionError::InvalidPlan("generation claim is missing".to_owned()))?;
    if !manifest_matches(&existing, &incomplete_manifest) {
        return Err(ProjectionError::InvalidPlan(
            "immutable generation already exists with different content".to_owned(),
        ));
    }
    if existing.complete {
        validate_candidate(database, plan).await?;
        upsert_pointer(database, plan).await?;
        return Ok(ActivationOutcome {
            generation_id: plan.generation_id.clone(),
            nodes: plan.nodes.len(),
            relations: plan.relations.len(),
            already_present: true,
        });
    }

    let mut mutations = 0_usize;
    let mut node_offset = 0_usize;
    while node_offset < plan.nodes.len() {
        interrupt_if_requested(interrupt, mutations)?;
        let batch_len = write_batch_len(interrupt, mutations, plan.nodes.len() - node_offset);
        insert_node_batch(database, &plan.nodes[node_offset..node_offset + batch_len]).await?;
        node_offset += batch_len;
        mutations += batch_len;
    }
    let mut relation_offset = 0_usize;
    while relation_offset < plan.relations.len() {
        interrupt_if_requested(interrupt, mutations)?;
        let family = plan.relations[relation_offset].family;
        let family_remaining = plan.relations[relation_offset..]
            .iter()
            .take_while(|relation| relation.family == family)
            .count();
        let batch_len = write_batch_len(interrupt, mutations, family_remaining);
        insert_relation_batch(
            database,
            &plan.relations[relation_offset..relation_offset + batch_len],
        )
        .await?;
        relation_offset += batch_len;
        mutations += batch_len;
    }
    interrupt_if_requested(interrupt, mutations)?;
    validate_candidate(database, plan).await?;
    let manifest = GenerationManifest {
        complete: true,
        ..incomplete_manifest
    };
    upsert_payload(
        database,
        manifest_id,
        serde_json::to_value(manifest)?,
        "write_generation_manifest",
    )
    .await?;
    upsert_pointer(database, plan).await?;
    Ok(ActivationOutcome {
        generation_id: plan.generation_id.clone(),
        nodes: plan.nodes.len(),
        relations: plan.relations.len(),
        already_present: false,
    })
}

fn manifest_matches(left: &GenerationManifest, right: &GenerationManifest) -> bool {
    left.repository_id == right.repository_id
        && left.generation_id == right.generation_id
        && left.schema_version == right.schema_version
        && left.projection_fingerprint == right.projection_fingerprint
        && left.source_tree_digest == right.source_tree_digest
        && left.schema_fingerprint == right.schema_fingerprint
        && left.node_count == right.node_count
        && left.relation_count == right.relation_count
        && left.projected_bytes == right.projected_bytes
}

async fn claim_generation(
    database: &Surreal<Db>,
    record: RecordId,
    manifest: &GenerationManifest,
) -> Result<(), ProjectionError> {
    let payload = database_record_value(serde_json::to_value(manifest)?, record, None)?;
    database
        .query(CLAIM_GENERATION)
        .bind(("payload", payload))
        .await
        .and_then(IndexedResults::check)
        .map(|_response| ())
        .map_err(|error| database_error("claim_generation", error))
}

fn interrupt_if_requested(
    interrupt: Option<InterruptAfter>,
    mutations: usize,
) -> Result<(), ProjectionError> {
    if interrupt.is_some_and(|interrupt| mutations >= interrupt.0) {
        return Err(ProjectionError::Interrupted {
            completed_mutations: mutations,
        });
    }
    Ok(())
}

fn write_batch_len(
    interrupt: Option<InterruptAfter>,
    completed_mutations: usize,
    remaining: usize,
) -> usize {
    let until_interrupt = interrupt
        .map(|value| value.0.saturating_sub(completed_mutations))
        .unwrap_or(PROJECTION_WRITE_BATCH);
    remaining.min(PROJECTION_WRITE_BATCH).min(until_interrupt)
}

async fn insert_node_batch(
    database: &Surreal<Db>,
    nodes: &[ProjectedNode],
) -> Result<(), ProjectionError> {
    let payloads = nodes
        .iter()
        .map(|node| {
            database_record_value(
                serde_json::to_value(node)?,
                RecordId::new("code_node", node.record_key.clone()),
                None,
            )
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    insert_batch(database, INSERT_NODE_BATCH, payloads, "write_node_batch").await
}

async fn insert_relation_batch(
    database: &Surreal<Db>,
    relations: &[ProjectedRelation],
) -> Result<(), ProjectionError> {
    let Some(first) = relations.first() else {
        return Ok(());
    };
    if relations
        .iter()
        .any(|relation| relation.family != first.family)
    {
        return Err(ProjectionError::InvalidPlan(
            "one projection write batch crossed relation families".to_owned(),
        ));
    }
    let statement = match first.family {
        RelationFamily::Structural => INSERT_STRUCTURAL_RELATION_BATCH,
        RelationFamily::Dependency => INSERT_DEPENDENCY_RELATION_BATCH,
        RelationFamily::Execution => INSERT_EXECUTION_RELATION_BATCH,
        RelationFamily::DataFlow => INSERT_DATA_FLOW_RELATION_BATCH,
        RelationFamily::Evidence => INSERT_EVIDENCE_RELATION_BATCH,
    };
    let payloads = relations
        .iter()
        .map(|relation| {
            database_record_value(
                serde_json::to_value(relation)?,
                RecordId::new(relation.family.as_str(), relation.record_key.clone()),
                Some((
                    RecordId::new("code_node", relation.source_record_key.clone()),
                    RecordId::new("code_node", relation.target_record_key.clone()),
                )),
            )
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    insert_batch(database, statement, payloads, "write_relation_batch").await
}

fn database_record_value(
    payload: Value,
    id: RecordId,
    endpoints: Option<(RecordId, RecordId)>,
) -> Result<DatabaseValue, ProjectionError> {
    let mut value = payload.into_value();
    let DatabaseValue::Object(object) = &mut value else {
        return Err(ProjectionError::InvalidPlan(
            "projected database payload is not an object".to_owned(),
        ));
    };
    object.insert("id", id);
    if let Some((source, target)) = endpoints {
        object.insert("in", source);
        object.insert("out", target);
    }
    Ok(value)
}

async fn insert_batch(
    database: &Surreal<Db>,
    statement: &'static str,
    payloads: Vec<DatabaseValue>,
    stage: &'static str,
) -> Result<(), ProjectionError> {
    database
        .query(statement)
        .bind(("payloads", payloads))
        .await
        .and_then(IndexedResults::check)
        .map(|_response| ())
        .map_err(|error| database_error(stage, error))
}

async fn upsert_pointer(
    database: &Surreal<Db>,
    plan: &ProjectionPlan,
) -> Result<(), ProjectionError> {
    let pointer = ActivePointer {
        repository_id: plan.repository_id.clone(),
        generation_id: plan.generation_id.clone(),
        schema_version: plan.schema_version.clone(),
        projection_fingerprint: plan.projection_fingerprint.clone(),
    };
    upsert_payload(
        database,
        RecordId::new("repository_pointer", pointer_key(&plan.repository_id)),
        serde_json::to_value(pointer)?,
        "activate_generation",
    )
    .await
}

async fn upsert_payload(
    database: &Surreal<Db>,
    record: RecordId,
    payload: Value,
    stage: &'static str,
) -> Result<(), ProjectionError> {
    database
        .query(UPSERT_RECORD)
        .bind(("record", record))
        .bind(("payload", payload))
        .await
        .and_then(IndexedResults::check)
        .map(|_response| ())
        .map_err(|error| database_error(stage, error))
}

async fn validate_candidate(
    database: &Surreal<Db>,
    plan: &ProjectionPlan,
) -> Result<(), ProjectionError> {
    let node_ids = query_ids(
        database,
        SELECT_NODE_IDS,
        &plan.repository_id,
        &plan.generation_id,
        None,
        plan.nodes.len(),
        "validate_node_ids",
    )
    .await?;
    let expected_nodes = plan
        .nodes
        .iter()
        .map(|node| node.compass_node_id.clone())
        .collect::<Vec<_>>();
    if node_ids != expected_nodes {
        return Err(ProjectionError::InvalidPlan(
            "staged node identities do not match the candidate".to_owned(),
        ));
    }
    let mut relation_ids = Vec::new();
    for family in RelationFamily::ALL {
        let remaining = plan.relations.len().saturating_sub(relation_ids.len());
        relation_ids.extend(
            query_ids(
                database,
                SELECT_RELATION_IDS,
                &plan.repository_id,
                &plan.generation_id,
                Some(family),
                remaining,
                "validate_relation_ids",
            )
            .await?,
        );
    }
    relation_ids.sort();
    let expected_relations = plan
        .relations
        .iter()
        .map(|relation| relation.compass_edge_id.clone())
        .collect::<Vec<_>>();
    if relation_ids != expected_relations {
        return Err(ProjectionError::InvalidPlan(
            "staged relation identities do not match the candidate".to_owned(),
        ));
    }
    Ok(())
}

async fn query_ids(
    database: &Surreal<Db>,
    statement: &'static str,
    repository_id: &str,
    generation_id: &str,
    family: Option<RelationFamily>,
    limit: usize,
    stage: &'static str,
) -> Result<Vec<String>, ProjectionError> {
    let mut query = database
        .query(statement)
        .bind(("repository", repository_id))
        .bind(("generation", generation_id))
        .bind(("limit", plus_one(limit)?));
    if let Some(family) = family {
        query = query.bind(("table", family.as_str()));
    }
    let mut response = query.await.map_err(|error| database_error(stage, error))?;
    response
        .take(0)
        .map_err(|error| database_error(stage, error))
}

async fn select_record<T>(
    database: &Surreal<Db>,
    record: RecordId,
    stage: &'static str,
) -> Result<Option<T>, ProjectionError>
where
    T: for<'de> Deserialize<'de>,
{
    let value: Option<Value> = database
        .select(record)
        .await
        .map_err(|error| database_error(stage, error))?;
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

async fn read_nodes(
    database: &Surreal<Db>,
    repository_id: &str,
    generation_id: &str,
    limit: usize,
) -> Result<Vec<ProjectedNode>, ProjectionError> {
    query_projected(
        database,
        SELECT_NODES,
        repository_id,
        generation_id,
        None,
        limit,
        "read_nodes",
    )
    .await
}

async fn read_relations(
    database: &Surreal<Db>,
    repository_id: &str,
    generation_id: &str,
    family: RelationFamily,
    limit: usize,
) -> Result<Vec<ProjectedRelation>, ProjectionError> {
    query_projected(
        database,
        SELECT_RELATIONS,
        repository_id,
        generation_id,
        Some(family),
        limit,
        "read_relations",
    )
    .await
}

async fn query_projected<T>(
    database: &Surreal<Db>,
    statement: &'static str,
    repository_id: &str,
    generation_id: &str,
    family: Option<RelationFamily>,
    limit: usize,
    stage: &'static str,
) -> Result<Vec<T>, ProjectionError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut query = database
        .query(statement)
        .bind(("repository", repository_id))
        .bind(("generation", generation_id))
        .bind(("limit", plus_one(limit)?));
    if let Some(family) = family {
        query = query.bind(("table", family.as_str()));
    }
    let mut response = query.await.map_err(|error| database_error(stage, error))?;
    let values: Vec<Value> = response
        .take(0)
        .map_err(|error| database_error(stage, error))?;
    values
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

fn validate_manifest_limits(
    manifest: &GenerationManifest,
    limits: ProjectionLimits,
) -> Result<(), ProjectionError> {
    for (resource, actual, limit) in [
        ("nodes", manifest.node_count, limits.max_nodes()),
        ("relations", manifest.relation_count, limits.max_relations()),
    ] {
        if actual > limit {
            return Err(ProjectionError::LimitExceeded {
                resource,
                actual: u64::try_from(actual).unwrap_or(u64::MAX),
                limit: u64::try_from(limit).unwrap_or(u64::MAX),
            });
        }
    }
    if manifest.projected_bytes > limits.max_projected_bytes() {
        return Err(ProjectionError::LimitExceeded {
            resource: "projected bytes",
            actual: manifest.projected_bytes,
            limit: limits.max_projected_bytes(),
        });
    }
    Ok(())
}

fn plus_one(limit: usize) -> Result<usize, ProjectionError> {
    limit.checked_add(1).ok_or(ProjectionError::LimitExceeded {
        resource: "query rows",
        actual: u64::MAX,
        limit: u64::try_from(limit).unwrap_or(u64::MAX),
    })
}

fn pointer_key(repository_id: &str) -> String {
    record_key("repository", &[repository_id])
}

fn manifest_key(repository_id: &str, generation_id: &str) -> String {
    record_key("generation", &[repository_id, generation_id])
}

fn database_error(stage: &'static str, error: impl std::fmt::Display) -> ProjectionError {
    ProjectionError::Database {
        stage,
        message: error.to_string(),
    }
}
