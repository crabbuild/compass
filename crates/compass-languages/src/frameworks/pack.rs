use std::collections::BTreeSet;

use crate::{AdapterRegistry, CandidateRelation, LanguageCapability, SemanticRole};

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

/// Static contract a framework pack must satisfy before registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameworkPackDescriptor {
    pub id: &'static str,
    pub kind: FrameworkPackKind,
    pub languages: &'static [&'static str],
    pub required_capabilities: &'static [LanguageCapability],
    pub dependency_markers: &'static [&'static str],
    pub manifest_policy: FrameworkManifestPolicy,
    pub activation_rules: &'static [&'static str],
    pub accepted_roles: &'static [SemanticRole],
    pub emitted_relation_families: &'static [CandidateRelation],
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
    #[error("framework pack {0:?} relationship families must be sorted and unique")]
    InvalidRelationOrder(&'static str),
    #[error(
        "framework pack {pack:?} emits relationship {relation:?} without its required capability"
    )]
    RelationCapabilityNotDeclared {
        pack: &'static str,
        relation: CandidateRelation,
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
        if !descriptor
            .required_capabilities
            .contains(&relation.required_capability())
        {
            return Err(FrameworkPackRegistryError::RelationCapabilityNotDeclared {
                pack: descriptor.id,
                relation,
            });
        }
    }
    for &language in descriptor.languages {
        let Some(profile) = AdapterRegistry::universal_profile(language) else {
            return Err(FrameworkPackRegistryError::NonUniversalLanguage {
                pack: descriptor.id,
                language,
            });
        };
        for &capability in descriptor.required_capabilities {
            if !profile.capabilities.contains(&capability) {
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

const UNIVERSAL_FRAMEWORK_PACKS: &[FrameworkPackDescriptor] = &[];
