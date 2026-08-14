//! Closed, statically dispatched language-policy selection.

use compass_languages::{DeclarationFact, HierarchyConstraint, RelationshipCandidate};

use super::super::ResolutionDecision;
use super::super::resolve::context::ResolutionDb;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::evidence) enum LanguagePolicyKind {
    TypeScript,
    CSharp,
    Java,
    Php,
    Rust,
    Generic,
}

impl LanguagePolicyKind {
    pub(in crate::evidence) fn for_language(language: &str) -> Self {
        match language {
            "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => Self::TypeScript,
            "csharp" => Self::CSharp,
            "java" => Self::Java,
            "php" => Self::Php,
            "rust" => Self::Rust,
            _ => Self::Generic,
        }
    }

    pub(in crate::evidence) fn resolve_candidate(
        self,
        db: &ResolutionDb<'_>,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        match self {
            Self::TypeScript => db.resolve_typescript_import_candidate(language, candidate),
            Self::CSharp => db.resolve_csharp_candidate(language, candidate),
            Self::Php => db.resolve_php_candidate(language, candidate),
            Self::Rust => {
                let HierarchyConstraint::RustAssociatedType {
                    receiver_declaration_id,
                    receiver_qualified_name,
                    trait_qualified_name,
                } = candidate.constraints.hierarchy.as_ref()?
                else {
                    return None;
                };
                Some(db.resolve_rust_associated_type(
                    language,
                    receiver_declaration_id,
                    receiver_qualified_name,
                    trait_qualified_name,
                    candidate,
                ))
            }
            Self::Java => db.resolve_java_same_package_builtin_collision(candidate),
            Self::Generic => None,
        }
    }

    pub(in crate::evidence) fn unique_applicable_overload<'a>(
        self,
        db: &ResolutionDb<'_>,
        overloads: &[&'a DeclarationFact],
        argument_types: &[Option<String>],
    ) -> Option<&'a str> {
        match self {
            Self::Java => db.unique_java_applicable_overload(overloads, argument_types),
            Self::CSharp | Self::Php | Self::TypeScript | Self::Rust | Self::Generic => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LanguagePolicyKind;

    #[test]
    fn policy_selection_is_closed_and_unknown_languages_are_generic() {
        assert_eq!(
            LanguagePolicyKind::for_language("typescriptreact"),
            LanguagePolicyKind::TypeScript
        );
        assert_eq!(
            LanguagePolicyKind::for_language("csharp"),
            LanguagePolicyKind::CSharp
        );
        assert_eq!(
            LanguagePolicyKind::for_language("java"),
            LanguagePolicyKind::Java
        );
        assert_eq!(
            LanguagePolicyKind::for_language("rust"),
            LanguagePolicyKind::Rust
        );
        assert_eq!(
            LanguagePolicyKind::for_language("php"),
            LanguagePolicyKind::Php
        );
        assert_eq!(
            LanguagePolicyKind::for_language("future-language"),
            LanguagePolicyKind::Generic
        );
    }
}
