mod aspnet;
mod axum;
mod dart;
mod domain;
mod jvm;
mod native;
mod node;
mod php;
mod python;
mod qualification;
mod react;
mod relations;
mod routes;
mod ruby;
mod spring;
mod swift;
mod target_index;
mod typescript;

pub(crate) use react::project_render_relations;
pub(crate) use relations::resolve_and_publish as resolve_and_publish_relations;
pub(crate) use typescript::edge_targets_declared_callable;

use compass_languages::{RawNodeRecord, make_id};
use rayon::join;
use serde_json::{Map, Value};

type RouteExpansion = fn(
    &[compass_languages::RawFrameworkFact],
    Vec<compass_languages::RawRouteFact>,
    compass_languages::FrameworkLimits,
) -> Result<Vec<compass_languages::RawRouteFact>, FrameworkResolutionError>;

struct RouteExpansionAdapter {
    pack_id: &'static str,
    frameworks: &'static [&'static str],
    expand: RouteExpansion,
}

/// Framework-owned route composition adapters. The shared route resolver only
/// drives this deterministic registry; framework-specific mount/include rules
/// remain in their owning module.
const ROUTE_EXPANSION_ADAPTERS: &[RouteExpansionAdapter] = &[
    RouteExpansionAdapter {
        pack_id: "axum-web",
        frameworks: &["axum"],
        expand: axum::expand_routes,
    },
    RouteExpansionAdapter {
        pack_id: "django-python",
        frameworks: &["django"],
        expand: python::expand_django_routes,
    },
    RouteExpansionAdapter {
        pack_id: "django-rest-framework-python",
        frameworks: &["django-rest-framework"],
        expand: python::expand_drf_routes,
    },
    RouteExpansionAdapter {
        pack_id: "express-web",
        frameworks: &["express"],
        expand: node::expand_routes,
    },
    RouteExpansionAdapter {
        pack_id: "fastapi-python",
        frameworks: &["fastapi"],
        expand: python::expand_fastapi_routes,
    },
    RouteExpansionAdapter {
        pack_id: "fastify-web",
        frameworks: &["fastify"],
        expand: node::expand_routes,
    },
    RouteExpansionAdapter {
        pack_id: "flask-python",
        frameworks: &["flask"],
        expand: python::expand_flask_routes,
    },
    RouteExpansionAdapter {
        pack_id: "hono-web",
        frameworks: &["hono"],
        expand: node::expand_routes,
    },
    RouteExpansionAdapter {
        pack_id: "starlette-python",
        frameworks: &["starlette"],
        expand: python::expand_starlette_routes,
    },
];

type UniversalFrameworkExpansion =
    fn(&mut compass_languages::Extraction) -> Result<(), FrameworkResolutionError>;

struct UniversalFrameworkPack {
    id: &'static str,
    expand: UniversalFrameworkExpansion,
}

/// Project-wide expansion adapters are registered by pack identity rather
/// than selected through a language-specific match. Adding a universal pack
/// therefore changes one registry entry and leaves the lifecycle unchanged.
const UNIVERSAL_FRAMEWORK_PACKS: &[UniversalFrameworkPack] = &[
    UniversalFrameworkPack {
        id: "aspnet-csharp",
        expand: aspnet::expand,
    },
    UniversalFrameworkPack {
        id: "php-frameworks",
        expand: php::expand,
    },
    UniversalFrameworkPack {
        id: "spring-java",
        expand: spring::expand,
    },
    UniversalFrameworkPack {
        id: "spring-kotlin",
        expand: spring::expand_kotlin,
    },
    UniversalFrameworkPack {
        id: "rails-ruby",
        expand: ruby::expand,
    },
    UniversalFrameworkPack {
        id: "vapor-swift",
        expand: swift::expand,
    },
    UniversalFrameworkPack {
        id: "dart-flutter-navigation",
        expand: dart::expand,
    },
    UniversalFrameworkPack {
        id: "dart-bloc",
        expand: dart::expand,
    },
    UniversalFrameworkPack {
        id: "dart-riverpod",
        expand: dart::expand,
    },
    UniversalFrameworkPack {
        id: "react-ui",
        expand: react::expand,
    },
    UniversalFrameworkPack {
        id: "django-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "django-rest-framework-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "fastapi-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "flask-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "pydantic-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "sqlalchemy-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "celery-python",
        expand: python::expand,
    },
    UniversalFrameworkPack {
        id: "starlette-python",
        expand: python::expand,
    },
];

