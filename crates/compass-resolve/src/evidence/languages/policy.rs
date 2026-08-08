//! Closed, statically dispatched language-policy selection.

use compass_languages::RelationshipCandidate;

use super::super::{ResolutionDecision, UniversalResolutionIndex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::evidence) enum LanguagePolicyKind {
    TypeScript,
    Java,
    Rust,
    Generic,
}

impl LanguagePolicyKind {
    pub(in crate::evidence) fn for_language(language: &str) -> Self {
        match language {
            "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => Self::TypeScript,
            "java" => Self::Java,
            "rust" => Self::Rust,
            _ => Self::Generic,
        }
    }

    pub(in crate::evidence) fn resolve_import_candidate(
        self,
        index: &UniversalResolutionIndex,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        match self {
            Self::TypeScript => index.resolve_typescript_import_candidate(language, candidate),
            Self::Java | Self::Rust | Self::Generic => None,
        }
    }
}
