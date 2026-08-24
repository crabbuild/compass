use std::collections::BTreeSet;
use std::path::{Component, Path};

use compass_languages::{Extraction, FrameworkLimits};
use compass_model::provenance::ResolutionState;
use serde::{Deserialize, Serialize};

use super::{
    FrameworkResolutionError, ResolvedRoute, materialize_universal_framework_targets,
    resolve_routes,
};

/// A route assertion reusable by framework fixtures and external qualification
/// harnesses. The handler is optional when a convention only promises the
/// endpoint shape (for example a Next.js page component).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameworkRouteExpectation {
    pub framework: String,
    pub operation: String,
    pub normalized_path: String,
    pub handler_reference: Option<String>,
}

impl FrameworkRouteExpectation {
    #[must_use]
    pub fn new(
        framework: impl Into<String>,
        operation: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            framework: framework.into(),
            operation: operation.into(),
            normalized_path: path.into(),
            handler_reference: None,
        }
    }

    #[must_use]
    pub fn with_handler(mut self, handler: impl Into<String>) -> Self {
        self.handler_reference = Some(handler.into());
        self
    }
}

/// A named, deterministic qualification case. Cases are intentionally data
/// only so the same runner can consume Rust fixtures, Java fixtures, or
/// generated framework repositories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameworkQualificationCase {
    pub id: String,
    pub expectations: Vec<FrameworkRouteExpectation>,
}

