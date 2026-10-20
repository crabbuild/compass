//! Shared pre-retention bounds for observed TypeScript source loads.

use super::{
    EvidenceError, EvidenceErrorCode, MAX_TYPESCRIPT_MERGE_DEPENDENCIES,
    MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES, MAX_TYPESCRIPT_MERGE_DEPENDENCY_TEXT_BYTES,
    TypeScriptMergeDependency,
};

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TypeScriptMergeDependencyUsage {
    pub count: usize,
    pub text_bytes: usize,
}

impl TypeScriptMergeDependencyUsage {
    pub fn retaining(self, specifier: &str, source: &str) -> Result<Self, EvidenceError> {
        if specifier.len() > MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES || source.len() > 4_096
        {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "TypeScript merge dependency specifier or source exceeds byte limit",
            ));
        }
        if specifier.is_empty() || specifier.contains(['\\', '\0']) {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "TypeScript merge dependency requires an unescaped nonempty specifier",
            ));
        }
        let next = Self {
            count: self.count.saturating_add(1),
            text_bytes: self
                .text_bytes
                .saturating_add(specifier.len())
                .saturating_add(source.len()),
        };
        if next.count > MAX_TYPESCRIPT_MERGE_DEPENDENCIES
            || next.text_bytes > MAX_TYPESCRIPT_MERGE_DEPENDENCY_TEXT_BYTES
        {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "TypeScript merge dependency inventory exceeds count or text limit",
            ));
        }
        Ok(next)
    }

    pub fn for_dependencies(
        dependencies: &[TypeScriptMergeDependency],
    ) -> Result<Self, EvidenceError> {
        if dependencies.len() > MAX_TYPESCRIPT_MERGE_DEPENDENCIES {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "TypeScript merge dependency inventory exceeds count limit",
            ));
        }
        dependencies
            .iter()
            .try_fold(Self::default(), |usage, dependency| {
                usage.retaining(&dependency.specifier, &dependency.range.source_file)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_retention_bounds_are_inclusive_and_count_copied_text() -> Result<(), EvidenceError>
    {
        let count = TypeScriptMergeDependencyUsage {
            count: MAX_TYPESCRIPT_MERGE_DEPENDENCIES - 1,
            text_bytes: 0,
        };
        let exact = count.retaining("a", "b")?;
        assert_eq!(exact.count, MAX_TYPESCRIPT_MERGE_DEPENDENCIES);
        assert!(exact.retaining("a", "b").is_err());
        let bytes = TypeScriptMergeDependencyUsage {
            count: 0,
            text_bytes: MAX_TYPESCRIPT_MERGE_DEPENDENCY_TEXT_BYTES - 2,
        };
        let exact = bytes.retaining("a", "b")?;
        assert_eq!(exact.text_bytes, MAX_TYPESCRIPT_MERGE_DEPENDENCY_TEXT_BYTES);
        assert!(exact.retaining("a", "b").is_err());
        TypeScriptMergeDependencyUsage::default().retaining(
            &"a".repeat(MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES),
            "main.ts",
        )?;
        assert!(
            TypeScriptMergeDependencyUsage::default()
                .retaining(
                    &"a".repeat(MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES + 1),
                    "main.ts"
                )
                .is_err()
        );
        Ok(())
    }
}
