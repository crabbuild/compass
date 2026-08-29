use std::collections::HashSet;

pub use compass_model::strip_diacritics;
use compass_model::{canonical_code_token, identifier_tokens};

const QUERY_STOPWORDS: &[&str] = &[
    "how",
    "what",
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
    "some",
    "work",
    "works",
    "working",
    "implement",
    "implemented",
    "implementation",
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
pub fn search_tokens(text: &str) -> Vec<String> {
    identifier_tokens(text)
        .into_iter()
        .map(canonical_search_token)
        .collect()
}

#[must_use]
pub fn query_terms(question: &str) -> Vec<String> {
    query_recall_terms(question)
        .into_iter()
        .map(canonical_query_token)
        .collect()
}

pub(crate) fn query_recall_terms(question: &str) -> Vec<String> {
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
            for token in search_tokens(raw) {
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
            "calls" | "called" | "invoke" | "invokes" | "invoked" | "invocation" => "call",
            "fields" | "property" | "properties" | "member" | "members" => "field",
            "imports" | "imported" | "module" | "modules" => "import",
            "exports" | "exported" => "export",
            "routes" | "routed" | "routing" => "route",
            "register" | "registered" | "registers" => "registration",
            "reads" | "reading" => "read",
            "writes" | "writing" => "write",
            "tests" | "tested" | "testing" => "test",
            "types" | "typing" => "type",
            "dependencies" | "depends" => "dependency",
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

fn canonical_search_token(token: String) -> String {
    match token.as_str() {
        "resolution" | "resolved" | "resolver" | "resolving" => "resolve".to_owned(),
        _ => token,
    }
}

pub(crate) fn canonical_query_token(token: String) -> String {
    if QUERY_STOPWORDS.contains(&token.as_str()) || !token.is_ascii() {
        return token;
    }

    canonical_code_token(token)
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