/// Versioned, extractor-independent expectations for relationship and role
/// evidence. The JSON form is intentionally stricter than the convenience
/// route case so a corpus cannot silently drift when a producer changes.
pub const FRAMEWORK_EVIDENCE_EXPECTATIONS_SCHEMA: &str = "compass.framework-evidence/1";
pub const MAX_FRAMEWORK_EVIDENCE_EXPECTATIONS: usize = 100_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkEvidenceExpectationSet {
    pub schema: String,
    pub corpus_id: String,
    pub records: Vec<FrameworkEvidenceExpectation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkEvidenceExpectation {
    pub id: String,
    pub source_file: String,
    pub start_byte: u64,
    pub end_byte: u64,
    #[serde(default)]
    pub source_identity: Option<String>,
    #[serde(default)]
    pub target_identity: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub framework: Option<String>,
    #[serde(default)]
    pub route_path: Option<String>,
    #[serde(default)]
    pub producer: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub expected_ambiguity: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameworkEvidenceExpectationError {
    #[error("unsupported framework evidence expectation schema {schema:?}")]
    UnsupportedSchema { schema: String },
    #[error("framework evidence corpus ID must not be empty")]
    EmptyCorpusId,
    #[error("framework evidence expectation count {observed} exceeds maximum {maximum}")]
    TooManyRecords { observed: usize, maximum: usize },
    #[error("duplicate framework evidence expectation ID {id:?}")]
    DuplicateId { id: String },
    #[error("framework evidence expectation {id:?} has an empty source file")]
    EmptySourceFile { id: String },
    #[error("framework evidence expectation {id:?} has a path outside the corpus root")]
    SourceOutsideRoot { id: String },
    #[error("framework evidence expectation {id:?} has an invalid byte range")]
    InvalidRange { id: String },
}

impl FrameworkEvidenceExpectationSet {
    /// Validate the machine contract and every source range before a runner
    /// is allowed to compare a corpus with published graph evidence.
    pub fn validate(&self, corpus_root: &Path) -> Result<(), FrameworkEvidenceExpectationError> {
        if self.schema != FRAMEWORK_EVIDENCE_EXPECTATIONS_SCHEMA {
            return Err(FrameworkEvidenceExpectationError::UnsupportedSchema {
                schema: self.schema.clone(),
            });
        }
        if self.corpus_id.trim().is_empty() {
            return Err(FrameworkEvidenceExpectationError::EmptyCorpusId);
        }
        if self.records.len() > MAX_FRAMEWORK_EVIDENCE_EXPECTATIONS {
            return Err(FrameworkEvidenceExpectationError::TooManyRecords {
                observed: self.records.len(),
                maximum: MAX_FRAMEWORK_EVIDENCE_EXPECTATIONS,
            });
        }
        let mut ids = BTreeSet::new();
        for record in &self.records {
            if !ids.insert(record.id.as_str()) {
                return Err(FrameworkEvidenceExpectationError::DuplicateId {
                    id: record.id.clone(),
                });
            }
            if record.source_file.trim().is_empty() {
                return Err(FrameworkEvidenceExpectationError::EmptySourceFile {
                    id: record.id.clone(),
                });
            }
            let path = Path::new(&record.source_file);
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(FrameworkEvidenceExpectationError::SourceOutsideRoot {
                    id: record.id.clone(),
                });
            }
            if record.start_byte >= record.end_byte {
                return Err(FrameworkEvidenceExpectationError::InvalidRange {
                    id: record.id.clone(),
                });
            }
            let joined = corpus_root.join(path);
            if !joined.starts_with(corpus_root) {
                return Err(FrameworkEvidenceExpectationError::SourceOutsideRoot {
                    id: record.id.clone(),
                });
            }
        }
        Ok(())
    }
}

impl FrameworkQualificationCase {
    #[must_use]
    pub fn new(id: impl Into<String>, expectations: Vec<FrameworkRouteExpectation>) -> Self {
        Self {
            id: id.into(),
            expectations,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameworkQualificationReport {
    pub case_id: String,
    pub expected_routes: usize,
    pub matched_routes: usize,
    pub resolved_routes: Vec<ResolvedRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameworkQualificationError {
    #[error(transparent)]
    Resolution(#[from] FrameworkResolutionError),
    #[error("qualification case {case_id:?} is missing {framework} {operation} {normalized_path}")]
    MissingRoute {
        case_id: String,
        framework: String,
        operation: String,
        normalized_path: String,
    },
    #[error(
        "qualification case {case_id:?} found a non-exact {state:?} {framework} {operation} {normalized_path}"
    )]
    NonExactRoute {
        case_id: String,
        framework: String,
        operation: String,
        normalized_path: String,
        state: ResolutionState,
    },
    #[error(
        "qualification case {case_id:?} found multiple exact {framework} {operation} {normalized_path} routes"
    )]
    DuplicateRoute {
        case_id: String,
        framework: String,
        operation: String,
        normalized_path: String,
    },
    #[error(
        "qualification case {case_id:?} expected handler {expected_handler:?} for {framework} {operation} {normalized_path}, found {actual_handler:?}"
    )]
    HandlerMismatch {
        case_id: String,
        framework: String,
        operation: String,
        normalized_path: String,
        expected_handler: String,
        actual_handler: String,
    },
}

