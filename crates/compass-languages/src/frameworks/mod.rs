mod csharp;
mod enterprise;
mod evidence;
mod file_routes;
mod go;
mod java;
mod model;
mod php;
mod play;
mod python;
mod ruby;
mod rust;
mod swift;
mod text;
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
        "php" => php::detect(path, source, root),
        "ruby" => ruby::detect(path, source, root),
        "java" | "kotlin" => java::detect(path, source, root),
        "go" => go::detect(path, source, root),
        "rust" => rust::detect(path, source, root),
        "csharp" => csharp::detect(path, source, root),
        "swift" => swift::detect(path, source, root),
        "javascript" | "typescript" | "tsx" => {
            let mut facts = typescript::detect(path, source, root, extraction);
            facts.extend(file_routes::detect(path, source, extraction));
            facts
        }
        _ => Vec::new(),
    };
    let mut facts = facts;
    facts.extend(enterprise::detect(path, source, language));
    if let Err(error) = FrameworkLimits::default().check_facts(facts.len()) {
        extraction
            .error
            .get_or_insert_with(|| format!("framework extraction failed: {error}"));
        return;
    }
    extraction.framework_facts.extend(facts);
}

pub(crate) fn detect_config_file(path: &Path, source: &[u8]) -> Extraction {
    let mut extraction = Extraction::default();
    let facts = if path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".routing.yml") || name.ends_with(".routing.yaml"))
    {
        php::detect_drupal_routing(path, source)
    } else {
        play::detect(path, source)
    };
    if let Err(error) = FrameworkLimits::default().check_facts(facts.len()) {
        extraction.error = Some(format!("framework extraction failed: {error}"));
    } else {
        extraction.framework_facts = facts;
    }
    extraction
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