pub use domain::{
    ResolvedDomainFact, publish_resolved_domains, publish_resolved_domains_with_root,
    resolve_and_publish_framework_domains, resolve_domains,
};
pub use qualification::{
    FRAMEWORK_EVIDENCE_EXPECTATIONS_SCHEMA, FrameworkEvidenceExpectation,
    FrameworkEvidenceExpectationError, FrameworkEvidenceExpectationSet, FrameworkQualificationCase,
    FrameworkQualificationError, FrameworkQualificationReport, FrameworkRouteExpectation,
    MAX_FRAMEWORK_EVIDENCE_EXPECTATIONS, qualify_framework_case,
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

pub(super) fn expand_framework_routes(
    facts: &[compass_languages::RawFrameworkFact],
    limits: compass_languages::FrameworkLimits,
) -> Result<Vec<compass_languages::RawRouteFact>, FrameworkResolutionError> {
    let claimed_frameworks = ROUTE_EXPANSION_ADAPTERS
        .iter()
        .flat_map(|adapter| adapter.frameworks.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    let mut routes = facts
        .iter()
        .filter_map(|fact| match fact {
            compass_languages::RawFrameworkFact::Route(route)
                if !claimed_frameworks.contains(route.framework.as_str()) =>
            {
                Some(route.clone())
            }
            compass_languages::RawFrameworkFact::Domain(_)
            | compass_languages::RawFrameworkFact::Annotation(_)
            | compass_languages::RawFrameworkFact::Role(_)
            | compass_languages::RawFrameworkFact::Relation(_)
            | compass_languages::RawFrameworkFact::Configuration(_)
            | compass_languages::RawFrameworkFact::FileSet(_)
            | compass_languages::RawFrameworkFact::Route(_) => None,
        })
        .collect::<Vec<_>>();

    for adapter in ROUTE_EXPANSION_ADAPTERS {
        let adapter_facts = facts
            .iter()
            .filter(|fact| adapter.frameworks.contains(&fact_framework(fact)))
            .cloned()
            .collect::<Vec<_>>();
        if adapter_facts.is_empty() {
            continue;
        }
        let adapter_routes = adapter_facts
            .iter()
            .filter_map(|fact| match fact {
                compass_languages::RawFrameworkFact::Route(route) => Some(route.clone()),
                compass_languages::RawFrameworkFact::Domain(_)
                | compass_languages::RawFrameworkFact::Annotation(_)
                | compass_languages::RawFrameworkFact::Role(_)
                | compass_languages::RawFrameworkFact::Relation(_)
                | compass_languages::RawFrameworkFact::Configuration(_)
                | compass_languages::RawFrameworkFact::FileSet(_) => None,
            })
            .collect::<Vec<_>>();
        let expanded = (adapter.expand)(&adapter_facts, adapter_routes, limits)?;
        for route in &expanded {
            if !adapter.frameworks.contains(&route.framework.as_str()) {
                return Err(FrameworkResolutionError::InvalidRoute {
                    framework: route.framework.clone(),
                    detail: format!(
                        "route expansion adapter {} emitted a framework it does not own",
                        adapter.pack_id
                    ),
                });
            }
        }
        routes.extend(expanded);
    }
    Ok(routes)
}

fn fact_framework(fact: &compass_languages::RawFrameworkFact) -> &str {
    match fact {
        compass_languages::RawFrameworkFact::Route(route) => &route.framework,
        compass_languages::RawFrameworkFact::Domain(domain) => &domain.framework,
        compass_languages::RawFrameworkFact::Annotation(annotation) => &annotation.framework,
        compass_languages::RawFrameworkFact::Role(role) => &role.framework,
        compass_languages::RawFrameworkFact::Relation(relation) => &relation.framework,
        compass_languages::RawFrameworkFact::Configuration(configuration) => {
            &configuration.framework
        }
        compass_languages::RawFrameworkFact::FileSet(file_set) => &file_set.framework,
    }
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
    let Some(batch) = extraction.semantic_evidence.as_ref().filter(|batch| {
        matches!(
            batch.pipeline.language.as_str(),
            "csharp" | "javascript" | "php" | "ruby" | "typescript"
        )
    }) else {
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

/// Universal C#/PHP/TypeScript/JavaScript extraction publishes declaration evidence
/// first and lets the project resolver materialize graph nodes. Framework
/// route/domain resolution can also be invoked directly on a single-file
/// extraction, so provide the target index with source-backed declaration
/// identities at this boundary without changing the universal evidence batch.
pub(super) fn materialize_universal_framework_targets(
    extraction: &compass_languages::Extraction,
) -> compass_languages::Extraction {
    let Some(batches) = extraction.semantic_evidence.as_ref().filter(|batch| {
        matches!(
            batch.pipeline.language.as_str(),
            "csharp" | "javascript" | "php" | "ruby" | "typescript"
        )
    }) else {
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

    use compass_languages::{
        Extraction, FrameworkLimits, FrameworkPackRegistry, RawFrameworkAnchor, RawFrameworkFact,
        RawFrameworkOrigin, RawRouteFact,
    };

    use super::{ROUTE_EXPANSION_ADAPTERS, UNIVERSAL_FRAMEWORK_PACKS, expand_framework_routes};

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

    #[test]
    fn route_expansion_adapters_have_unique_pack_and_framework_ownership() {
        let mut pack_ids = std::collections::BTreeSet::new();
        let mut frameworks = std::collections::BTreeSet::new();
        for adapter in ROUTE_EXPANSION_ADAPTERS {
            assert!(
                pack_ids.insert(adapter.pack_id),
                "duplicate route expansion adapter {}",
                adapter.pack_id
            );
            assert!(
                !adapter.frameworks.is_empty(),
                "route expansion adapter {} has no activation frameworks",
                adapter.pack_id
            );
            assert!(
                adapter.frameworks.windows(2).all(|pair| pair[0] < pair[1]),
                "route expansion adapter {} frameworks must be sorted and unique",
                adapter.pack_id
            );
            for framework in adapter.frameworks {
                assert!(
                    frameworks.insert(framework),
                    "framework {framework} is owned by multiple route expansion adapters"
                );
            }
        }
        assert!(
            ROUTE_EXPANSION_ADAPTERS
                .windows(2)
                .all(|pair| pair[0].pack_id < pair[1].pack_id),
            "route expansion adapters must be sorted by pack ID"
        );
    }

    #[test]
    fn route_expansion_dispatches_mixed_frameworks_without_cross_framework_mutation() {
        let custom = route("custom", "/custom");
        let facts = vec![
            RawFrameworkFact::Route(route("fastapi", "/python")),
            RawFrameworkFact::Route(custom.clone()),
            RawFrameworkFact::Route(route("axum", "/rust")),
        ];

        let result = expand_framework_routes(&facts, FrameworkLimits::default());
        assert!(result.is_ok(), "mixed framework routes should expand");
        let expanded = result.unwrap_or_default();
        let by_framework = expanded
            .into_iter()
            .map(|route| (route.framework.clone(), route))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(by_framework.len(), 3);
        assert_eq!(by_framework.get("custom"), Some(&custom));
        assert_eq!(
            by_framework
                .get("fastapi")
                .map(|route| route.normalized_path.as_str()),
            Some("/python")
        );
        assert_eq!(
            by_framework
                .get("axum")
                .map(|route| route.normalized_path.as_str()),
            Some("/rust")
        );
    }

    fn route(framework: &str, path: &str) -> RawRouteFact {
        RawRouteFact {
            framework: framework.to_owned(),
            operation: "GET".to_owned(),
            raw_path: path.to_owned(),
            normalized_path: path.to_owned(),
            declaring_scope: "test".to_owned(),
            anchor: RawFrameworkAnchor {
                source_file: "routes.test".to_owned(),
                start_byte: 0,
                end_byte: 1,
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 1,
            },
            handler_reference: "handler".to_owned(),
            middleware_references: Vec::new(),
            stages: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: None,
            detail: serde_json::Map::new(),
        }
    }
}
