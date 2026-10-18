//! Bounded immutable source-body recall shared by JSON and local-store queries.
use crate::{CodeQueryEngine, QueryError, QueryErrorKind};
use compass_model::provenance::SourceAnchor;
use compass_model::source_query::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) struct Index {
    pub digest: String,
    profile: String,
    pub owners: Vec<SourceQueryOwner>,
    by_id: BTreeMap<String, usize>,
    postings: BTreeMap<String, Vec<usize>>,
    pub frequencies: BTreeMap<String, u64>,
    pub bounded: bool,
    pub coverage_message: Option<String>,
}
fn corrupt(message: impl Into<String>) -> QueryError {
    QueryError::new(
        QueryErrorKind::CorruptArtifact,
        "source_query_artifact_invalid",
        format!(
            "{}; run compass update --force to rebuild current source evidence",
            message.into()
        ),
    )
}
fn bytes(graph: &Path) -> Result<Option<Vec<u8>>, QueryError> {
    let path = graph
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SOURCE_QUERY_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(corrupt(e.to_string())),
        Ok(m) if !m.file_type().is_file() => {
            return Err(corrupt("source-query artifact must be a regular file"));
        }
        Ok(_) => {}
    }
    compass_files::read_bytes_bounded(&path, SOURCE_QUERY_MAX_BYTES)
        .map(Some)
        .map_err(|e| corrupt(e.to_string()))
}
pub(crate) fn identity(graph: &Path) -> Result<Option<String>, QueryError> {
    Ok(bytes(graph)?.map(|b| format!("{:x}", Sha256::digest(&b))))
}
impl Index {
    fn open(graph: &Path, graph_digest: &str) -> Result<Option<Self>, QueryError> {
        let Some(bytes) = bytes(graph)? else {
            return Ok(None);
        };
        let digest = format!("{:x}", Sha256::digest(&bytes));
        // Inspect the bounded version envelope before decoding current fields.
        // A future major need not contain any of the current payload shape.
        #[derive(serde::Deserialize)]
        struct Header {
            schema: String,
            #[serde(default)]
            profile: Option<String>,
        }
        let header: Header = serde_json::from_slice(&bytes).map_err(|e| corrupt(e.to_string()))?;
        if header.schema != SOURCE_QUERY_SCHEMA {
            return Err(QueryError::new(
                QueryErrorKind::UnsupportedSchema,
                "unsupported_source_query_schema",
                format!(
                    "Unsupported source-query schema {}; update Compass or rebuild current source evidence",
                    header.schema
                ),
            ));
        }
        if header
            .profile
            .as_deref()
            .is_some_and(|profile| !is_supported_source_query_profile(profile))
        {
            return Err(QueryError::new(
                QueryErrorKind::UnsupportedSchema,
                "unsupported_source_query_profile",
                "Unsupported source-query projection profile; update Compass or rebuild current source evidence",
            ));
        }
        let artifact: SourceQueryArtifact =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(e.to_string()))?;
        artifact.validate().map_err(|e| corrupt(e.to_string()))?;
        if artifact.graph_digest != graph_digest {
            return Err(corrupt(
                "source-query artifact is bound to a different graph",
            ));
        }
        let bounded = artifact.aggregate_truncated
            || artifact.omitted_files > 0
            || artifact.omitted_owners > 0;
        let omitted_files = artifact.omitted_files;
        let omitted_owners = artifact.omitted_owners;
        let mut clipped_owners = 0_usize;
        let mut result = Self {
            digest,
            profile: artifact.profile,
            owners: artifact.owners,
            by_id: BTreeMap::new(),
            postings: BTreeMap::new(),
            frequencies: artifact.additional_document_frequencies,
            bounded,
            coverage_message: None,
        };
        let mut count = 0_usize;
        for (at, owner) in result.owners.iter().enumerate() {
            result.by_id.insert(owner.node_id.clone(), at);
            let document = result.document(owner)?;
            clipped_owners += usize::from(document.truncated);
            result.bounded |= document.truncated;
            for term in document.terms() {
                count += 1;
                if count > 262_144
                    || result.postings.len() >= 65_536 && !result.postings.contains_key(&term)
                {
                    return Err(QueryError::new(
                        QueryErrorKind::MemoryLimit,
                        "source_query_index_limit",
                        "Source-body query index exceeds its bounded posting/term capacity; narrow the build scope",
                    ));
                }
                result.postings.entry(term).or_default().push(at);
            }
        }
        if result.bounded {
            result.coverage_message = Some(format!(
                "Callable-body recall is bounded: {omitted_files} source files are omitted or partial, {omitted_owners} AST scopes have no indexed owner, and {clipped_owners} owner projections reached source/field limits (8192 bytes and 512 terms per field). Narrow the build scope to reduce retained evidence"
            ));
        }
        Ok(Some(result))
    }
    pub fn document(
        &self,
        owner: &SourceQueryOwner,
    ) -> Result<compass_model::search::SearchDocument, QueryError> {
        owner
            .document_for_profile(&self.profile)
            .map_err(|e| corrupt(e.to_string()))
    }
    pub fn preference_truncated(&self, owner: &SourceQueryOwner) -> Result<bool, QueryError> {
        // Existing identifier/comment preferences need completeness of those
        // fields, independent of the newer code-window channel. This validated
        // historical projection preserves their byte/term clipping checks.
        let profile = if source_query_uses_code_lexemes(&self.profile) {
            compass_model::source_query::SOURCE_QUERY_PROFILE_V4
        } else {
            &self.profile
        };
        owner
            .document_for_profile(profile)
            .map(|document| document.truncated)
            .map_err(|e| corrupt(e.to_string()))
    }
    pub fn supports_parameter_bindings(&self) -> bool {
        source_query_uses_parameter_bindings(&self.profile)
    }
    pub fn owner(&self, id: &str) -> Option<&SourceQueryOwner> {
        self.by_id.get(id).and_then(|at| self.owners.get(*at))
    }
    pub fn candidates<'a>(&'a self, term: &str, limit: usize) -> (Vec<&'a SourceQueryOwner>, bool) {
        let Some(posting) = self.postings.get(term) else {
            return (Vec::new(), false);
        };
        (
            posting
                .iter()
                .take(limit)
                .filter_map(|at| self.owners.get(*at))
                .collect(),
            posting.len() > limit,
        )
    }
}
pub(crate) fn attach(
    mut engine: CodeQueryEngine,
    graph: &Path,
    cache: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    engine.source_query = Index::open(graph, &engine.graph_identity)?;
    if let Some(index) = &engine.source_query {
        engine.response_cache = crate::response_cache::ResponseCache::open(
            cache,
            &format!("{}:{}", engine.graph_identity, index.digest),
            engine.program.as_ref(),
        );
    }
    Ok(engine)
}
pub(crate) fn matching_sources(
    owner: &SourceQueryOwner,
    terms: &BTreeSet<String>,
    include_code: bool,
) -> (BTreeSet<SourceAnchor>, bool) {
    let mut sources = BTreeSet::new();
    let mut bounded = false;
    let code = if include_code {
        owner.code_windows.as_slice()
    } else {
        &[]
    };
    for site in code.iter().chain(&owner.identifiers).chain(&owner.comments) {
        if compass_model::search::identifier_search_terms(&site.text)
            .into_iter()
            .map(compass_model::canonical_code_token)
            .any(|t| terms.contains(&t))
        {
            if sources.len() >= 8 {
                bounded = true;
                break;
            }
            sources.insert(site.source.clone());
        }
    }
    (sources, bounded)
}
