//! Ranking-only lexical cohesion; no recall postings or behavioral inference.
use std::collections::BTreeSet;

use compass_model::provenance::SourceAnchor;
use compass_model::source_query::SourceQuerySite;

#[derive(Default)]
pub(super) struct Compounds {
    phrases: BTreeSet<Vec<String>>,
    components: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct Matches {
    pub phrases: BTreeSet<String>,
    pub sources: BTreeSet<SourceAnchor>,
}

fn words(value: &str) -> Vec<String> {
    compass_model::identifier_tokens(&value.replace(['_', '-'], " "))
        .into_iter()
        .map(compass_model::canonical_code_token)
        .collect()
}

fn together(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|part| part == needle)
}

pub(super) fn inspection_signature(signature: Option<&str>) -> Option<&str> {
    signature.filter(|text| !text.trim().is_empty() && !text.trim_end().ends_with('…'))
}

impl Compounds {
    pub fn parse(question: &str) -> Self {
        let mut result = Self::default();
        for raw in question.split_whitespace() {
            let raw = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
            if !raw.contains('-')
                || raw
                    .split('-')
                    .any(|part| part.is_empty() || !part.chars().all(char::is_alphanumeric))
            {
                continue;
            }
            let phrase = words(raw);
            if !(2..=4).contains(&phrase.len()) {
                continue;
            }
            result.components.extend(phrase.iter().cloned());
            result.phrases.insert(phrase);
            // Keep ordinary ranking for oversized compound requests rather
            // than selecting a wording/order-dependent subset. Recall is unchanged.
            if result.phrases.len() > 8 {
                return Self::default();
            }
        }
        result
    }

    pub fn contains_component_group(&self, term: &str) -> bool {
        let terms = words(term);
        !terms.is_empty() && terms.iter().all(|term| self.components.contains(term))
    }

    /// The caller validates owner identity/provenance and excludes clipped
    /// projections. Parameter/type identifiers in retained signatures cannot
    /// supply a code signal. A declaration's own name remains a direct signal.
    pub fn matches(
        &self,
        name: &str,
        signature: Option<&str>,
        identifiers: &[SourceQuerySite],
    ) -> Matches {
        let mut result = Matches::default();
        if self.phrases.is_empty() {
            return result;
        }
        let name = words(name);
        let signature = inspection_signature(signature);
        let header = signature
            .into_iter()
            .flat_map(|text| text.split(|c: char| !c.is_alphanumeric() && c != '_'))
            .filter(|word| !word.is_empty())
            .map(words)
            .collect::<Vec<_>>();
        let sites = identifiers
            .iter()
            .filter(|site| {
                signature.is_some()
                    && !site.text.is_empty()
                    && site.text.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
            .map(|site| (words(&site.text), &site.source))
            .collect::<Vec<_>>();
        for phrase in &self.phrases {
            let label = phrase.join("-");
            if together(&name, phrase) {
                result.phrases.insert(label);
            } else if !header.iter().any(|word| together(word, phrase))
                && let Some((_, source)) = sites.iter().find(|(tokens, _)| together(tokens, phrase))
            {
                result.phrases.insert(label);
                result.sources.insert((*source).clone());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(text: &str) -> SourceQuerySite {
        SourceQuerySite {
            source: SourceAnchor {
                file: "sample.py".into(),
                start_byte: 10,
                end_byte: 10 + text.len() as u64,
                start_line: 2,
                end_line: 2,
                start_column: 0,
                end_column: text.len() as u32,
            },
            text: text.into(),
        }
    }

    #[test]
    fn compound_words_require_contiguous_identifier_terms() {
        let request = Compounds::parse("Where are read-only fields prepared?");
        for name in ["read_only", "readOnly", "isReadOnlyField"] {
            let item = site(name);
            let found = request.matches(
                "opaque",
                Some("fn(fields: Fields)"),
                std::slice::from_ref(&item),
            );
            assert_eq!(found.phrases, BTreeSet::from(["read-only".into()]));
            assert_eq!(found.sources, BTreeSet::from([item.source]));
        }
        for items in [
            vec![site("read"), site("only")],
            vec![site("only_read")],
            vec![site("already_only")],
            vec![site("read only")],
            vec![site("cfg.readOnly")],
        ] {
            assert!(
                request
                    .matches("opaque", Some("fn(fields: Fields)"), &items)
                    .phrases
                    .is_empty()
            );
        }
        assert!(request.contains_component_group("read"));
        assert!(request.contains_component_group("read only"));
        assert!(!request.contains_component_group("field"));
    }

    #[test]
    fn retained_header_overlap_and_missing_signatures_do_not_supply_code_preference() {
        let request = Compounds::parse("Where are read-only fields prepared?");
        let items = [site("read_only")];
        for signature in [
            None,
            Some(""),
            Some("fn(fields: Fields, …"),
            Some("fn(fields: Fields, read_only: Mode)"),
            Some("fn(fields: ReadOnlyFields)"),
        ] {
            assert!(
                request
                    .matches("opaque", signature, &items)
                    .phrases
                    .is_empty()
            );
        }
        assert!(
            !request
                .matches(
                    "opaque",
                    Some("fn(fields: Fields, read: X, only: Y)"),
                    &items
                )
                .phrases
                .is_empty()
        );
        assert!(
            !request
                .matches("prepare_read_only_fields", None, &[])
                .phrases
                .is_empty()
        );
    }

    #[test]
    fn compound_limits_retain_ordinary_ranking_without_an_arbitrary_subset() {
        assert!(Compounds::parse("plain words").phrases.is_empty());
        assert!(Compounds::parse("a-b-c-d-e").phrases.is_empty());
        let question = (0..9)
            .map(|i| format!("mode{i}-only"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(Compounds::parse(&question).phrases.is_empty());
    }
}
