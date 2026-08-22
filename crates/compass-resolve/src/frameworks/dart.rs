//! Project-wide adapters for the bounded Dart convention packs.

use super::FrameworkResolutionError;

/// Convention relationships are already source-anchored by the language-side
/// framework bridge. Keep explicit no-op adapters so pack IDs have a stable,
/// one-to-one resolver registration and cannot silently fall through a broad
/// Dart/JVM/native resolver.
pub(super) fn expand(
    _extraction: &mut compass_languages::Extraction,
) -> Result<(), FrameworkResolutionError> {
    Ok(())
}
