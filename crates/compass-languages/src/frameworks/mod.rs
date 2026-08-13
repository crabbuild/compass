mod axum;
mod csharp;
mod enterprise;
mod evidence;
mod express;
mod fastify;
mod file_routes;
mod go;
mod hono;
mod java;
mod model;
mod next;
mod pack;
mod php;
mod play;
mod python;
mod remix;
mod ruby;
mod rust;
mod spring;
mod spring_kotlin;
mod swift;
mod text;
mod typescript;
mod vite;

pub use model::{
    FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkAnchor,
    RawFrameworkAnnotationFact, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};
pub use pack::{
    FrameworkCapability, FrameworkManifestPolicy, FrameworkOccurrencePolicy,
    FrameworkPackDescriptor, FrameworkPackKind, FrameworkPackRegistry, FrameworkPackRegistryError,
    FrameworkRelation,
};

use std::path::Path;

use tree_sitter::Node;

use crate::SemanticEvidenceBatch;
use crate::{Extraction, ProjectEvidence};

struct DetectionContext<'source, 'tree> {
    path: &'source Path,
    source: &'source [u8],
    root: Node<'tree>,
    language: &'source str,
    project: Option<&'source ProjectEvidence>,
}

type SourceDetector = for<'source, 'tree> fn(
    &DetectionContext<'source, 'tree>,
    &mut Extraction,
) -> Vec<RawFrameworkFact>;

struct UniversalDetectionContext<'source, 'tree> {
    source: &'source [u8],
    root: Node<'tree>,
    project: Option<&'source ProjectEvidence>,
    evidence: &'source SemanticEvidenceBatch,
}

type UniversalSourceDetector =
    for<'source, 'tree> fn(&UniversalDetectionContext<'source, 'tree>) -> Vec<RawFrameworkFact>;

type ConfigMatcher = fn(&Path) -> bool;
type ConfigDetector = fn(&Path, &[u8]) -> Vec<RawFrameworkFact>;

type TemplateDetector =
    fn(&Path, &[u8], Option<&ProjectEvidence>, &mut Extraction) -> Vec<RawFrameworkFact>;

/// The concrete implementation stored behind one framework-pack seam.
///
/// The public descriptor describes the universal evidence contract. This
/// internal adapter also carries the established source, config, and template
/// implementations so selection, activation, limits, and publication stay in
/// one runtime instead of being copied across four registries.
#[derive(Clone, Copy)]
enum FrameworkPackAdapter {
    Source(SourceDetector),
    Universal {
        descriptor: &'static FrameworkPackDescriptor,
        detector: UniversalSourceDetector,
    },
    Config {
        matcher: ConfigMatcher,
        detector: ConfigDetector,
    },
    Template(TemplateDetector),
}

#[derive(Clone, Copy)]
struct FrameworkPack {
    id: &'static str,
    kind: FrameworkPackKind,
    languages: &'static [&'static str],
    dependency_markers: &'static [&'static str],
    configuration_markers: &'static [&'static str],
    manifest_policy: FrameworkManifestPolicy,
    limits: FrameworkLimits,
    adapter: FrameworkPackAdapter,
}

