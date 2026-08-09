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

use compass_languages::{RawNodeRecord, make_id};
use rayon::join;
use serde_json::{Map, Value};

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
    if universal_framework_targets_are_materialized(extraction) {
        let targets = target_index::FrameworkTargetIndex::new_with_root(extraction, Some(root));
        return join(
            || routes::resolve_routes_with_targets(extraction, limits, &targets, Some(root)),
            || domain::resolve_domains_with_targets(extraction, limits, &targets),
        );
    }
    let target_extraction = materialize_universal_framework_targets(extraction);
    let targets = target_index::FrameworkTargetIndex::new_with_root(&target_extraction, Some(root));
    join(
        || routes::resolve_routes_with_targets(&target_extraction, limits, &targets, Some(root)),
        || domain::resolve_domains_with_targets(&target_extraction, limits, &targets),
    )
}

fn universal_framework_targets_are_materialized(
    extraction: &compass_languages::Extraction,
) -> bool {
    let Some(batch) = extraction
        .semantic_evidence
        .as_ref()
        .filter(|batch| matches!(batch.adapter.language.as_str(), "javascript" | "typescript"))
    else {
        return true;
    };
    let existing = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut graph_id_counts = std::collections::BTreeMap::<&str, usize>::new();
    for declaration in &batch.declarations {
        *graph_id_counts
            .entry(declaration.graph_node_id.as_str())
            .or_default() += 1;
    }
    batch.declarations.iter().all(|declaration| {
        if graph_id_counts
            .get(declaration.graph_node_id.as_str())
            .copied()
            == Some(1)
        {
            existing.contains(declaration.graph_node_id.as_str())
        } else {
            let id = make_id(&[&declaration.graph_node_id, &declaration.id]);
            existing.contains(id.as_str())
        }
    })
}

/// Universal TypeScript/JavaScript extraction publishes declaration evidence
/// first and lets the project resolver materialize graph nodes. Framework
/// route/domain resolution can also be invoked directly on a single-file
/// extraction, so provide the target index with source-backed declaration
/// identities at this boundary without changing the universal evidence batch.
pub(super) fn materialize_universal_framework_targets(
    extraction: &compass_languages::Extraction,
) -> compass_languages::Extraction {
    let Some(batches) = extraction
        .semantic_evidence
        .as_ref()
        .filter(|batch| matches!(batch.adapter.language.as_str(), "javascript" | "typescript"))
    else {
        return extraction.clone();
    };
    let mut enriched = extraction.clone();
    let existing = enriched
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let mut graph_id_counts = std::collections::BTreeMap::<&str, usize>::new();
    for declaration in &batches.declarations {
        *graph_id_counts
            .entry(declaration.graph_node_id.as_str())
            .or_default() += 1;
    }
    for declaration in &batches.declarations {
        let id = if graph_id_counts
            .get(declaration.graph_node_id.as_str())
            .copied()
            == Some(1)
        {
            declaration.graph_node_id.clone()
        } else {
            make_id(&[&declaration.graph_node_id, &declaration.id])
        };
        if existing.contains(&id) {
            continue;
        }
        let mut attributes = Map::from_iter([
            ("label".to_owned(), Value::String(declaration.name.clone())),
            ("name".to_owned(), Value::String(declaration.name.clone())),
            (
                "qualified_name".to_owned(),
                Value::String(declaration.qualified_name.clone()),
            ),
            (
                "symbol_kind".to_owned(),
                Value::String(declaration.kind.clone()),
            ),
            (
                "source_file".to_owned(),
                Value::String(declaration.range.source_file.clone()),
            ),
            (
                "source_location".to_owned(),
                Value::String(format!("L{}", declaration.range.start_line)),
            ),
            (
                "start_byte".to_owned(),
                Value::from(declaration.range.start_byte),
            ),
            (
                "end_byte".to_owned(),
                Value::from(declaration.range.end_byte),
            ),
            (
                "line_start".to_owned(),
                Value::from(declaration.range.start_line),
            ),
            (
                "line_end".to_owned(),
                Value::from(declaration.range.end_line),
            ),
            (
                "column_start".to_owned(),
                Value::from(declaration.range.start_column),
            ),
            (
                "column_end".to_owned(),
                Value::from(declaration.range.end_column),
            ),
            (
                "language".to_owned(),
                Value::String(declaration.language.clone()),
            ),
            (
                "extractor".to_owned(),
                Value::String(format!(
                    "compass.languages.{}.universal",
                    declaration.language
                )),
            ),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            ("_origin".to_owned(), Value::String("ast".to_owned())),
        ]);
        if let Some(signature) = declaration.signature.as_ref() {
            attributes.insert("signature".to_owned(), Value::String(signature.clone()));
        }
        enriched.nodes.push(RawNodeRecord { id, attributes });
    }
    enriched
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
