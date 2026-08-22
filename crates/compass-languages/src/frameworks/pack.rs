use std::collections::BTreeSet;

use crate::{LanguageCapability, SemanticRole, UniversalEvidenceRegistry};

use super::FrameworkLimits;

/// Input boundary used by a universal framework evidence pack.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrameworkPackKind {
    Source,
    Config,
    Template,
}

/// Whether dependency-manifest evidence is required before a pack may run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrameworkManifestPolicy {
    Advisory,
    Required,
}

/// Provenance permitted for relationships emitted by a framework pack.
///
/// Both policies require a real source range. The heuristic form additionally
/// requires a named, reviewable activation rule on the descriptor.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrameworkOccurrencePolicy {
    ExactEvidence,
    ExactAnchoredHeuristic,
}

/// Framework meaning a pack is qualified to derive from language evidence.
///
/// These capabilities are deliberately separate from `LanguageCapability`:
/// Java may truthfully emit annotations and ownership without every
/// annotation consumer being qualified to infer Spring HTTP or bean meaning.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrameworkCapability {
    HttpRoutes,
    Beans,
    DependencyInjection,
    Messaging,
    Scheduling,
    Persistence,
    Transactions,
    Security,
}

/// Closed framework relationship vocabulary advertised by a universal pack.
///
/// The persisted spelling intentionally matches the Code Graph v1 edge
/// vocabulary. Pack registration therefore cannot hide a framework relation
/// behind a language-level `CandidateRelation` such as `Calls`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrameworkRelation {
    Decorates,
    RoutesTo,
    Registers,
    Handles,
    Publishes,
    Subscribes,
    Produces,
    Consumes,
    Schedules,
    Triggers,
    DependsOn,
    MapsTo,
}

impl FrameworkRelation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Decorates => "decorates",
            Self::RoutesTo => "routes_to",
            Self::Registers => "registers",
            Self::Handles => "handles",
            Self::Publishes => "publishes",
            Self::Subscribes => "subscribes",
            Self::Produces => "produces",
            Self::Consumes => "consumes",
            Self::Schedules => "schedules",
            Self::Triggers => "triggers",
            Self::DependsOn => "depends_on",
            Self::MapsTo => "maps_to",
        }
    }

    #[must_use]
    pub fn is_supported_by(self, capabilities: &[FrameworkCapability]) -> bool {
        let required = match self {
            Self::RoutesTo => Some(FrameworkCapability::HttpRoutes),
            Self::Registers => Some(FrameworkCapability::Beans),
            Self::DependsOn => Some(FrameworkCapability::DependencyInjection),
            Self::Handles
            | Self::Publishes
            | Self::Subscribes
            | Self::Produces
            | Self::Consumes => Some(FrameworkCapability::Messaging),
            Self::Schedules | Self::Triggers => Some(FrameworkCapability::Scheduling),
            Self::MapsTo => Some(FrameworkCapability::Persistence),
            Self::Decorates => None,
        };
        required.map_or_else(
            || {
                capabilities.contains(&FrameworkCapability::Transactions)
                    || capabilities.contains(&FrameworkCapability::Security)
            },
            |required| capabilities.contains(&required),
        )
    }
}