impl FrameworkPack {
    const fn source(
        id: &'static str,
        languages: &'static [&'static str],
        dependency_markers: &'static [&'static str],
        detector: SourceDetector,
    ) -> Self {
        Self {
            id,
            kind: FrameworkPackKind::Source,
            languages,
            dependency_markers,
            configuration_markers: &[],
            manifest_policy: FrameworkManifestPolicy::Advisory,
            limits: FrameworkLimits::DEFAULT,
            adapter: FrameworkPackAdapter::Source(detector),
        }
    }

    const fn source_required(
        id: &'static str,
        languages: &'static [&'static str],
        dependency_markers: &'static [&'static str],
        configuration_markers: &'static [&'static str],
        detector: SourceDetector,
    ) -> Self {
        Self {
            id,
            kind: FrameworkPackKind::Source,
            languages,
            dependency_markers,
            configuration_markers,
            manifest_policy: FrameworkManifestPolicy::Required,
            limits: FrameworkLimits::DEFAULT,
            adapter: FrameworkPackAdapter::Source(detector),
        }
    }

    const fn universal(
        descriptor: &'static FrameworkPackDescriptor,
        detector: UniversalSourceDetector,
    ) -> Self {
        Self {
            id: descriptor.id,
            kind: descriptor.kind,
            languages: descriptor.languages,
            dependency_markers: descriptor.dependency_markers,
            configuration_markers: &[],
            manifest_policy: descriptor.manifest_policy,
            limits: descriptor.limits,
            adapter: FrameworkPackAdapter::Universal {
                descriptor,
                detector,
            },
        }
    }

    const fn config(id: &'static str, matcher: ConfigMatcher, detector: ConfigDetector) -> Self {
        Self {
            id,
            kind: FrameworkPackKind::Config,
            languages: &[],
            dependency_markers: &[],
            configuration_markers: &[],
            manifest_policy: FrameworkManifestPolicy::Advisory,
            limits: FrameworkLimits::DEFAULT,
            adapter: FrameworkPackAdapter::Config { matcher, detector },
        }
    }

    const fn template(
        id: &'static str,
        dependency_markers: &'static [&'static str],
        detector: TemplateDetector,
    ) -> Self {
        Self {
            id,
            kind: FrameworkPackKind::Template,
            languages: &[],
            dependency_markers,
            configuration_markers: &[],
            manifest_policy: FrameworkManifestPolicy::Required,
            limits: FrameworkLimits::DEFAULT,
            adapter: FrameworkPackAdapter::Template(detector),
        }
    }

    fn enabled(self, project: Option<&ProjectEvidence>) -> bool {
        match self.manifest_policy {
            FrameworkManifestPolicy::Advisory => true,
            FrameworkManifestPolicy::Required => project.is_none_or(|project| {
                project.has_any_dependency(self.dependency_markers)
                    || project.has_any_configuration(self.configuration_markers)
            }),
        }
    }

    fn matches_source(self, language: &str, project: Option<&ProjectEvidence>) -> bool {
        matches!(self.kind, FrameworkPackKind::Source)
            && self.languages.contains(&language)
            && self.enabled(project)
    }

    fn matches_template(self, project: Option<&ProjectEvidence>) -> bool {
        matches!(self.kind, FrameworkPackKind::Template) && self.enabled(project)
    }

    fn matches_config(self, path: &Path) -> bool {
        matches!(self.adapter, FrameworkPackAdapter::Config { matcher, .. } if matcher(path))
    }

    fn collect_source(
        self,
        context: &DetectionContext<'_, '_>,
        extraction: &mut Extraction,
    ) -> Result<Vec<RawFrameworkFact>, String> {
        let facts = match self.adapter {
            FrameworkPackAdapter::Source(detector) => detector(context, extraction),
            FrameworkPackAdapter::Universal {
                descriptor,
                detector,
            } => {
                debug_assert_eq!(self.id, descriptor.id);
                let Some(evidence) = extraction.semantic_evidence.as_ref() else {
                    return Ok(Vec::new());
                };
                detector(&UniversalDetectionContext {
                    source: context.source,
                    root: context.root,
                    project: context.project,
                    evidence,
                })
            }
            FrameworkPackAdapter::Config { .. } | FrameworkPackAdapter::Template(_) => {
                return Ok(Vec::new());
            }
        };
        self.check_fact_limit(facts.len()).map(|()| facts)
    }

    fn collect_config(self, path: &Path, source: &[u8]) -> Option<Vec<RawFrameworkFact>> {
        let FrameworkPackAdapter::Config { detector, .. } = self.adapter else {
            return None;
        };
        Some(detector(path, source))
    }

    fn collect_template(
        self,
        path: &Path,
        source: &[u8],
        project: Option<&ProjectEvidence>,
        extraction: &mut Extraction,
    ) -> Result<Vec<RawFrameworkFact>, String> {
        let FrameworkPackAdapter::Template(detector) = self.adapter else {
            return Ok(Vec::new());
        };
        let facts = detector(path, source, project, extraction);
        self.check_fact_limit(facts.len()).map(|()| facts)
    }

    fn check_fact_limit(self, observed: usize) -> Result<(), String> {
        self.limits.check_facts(observed).map_err(|error| {
            format!(
                "framework pack {:?} exceeded its fact budget: {error}",
                self.id
            )
        })
    }
}

