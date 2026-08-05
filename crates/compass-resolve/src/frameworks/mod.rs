mod domain;
mod jvm;
mod native;
mod php;
mod python;
mod qualification;
mod routes;
mod ruby;
mod spring;
mod target_index;
mod typescript;

use rayon::join;

type UniversalFrameworkExpansion =
    fn(&mut compass_languages::Extraction) -> Result<(), FrameworkResolutionError>;

struct UniversalFrameworkPack {
    id: &'static str,
    expand: UniversalFrameworkExpansion,
}

/// Project-wide expansion adapters are registered by pack identity rather
/// than selected through a language-specific match. Adding a universal pack
/// therefore changes one registry entry and leaves the lifecycle unchanged.
const UNIVERSAL_FRAMEWORK_PACKS: &[UniversalFrameworkPack] = &[UniversalFrameworkPack {
    id: "spring-java",
    expand: spring::expand,
}];

pub use domain::{
    ResolvedDomainFact, publish_resolved_domains, resolve_and_publish_framework_domains,
    resolve_domains,
};
pub use qualification::{
    FrameworkQualificationCase, FrameworkQualificationError, FrameworkQualificationReport,
    FrameworkRouteExpectation, qualify_framework_case,
};
pub use routes::{
    FrameworkResolutionError, ResolvedRoute, RouteStage, RouteStageRole, publish_resolved_routes,
    resolve_and_publish_framework_routes, resolve_routes,
};

pub(crate) fn expand_universal_framework_facts(
    extraction: &mut compass_languages::Extraction,
) -> Result<(), FrameworkResolutionError> {
    for pack in UNIVERSAL_FRAMEWORK_PACKS {
        debug_assert!(
            compass_languages::FrameworkPackRegistry::descriptors()
                .iter()
                .any(|descriptor| descriptor.id == pack.id),
            "expansion adapter is not registered by the language pack: {}",
            pack.id
        );
        (pack.expand)(extraction)?;
    }
    Ok(())
}

pub(crate) fn resolve_framework_facts(
    extraction: &compass_languages::Extraction,
    limits: compass_languages::FrameworkLimits,
    root: &std::path::Path,
) -> (
    Result<Vec<ResolvedRoute>, FrameworkResolutionError>,
    Result<Vec<ResolvedDomainFact>, FrameworkResolutionError>,
) {
    if extraction.framework_facts.is_empty() {
        return (Ok(Vec::new()), Ok(Vec::new()));
    }
    let targets = target_index::FrameworkTargetIndex::new_with_root(extraction, Some(root));
    join(
        || routes::resolve_routes_with_targets(extraction, limits, &targets, Some(root)),
        || domain::resolve_domains_with_targets(extraction, limits, &targets),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use compass_languages::FrameworkPackRegistry;
    use compass_languages::{Extraction, FrameworkLimits};

    use super::UNIVERSAL_FRAMEWORK_PACKS;

    #[test]
    fn empty_framework_facts_skip_target_indexing() {
        let (routes, domains) = super::resolve_framework_facts(
            &Extraction::default(),
            FrameworkLimits::default(),
            Path::new("."),
        );
        assert!(routes.is_ok_and(|routes| routes.is_empty()));
        assert!(domains.is_ok_and(|domains| domains.is_empty()));
    }

    #[test]
    fn every_universal_language_pack_has_one_expansion_adapter() {
        let registered = UNIVERSAL_FRAMEWORK_PACKS
            .iter()
            .map(|pack| pack.id)
            .collect::<std::collections::BTreeSet<_>>();
        let descriptors = FrameworkPackRegistry::descriptors();
        for descriptor in descriptors {
            assert!(
                registered.contains(descriptor.id),
                "missing expansion adapter for universal framework pack {}",
                descriptor.id
            );
        }
        for pack in UNIVERSAL_FRAMEWORK_PACKS {
            assert!(
                descriptors
                    .iter()
                    .any(|descriptor| descriptor.id == pack.id),
                "expansion adapter has no universal descriptor: {}",
                pack.id
            );
        }
        assert_eq!(registered.len(), descriptors.len());
    }

    #[test]
    fn expansion_adapter_ids_are_unique() {
        let mut ids = std::collections::BTreeSet::new();
        for pack in UNIVERSAL_FRAMEWORK_PACKS {
            assert!(
                ids.insert(pack.id),
                "duplicate expansion adapter {}",
                pack.id
            );
        }
    }
}
