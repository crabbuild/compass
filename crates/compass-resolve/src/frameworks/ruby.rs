pub(super) fn canonical_reference(reference: &str) -> String {
    reference.trim().replace(['#', '/'], ".")
}
