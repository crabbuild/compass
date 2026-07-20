use std::path::Path;

use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

#[must_use]
pub fn normalize_id(value: &str) -> String {
    let normalized = value.nfkc().collect::<String>();
    let mut output = String::with_capacity(normalized.len());
    let mut separator = false;
    for character in normalized.chars() {
        if character.is_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('_');
            }
            output.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    output.trim_matches('_').case_fold().collect::<String>()
}

#[must_use]
pub fn make_id(parts: &[&str]) -> String {
    normalize_id(
        &parts
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| part.trim_matches(['_', '.']))
            .collect::<Vec<_>>()
            .join("_"),
    )
}

#[must_use]
pub fn file_stem(path: &Path) -> String {
    if path.file_name().is_none() {
        return String::new();
    }
    path.with_extension("").to_string_lossy().replace('\\', "/")
}
