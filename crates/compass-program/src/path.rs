use std::path::{Component, Path};

use crate::ProviderError;

pub fn normalize_source_path(path: &str) -> Result<String, ProviderError> {
    if path.is_empty() || path.contains('\0') {
        return Err(ProviderError::UnsafePath(path.to_owned()));
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/')
        || has_windows_prefix(&normalized)
        || normalized.split('/').any(|part| part.is_empty())
    {
        return Err(ProviderError::UnsafePath(path.to_owned()));
    }
    let parsed = Path::new(&normalized);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProviderError::UnsafePath(path.to_owned()));
    }
    let first = normalized.split('/').next().unwrap_or_default();
    if matches!(first, "compass-out" | "graphify-out" | ".compass-cache")
        || normalized.contains("/cache/program-")
    {
        return Err(ProviderError::UnsafePath(path.to_owned()));
    }
    Ok(normalized)
}

fn has_windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::normalize_source_path;

    #[test]
    fn normalizes_separators_and_rejects_unsafe_paths() -> Result<(), crate::ProviderError> {
        assert_eq!(normalize_source_path("src\\lib.rs")?, "src/lib.rs");
        for path in [
            "",
            "/tmp/lib.rs",
            "C:/src/lib.rs",
            "src/../lib.rs",
            "./src/lib.rs",
            "src//lib.rs",
            "compass-out/program.json",
        ] {
            assert!(normalize_source_path(path).is_err(), "{path}");
        }
        Ok(())
    }
}
