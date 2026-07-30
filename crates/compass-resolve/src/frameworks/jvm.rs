pub(super) fn canonical_reference(reference: &str) -> String {
    reference
        .trim()
        .trim_start_matches('@')
        .trim_start_matches("controllers.")
        .trim_end_matches("()")
        .to_owned()
}