/// All framework implementations pass through this table. The table is
/// intentionally static: pack identity, ordering, activation policy, and
/// adapter ownership remain deterministic and do not require a plugin ABI.
const FRAMEWORK_PACKS: &[FrameworkPack] = &[
    FrameworkPack::universal(&pack::SPRING_JAVA_DESCRIPTOR, spring::detect),
    FrameworkPack::source("python-web", &["python"], &[], detect_python),
    FrameworkPack::source(
        "php-frameworks",
        &["php"],
        &["laravel/framework", "drupal/core"],
        detect_php,
    ),
    FrameworkPack::source("rails-routes", &["ruby"], &["rails"], detect_ruby),
    FrameworkPack::source(
        "spring-web-kotlin",
        &["kotlin"],
        &[
            "org.springframework:spring-web",
            "org.springframework.boot:spring-boot",
        ],
        detect_kotlin,
    ),
    FrameworkPack::source("go-web", &["go"], &[], detect_go),
    FrameworkPack::source("axum-web", &["rust"], &["axum"], detect_axum),
    FrameworkPack::source("rust-web", &["rust"], &[], detect_rust),
    FrameworkPack::source(
        "aspnet-web",
        &["csharp"],
        &["microsoft.aspnetcore.app"],
        detect_csharp,
    ),
    FrameworkPack::source("vapor-routes", &["swift"], &["vapor"], detect_swift),
    FrameworkPack::source(
        "express-web",
        &["javascript", "typescript", "tsx"],
        &["express"],
        detect_express,
    ),
    FrameworkPack::source(
        "fastify-web",
        &["javascript", "typescript", "tsx"],
        &["fastify"],
        detect_fastify,
    ),
    FrameworkPack::source(
        "hono-web",
        &["javascript", "typescript", "tsx"],
        &["hono"],
        detect_hono,
    ),
    FrameworkPack::source(
        "typescript-web",
        &["javascript", "typescript", "tsx"],
        &[
            "@angular/router",
            "@nestjs/common",
            "react-router",
            "react-router-dom",
            "vue-router",
        ],
        detect_typescript,
    ),
    FrameworkPack::source_required(
        "nextjs-routes",
        &["javascript", "typescript", "tsx"],
        &["next"],
        &["next.config.js", "next.config.mjs", "next.config.ts"],
        detect_next,
    ),
    FrameworkPack::source_required(
        "remix-routes",
        &["javascript", "typescript", "tsx"],
        &[
            "@remix-run/dev",
            "@remix-run/node",
            "@remix-run/react",
            "@remix-run/router",
            "@remix-run/serve",
        ],
        &[
            "remix.config.cjs",
            "remix.config.js",
            "remix.config.mjs",
            "remix.config.ts",
        ],
        detect_remix,
    ),
    FrameworkPack::source_required(
        "vite-config",
        &["javascript", "typescript", "tsx"],
        &["vite"],
        &[
            "vite.config.cjs",
            "vite.config.js",
            "vite.config.mjs",
            "vite.config.ts",
        ],
        detect_vite,
    ),
    FrameworkPack {
        id: "filesystem-routes",
        kind: FrameworkPackKind::Source,
        languages: &["javascript", "typescript", "tsx"],
        dependency_markers: &["@sveltejs/kit", "nuxt", "astro"],
        configuration_markers: &[],
        manifest_policy: FrameworkManifestPolicy::Required,
        limits: FrameworkLimits::DEFAULT,
        adapter: FrameworkPackAdapter::Source(detect_file_routes),
    },
    FrameworkPack {
        id: "enterprise-domain-facts",
        kind: FrameworkPackKind::Source,
        languages: &[
            "python",
            "typescript",
            "tsx",
            "javascript",
            "csharp",
            "ruby",
            "php",
            "go",
            "rust",
        ],
        dependency_markers: &[],
        configuration_markers: &[],
        manifest_policy: FrameworkManifestPolicy::Advisory,
        limits: FrameworkLimits::DEFAULT,
        adapter: FrameworkPackAdapter::Source(detect_enterprise),
    },
    FrameworkPack::config(
        "drupal-routing-config",
        is_drupal_routing,
        php::detect_drupal_routing,
    ),
    FrameworkPack::config("play-routes-config", is_play_routes, play::detect),
    FrameworkPack::template(
        "filesystem-template-routes",
        &["@sveltejs/kit", "nuxt", "astro"],
        file_routes::detect,
    ),
];

