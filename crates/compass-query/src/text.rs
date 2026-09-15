use std::collections::HashSet;

pub use compass_model::strip_diacritics;
use compass_model::{canonical_code_token, identifier_tokens};

const GENERIC_RELATIONAL_TERMS: &[&str] = &[
    "call",
    "called",
    "caller",
    "callers",
    "calls",
    "connect",
    "connected",
    "connection",
    "connections",
    "depend",
    "depended",
    "dependency",
    "dependent",
    "dependents",
    "depends",
    "path",
    "reach",
    "reaches",
    "relate",
    "related",
    "relation",
    "relations",
    "relationship",
    "relationships",
    "route",
    "use",
    "used",
    "uses",
    "using",
];

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveryTermSelection {
    pub recall_terms: Vec<String>,
    pub ranking_terms: Vec<String>,
    pub discarded_generic_terms: Vec<String>,
}

/// Select concrete discovery anchors while keeping relational vocabulary
/// available to the intent and direction classifiers. High-confidence natural
/// query shapes seed from their parsed operands; broad questions use their
/// remaining concrete terms. Generic relationship verbs never become graph
/// anchors by themselves.
pub(crate) fn discovery_term_selection(question: &str) -> DiscoveryTermSelection {
    let operands = discovery_operands(question);
    let seed_source = if operands.is_empty() {
        question.to_owned()
    } else {
        operands.join(" ")
    };
    let explicit_operand_terms = operands
        .iter()
        .flat_map(|operand| search_tokens(operand))
        .map(canonical_query_token)
        .collect::<HashSet<_>>();
    let mut recall_terms = Vec::new();
    let mut ranking_terms = Vec::new();
    let mut seen_recall = HashSet::new();
    let mut seen_ranking = HashSet::new();
    for term in query_recall_terms(&seed_source) {
        let canonical = canonical_query_token(term.clone());
        if (is_generic_relational_term(&term) || is_generic_relational_term(&canonical))
            && !explicit_operand_terms.contains(&canonical)
        {
            continue;
        }
        if seen_recall.insert(term.clone()) {
            recall_terms.push(term);
        }
        if seen_ranking.insert(canonical.clone()) {
            ranking_terms.push(canonical);
        }
    }
    ranking_terms.sort();

    let mut discarded_generic_terms = search_tokens(question)
        .into_iter()
        .map(canonical_query_token)
        .filter(|term| is_generic_relational_term(term) && !explicit_operand_terms.contains(term))
        .collect::<Vec<_>>();
    discarded_generic_terms.sort();
    discarded_generic_terms.dedup();
    DiscoveryTermSelection {
        recall_terms,
        ranking_terms,
        discarded_generic_terms,
    }
}

/// Return the concrete symbol operands named by a supported natural query
/// shape. Neutral comparisons need a small explicit grammar because their
/// relationship word classifies the request but neither subject may be lost.
pub(crate) fn discovery_operands(question: &str) -> Vec<String> {
    let planned = crate::intent::plan_natural_query(question)
        .ok()
        .filter(|plan| plan.routes_to_typed_query())
        .map(|plan| plan.operands().to_vec())
        .unwrap_or_default();
    if !planned.is_empty() {
        return planned;
    }

    let trimmed = question.trim().trim_end_matches('?').trim();
    let lowered = trimmed.to_ascii_lowercase();
    let Some(body) = lowered.strip_prefix("how are ") else {
        return Vec::new();
    };
    let body_offset = trimmed.len().saturating_sub(body.len());
    for suffix in [" related", " connected", " dependent"] {
        let Some(subjects) = body.strip_suffix(suffix) else {
            continue;
        };
        let Some(separator) = subjects.find(" and ") else {
            continue;
        };
        let left = trimmed[body_offset..body_offset + separator].trim();
        let right_start = body_offset + separator + " and ".len();
        let right_end = body_offset + subjects.len();
        let right = trimmed[right_start..right_end].trim();
        if !left.is_empty() && !right.is_empty() {
            return vec![left.to_owned(), right.to_owned()];
        }
    }
    Vec::new()
}

fn is_generic_relational_term(term: &str) -> bool {
    GENERIC_RELATIONAL_TERMS.contains(&term)
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

#[cfg(test)]
mod tests {
    use super::discovery_term_selection;

    #[test]
    fn discovery_terms_discard_generic_relationship_words() {
        let selected =
            discovery_term_selection("how are PaymentGateway and CheckoutHandler related?");
        assert!(selected.ranking_terms.contains(&"payment".to_owned()));
        assert!(selected.ranking_terms.contains(&"gateway".to_owned()));
        assert!(selected.ranking_terms.contains(&"checkout".to_owned()));
        assert!(selected.ranking_terms.contains(&"handler".to_owned()));
        assert!(!selected.ranking_terms.contains(&"relate".to_owned()));
        assert!(
            selected
                .discarded_generic_terms
                .contains(&"relate".to_owned())
        );
    }

    #[test]
    fn parsed_operands_keep_symbols_that_happen_to_use_generic_words() {
        let selected = discovery_term_selection("path from Caller to Target");
        assert!(selected.ranking_terms.contains(&"caller".to_owned()));
        assert!(selected.ranking_terms.contains(&"target".to_owned()));
        assert!(!selected.ranking_terms.contains(&"path".to_owned()));
        assert!(
            selected
                .discarded_generic_terms
                .contains(&"path".to_owned())
        );
    }

    #[test]
    fn comparison_questions_keep_both_explicit_subjects() {
        let selected = discovery_term_selection("how are Target and Caller connected?");
        assert!(selected.ranking_terms.contains(&"target".to_owned()));
        assert!(selected.ranking_terms.contains(&"caller".to_owned()));
        assert_eq!(selected.discarded_generic_terms, ["connect"]);
    }

    #[test]
    fn generic_relationship_words_cannot_seed_discovery_alone() {
        let selected = discovery_term_selection("how are these connected and related?");
        assert!(selected.ranking_terms.is_empty());
        assert_eq!(selected.discarded_generic_terms, ["connect", "relate"]);
    }
}
