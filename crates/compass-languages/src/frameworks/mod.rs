mod file_routes;
mod model;
mod python;
mod typescript;

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
        "javascript" | "typescript" | "tsx" => {
            let mut facts = typescript::detect(path, source, root);
            typescript::attach_import_aliases(path, source, root, extraction);
            facts.extend(file_routes::detect(path, source, extraction));
            facts
        }
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

pub(crate) fn detect_template_file_route(path: &Path, source: &[u8], extraction: &mut Extraction) {
    let facts = file_routes::detect(path, source, extraction);
    if let Err(error) = FrameworkLimits::default().check_facts(facts.len()) {
        extraction
            .error
            .get_or_insert_with(|| format!("framework extraction failed: {error}"));
        return;
    }
    extraction.framework_facts.extend(facts);
}
