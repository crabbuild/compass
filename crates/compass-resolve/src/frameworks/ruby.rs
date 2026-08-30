use super::FrameworkResolutionError;

/// Rails routing is already fully source-derived by the language-side
/// universal pack. Keep a project-wide expansion hook registered for the
/// pack so the universal framework lifecycle remains uniform; Ruby does not
/// currently need a second pass over project facts.
pub(super) fn expand(
    _extraction: &mut compass_languages::Extraction,
) -> Result<(), FrameworkResolutionError> {
    Ok(())
}

pub(super) fn canonical_reference(reference: &str) -> String {
    reference.trim().replace(['#', '/'], ".")
}
