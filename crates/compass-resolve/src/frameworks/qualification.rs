use compass_languages::{Extraction, FrameworkLimits};
use compass_model::provenance::ResolutionState;

use super::{FrameworkResolutionError, ResolvedRoute, resolve_routes};

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
    let resolved_routes = resolve_routes(extraction, limits)?;
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
