use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

const QUERY_STOPWORDS: &[&str] = &[
    "how",
    "what",
    "why",
    "when",
    "where",
    "which",
    "who",
    "whom",
    "whose",
    "does",
    "did",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "can",
    "could",
    "should",
    "would",
    "will",
    "shall",
    "may",
    "might",
    "must",
    "has",
    "have",
    "had",
    "the",
    "and",
    "but",
    "not",
    "for",
    "from",
    "with",
    "without",
    "into",
    "onto",
    "off",
    "that",
    "this",
    "these",
    "those",
    "there",
    "here",
    "its",
    "their",
    "them",
    "they",
    "about",
    "any",
    "all",
    "some",
    "work",
    "works",
    "working",
    "der",
    "die",
    "das",
    "den",
    "dem",
    "ein",
    "eine",
    "und",
    "oder",
    "nicht",
    "wie",
    "wer",
    "wann",
    "wo",
    "warum",
    "wieso",
    "welche",
    "welcher",
    "welches",
    "ist",
    "sind",
    "wird",
    "wurde",
    "hat",
    "haben",
    "kann",
    "koennen",
    "können",
    "soll",
    "muss",
    "sich",
    "bei",
    "mit",
    "von",
    "fuer",
    "für",
    "ueber",
    "über",
    "nach",
    "aus",
    "gibt",
    "es",
    "funktioniert",
    "geaendert",
    "geändert",
    "aendert",
    "ändert",
    "pourquoi",
    "quand",
    "quel",
    "quelle",
    "quels",
    "quelles",
    "quoi",
    "qui",
    "que",
    "est",
    "sont",
    "fonctionne",
    "cette",
    "dans",
    "avec",
    "où",
    "cómo",
    "como",
    "qué",
    "cuál",
    "cuáles",
    "cuándo",
    "dónde",
    "donde",
    "porque",
    "por",
    "para",
    "funciona",
    "está",
    "están",
    "hay",
    "qual",
    "quais",
    "quando",
    "onde",
    "são",
    "estão",
    "tem",
    "uma",
    "não",
    "perché",
    "cosa",
    "quale",
    "quali",
    "dove",
    "funziona",
    "sono",
    "che",
    "della",
];

#[must_use]
pub fn strip_diacritics(text: &str) -> String {
    text.nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect()
}

#[must_use]
pub fn search_tokens(text: &str) -> Vec<String> {
    unicode_words(&strip_diacritics(text).to_lowercase())
}

#[must_use]
pub fn query_terms(question: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in question.split_whitespace() {
        if raw.chars().any(is_chinese) {
            let lowered = raw.to_lowercase();
            let characters = lowered.chars().collect::<Vec<_>>();
            if characters.len() < 2 {
                if is_searchable(&lowered) {
                    terms.push(lowered);
                }
            } else {
                for window in characters.windows(2) {
                    let segment = window.iter().collect::<String>();
                    if is_searchable(&segment) {
                        terms.push(segment);
                    }
                }
                if is_searchable(&lowered) && !terms.iter().any(|term| term == &lowered) {
                    terms.push(lowered);
                }
            }
        } else {
            for token in unicode_words(&raw.to_lowercase()) {
                if is_searchable(&token) {
                    terms.push(token);
                }
            }
        }
    }
    let content = terms
        .iter()
        .filter(|term| !QUERY_STOPWORDS.contains(&term.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if content.is_empty() { terms } else { content }
}

#[must_use]
pub fn sanitize_label(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

#[must_use]
pub fn normalize_context_filters(filters: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for value in filters {
        let key = strip_diacritics(value).trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let canonical = match key.as_str() {
            "param" | "params" | "parameter" | "parameters" | "argument" | "arguments" | "arg"
            | "args" => "parameter_type",
            "return" | "returns" | "returned" => "return_type",
            "generic" | "generics" | "template" | "templates" => "generic_arg",
            "annotation" | "annotations" | "decorator" | "decorators" => "attribute",
            "calls" | "called" | "invoke" | "invocation" => "call",
            "fields" | "property" | "properties" | "member" | "members" => "field",
            "imports" | "imported" | "module" | "modules" => "import",
            "exports" | "exported" => "export",
            _ => &key,
        }
        .to_owned();
        if seen.insert(canonical.clone()) {
            normalized.push(canonical);
        }
    }
    normalized
}

#[must_use]
pub fn infer_context_filters(question: &str) -> Vec<String> {
    const HINTS: &[(&str, &[&str])] = &[
        (
            "call",
            &["call", "calls", "called", "invoke", "invokes", "invoked"],
        ),
        (
            "import",
            &["import", "imports", "imported", "module", "modules"],
        ),
        (
            "field",
            &[
                "field",
                "fields",
                "member",
                "members",
                "property",
                "properties",
            ],
        ),
        (
            "parameter_type",
            &[
                "parameter",
                "parameters",
                "param",
                "params",
                "argument",
                "arguments",
            ],
        ),
        ("return_type", &["return", "returns", "returned"]),
        (
            "generic_arg",
            &["generic", "generics", "template", "templates"],
        ),
    ];
    let lowered = question
        .replace(['?', ','], " ")
        .split_whitespace()
        .map(|token| strip_diacritics(token).to_lowercase())
        .collect::<HashSet<_>>();
    HINTS
        .iter()
        .filter(|(_, hints)| hints.iter().any(|hint| lowered.contains(*hint)))
        .map(|(context, _)| (*context).to_owned())
        .collect()
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

fn is_chinese(character: char) -> bool {
    ('一'..='鿿').contains(&character)
}

fn is_searchable(term: &str) -> bool {
    if term.chars().all(|character| character.is_ascii_lowercase()) {
        term.chars().count() > 2
    } else {
        true
    }
}