/// Resolve and validate a framework case without publishing graph nodes or
/// edges. A case succeeds only when every expectation has exactly one exact
/// route, making the helper safe for CI qualification and fixture snapshots.
#[allow(clippy::result_large_err)]
pub fn qualify_framework_case(
    extraction: &Extraction,
    limits: FrameworkLimits,
    case: &FrameworkQualificationCase,
) -> Result<FrameworkQualificationReport, FrameworkQualificationError> {
    // A qualifying universal pipeline intentionally keeps the per-file extraction
    // evidence-only: declaration nodes are projected by the collection
    // resolver. Qualification fixtures also exercise a single extracted file,
    // so provide the target index with the same source-backed declaration
    // identities without publishing a second graph route.
    let target_extraction = materialize_universal_framework_targets(extraction);
    let resolved_routes = resolve_routes(&target_extraction, limits)?;
    let mut matched_routes = 0_usize;
    for expected in &case.expectations {
        let candidates = resolved_routes
            .iter()
            .filter(|route| {
                route.route.framework == expected.framework
                    && route.route.operation == expected.operation
                    && route.route.normalized_path == expected.normalized_path
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(FrameworkQualificationError::MissingRoute {
                case_id: case.id.clone(),
                framework: expected.framework.clone(),
                operation: expected.operation.clone(),
                normalized_path: expected.normalized_path.clone(),
            });
        }
        if candidates.len() > 1 {
            return Err(FrameworkQualificationError::DuplicateRoute {
                case_id: case.id.clone(),
                framework: expected.framework.clone(),
                operation: expected.operation.clone(),
                normalized_path: expected.normalized_path.clone(),
            });
        }
        let route = candidates[0];
        if route.state != ResolutionState::Exact {
            return Err(FrameworkQualificationError::NonExactRoute {
                case_id: case.id.clone(),
                framework: expected.framework.clone(),
                operation: expected.operation.clone(),
                normalized_path: expected.normalized_path.clone(),
                state: route.state,
            });
        }
        if let Some(expected_handler) = expected.handler_reference.as_deref()
            && route.route.handler_reference != expected_handler
        {
            return Err(FrameworkQualificationError::HandlerMismatch {
                case_id: case.id.clone(),
                framework: expected.framework.clone(),
                operation: expected.operation.clone(),
                normalized_path: expected.normalized_path.clone(),
                expected_handler: expected_handler.to_owned(),
                actual_handler: route.route.handler_reference.clone(),
            });
        }
        matched_routes += 1;
    }
    Ok(FrameworkQualificationReport {
        case_id: case.id.clone(),
        expected_routes: case.expectations.len(),
        matched_routes,
        resolved_routes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> FrameworkEvidenceExpectation {
        FrameworkEvidenceExpectation {
            id: id.to_owned(),
            source_file: "src/App.tsx".to_owned(),
            start_byte: 0,
            end_byte: 4,
            source_identity: Some("App".to_owned()),
            target_identity: Some("Card".to_owned()),
            relation: Some("renders".to_owned()),
            role: Some("ui_component".to_owned()),
            framework: Some("react".to_owned()),
            route_path: None,
            producer: Some("compass.languages.react-ui".to_owned()),
            origin: Some("ast".to_owned()),
            resolution: Some("exact".to_owned()),
            expected_ambiguity: false,
        }
    }

    #[test]
    fn versioned_expectations_round_trip_and_validate() -> Result<(), Box<dyn std::error::Error>> {
        let set = FrameworkEvidenceExpectationSet {
            schema: FRAMEWORK_EVIDENCE_EXPECTATIONS_SCHEMA.to_owned(),
            corpus_id: "frontend-fixture".to_owned(),
            records: vec![record("render-1")],
        };
        let encoded = serde_json::to_vec(&set)?;
        let decoded: FrameworkEvidenceExpectationSet = serde_json::from_slice(&encoded)?;
        decoded.validate(Path::new("/workspace"))?;
        Ok(())
    }

    #[test]
    fn invalid_version_duplicate_path_and_range_fail_closed() {
        let mut set = FrameworkEvidenceExpectationSet {
            schema: "compass.framework-evidence/2".to_owned(),
            corpus_id: "fixture".to_owned(),
            records: vec![record("same"), record("same")],
        };
        assert!(matches!(
            set.validate(Path::new("/workspace")),
            Err(FrameworkEvidenceExpectationError::UnsupportedSchema { .. })
        ));
        set.schema = FRAMEWORK_EVIDENCE_EXPECTATIONS_SCHEMA.to_owned();
        assert!(matches!(
            set.validate(Path::new("/workspace")),
            Err(FrameworkEvidenceExpectationError::DuplicateId { .. })
        ));
        set.records[1].id = "other".to_owned();
        set.records[0].source_file = "../escape.tsx".to_owned();
        assert!(matches!(
            set.validate(Path::new("/workspace")),
            Err(FrameworkEvidenceExpectationError::SourceOutsideRoot { .. })
        ));
        set.records[0].source_file = "src/App.tsx".to_owned();
        set.records[0].start_byte = 4;
        assert!(matches!(
            set.validate(Path::new("/workspace")),
            Err(FrameworkEvidenceExpectationError::InvalidRange { .. })
        ));
    }
}
