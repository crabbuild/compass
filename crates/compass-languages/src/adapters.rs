use crate::LanguageCapability;

/// Publication maturity of one hard-cut universal adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalAdapterProfile {
    UniversalCandidate,
    UniversalComplete,
}

/// The universal evidence capabilities implemented by one hard-cut adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterProfile {
    pub id: &'static str,
    pub language: &'static str,
    pub version: u32,
    pub evidence_schema: &'static str,
    pub profile: UniversalAdapterProfile,
    pub capabilities: &'static [LanguageCapability],
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AdapterRegistryError {
    #[error("universal adapter id must not be empty")]
    EmptyId,
    #[error("universal adapter language must not be empty")]
    EmptyLanguage,
    #[error("duplicate universal adapter id {0:?}")]
    DuplicateId(&'static str),
    #[error("universal adapter {0:?} must declare at least one capability")]
    EmptyCapabilities(&'static str),
    #[error("duplicate universal adapter language {0:?}")]
    DuplicateLanguage(&'static str),
    #[error("universal adapter languages must be sorted before {0:?}")]
    UnsortedLanguages(&'static str),
    #[error("universal adapter {0:?} capabilities must be sorted and unique")]
    InvalidCapabilityOrder(&'static str),
    #[error("universal adapter {0:?} must declare a positive adapter version")]
    InvalidVersion(&'static str),
    #[error("universal adapter {0:?} declares an unsupported evidence schema")]
    InvalidEvidenceSchema(&'static str),
}

/// Registry of languages that have atomically hard-cut to universal evidence.
#[derive(Debug, Default)]
pub struct AdapterRegistry;

impl AdapterRegistry {
    #[must_use]
    pub fn universal_profile(language: &str) -> Option<&'static AdapterProfile> {
        UNIVERSAL_ADAPTERS
            .binary_search_by_key(&language, |profile| profile.language)
            .ok()
            .map(|index| &UNIVERSAL_ADAPTERS[index])
    }

    #[must_use]
    pub const fn universal_profiles() -> &'static [AdapterProfile] {
        UNIVERSAL_ADAPTERS
    }

    pub fn validate() -> Result<(), AdapterRegistryError> {
        let mut previous_language = None;
        let mut ids = std::collections::BTreeSet::new();
        for profile in UNIVERSAL_ADAPTERS {
            if profile.id.is_empty() {
                return Err(AdapterRegistryError::EmptyId);
            }
            if profile.language.is_empty() {
                return Err(AdapterRegistryError::EmptyLanguage);
            }
            if !ids.insert(profile.id) {
                return Err(AdapterRegistryError::DuplicateId(profile.id));
            }
            if profile.version == 0 {
                return Err(AdapterRegistryError::InvalidVersion(profile.language));
            }
            if profile.evidence_schema != crate::UNIVERSAL_EVIDENCE_SCHEMA {
                return Err(AdapterRegistryError::InvalidEvidenceSchema(
                    profile.language,
                ));
            }
            if profile.capabilities.is_empty() {
                return Err(AdapterRegistryError::EmptyCapabilities(profile.language));
            }
            if previous_language == Some(profile.language) {
                return Err(AdapterRegistryError::DuplicateLanguage(profile.language));
            }
            if previous_language.is_some_and(|previous| previous > profile.language) {
                return Err(AdapterRegistryError::UnsortedLanguages(profile.language));
            }
            if profile
                .capabilities
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(AdapterRegistryError::InvalidCapabilityOrder(
                    profile.language,
                ));
            }
            previous_language = Some(profile.language);
        }
        Ok(())
    }
}

const GO_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::Embedding,
    LanguageCapability::ExternalReferences,
];

const PYTHON_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Imports,
    LanguageCapability::Reexports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::Decorators,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::ExternalReferences,
];

const JAVA_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::Decorators,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const RUST_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Traits,
    LanguageCapability::ImplOwnership,
    LanguageCapability::Macros,
    LanguageCapability::Tests,
    LanguageCapability::Imports,
    LanguageCapability::Reexports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::ExternalReferences,
];

const UNIVERSAL_ADAPTERS: &[AdapterProfile] = &[
    AdapterProfile {
        id: "compass.go",
        language: "go",
        version: 3,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        profile: UniversalAdapterProfile::UniversalCandidate,
        capabilities: GO_CAPABILITIES,
    },
    AdapterProfile {
        id: "compass.java",
        language: "java",
        version: 3,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        profile: UniversalAdapterProfile::UniversalCandidate,
        capabilities: JAVA_CAPABILITIES,
    },
    AdapterProfile {
        id: "compass.python",
        language: "python",
        version: 11,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        profile: UniversalAdapterProfile::UniversalCandidate,
        capabilities: PYTHON_CAPABILITIES,
    },
    AdapterProfile {
        id: "compass.rust",
        language: "rust",
        version: 9,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        profile: UniversalAdapterProfile::UniversalCandidate,
        capabilities: RUST_CAPABILITIES,
    },
];
