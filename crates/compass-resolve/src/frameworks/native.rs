pub(super) fn canonical_reference(reference: &str) -> String {
    reference
        .trim()
        .trim_start_matches(['&', '*'])
        .trim_end_matches("...")
        .trim()
        .replace("::", ".")
}
