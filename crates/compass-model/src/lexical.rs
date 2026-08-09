use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Remove combining marks from a Unicode normalization of `text`.
#[must_use]
pub fn strip_diacritics(text: &str) -> String {
    text.nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect()
}

/// Split a code-oriented identifier into deterministic lowercase terms.
///
/// This handles Unicode words, punctuation, snake case, camel case, and
/// acronym boundaries. It intentionally does not remove query stop words.
#[must_use]
pub fn identifier_tokens(text: &str) -> Vec<String> {
    unicode_words(&split_identifier_words(&strip_diacritics(text)).to_lowercase())
}

/// Canonicalize one ASCII code-search term with bounded morphology.
///
/// Callers remain responsible for query-specific stop-word policy. Non-ASCII
/// terms are returned unchanged so language-specific text is not rewritten by
/// English suffix rules.
#[must_use]
pub fn canonical_code_token(token: String) -> String {
    if !token.is_ascii() {
        return token;
    }

    match token.as_str() {
        "resolution" | "resolved" | "resolver" | "resolving" => "resolve".to_owned(),
        "solved" | "solving" => "solve".to_owned(),
        "compiled" | "compiling" => "compile".to_owned(),
        "registered" | "registering" => "register".to_owned(),
        "routing" => "route".to_owned(),
        "aliases" => "alias".to_owned(),
        "using" => "use".to_owned(),
        "searched" | "searching" => "search".to_owned(),
        "represented" | "representing" => "represent".to_owned(),
        "created" | "creating" => "create".to_owned(),
        "mapped" | "mapping" => "map".to_owned(),
        "tracked" | "tracking" => "track".to_owned(),
        "enabled" | "enabling" => "enable".to_owned(),
        "sent" | "sending" => "send".to_owned(),
        "opened" | "opening" => "open".to_owned(),
        "loaded" | "loading" => "load".to_owned(),
        "formatted" | "formatting" => "format".to_owned(),
        "parsed" | "parsing" => "parse".to_owned(),
        "dispatched" | "dispatching" => "dispatch".to_owned(),
        "implemented" | "implementing" => "implement".to_owned(),
        "handled" | "handling" => "handle".to_owned(),
        _ => canonical_suffix(token),
    }
}

fn unicode_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            current.push(character);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn split_identifier_words(text: &str) -> String {
    let characters = text.chars().collect::<Vec<_>>();
    let mut words = String::with_capacity(text.len());
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

fn canonical_suffix(token: String) -> String {
    if let Some(stem) = token.strip_suffix("ies")
        && stem.len() >= 2
    {
        return format!("{stem}y");
    }

    if token.ends_with("sses")
        || token.ends_with("xes")
        || token.ends_with("zes")
        || token.ends_with("ches")
        || token.ends_with("shes")
        || token.ends_with("uses")
    {
        return token[..token.len().saturating_sub(2)].to_owned();
    }

    if let Some(stem) = token.strip_suffix('s')
        && !stem.ends_with(['s', 'u', 'i'])
        && !stem.ends_with('a')
        && stem.len() >= 3
    {
        return stem.to_owned();
    }

    if let Some(stem) = token.strip_suffix("ing")
        && stem.len() >= 3
    {
        return add_silent_e_or_remove_double(stem);
    }

    if let Some(stem) = token.strip_suffix("ied")
        && stem.len() >= 2
    {
        return format!("{stem}y");
    }

    if let Some(stem) = token.strip_suffix("ed")
        && stem.len() >= 3
    {
        return add_silent_e_or_remove_double(stem);
    }

    token
}

fn add_silent_e_or_remove_double(stem: &str) -> String {
    let mut characters = stem.chars().collect::<Vec<_>>();
    if characters.len() >= 2 && characters[characters.len() - 1] == characters[characters.len() - 2]
    {
        characters.pop();
    }
    let mut value = characters.into_iter().collect::<String>();
    if value.ends_with("at")
        || value.ends_with("abl")
        || value.ends_with("il")
        || value.ends_with('v')
    {
        value.push('e');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{canonical_code_token, identifier_tokens, strip_diacritics};

    #[test]
    fn identifier_tokens_preserve_code_boundaries_and_unicode() {
        assert_eq!(
            identifier_tokens("HTTPRoute_mapValues crème"),
            ["http", "route_map", "values", "creme"]
        );
        assert_eq!(identifier_tokens("路由/处理"), ["路由", "处理"]);
        assert_eq!(strip_diacritics("Crème brûlée"), "Creme brulee");
    }

    #[test]
    fn code_token_morphology_is_bounded_and_deterministic() {
        let cases = [
            ("routes", "route"),
            ("registered", "register"),
            ("dependencies", "dependency"),
            ("mapped", "map"),
            ("resolution", "resolve"),
            ("using", "use"),
            ("analysis", "analysis"),
            ("路由", "路由"),
        ];
        for (input, expected) in cases {
            assert_eq!(canonical_code_token(input.to_owned()), expected);
        }
    }
}
