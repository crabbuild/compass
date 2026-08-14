use super::FrameworkResolutionError;

pub(super) fn expand(
    _extraction: &mut compass_languages::Extraction,
) -> Result<(), FrameworkResolutionError> {
    // Laravel resource/group composition is source-local and already expanded
    // by the evidence-gated language pack. Drupal hook and routing facts need
    // no project-wide composition, but the universal pack still participates
    // explicitly in the shared expansion lifecycle.
    Ok(())
}

pub(super) fn canonical_reference(reference: &str) -> String {
    reference
        .trim()
        .trim_matches(['\'', '"'])
        .trim_start_matches('\\')
        .trim_end_matches("::class")
        .replace(['\\', '@'], ".")
        .replace("::", ".")
        .to_ascii_lowercase()
}