/// Static contract a framework pack must satisfy before registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameworkPackDescriptor {
    pub id: &'static str,
    pub kind: FrameworkPackKind,
    pub languages: &'static [&'static str],
    pub required_capabilities: &'static [LanguageCapability],
    pub framework_capabilities: &'static [FrameworkCapability],
    pub dependency_markers: &'static [&'static str],
    pub manifest_policy: FrameworkManifestPolicy,
    pub activation_rules: &'static [&'static str],
    pub accepted_roles: &'static [SemanticRole],
    pub emitted_relation_families: &'static [FrameworkRelation],
    pub occurrence_policy: FrameworkOccurrencePolicy,
    pub limits: FrameworkLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameworkPackRegistryError {
    #[error("framework pack ID must not be empty")]
    EmptyId,
    #[error("duplicate framework pack ID {0:?}")]
    DuplicateId(&'static str),
    #[error("framework pack {0:?} must declare at least one language")]
    EmptyLanguages(&'static str),
    #[error("framework pack {0:?} languages must be non-empty, sorted, and unique")]
    InvalidLanguages(&'static str),
    #[error("framework pack {0:?} must declare at least one required capability")]
    EmptyCapabilities(&'static str),
    #[error("framework pack {pack:?} references non-universal language {language:?}")]
    NonUniversalLanguage {
        pack: &'static str,
        language: &'static str,
    },
    #[error(
        "framework pack {pack:?} requires undeclared capability {capability:?} for {language:?}"
    )]
    UnsupportedCapability {
        pack: &'static str,
        language: &'static str,
        capability: LanguageCapability,
    },
    #[error("framework pack {0:?} capabilities must be sorted and unique")]
    InvalidCapabilityOrder(&'static str),
    #[error("framework pack {0:?} must accept at least one semantic role")]
    EmptyAcceptedRoles(&'static str),
    #[error("framework pack {0:?} semantic roles must be sorted and unique")]
    InvalidRoleOrder(&'static str),
    #[error("framework pack {pack:?} accepts role {role:?} without its required capability")]
    RoleCapabilityNotDeclared {
        pack: &'static str,
        role: SemanticRole,
    },
    #[error("framework pack {0:?} must emit at least one relationship family")]
    EmptyRelationFamilies(&'static str),
    #[error("framework pack {0:?} must declare at least one framework capability")]
    EmptyFrameworkCapabilities(&'static str),
    #[error("framework pack {0:?} framework capabilities must be sorted and unique")]
    InvalidFrameworkCapabilityOrder(&'static str),
    #[error("framework pack {0:?} relationship families must be sorted and unique")]
    InvalidRelationOrder(&'static str),
    #[error(
        "framework pack {pack:?} emits relationship {relation:?} without its required framework capability"
    )]
    RelationCapabilityNotDeclared {
        pack: &'static str,
        relation: FrameworkRelation,
    },
    #[error("framework pack {0:?} dependency markers must be non-empty, sorted, and unique")]
    InvalidDependencyMarkers(&'static str),
    #[error("framework pack {0:?} activation rules must be non-empty, sorted, and unique")]
    InvalidActivationRules(&'static str),
    #[error("framework pack {0:?} requires dependency markers for manifest activation")]
    MissingRequiredDependencyMarkers(&'static str),
    #[error("heuristic framework pack {0:?} requires at least one named activation rule")]
    MissingHeuristicRule(&'static str),
    #[error("framework pack {pack:?} has zero resource limit {limit}")]
    ZeroLimit {
        pack: &'static str,
        limit: &'static str,
    },
}

/// Registry boundary for packs that have atomically cut over to universal
/// semantic evidence. Existing raw framework detectors are intentionally not
/// projected into this registry.
#[derive(Debug, Default)]
pub struct FrameworkPackRegistry;

impl FrameworkPackRegistry {
    #[must_use]
    pub const fn descriptors() -> &'static [FrameworkPackDescriptor] {
        UNIVERSAL_FRAMEWORK_PACKS
    }

    pub fn validate() -> Result<(), FrameworkPackRegistryError> {
        Self::validate_descriptors(UNIVERSAL_FRAMEWORK_PACKS)
    }

    pub fn validate_descriptors(
        descriptors: &[FrameworkPackDescriptor],
    ) -> Result<(), FrameworkPackRegistryError> {
        let mut ids = BTreeSet::new();
        for descriptor in descriptors {
            validate_descriptor(descriptor)?;
            if !ids.insert(descriptor.id) {
                return Err(FrameworkPackRegistryError::DuplicateId(descriptor.id));
            }
        }
        Ok(())
    }
}

fn validate_descriptor(
    descriptor: &FrameworkPackDescriptor,
) -> Result<(), FrameworkPackRegistryError> {
    if descriptor.id.trim().is_empty() {
        return Err(FrameworkPackRegistryError::EmptyId);
    }
    if descriptor.languages.is_empty() {
        return Err(FrameworkPackRegistryError::EmptyLanguages(descriptor.id));
    }
    validate_strings(descriptor.languages)
        .map_err(|()| FrameworkPackRegistryError::InvalidLanguages(descriptor.id))?;
    validate_strings(descriptor.dependency_markers)
        .map_err(|()| FrameworkPackRegistryError::InvalidDependencyMarkers(descriptor.id))?;
    validate_strings(descriptor.activation_rules)
        .map_err(|()| FrameworkPackRegistryError::InvalidActivationRules(descriptor.id))?;
    if descriptor.manifest_policy == FrameworkManifestPolicy::Required
        && descriptor.dependency_markers.is_empty()
    {
        return Err(FrameworkPackRegistryError::MissingRequiredDependencyMarkers(descriptor.id));
    }
    if descriptor.occurrence_policy == FrameworkOccurrencePolicy::ExactAnchoredHeuristic
        && descriptor.activation_rules.is_empty()
    {
        return Err(FrameworkPackRegistryError::MissingHeuristicRule(
            descriptor.id,
        ));
    }
    if descriptor
        .required_capabilities
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(FrameworkPackRegistryError::InvalidCapabilityOrder(
            descriptor.id,
        ));
    }
    if descriptor.required_capabilities.is_empty() {
        return Err(FrameworkPackRegistryError::EmptyCapabilities(descriptor.id));
    }
    if descriptor.accepted_roles.is_empty() {
        return Err(FrameworkPackRegistryError::EmptyAcceptedRoles(
            descriptor.id,
        ));
    }
    if descriptor.framework_capabilities.is_empty() {
        return Err(FrameworkPackRegistryError::EmptyFrameworkCapabilities(
            descriptor.id,
        ));
    }
    if descriptor
        .framework_capabilities
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(FrameworkPackRegistryError::InvalidFrameworkCapabilityOrder(
            descriptor.id,
        ));
    }
    if descriptor
        .accepted_roles
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(FrameworkPackRegistryError::InvalidRoleOrder(descriptor.id));
    }
    if descriptor.emitted_relation_families.is_empty() {
        return Err(FrameworkPackRegistryError::EmptyRelationFamilies(
            descriptor.id,
        ));
    }
    if descriptor
        .emitted_relation_families
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(FrameworkPackRegistryError::InvalidRelationOrder(
            descriptor.id,
        ));
    }
    for &role in descriptor.accepted_roles {
        if !descriptor
            .required_capabilities
            .contains(&role.required_capability())
        {
            return Err(FrameworkPackRegistryError::RoleCapabilityNotDeclared {
                pack: descriptor.id,
                role,
            });
        }
    }
    for &relation in descriptor.emitted_relation_families {
        if !relation.is_supported_by(descriptor.framework_capabilities) {
            return Err(FrameworkPackRegistryError::RelationCapabilityNotDeclared {
                pack: descriptor.id,
                relation,
            });
        }
    }
    for &language in descriptor.languages {
        let Some(pipeline) = UniversalEvidenceRegistry::pipeline(language) else {
            return Err(FrameworkPackRegistryError::NonUniversalLanguage {
                pack: descriptor.id,
                language,
            });
        };
        for &capability in descriptor.required_capabilities {
            if !pipeline.producer.capabilities.contains(&capability) {
                return Err(FrameworkPackRegistryError::UnsupportedCapability {
                    pack: descriptor.id,
                    language,
                    capability,
                });
            }
        }
    }
    for (limit, value) in [
        ("max_candidates", descriptor.limits.max_candidates),
        ("max_include_depth", descriptor.limits.max_include_depth),
        (
            "max_alias_expansions",
            descriptor.limits.max_alias_expansions,
        ),
        ("max_facts_per_file", descriptor.limits.max_facts_per_file),
    ] {
        if value == 0 {
            return Err(FrameworkPackRegistryError::ZeroLimit {
                pack: descriptor.id,
                limit,
            });
        }
    }
    Ok(())
}

fn validate_strings(values: &[&str]) -> Result<(), ()> {
    if values.iter().any(|value| value.trim().is_empty())
        || values.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(());
    }
    Ok(())
}

pub(super) const SPRING_JAVA_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "spring-java",
    kind: FrameworkPackKind::Source,
    languages: &["java"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Calls,
        LanguageCapability::Construction,
        LanguageCapability::TypeReferences,
        LanguageCapability::BaseTypes,
        LanguageCapability::Members,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[
        FrameworkCapability::HttpRoutes,
        FrameworkCapability::Beans,
        FrameworkCapability::DependencyInjection,
        FrameworkCapability::Messaging,
        FrameworkCapability::Scheduling,
        FrameworkCapability::Persistence,
        FrameworkCapability::Transactions,
        FrameworkCapability::Security,
    ],
    dependency_markers: &[
        "org.springframework.boot:spring-boot",
        "org.springframework:spring-web",
    ],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &[
        "spring-annotation-import",
        "spring-direct-annotation",
        "spring-project-dependency",
    ],
    accepted_roles: &[
        SemanticRole::Import,
        SemanticRole::Call,
        SemanticRole::Construction,
        SemanticRole::Annotation,
        SemanticRole::BaseType,
        SemanticRole::TypeReference,
        SemanticRole::Ownership,
    ],
    emitted_relation_families: &[
        FrameworkRelation::Decorates,
        FrameworkRelation::RoutesTo,
        FrameworkRelation::Registers,
        FrameworkRelation::Handles,
        FrameworkRelation::Publishes,
        FrameworkRelation::Subscribes,
        FrameworkRelation::Produces,
        FrameworkRelation::Consumes,
        FrameworkRelation::Schedules,
        FrameworkRelation::Triggers,
        FrameworkRelation::DependsOn,
        FrameworkRelation::MapsTo,
    ],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits {
        max_candidates: 20,
        max_include_depth: 32,
        max_alias_expansions: 1_000,
        max_facts_per_file: 100_000,
    },
};

pub(super) const SPRING_KOTLIN_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "spring-kotlin",
    kind: FrameworkPackKind::Source,
    languages: &["kotlin"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Aliases,
        LanguageCapability::Calls,
        LanguageCapability::Construction,
        LanguageCapability::TypeReferences,
        LanguageCapability::BaseTypes,
        LanguageCapability::Members,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[
        FrameworkCapability::HttpRoutes,
        FrameworkCapability::Beans,
        FrameworkCapability::DependencyInjection,
        FrameworkCapability::Messaging,
        FrameworkCapability::Scheduling,
        FrameworkCapability::Persistence,
        FrameworkCapability::Transactions,
        FrameworkCapability::Security,
    ],
    dependency_markers: &[
        "org.springframework.boot:spring-boot",
        "org.springframework:spring-web",
    ],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &[
        "spring-annotation-import",
        "spring-direct-annotation",
        "spring-project-dependency",
    ],
    accepted_roles: &[
        SemanticRole::Import,
        SemanticRole::Call,
        SemanticRole::Construction,
        SemanticRole::Annotation,
        SemanticRole::BaseType,
        SemanticRole::TypeReference,
        SemanticRole::Ownership,
    ],
    emitted_relation_families: &[
        FrameworkRelation::Decorates,
        FrameworkRelation::RoutesTo,
        FrameworkRelation::Registers,
        FrameworkRelation::Handles,
        FrameworkRelation::Publishes,
        FrameworkRelation::Subscribes,
        FrameworkRelation::Produces,
        FrameworkRelation::Consumes,
        FrameworkRelation::Schedules,
        FrameworkRelation::Triggers,
        FrameworkRelation::DependsOn,
        FrameworkRelation::MapsTo,
    ],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits {
        max_candidates: 20,
        max_include_depth: 32,
        max_alias_expansions: 1_000,
        max_facts_per_file: 100_000,
    },
};

pub(super) const RAILS_RUBY_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "rails-ruby",
    kind: FrameworkPackKind::Source,
    languages: &["ruby"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Namespaces,
        LanguageCapability::Traits,
        LanguageCapability::Calls,
        LanguageCapability::Members,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[FrameworkCapability::HttpRoutes],
    dependency_markers: &["rails"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &["rails-routes-draw"],
    accepted_roles: &[SemanticRole::Call, SemanticRole::Ownership],
    emitted_relation_families: &[FrameworkRelation::RoutesTo],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits {
        max_candidates: 20,
        max_include_depth: 32,
        max_alias_expansions: 1_000,
        max_facts_per_file: 100_000,
    },
};

pub(super) const ASPNET_CSHARP_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "aspnet-csharp",
    kind: FrameworkPackKind::Source,
    languages: &["csharp"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Aliases,
        LanguageCapability::Decorators,
        LanguageCapability::TypeReferences,
        LanguageCapability::BaseTypes,
        LanguageCapability::Members,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[FrameworkCapability::HttpRoutes],
    dependency_markers: &["microsoft.aspnetcore.app"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &["aspnet-mvc-attribute-binding", "aspnet-project-dependency"],
    accepted_roles: &[
        SemanticRole::Import,
        SemanticRole::Annotation,
        SemanticRole::BaseType,
        SemanticRole::TypeReference,
        SemanticRole::Ownership,
    ],
    emitted_relation_families: &[FrameworkRelation::RoutesTo],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits {
        max_candidates: 64,
        max_include_depth: 32,
        max_alias_expansions: 1_000,
        max_facts_per_file: 100_000,
    },
};

pub(super) const PHP_FRAMEWORKS_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "php-frameworks",
    kind: FrameworkPackKind::Source,
    languages: &["php"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Aliases,
        LanguageCapability::Calls,
        LanguageCapability::Members,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[FrameworkCapability::HttpRoutes],
    dependency_markers: &["drupal/core", "laravel/framework"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &[
        "composer-dependency",
        "drupal-hook-declaration",
        "laravel-route-call",
    ],
    accepted_roles: &[
        SemanticRole::Import,
        SemanticRole::Call,
        SemanticRole::Ownership,
    ],
    emitted_relation_families: &[FrameworkRelation::RoutesTo],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits::DEFAULT,
};

pub(super) const VAPOR_SWIFT_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "vapor-swift",
    kind: FrameworkPackKind::Source,
    languages: &["swift"],
    required_capabilities: &[
        LanguageCapability::Declarations,
        LanguageCapability::LexicalScopes,
        LanguageCapability::Imports,
        LanguageCapability::Calls,
        LanguageCapability::Ownership,
    ],
    framework_capabilities: &[FrameworkCapability::HttpRoutes],
    dependency_markers: &["vapor"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &["vapor-import", "vapor-route-call"],
    accepted_roles: &[
        SemanticRole::Import,
        SemanticRole::Call,
        SemanticRole::Ownership,
    ],
    emitted_relation_families: &[FrameworkRelation::RoutesTo],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits::DEFAULT,
};

pub(super) const DART_FLUTTER_NAVIGATION_DESCRIPTOR: FrameworkPackDescriptor =
    FrameworkPackDescriptor {
        id: "dart-flutter-navigation",
        kind: FrameworkPackKind::Source,
        languages: &["dart"],
        required_capabilities: &[
            LanguageCapability::Imports,
            LanguageCapability::Calls,
            LanguageCapability::Receivers,
        ],
        framework_capabilities: &[FrameworkCapability::HttpRoutes],
        dependency_markers: &["flutter"],
        manifest_policy: FrameworkManifestPolicy::Advisory,
        activation_rules: &["flutter-navigation-call", "flutter-navigation-import"],
        accepted_roles: &[
            SemanticRole::Import,
            SemanticRole::Call,
            SemanticRole::Receiver,
        ],
        emitted_relation_families: &[FrameworkRelation::RoutesTo],
        occurrence_policy: FrameworkOccurrencePolicy::ExactAnchoredHeuristic,
        limits: FrameworkLimits::DEFAULT,
    };

pub(super) const DART_BLOC_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "dart-bloc",
    kind: FrameworkPackKind::Source,
    languages: &["dart"],
    required_capabilities: &[
        LanguageCapability::Calls,
        LanguageCapability::TypeReferences,
        LanguageCapability::Members,
    ],
    framework_capabilities: &[FrameworkCapability::Messaging],
    dependency_markers: &["bloc", "flutter_bloc"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &["bloc-builder", "bloc-event", "bloc-provider"],
    accepted_roles: &[
        SemanticRole::Call,
        SemanticRole::TypeReference,
        SemanticRole::MemberAccess,
    ],
    emitted_relation_families: &[FrameworkRelation::Handles],
    occurrence_policy: FrameworkOccurrencePolicy::ExactAnchoredHeuristic,
    limits: FrameworkLimits::DEFAULT,
};

pub(super) const DART_RIVERPOD_DESCRIPTOR: FrameworkPackDescriptor = FrameworkPackDescriptor {
    id: "dart-riverpod",
    kind: FrameworkPackKind::Source,
    languages: &["dart"],
    required_capabilities: &[LanguageCapability::Calls, LanguageCapability::Members],
    framework_capabilities: &[FrameworkCapability::Messaging],
    dependency_markers: &["hooks_riverpod", "riverpod"],
    manifest_policy: FrameworkManifestPolicy::Advisory,
    activation_rules: &["riverpod-provider", "riverpod-reference"],
    accepted_roles: &[SemanticRole::Call, SemanticRole::MemberAccess],
    emitted_relation_families: &[FrameworkRelation::Handles],
    occurrence_policy: FrameworkOccurrencePolicy::ExactAnchoredHeuristic,
    limits: FrameworkLimits::DEFAULT,
};

const UNIVERSAL_FRAMEWORK_PACKS: &[FrameworkPackDescriptor] = &[
    ASPNET_CSHARP_DESCRIPTOR,
    PHP_FRAMEWORKS_DESCRIPTOR,
    SPRING_JAVA_DESCRIPTOR,
    SPRING_KOTLIN_DESCRIPTOR,
    RAILS_RUBY_DESCRIPTOR,
    VAPOR_SWIFT_DESCRIPTOR,
    DART_BLOC_DESCRIPTOR,
    DART_FLUTTER_NAVIGATION_DESCRIPTOR,
    DART_RIVERPOD_DESCRIPTOR,
];