struct FrameworkFactAccumulator {
    facts: Vec<RawFrameworkFact>,
}

impl FrameworkFactAccumulator {
    fn new() -> Self {
        Self { facts: Vec::new() }
    }

    fn add(&mut self, pack: FrameworkPack, facts: Vec<RawFrameworkFact>) -> Result<(), String> {
        pack.check_fact_limit(facts.len())?;
        let observed = self.facts.len().saturating_add(facts.len());
        FrameworkLimits::DEFAULT
            .check_facts(observed)
            .map_err(|error| format!("framework fact budget exceeded: {error}"))?;
        self.facts.extend(facts);
        Ok(())
    }

    fn publish(self, extraction: &mut Extraction) {
        extraction.framework_facts.extend(self.facts);
    }
}

fn record_framework_error(extraction: &mut Extraction, error: String) {
    extraction
        .error
        .get_or_insert_with(|| format!("framework extraction failed: {error}"));
}

pub(crate) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    language: &str,
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) {
    let context = DetectionContext {
        path,
        source,
        root,
        language,
        project,
    };
    let mut accumulator = FrameworkFactAccumulator::new();
    for pack in FRAMEWORK_PACKS {
        if !pack.matches_source(language, project) {
            continue;
        }
        let facts = match pack.collect_source(&context, extraction) {
            Ok(facts) => facts,
            Err(error) => {
                record_framework_error(extraction, error);
                return;
            }
        };
        if let Err(error) = accumulator.add(*pack, facts) {
            record_framework_error(extraction, error);
            return;
        }
    }
    accumulator.publish(extraction);
}

pub(crate) fn detect_config_file(
    path: &Path,
    source: &[u8],
    _project: Option<&ProjectEvidence>,
) -> Extraction {
    let mut extraction = Extraction::default();
    let Some(pack) = FRAMEWORK_PACKS
        .iter()
        .find(|pack| pack.kind == FrameworkPackKind::Config && pack.matches_config(path))
    else {
        return extraction;
    };
    let Some(facts) = pack.collect_config(path, source) else {
        return extraction;
    };
    let mut accumulator = FrameworkFactAccumulator::new();
    if let Err(error) = accumulator.add(*pack, facts) {
        record_framework_error(&mut extraction, error);
    } else {
        accumulator.publish(&mut extraction);
    }
    extraction
}

pub(crate) fn detect_template_file_route(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) {
    let mut accumulator = FrameworkFactAccumulator::new();
    for pack in FRAMEWORK_PACKS {
        if !pack.matches_template(project) {
            continue;
        }
        let facts = match pack.collect_template(path, source, project, extraction) {
            Ok(facts) => facts,
            Err(error) => {
                record_framework_error(extraction, error);
                return;
            }
        };
        if let Err(error) = accumulator.add(*pack, facts) {
            record_framework_error(extraction, error);
            return;
        }
    }
    accumulator.publish(extraction);
}

