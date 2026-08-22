//! Project-wide expansion for the universal Vapor/Swift framework pack.

use super::FrameworkResolutionError;

/// Vapor route facts are already emitted by the source-side universal pack.
/// Keep an explicit resolver adapter so the pack participates in the same
/// lifecycle and cannot silently bypass the universal framework registry.
pub(super) fn expand(
    _extraction: &mut compass_languages::Extraction,
) -> Result<(), FrameworkResolutionError> {
    Ok(())
}
