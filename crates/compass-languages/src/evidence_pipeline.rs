use crate::LanguageCapability;

/// Qualification state of one universal evidence pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalEvidenceQualification {
    /// The shared evidence route is production-active while its audit runs.
    Qualifying,
    /// The shared evidence route passed the complete audit gates.
    Qualified,
}

/// Language-specific metadata for a universal evidence producer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalEvidenceProducer {
    pub id: &'static str,
    pub language: &'static str,
    pub version: u32,
    pub evidence_schema: &'static str,
    pub capabilities: &'static [LanguageCapability],
}

/// The shared evidence pipeline and the qualification state of its producer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalEvidencePipeline {
    pub producer: UniversalEvidenceProducer,
    pub qualification: UniversalEvidenceQualification,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum UniversalEvidenceRegistryError {
    #[error("universal evidence producer id must not be empty")]
    EmptyId,
    #[error("universal evidence producer language must not be empty")]
    EmptyLanguage,
    #[error("duplicate universal evidence producer id {0:?}")]
    DuplicateId(&'static str),
    #[error("universal evidence producer {0:?} must declare at least one capability")]
    EmptyCapabilities(&'static str),
    #[error("duplicate universal evidence producer language {0:?}")]
    DuplicateLanguage(&'static str),
    #[error("universal evidence producer languages must be sorted before {0:?}")]
    UnsortedLanguages(&'static str),
    #[error("universal evidence producer {0:?} capabilities must be sorted and unique")]
    InvalidCapabilityOrder(&'static str),
    #[error("universal evidence producer {0:?} must declare a positive producer version")]
    InvalidVersion(&'static str),
    #[error("universal evidence producer {0:?} declares an unsupported evidence schema")]
    InvalidEvidenceSchema(&'static str),
}

/// Registry of languages that have atomically hard-cut to the shared evidence pipeline.
#[derive(Debug, Default)]
pub struct UniversalEvidenceRegistry;

impl UniversalEvidenceRegistry {
    #[must_use]
    pub fn pipeline(language: &str) -> Option<&'static UniversalEvidencePipeline> {
        UNIVERSAL_EVIDENCE_PIPELINES
            .binary_search_by_key(&language, |pipeline| pipeline.producer.language)
            .ok()
            .map(|index| &UNIVERSAL_EVIDENCE_PIPELINES[index])
    }

    #[must_use]
    pub const fn pipelines() -> &'static [UniversalEvidencePipeline] {
        UNIVERSAL_EVIDENCE_PIPELINES
    }

    pub fn validate() -> Result<(), UniversalEvidenceRegistryError> {
        let mut previous_language = None;
        let mut ids = std::collections::BTreeSet::new();
        for pipeline in UNIVERSAL_EVIDENCE_PIPELINES {
            let producer = pipeline.producer;
            if producer.id.is_empty() {
                return Err(UniversalEvidenceRegistryError::EmptyId);
            }
            if producer.language.is_empty() {
                return Err(UniversalEvidenceRegistryError::EmptyLanguage);
            }
            if !ids.insert(producer.id) {
                return Err(UniversalEvidenceRegistryError::DuplicateId(producer.id));
            }
            if producer.version == 0 {
                return Err(UniversalEvidenceRegistryError::InvalidVersion(
                    producer.language,
                ));
            }
            if producer.evidence_schema != crate::UNIVERSAL_EVIDENCE_SCHEMA {
                return Err(UniversalEvidenceRegistryError::InvalidEvidenceSchema(
                    producer.language,
                ));
            }
            if producer.capabilities.is_empty() {
                return Err(UniversalEvidenceRegistryError::EmptyCapabilities(
                    producer.language,
                ));
            }
            if previous_language == Some(producer.language) {
                return Err(UniversalEvidenceRegistryError::DuplicateLanguage(
                    producer.language,
                ));
            }
            if previous_language.is_some_and(|previous| previous > producer.language) {
                return Err(UniversalEvidenceRegistryError::UnsortedLanguages(
                    producer.language,
                ));
            }
            if producer
                .capabilities
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(UniversalEvidenceRegistryError::InvalidCapabilityOrder(
                    producer.language,
                ));
            }
            previous_language = Some(producer.language);
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

const CSHARP_CAPABILITIES: &[LanguageCapability] = &[
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
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
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

const JAVASCRIPT_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
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

const TYPESCRIPT_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
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
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const PHP_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Traits,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::Decorators,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const KOTLIN_CAPABILITIES: &[LanguageCapability] = &[
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
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

pub(crate) const RUBY_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Traits,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

// Conservative capabilities emitted by the AST-first language-specific
// producers. Project-wide target selection and framework conventions remain
// outside the language boundary.
const DART_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Imports,
    LanguageCapability::Reexports,
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

const GROOVY_CAPABILITIES: &[LanguageCapability] = &[
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
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const SCALA_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Traits,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const SWIFT_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Traits,
    LanguageCapability::Imports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const DART_EVIDENCE_PIPELINE: UniversalEvidencePipeline = UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer {
        id: "compass.dart",
        language: "dart",
        version: 1,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        capabilities: DART_CAPABILITIES,
    },
    qualification: UniversalEvidenceQualification::Qualifying,
};

const GROOVY_EVIDENCE_PIPELINE: UniversalEvidencePipeline = UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer {
        id: "compass.groovy",
        language: "groovy",
        version: 1,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        capabilities: GROOVY_CAPABILITIES,
    },
    qualification: UniversalEvidenceQualification::Qualifying,
};

const SCALA_EVIDENCE_PIPELINE: UniversalEvidencePipeline = UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer {
        id: "compass.scala",
        language: "scala",
        version: 1,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        capabilities: SCALA_CAPABILITIES,
    },
    qualification: UniversalEvidenceQualification::Qualifying,
};

const SWIFT_EVIDENCE_PIPELINE: UniversalEvidencePipeline = UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer {
        id: "compass.swift",
        language: "swift",
        version: 1,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        capabilities: SWIFT_CAPABILITIES,
    },
    qualification: UniversalEvidenceQualification::Qualifying,
};

pub(crate) const RUBY_EVIDENCE_PIPELINE: UniversalEvidencePipeline = UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer {
        id: "compass.ruby",
        language: "ruby",
        version: 1,
        evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
        capabilities: RUBY_CAPABILITIES,
    },
    qualification: UniversalEvidenceQualification::Qualifying,
};

const UNIVERSAL_EVIDENCE_PIPELINES: &[UniversalEvidencePipeline] = &[
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.csharp",
            language: "csharp",
            version: 1,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: CSHARP_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    DART_EVIDENCE_PIPELINE,
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.go",
            language: "go",
            version: 3,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: GO_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    GROOVY_EVIDENCE_PIPELINE,
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.java",
            language: "java",
            version: 3,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: JAVA_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.javascript",
            language: "javascript",
            version: 5,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: JAVASCRIPT_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.kotlin",
            language: "kotlin",
            version: 1,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: KOTLIN_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.php",
            language: "php",
            version: 1,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: PHP_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.python",
            language: "python",
            version: 11,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: PYTHON_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    RUBY_EVIDENCE_PIPELINE,
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.rust",
            language: "rust",
            version: 15,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: RUST_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
    SCALA_EVIDENCE_PIPELINE,
    SWIFT_EVIDENCE_PIPELINE,
    UniversalEvidencePipeline {
        producer: UniversalEvidenceProducer {
            id: "compass.typescript",
            language: "typescript",
            version: 5,
            evidence_schema: crate::UNIVERSAL_EVIDENCE_SCHEMA,
            capabilities: TYPESCRIPT_CAPABILITIES,
        },
        qualification: UniversalEvidenceQualification::Qualifying,
    },
];