fn detect_python(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    python::detect(context.path, context.source, context.root)
}

fn detect_php(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    php::detect(context.path, context.source, context.root)
}

fn detect_ruby(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    ruby::detect(context.path, context.source, context.root)
}

fn detect_kotlin(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    spring_kotlin::detect(context.path, context.source, context.root)
}

fn detect_go(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    go::detect(context.path, context.source, context.root)
}

fn detect_rust(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    rust::detect_non_axum(context.path, context.source, context.root)
}

fn detect_axum(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    axum::detect(context.path, context.source, context.root)
}

fn detect_csharp(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    csharp::detect(context.path, context.source, context.root)
}

fn detect_swift(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    swift::detect(context.path, context.source, context.root)
}

fn detect_typescript(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    typescript::detect_non_express(context.path, context.source, context.root, extraction)
}

fn detect_express(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    express::detect(context.path, context.source, context.root, extraction)
}

fn detect_fastify(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    fastify::detect(context.path, context.source, context.root, extraction)
}

fn detect_hono(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    hono::detect(context.path, context.source, context.root, extraction)
}

fn detect_next(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    next::detect(context.path, context.source, context.project, extraction)
}

fn detect_remix(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    remix::detect(context.path, context.source, context.project, extraction)
}

fn detect_vite(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    vite::detect(context.path, context.source, context.project, extraction)
}

fn detect_file_routes(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    file_routes::detect(context.path, context.source, context.project, extraction)
}

fn detect_enterprise(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    enterprise::detect(context.path, context.source, context.language)
}

fn is_drupal_routing(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".routing.yml") || name.ends_with(".routing.yaml"))
}

fn is_play_routes(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("routes")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("conf")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{FRAMEWORK_PACKS, FrameworkPackKind};

    #[test]
    fn framework_pack_registry_ids_are_unique_and_well_formed() {
        let mut ids = HashSet::new();
        for pack in FRAMEWORK_PACKS {
            assert!(!pack.id.is_empty());
            assert!(ids.insert(pack.id));
            if pack.kind == FrameworkPackKind::Source {
                assert!(!pack.languages.is_empty());
            }
        }
    }

    #[test]
    fn runtime_registry_covers_every_existing_framework_adapter() {
        let ids = FRAMEWORK_PACKS
            .iter()
            .map(|pack| pack.id)
            .collect::<HashSet<_>>();
        for expected in [
            "spring-java",
            "python-web",
            "php-frameworks",
            "rails-routes",
            "spring-web-kotlin",
            "go-web",
            "axum-web",
            "rust-web",
            "aspnet-web",
            "vapor-routes",
            "express-web",
            "fastify-web",
            "hono-web",
            "typescript-web",
            "nextjs-routes",
            "remix-routes",
            "vite-config",
            "filesystem-routes",
            "enterprise-domain-facts",
            "drupal-routing-config",
            "play-routes-config",
            "filesystem-template-routes",
        ] {
            assert!(ids.contains(expected), "missing framework pack {expected}");
        }
    }

    #[test]
    fn runtime_registry_uses_one_manifest_policy_and_budget_source() {
        for pack in FRAMEWORK_PACKS {
            assert!(pack.limits.max_facts_per_file > 0);
            if pack.manifest_policy == super::FrameworkManifestPolicy::Required {
                assert!(!pack.dependency_markers.is_empty());
            }
        }
    }

    #[test]
    fn universal_descriptors_have_one_runtime_adapter_each() {
        for descriptor in super::FrameworkPackRegistry::descriptors() {
            let matches = FRAMEWORK_PACKS
                .iter()
                .filter(|pack| {
                    matches!(
                        pack.adapter,
                        super::FrameworkPackAdapter::Universal { descriptor: registered, .. }
                            if registered.id == descriptor.id
                    )
                })
                .count();
            assert_eq!(matches, 1, "runtime adapter count for {}", descriptor.id);
        }
        assert_eq!(super::FrameworkPackRegistry::validate(), Ok(()));
    }
}
