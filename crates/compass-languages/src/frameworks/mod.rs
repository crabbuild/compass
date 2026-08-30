mod axum;
mod csharp;
mod dart;
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
mod react;
mod remix;
mod ruby;
mod rust;
mod spring;
mod swift;
mod tanstack;
mod text;
mod typescript;
pub(crate) mod typescript_syntax;
mod vite;

pub use model::{
    FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkAnchor,
    RawFrameworkAnnotationFact, RawFrameworkConfigurationFact, RawFrameworkFact,
    RawFrameworkFileSetFact, RawFrameworkOrigin, RawFrameworkRelationFact, RawFrameworkRoleFact,
    RawRouteFact, RawRouteStageFact, RawRouteStageRole,
};
pub use pack::{
    FrameworkCapability, FrameworkManifestPolicy, FrameworkOccurrencePolicy,
    FrameworkPackDescriptor, FrameworkPackKind, FrameworkPackRegistry, FrameworkPackRegistryError,
    FrameworkRelation,
};

use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};
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
    path: &'source Path,
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

/// Cache identity for the framework-pack registry. The value is deliberately
/// separate from the language producer version: changing framework activation,
/// descriptor capabilities, or resource limits must invalidate framework facts
/// without pretending that the parser/evidence producer changed.
pub const FRAMEWORK_PACK_SEMANTICS_VERSION: &str = "compass.framework-packs/6";

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
    /// Explicit maintainer-controlled semantics version for established
    /// source/config/template adapters. Universal descriptors carry the same
    /// contract directly in their descriptor.
    semantics_version: u32,
    kind: FrameworkPackKind,
    languages: &'static [&'static str],
    dependency_markers: &'static [&'static str],
    configuration_markers: &'static [&'static str],
    manifest_policy: FrameworkManifestPolicy,
    limits: FrameworkLimits,
    adapter: FrameworkPackAdapter,
}

