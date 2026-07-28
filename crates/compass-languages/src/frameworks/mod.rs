mod model;
mod python;

pub use model::{
    FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact,
    RawFrameworkOrigin, RawRouteFact,
};

use std::path::Path;

use tree_sitter::Node;

use crate::Extraction;

pub(crate) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    language: &str,
    extraction: &mut Extraction,
) {
    let facts = match language {
        "python" => python::detect(path, source, root),
        _ => Vec::new(),
    };
    if let Err(error) = FrameworkLimits::default().check_facts(facts.len()) {
        extraction
            .error
            .get_or_insert_with(|| format!("framework extraction failed: {error}"));
        return;
    }
    extraction.framework_facts.extend(facts);
}
