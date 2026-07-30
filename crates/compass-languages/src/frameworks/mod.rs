mod csharp;
mod enterprise;
mod evidence;
mod file_routes;
mod go;
mod java;
mod model;
mod php;
mod play;
mod python;
mod ruby;
mod rust;
mod swift;
mod text;
mod typescript;

pub use model::{
    FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact,
    RawFrameworkOrigin, RawRouteFact,
};

use std::path::Path;

use tree_sitter::Node;

use crate::{Extraction, ProjectEvidence};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManifestPolicy {
    Advisory,
    Required,
}

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

struct SourcePack {
    id: &'static str,
    languages: &'static [&'static str],
    dependency_markers: &'static [&'static str],
    manifest_policy: ManifestPolicy,
    detector: SourceDetector,
}

type ConfigMatcher = fn(&Path) -> bool;
type ConfigDetector = fn(&Path, &[u8]) -> Vec<RawFrameworkFact>;

struct ConfigPack {
    id: &'static str,
    matcher: ConfigMatcher,
    detector: ConfigDetector,
}

type TemplateDetector =
    fn(&Path, &[u8], Option<&ProjectEvidence>, &mut Extraction) -> Vec<RawFrameworkFact>;

struct TemplatePack {
    id: &'static str,
    dependency_markers: &'static [&'static str],
    manifest_policy: ManifestPolicy,
    detector: TemplateDetector,
}

const SOURCE_PACKS: &[SourcePack] = &[
    source_pack("python-web", &["python"], &[], detect_python),
    source_pack(
        "php-frameworks",
        &["php"],
        &["laravel/framework", "drupal/core"],
        detect_php,
    ),
    source_pack("rails-routes", &["ruby"], &["rails"], detect_ruby),
    source_pack(
        "spring-web",
        &["java", "kotlin"],
        &[
            "org.springframework:spring-web",
            "org.springframework.boot:spring-boot",
        ],
        detect_java,
    ),
    source_pack("go-web", &["go"], &[], detect_go),
    source_pack("rust-web", &["rust"], &[], detect_rust),
    source_pack(
        "aspnet-web",
        &["csharp"],
        &["microsoft.aspnetcore.app"],
        detect_csharp,
    ),
    source_pack("vapor-routes", &["swift"], &["vapor"], detect_swift),
    source_pack(
        "typescript-web",
        &["javascript", "typescript", "tsx"],
        &[
            "express",
            "@nestjs/common",
            "react-router",
            "react-router-dom",
            "vue-router",
        ],
        detect_typescript,
    ),
    SourcePack {
        id: "filesystem-routes",
        languages: &["javascript", "typescript", "tsx"],
        dependency_markers: &["@sveltejs/kit", "nuxt", "astro"],
        manifest_policy: ManifestPolicy::Required,
        detector: detect_file_routes,
    },
    SourcePack {
        id: "enterprise-domain-facts",
        languages: &[
            "python",
            "typescript",
            "tsx",
            "javascript",
            "java",
            "csharp",
            "ruby",
            "php",
            "go",
            "rust",
        ],
        dependency_markers: &[],
        manifest_policy: ManifestPolicy::Advisory,
        detector: detect_enterprise,
    },
];

const CONFIG_PACKS: &[ConfigPack] = &[
    ConfigPack {
        id: "drupal-routing-config",
        matcher: is_drupal_routing,
        detector: php::detect_drupal_routing,
    },
    ConfigPack {
        id: "play-routes-config",
        matcher: is_play_routes,
        detector: play::detect,
    },
];

const TEMPLATE_PACKS: &[TemplatePack] = &[TemplatePack {
    id: "filesystem-template-routes",
    dependency_markers: &["@sveltejs/kit", "nuxt", "astro"],
    manifest_policy: ManifestPolicy::Required,
    detector: file_routes::detect,
}];