impl FrameworkPack {
    const fn source_versioned(
        id: &'static str,
        languages: &'static [&'static str],
        dependency_markers: &'static [&'static str],
        semantics_version: u32,
        detector: SourceDetector,
    ) -> Self {
        Self::source_versioned_with_limits(
            id,
            languages,
            dependency_markers,
            semantics_version,
            FrameworkLimits::DEFAULT,
            detector,
        )
    }

    const fn source_versioned_with_limits(
        id: &'static str,
        languages: &'static [&'static str],
        dependency_markers: &'static [&'static str],
        semantics_version: u32,
        limits: FrameworkLimits,
        detector: SourceDetector,
    ) -> Self {
        Self {
            id,
            semantics_version,
            kind: FrameworkPackKind::Source,
            languages,
            dependency_markers,
            configuration_markers: &[],
            manifest_policy: FrameworkManifestPolicy::Advisory,
            limits,
            adapter: FrameworkPackAdapter::Source(detector),
        }
    }

    const fn source_required_versioned(
        id: &'static str,
        languages: &'static [&'static str],
        dependency_markers: &'static [&'static str],
        configuration_markers: &'static [&'static str],
        semantics_version: u32,
        detector: SourceDetector,
    ) -> Self {
        Self {
            id,
            semantics_version,
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
            semantics_version: descriptor.semantics_version,
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

    const fn config_versioned(
        id: &'static str,
        matcher: ConfigMatcher,
        semantics_version: u32,
        detector: ConfigDetector,
    ) -> Self {
        Self {
            id,
            semantics_version,
            kind: FrameworkPackKind::Config,
            languages: &[],
            dependency_markers: &[],
            configuration_markers: &[],
            manifest_policy: FrameworkManifestPolicy::Advisory,
            limits: FrameworkLimits::DEFAULT,
            adapter: FrameworkPackAdapter::Config { matcher, detector },
        }
    }

    const fn template_versioned(
        id: &'static str,
        dependency_markers: &'static [&'static str],
        semantics_version: u32,
        detector: TemplateDetector,
    ) -> Self {
        Self {
            id,
            semantics_version,
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
        // TSX uses the TypeScript universal evidence producer while the
        // registry still preserves the source-language spelling for legacy
        // adapters. Universal descriptors therefore match its canonical
        // pipeline language here without making every descriptor duplicate a
        // parser dialect entry.
        let language = if language == "tsx" {
            "typescript"
        } else {
            language
        };
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
        self.limits
            .check_source_bytes(context.source.len())
            .map_err(|error| {
                format!(
                    "framework pack {:?} exceeded its source budget: {error}",
                    self.id
                )
            })?;
        let syntax = typescript_syntax::TypeScriptSyntax::new(context.root, context.source);
        let (syntax_nodes, syntax_depth) = syntax.node_count_and_depth();
        self.limits
            .check_syntax_nodes(syntax_nodes)
            .map_err(|error| format!("framework pack {:?}: {error}", self.id))?;
        self.limits
            .check_syntax_depth(syntax_depth)
            .map_err(|error| format!("framework pack {:?}: {error}", self.id))?;
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
                    path: context.path,
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

    fn collect_config(
        self,
        path: &Path,
        source: &[u8],
    ) -> Result<Option<Vec<RawFrameworkFact>>, String> {
        let FrameworkPackAdapter::Config { detector, .. } = self.adapter else {
            return Ok(None);
        };
        self.limits
            .check_config_bytes(source.len())
            .map_err(|error| {
                format!(
                    "framework pack {:?} exceeded its config budget: {error}",
                    self.id
                )
            })?;
        Ok(Some(detector(path, source)))
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
        self.limits
            .check_source_bytes(source.len())
            .map_err(|error| {
                format!(
                    "framework pack {:?} exceeded its source budget: {error}",
                    self.id
                )
            })?;
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
    FrameworkPack::universal(&pack::ASPNET_CSHARP_DESCRIPTOR, csharp::detect),
    FrameworkPack::source_versioned(
        "aspnet-minimal-csharp",
        &["csharp"],
        &["microsoft.aspnetcore.app"],
        1,
        csharp::detect_minimal,
    ),
    FrameworkPack::universal(&pack::SPRING_JAVA_DESCRIPTOR, spring::detect),
    FrameworkPack::universal(&pack::SPRING_KOTLIN_DESCRIPTOR, spring::detect_kotlin),
    FrameworkPack::universal(&pack::DJANGO_PYTHON_DESCRIPTOR, python::detect_django),
    FrameworkPack::universal(
        &pack::DJANGO_REST_FRAMEWORK_PYTHON_DESCRIPTOR,
        python::detect_drf,
    ),
    FrameworkPack::universal(&pack::FASTAPI_PYTHON_DESCRIPTOR, python::detect_fastapi),
    FrameworkPack::universal(&pack::FLASK_PYTHON_DESCRIPTOR, python::detect_flask),
    FrameworkPack::universal(&pack::PYDANTIC_PYTHON_DESCRIPTOR, python::detect_pydantic),
    FrameworkPack::universal(
        &pack::SQLALCHEMY_PYTHON_DESCRIPTOR,
        python::detect_sqlalchemy,
    ),
    FrameworkPack::universal(&pack::CELERY_PYTHON_DESCRIPTOR, python::detect_celery),
    FrameworkPack::universal(&pack::STARLETTE_PYTHON_DESCRIPTOR, python::detect_starlette),
    FrameworkPack::universal(&pack::PHP_FRAMEWORKS_DESCRIPTOR, php::detect),
    FrameworkPack::universal(&pack::RAILS_RUBY_DESCRIPTOR, ruby::detect_universal),
    FrameworkPack::source_versioned_with_limits(
        "go-web",
        &["go"],
        &[],
        1,
        FrameworkLimits {
            // Generated OpenAPI Go files in the pinned qualification corpus
            // exceed the shared 100k budget while remaining within the
            // bounded parser-inspection ceiling.
            max_syntax_nodes: 200_000,
            ..FrameworkLimits::DEFAULT
        },
        detect_go,
    ),
    FrameworkPack::source_versioned("axum-web", &["rust"], &["axum"], 1, detect_axum),
    FrameworkPack::source_versioned("rust-web", &["rust"], &[], 1, detect_rust),
    FrameworkPack::universal(&pack::VAPOR_SWIFT_DESCRIPTOR, detect_swift_universal),
    FrameworkPack::universal(
        &pack::DART_FLUTTER_NAVIGATION_DESCRIPTOR,
        dart::detect_flutter_navigation,
    ),
    FrameworkPack::universal(&pack::DART_BLOC_DESCRIPTOR, dart::detect_bloc),
    FrameworkPack::universal(&pack::DART_RIVERPOD_DESCRIPTOR, dart::detect_riverpod),
    FrameworkPack::universal(&pack::REACT_UI_DESCRIPTOR, react::detect),
    FrameworkPack::source_versioned(
        "express-web",
        &["javascript", "typescript", "tsx"],
        &["express"],
        1,
        detect_express,
    ),
    FrameworkPack::source_versioned(
        "fastify-web",
        &["javascript", "typescript", "tsx"],
        &["fastify"],
        1,
        detect_fastify,
    ),
    FrameworkPack::source_versioned(
        "hono-web",
        &["javascript", "typescript", "tsx"],
        &["hono"],
        1,
        detect_hono,
    ),
    FrameworkPack::source_versioned(
        "typescript-web",
        &["javascript", "typescript", "tsx"],
        &["@angular/router", "@nestjs/common", "vue-router"],
        2,
        detect_typescript,
    ),
    FrameworkPack::source_required_versioned(
        "react-router-routes",
        &["javascript", "typescript", "tsx"],
        &["react-router", "react-router-dom"],
        &[],
        2,
        detect_react_router,
    ),
    FrameworkPack::source_versioned(
        "tanstack-router",
        &["javascript", "typescript", "tsx"],
        &[
            "@tanstack/react-router",
            "@tanstack/react-start",
            "@tanstack/router-core",
            "@tanstack/router-generator",
            "@tanstack/start",
        ],
        2,
        detect_tanstack,
    ),
    FrameworkPack::source_versioned(
        "tanstack-start",
        &["javascript", "typescript", "tsx"],
        &["@tanstack/react-start", "@tanstack/start"],
        2,
        detect_tanstack_start,
    ),
    FrameworkPack::source_required_versioned(
        "nextjs-routes",
        &["javascript", "typescript", "tsx"],
        &["next"],
        &["next.config.js", "next.config.mjs", "next.config.ts"],
        2,
        detect_next,
    ),
    FrameworkPack::source_required_versioned(
        "remix-routes",
        &["javascript", "typescript", "tsx"],
        &[
            "@remix-run/dev",
            "@remix-run/node",
            "@remix-run/react",
            "@remix-run/router",
            "@remix-run/serve",
            "remix",
        ],
        &[
            "remix.config.cjs",
            "remix.config.js",
            "remix.config.mjs",
            "remix.config.ts",
        ],
        3,
        detect_remix,
    ),
    FrameworkPack::source_required_versioned(
        "vite-config",
        &["javascript", "typescript", "tsx"],
        &["vite"],
        &[
            "vite.config.cjs",
            "vite.config.js",
            "vite.config.mjs",
            "vite.config.ts",
        ],
        2,
        detect_vite,
    ),
    FrameworkPack {
        id: "filesystem-routes",
        semantics_version: 2,
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
        semantics_version: 1,
        kind: FrameworkPackKind::Source,
        languages: &[
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
        limits: FrameworkLimits {
            // Generated OpenAPI Go files can also activate the language-wide
            // enterprise detector, which does not traverse syntax but still
            // shares this pack-level preflight budget.
            max_syntax_nodes: 200_000,
            ..FrameworkLimits::DEFAULT
        },
        adapter: FrameworkPackAdapter::Source(detect_enterprise),
    },
    FrameworkPack::config_versioned(
        "drupal-routing-config",
        is_drupal_routing,
        1,
        php::detect_drupal_routing,
    ),
    FrameworkPack::config_versioned("play-routes-config", is_play_routes, 1, play::detect),
    FrameworkPack::template_versioned(
        "filesystem-template-routes",
        &["@sveltejs/kit", "nuxt", "astro"],
        2,
        file_routes::detect,
    ),
];

/// Return the deterministic cache identity for the complete runtime framework
/// registry. This includes both universal descriptor contracts and established
/// adapters, because either path can change the meaning of a source file.
#[must_use]
pub fn framework_semantics_digest() -> String {
    let mut digest = Sha256::new();
    digest.update(FRAMEWORK_PACK_SEMANTICS_VERSION.as_bytes());
    digest.update(typescript_syntax::SYNTAX_VIEW_VERSION.as_bytes());
    for pack in FRAMEWORK_PACKS {
        let mut fields = vec![
            pack.id.to_owned(),
            format!("semantics_version:{}", pack.semantics_version),
            format!("kind:{:?}", pack.kind),
            format!("languages:{:?}", pack.languages),
            format!("dependencies:{:?}", pack.dependency_markers),
            format!("configuration:{:?}", pack.configuration_markers),
            format!("manifest:{:?}", pack.manifest_policy),
            format!("limits:{:?}", pack.limits),
        ];
        if let FrameworkPackAdapter::Universal { descriptor, .. } = pack.adapter {
            fields.push(format!("required:{:?}", descriptor.required_capabilities));
            fields.push(format!(
                "descriptor_semantics_version:{}",
                descriptor.semantics_version
            ));
            fields.push(format!("framework:{:?}", descriptor.framework_capabilities));
            fields.push(format!("activation:{:?}", descriptor.activation_rules));
            fields.push(format!("roles:{:?}", descriptor.accepted_roles));
            fields.push(format!(
                "relations:{:?}",
                descriptor.emitted_relation_families
            ));
            fields.push(format!("occurrence:{:?}", descriptor.occurrence_policy));
        }
        let encoded = fields.join("\u{1f}");
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

/// Return the reviewed semantics version for one registered framework pack.
///
/// Agent-facing consumers publish this value beside a pack ID so a response
/// cannot silently mix evidence from different extractor contracts. Unknown
/// IDs deliberately return `None`; callers must fail closed instead of
/// inventing a version.
#[must_use]
pub fn framework_pack_semantics_version(pack_id: &str) -> Option<u32> {
    FRAMEWORK_PACKS
        .iter()
        .find(|pack| pack.id == pack_id)
        .map(|pack| pack.semantics_version)
}

struct FrameworkFactAccumulator {
    facts: Vec<RawFrameworkFact>,
}

impl FrameworkFactAccumulator {
    fn new() -> Self {
        Self { facts: Vec::new() }
    }

    fn add(&mut self, pack: FrameworkPack, facts: Vec<RawFrameworkFact>) -> Result<(), String> {
        pack.check_fact_limit(facts.len())?;
        for fact in &facts {
            fact.validate().map_err(|error| {
                format!(
                    "framework pack {:?} emitted invalid {} fact: {error}",
                    pack.id,
                    fact_variant_name(fact)
                )
            })?;
        }
        let role_count = facts
            .iter()
            .filter(|fact| matches!(fact, RawFrameworkFact::Role(_)))
            .count()
            .saturating_add(
                self.facts
                    .iter()
                    .filter(|fact| matches!(fact, RawFrameworkFact::Role(_)))
                    .count(),
            );
        pack.limits
            .check_role_facts(role_count)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let relation_count = facts
            .iter()
            .filter(|fact| matches!(fact, RawFrameworkFact::Relation(_)))
            .count()
            .saturating_add(
                self.facts
                    .iter()
                    .filter(|fact| matches!(fact, RawFrameworkFact::Relation(_)))
                    .count(),
            );
        pack.limits
            .check_relation_facts(relation_count)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let route_count = facts
            .iter()
            .filter(|fact| matches!(fact, RawFrameworkFact::Route(_)))
            .count()
            .saturating_add(
                self.facts
                    .iter()
                    .filter(|fact| matches!(fact, RawFrameworkFact::Route(_)))
                    .count(),
            );
        pack.limits
            .check_route_nodes(route_count)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let route_stage_count = facts
            .iter()
            .filter_map(|fact| match fact {
                RawFrameworkFact::Route(route) => Some(route.stages.len()),
                _ => None,
            })
            .sum::<usize>()
            .saturating_add(
                self.facts
                    .iter()
                    .filter_map(|fact| match fact {
                        RawFrameworkFact::Route(route) => Some(route.stages.len()),
                        _ => None,
                    })
                    .sum::<usize>(),
            );
        pack.limits
            .check_route_stages(route_stage_count)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let mut retained_literal_bytes = 0usize;
        let mut regex_pattern_length = 0usize;
        let mut regex_complexity = 0usize;
        for fact in self.facts.iter().chain(facts.iter()) {
            match fact {
                RawFrameworkFact::Configuration(configuration) => {
                    if let Some(value) = configuration.value.as_ref() {
                        observe_json_budget(
                            value,
                            &mut retained_literal_bytes,
                            &mut regex_pattern_length,
                            &mut regex_complexity,
                        );
                    }
                    observe_json_budget(
                        &Value::Object(configuration.detail.clone()),
                        &mut retained_literal_bytes,
                        &mut regex_pattern_length,
                        &mut regex_complexity,
                    );
                }
                RawFrameworkFact::FileSet(file_set) => {
                    observe_json_budget(
                        &serde_json::json!({
                            "patterns": &file_set.patterns,
                            "negative_patterns": &file_set.negative_patterns,
                            "detail": &file_set.detail,
                        }),
                        &mut retained_literal_bytes,
                        &mut regex_pattern_length,
                        &mut regex_complexity,
                    );
                }
                _ => {}
            }
        }
        pack.limits
            .check_retained_literal_bytes(retained_literal_bytes)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        pack.limits
            .check_regex_pattern_length(regex_pattern_length)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        pack.limits
            .check_regex_complexity(regex_complexity)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let glob_patterns = self
            .facts
            .iter()
            .chain(facts.iter())
            .filter_map(|fact| match fact {
                RawFrameworkFact::FileSet(file_set) => Some(
                    file_set
                        .patterns
                        .len()
                        .saturating_add(file_set.negative_patterns.len()),
                ),
                _ => None,
            })
            .sum::<usize>();
        pack.limits
            .check_glob_patterns(glob_patterns)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        let observed = self.facts.len().saturating_add(facts.len());
        pack.limits
            .check_facts(observed)
            .map_err(|error| format!("framework pack {:?}: {error}", pack.id))?;
        self.facts.extend(facts);
        Ok(())
    }

    fn publish(self, extraction: &mut Extraction) {
        extraction.framework_facts.extend(self.facts);
    }
}

fn observe_json_budget(
    value: &Value,
    retained_literal_bytes: &mut usize,
    regex_pattern_length: &mut usize,
    regex_complexity: &mut usize,
) {
    match value {
        Value::String(value) => {
            *retained_literal_bytes = retained_literal_bytes.saturating_add(value.len());
        }
        Value::Array(values) => {
            for value in values {
                observe_json_budget(
                    value,
                    retained_literal_bytes,
                    regex_pattern_length,
                    regex_complexity,
                );
            }
        }
        Value::Object(values) => {
            let regex = values.get("kind").and_then(Value::as_str) == Some("regex");
            if regex && let Some(pattern) = values.get("find").and_then(Value::as_str) {
                *regex_pattern_length = regex_pattern_length.saturating_add(pattern.len());
                *regex_complexity = regex_complexity.saturating_add(
                    pattern
                        .chars()
                        .filter(|character| {
                            matches!(
                                character,
                                '*' | '+' | '?' | '{' | '}' | '(' | ')' | '[' | ']' | '|'
                            )
                        })
                        .count(),
                );
            }
            for value in values.values() {
                observe_json_budget(
                    value,
                    retained_literal_bytes,
                    regex_pattern_length,
                    regex_complexity,
                );
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn fact_variant_name(fact: &RawFrameworkFact) -> &'static str {
    match fact {
        RawFrameworkFact::Route(_) => "route",
        RawFrameworkFact::Domain(_) => "domain",
        RawFrameworkFact::Annotation(_) => "annotation",
        RawFrameworkFact::Role(_) => "role",
        RawFrameworkFact::Relation(_) => "relation",
        RawFrameworkFact::Configuration(_) => "configuration",
        RawFrameworkFact::FileSet(_) => "file_set",
    }
}

fn record_framework_error(extraction: &mut Extraction, error: String) {
    extraction
        .error
        .get_or_insert_with(|| format!("framework extraction failed: {error}"));
}

#[cfg(test)]
mod runtime_registry_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_runtime_pack_has_explicit_nonzero_version_and_unique_id() {
        let mut ids = BTreeSet::new();
        for pack in FRAMEWORK_PACKS {
            assert!(
                pack.semantics_version > 0,
                "pack {} has no semantics version",
                pack.id
            );
            assert!(ids.insert(pack.id), "duplicate runtime pack {}", pack.id);
        }
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
    if language == "dart" && extraction.semantic_evidence.is_some() {
        // Framework convention meaning is deliberately owned by this
        // registry boundary; the language producer only emits structural
        // universal evidence. Contextual facts additionally require positive
        // source/manifest activation in their owning pack.
        dart::append_convention_facts(path, source, project, extraction);
    }
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
    let facts = match pack.collect_config(path, source) {
        Ok(Some(facts)) => facts,
        Ok(None) => return extraction,
        Err(error) => {
            record_framework_error(&mut extraction, error);
            return extraction;
        }
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

fn detect_swift_universal(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let vapor_import = context.evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == crate::SemanticRole::Import
            && occurrence.spelling.eq_ignore_ascii_case("Vapor")
    });
    if !vapor_import {
        return Vec::new();
    }
    swift::detect(context.path, context.source, context.root)
}

fn detect_typescript(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    typescript::detect_non_express(context.path, context.source, context.root, extraction)
}

fn detect_react_router(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    typescript::detect_react_router(context.path, context.source, context.root, extraction)
}

fn detect_tanstack(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    tanstack::detect(context.path, context.source, context.root, context.project)
}

fn detect_tanstack_start(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    tanstack::detect_start(context.path, context.source, context.root, context.project)
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
    next::detect(
        context.path,
        context.source,
        context.root,
        context.project,
        extraction,
    )
}

fn detect_remix(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    remix::detect(
        context.path,
        context.source,
        context.root,
        context.project,
        extraction,
    )
}

fn detect_vite(
    context: &DetectionContext<'_, '_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    vite::detect(
        context.path,
        context.source,
        context.root,
        context.project,
        extraction,
    )
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

    use super::{FRAMEWORK_PACKS, FrameworkLimits, FrameworkPackKind};

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
            "django-python",
            "django-rest-framework-python",
            "fastapi-python",
            "flask-python",
            "pydantic-python",
            "sqlalchemy-python",
            "celery-python",
            "starlette-python",
            "php-frameworks",
            "rails-ruby",
            "spring-kotlin",
            "go-web",
            "axum-web",
            "rust-web",
            "aspnet-csharp",
            "vapor-swift",
            "dart-flutter-navigation",
            "dart-bloc",
            "dart-riverpod",
            "react-ui",
            "express-web",
            "fastify-web",
            "hono-web",
            "typescript-web",
            "react-router-routes",
            "tanstack-router",
            "tanstack-start",
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
            assert!(
                pack.semantics_version > 0,
                "missing semantics version for {}",
                pack.id
            );
            assert!(pack.limits.max_facts_per_file > 0);
            if pack.manifest_policy == super::FrameworkManifestPolicy::Required {
                assert!(!pack.dependency_markers.is_empty());
            }
        }
    }

    #[test]
    fn go_web_uses_explicit_budget_for_large_generated_sources() {
        let go_web = FRAMEWORK_PACKS
            .iter()
            .find(|pack| pack.id == "go-web")
            .expect("go-web framework pack");
        assert_eq!(go_web.limits.max_syntax_nodes, 200_000);
        assert_eq!(
            FrameworkLimits::DEFAULT.max_syntax_nodes,
            100_000,
            "the expanded budget must not change the shared default"
        );
    }

    #[test]
    fn enterprise_domain_pack_matches_large_source_budget() {
        let enterprise = FRAMEWORK_PACKS
            .iter()
            .find(|pack| pack.id == "enterprise-domain-facts");
        assert_eq!(
            enterprise.map(|pack| pack.limits.max_syntax_nodes),
            Some(200_000)
        );
        assert_eq!(
            enterprise.map(|pack| pack.languages),
            Some(
                &[
                    "typescript",
                    "tsx",
                    "javascript",
                    "csharp",
                    "ruby",
                    "php",
                    "go",
                    "rust",
                ][..]
            )
        );
        assert!(enterprise.is_some_and(|pack| !pack.languages.contains(&"python")));
    }

    #[test]
    fn react_router_dependency_is_owned_only_by_the_dedicated_route_pack() {
        let owners = FRAMEWORK_PACKS
            .iter()
            .filter(|pack| pack.dependency_markers.contains(&"react-router-dom"))
            .map(|pack| pack.id)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec!["react-ui", "react-router-routes"]);
        let broad = FRAMEWORK_PACKS
            .iter()
            .find(|pack| pack.id == "typescript-web")
            .expect("TypeScript web pack");
        assert!(!broad.dependency_markers.contains(&"react-router"));
        assert!(!broad.dependency_markers.contains(&"react-router-dom"));
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

    #[test]
    fn framework_semantics_digest_is_versioned_and_deterministic() {
        let first = super::framework_semantics_digest();
        let second = super::framework_semantics_digest();
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
        assert_eq!(first.len(), "sha256:".len() + 64);
    }
}
