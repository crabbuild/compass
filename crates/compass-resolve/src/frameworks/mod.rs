mod domain;
mod jvm;
mod native;
mod php;
mod python;
mod routes;
mod ruby;
mod target_index;
mod typescript;

pub use domain::{
    ResolvedDomainFact, publish_resolved_domains, resolve_and_publish_framework_domains,
    resolve_domains,
};
pub use routes::{
    FrameworkResolutionError, ResolvedRoute, RouteStage, RouteStageRole, publish_resolved_routes,
    resolve_and_publish_framework_routes, resolve_routes,
};

pub(crate) fn resolve_framework_facts(
    extraction: &compass_languages::Extraction,
    limits: compass_languages::FrameworkLimits,
    root: &std::path::Path,
) -> (
    Result<Vec<ResolvedRoute>, FrameworkResolutionError>,
    Result<Vec<ResolvedDomainFact>, FrameworkResolutionError>,
) {
    let targets = target_index::FrameworkTargetIndex::new_with_root(extraction, Some(root));
    (
        routes::resolve_routes_with_targets(extraction, limits, &targets),
        domain::resolve_domains_with_targets(extraction, limits, &targets),
    )
}