const fn source_pack(
    id: &'static str,
    languages: &'static [&'static str],
    dependency_markers: &'static [&'static str],
    detector: SourceDetector,
) -> SourcePack {
    SourcePack {
        id,
        languages,
        dependency_markers,
        manifest_policy: ManifestPolicy::Advisory,
        detector,
    }
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
    let mut facts = Vec::new();
    for pack in SOURCE_PACKS {
        debug_assert!(!pack.id.is_empty());
        if pack.languages.contains(&language) && pack_enabled(pack, project) {
            facts.extend((pack.detector)(&context, extraction));
        }
    }
    publish_facts(facts, extraction);
}

pub(crate) fn detect_config_file(
    path: &Path,
    source: &[u8],
    _project: Option<&ProjectEvidence>,
) -> Extraction {
    let mut extraction = Extraction::default();
    let facts = CONFIG_PACKS
        .iter()
        .find(|pack| (pack.matcher)(path))
        .map_or_else(Vec::new, |pack| {
            debug_assert!(!pack.id.is_empty());
            (pack.detector)(path, source)
        });
    publish_facts(facts, &mut extraction);
    extraction
}

pub(crate) fn detect_template_file_route(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) {
    let mut facts = Vec::new();
    for pack in TEMPLATE_PACKS {
        debug_assert!(!pack.id.is_empty());
        if template_pack_enabled(pack, project) {
            facts.extend((pack.detector)(path, source, project, extraction));
        }
    }
    publish_facts(facts, extraction);
}

fn pack_enabled(pack: &SourcePack, project: Option<&ProjectEvidence>) -> bool {
    manifest_policy_allows(pack.manifest_policy, pack.dependency_markers, project)
}

fn template_pack_enabled(pack: &TemplatePack, project: Option<&ProjectEvidence>) -> bool {
    manifest_policy_allows(pack.manifest_policy, pack.dependency_markers, project)
}

fn manifest_policy_allows(
    policy: ManifestPolicy,
    dependency_markers: &[&str],
    project: Option<&ProjectEvidence>,
) -> bool {
    match policy {
        ManifestPolicy::Advisory => true,
        ManifestPolicy::Required => {
            project.is_none_or(|project| project.has_any_dependency(dependency_markers))
        }
    }
}

fn publish_facts(facts: Vec<RawFrameworkFact>, extraction: &mut Extraction) {
    if let Err(error) = FrameworkLimits::default().check_facts(facts.len()) {
        extraction
            .error
            .get_or_insert_with(|| format!("framework extraction failed: {error}"));
        return;
    }
    extraction.framework_facts.extend(facts);
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

fn detect_java(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    java::detect(context.path, context.source, context.root)
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
    rust::detect(context.path, context.source, context.root)
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
    typescript::detect(context.path, context.source, context.root, extraction)
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

    use super::{CONFIG_PACKS, SOURCE_PACKS, TEMPLATE_PACKS};

    #[test]
    fn framework_pack_registry_ids_are_unique_and_well_formed() {
        let mut ids = HashSet::new();
        for pack in SOURCE_PACKS {
            assert!(!pack.id.is_empty());
            assert!(!pack.languages.is_empty());
            assert!(ids.insert(pack.id));
        }
        for pack in CONFIG_PACKS {
            assert!(!pack.id.is_empty());
            assert!(ids.insert(pack.id));
        }
        for pack in TEMPLATE_PACKS {
            assert!(!pack.id.is_empty());
            assert!(ids.insert(pack.id));
        }
    }

    #[test]
    fn source_registry_covers_every_existing_framework_module() {
        let ids = SOURCE_PACKS
            .iter()
            .map(|pack| pack.id)
            .collect::<HashSet<_>>();
        for expected in [
            "python-web",
            "php-frameworks",
            "rails-routes",
            "spring-web",
            "go-web",
            "rust-web",
            "aspnet-web",
            "vapor-routes",
            "typescript-web",
            "filesystem-routes",
            "enterprise-domain-facts",
        ] {
            assert!(ids.contains(expected), "missing framework pack {expected}");
        }
    }
}
