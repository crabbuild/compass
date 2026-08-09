use std::collections::BTreeSet;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Return deterministic normalized full and identifier-subword terms.
///
/// The full tokens preserve compatibility with existing search indexes while
/// the subwords make `OpenRepository`, `session_state`, and acronym-bearing
/// identifiers discoverable by their constituent words.
#[must_use]
pub fn identifier_search_terms(value: &str) -> BTreeSet<String> {
    let normalized = value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect::<String>();
    let mut terms = normalized
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>();

    for word in split_identifier_words(&normalized)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        terms.insert(word.to_lowercase());
    }
    terms
}

fn split_identifier_words(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut words = String::with_capacity(value.len());
    for (index, &character) in characters.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|at| characters.get(at));
        let next = characters.get(index + 1);
        let boundary = character.is_uppercase()
            && previous.is_some_and(|value| {
                value.is_lowercase()
                    || value.is_numeric()
                    || (value.is_uppercase() && next.is_some_and(|next| next.is_lowercase()))
            });
        if boundary {
            words.push(' ');
        }
        words.push(character);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::identifier_search_terms;

    #[test]
    fn preserves_full_tokens_and_adds_identifier_subwords() {
        assert_eq!(
            identifier_search_terms("HTTPCheckpoint_session_state"),
            [
                "checkpoint",
                "http",
                "httpcheckpoint_session_state",
                "session",
                "state",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
    }
}
