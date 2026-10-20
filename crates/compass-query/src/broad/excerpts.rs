//! Exact retained source quotations. Callers must establish ownership first.
use compass_model::provenance::SourceAnchor;
use compass_model::source_query::SourceQuerySite;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedExcerptKind {
    CallableComment,
    CallableCode,
    /// Retained helper code without a question-term match; navigation only.
    LinkedCallablePreview,
    NestedCallableCode,
    AstDocumentation,
}

/// A quotation is inspection evidence, not a behavioral assertion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedSourceExcerpt {
    pub kind: RankedExcerptKind,
    pub source: SourceAnchor,
    pub text: String,
    pub clipped: bool,
    /// Full lexical anonymous-callable scope, never an execution assertion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_scope: Option<SourceAnchor>,
}

/// Internal priority evidence; never serialized or inferred by a renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedExcerpt {
    pub excerpt: RankedSourceExcerpt,
    pub terms: BTreeSet<String>,
}

impl RankedSourceExcerpt {
    /// Check bounded quote/anchor coherence. Ownership is established by the
    /// query producer against its pinned graph, not by this shape check.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        self.text.len() <= 512
            && self.source.file.len() <= 4096
            && exact_shape(&self.source, &self.text)
            && match self.kind {
                RankedExcerptKind::NestedCallableCode => {
                    self.nested_scope.as_ref().is_some_and(|scope| {
                        scope.is_valid()
                            && scope.file.len() <= 4096
                            && scope.file == self.source.file
                            && scope.start_byte <= self.source.start_byte
                            && self.source.end_byte <= scope.end_byte
                            && (scope.start_line, scope.start_column)
                                <= (self.source.start_line, self.source.start_column)
                            && (self.source.end_line, self.source.end_column)
                                <= (scope.end_line, scope.end_column)
                    })
                }
                _ => self.nested_scope.is_none(),
            }
    }
}

fn advance(mut point: (u32, u32), text: &str) -> Option<(u32, u32)> {
    for byte in text.bytes() {
        if byte == b'\n' {
            point.0 = point.0.checked_add(1)?;
            point.1 = 0;
        } else {
            point.1 = point.1.checked_add(1)?;
        }
    }
    Some(point)
}

/// Check anchor/text coherence, not source-byte truth. The caller must establish
/// raw retention and ownership through the versioned producer contract.
pub(crate) fn exact_shape(source: &SourceAnchor, text: &str) -> bool {
    source.is_valid()
        && !text.is_empty()
        && source.end_byte.checked_sub(source.start_byte) == Some(text.len() as u64)
        && advance((source.start_line, source.start_column), text)
            == Some((source.end_line, source.end_column))
}

#[cfg(test)]
fn first_match(text: &str, terms: &BTreeSet<String>) -> Option<usize> {
    let mut start = None;
    for (at, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if character.is_alphanumeric() || character == '_' {
            start.get_or_insert(at);
        } else if let Some(begin) = start.take()
            && compass_model::search::identifier_search_terms(&text[begin..at])
                .into_iter()
                .map(compass_model::canonical_code_token)
                .any(|term| terms.contains(&term))
        {
            return Some(begin);
        }
    }
    None
}

struct SourceToken {
    start: usize,
    end: usize,
    terms: BTreeSet<String>,
}

fn source_tokens(
    source: &SourceAnchor,
    text: &str,
    excluded: &[SourceQuerySite],
) -> Option<Vec<SourceToken>> {
    let mut tokens = Vec::new();
    let mut token_start = None;
    for (at, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if character.is_alphanumeric()
            || character == '_'
            || unicode_normalization::char::is_combining_mark(character)
        {
            token_start.get_or_insert(at);
        } else if let Some(start) = token_start.take() {
            let begin = source.start_byte.checked_add(start as u64)?;
            let finish = source.start_byte.checked_add(at as u64)?;
            if excluded.iter().any(|site| {
                site.source.file == source.file
                    && site.source.start_byte < finish
                    && begin < site.source.end_byte
            }) {
                continue;
            }
            let terms = compass_model::search::identifier_search_terms(&text[start..at])
                .into_iter()
                .map(compass_model::canonical_code_token)
                .collect();
            tokens.push(SourceToken {
                start,
                end: at,
                terms,
            });
        }
    }
    Some(tokens)
}

// Normalize each bounded raw token once. Sliding counters score candidate
// windows, while original spans also supply the final quotation priority.
fn best_window(
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
    tokens: &[SourceToken],
) -> Option<(usize, usize)> {
    let ids = terms
        .iter()
        .enumerate()
        .map(|(id, term)| (term.as_str(), id))
        .collect::<BTreeMap<_, _>>();
    let matches = tokens
        .iter()
        .filter_map(|token| {
            let found = token
                .terms
                .iter()
                .filter_map(|term| ids.get(term.as_str()).copied())
                .collect::<BTreeSet<_>>();
            (!found.is_empty()).then_some((token.start, token.end, found))
        })
        .collect::<Vec<_>>();
    let mut counts = vec![0_usize; terms.len()];
    let (mut left, mut right, mut distinct, mut best_score) = (0, 0, 0, 0);
    let mut best = None;
    let mut starts = BTreeSet::new();
    for (matched, _, _) in &matches {
        // The token-aligned start can contain terms too far apart for a
        // centered window. Context starts keep surrounding syntax on ties.
        starts.insert(*matched);
        let mut start = matched.saturating_sub(max_bytes / 3);
        while !text.is_char_boundary(start) {
            start = start.checked_sub(1)?;
        }
        starts.insert(start);
    }
    for start in starts {
        let mut end = start.saturating_add(max_bytes).min(text.len());
        while !text.is_char_boundary(end) {
            end = end.checked_sub(1)?;
        }
        while right < matches.len() && matches[right].1 <= end {
            for id in &matches[right].2 {
                if counts[*id] == 0 {
                    distinct += 1;
                }
                counts[*id] += 1;
            }
            right += 1;
        }
        while left < right && matches[left].0 < start {
            for id in &matches[left].2 {
                counts[*id] -= 1;
                if counts[*id] == 0 {
                    distinct -= 1;
                }
            }
            left += 1;
        }
        // Only complete matching tokens count. Repeated generic words cannot
        // outweigh additional requested terms, and equal coverage keeps source order.
        if distinct > best_score {
            best_score = distinct;
            best = Some((start, end));
        }
    }
    best
}

/// Choose an exact UTF-8 window covering the most distinct requested terms.
/// Equal coverage retains source order. This selects display evidence only;
/// it does not change candidate admission, relevance or ranking.
/// No match means no quotation. Source coordinates describe retained bytes only.
#[cfg(test)]
fn select(
    kind: RankedExcerptKind,
    source: &SourceAnchor,
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
) -> Option<RankedSourceExcerpt> {
    select_ranked(kind, source, text, terms, max_bytes).map(|selected| selected.excerpt)
}

pub(crate) fn select_ranked(
    kind: RankedExcerptKind,
    source: &SourceAnchor,
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
) -> Option<SelectedExcerpt> {
    select_masked(kind, source, text, terms, max_bytes, &[])
}

/// Code window selection cannot borrow matching terms from known owned comments.
/// The displayed quotation remains original bytes, including surrounding syntax.
pub(crate) fn select_code(
    kind: RankedExcerptKind,
    source: &SourceAnchor,
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
    comments: &[SourceQuerySite],
) -> Option<SelectedExcerpt> {
    select_masked(kind, source, text, terms, max_bytes, comments)
}

/// Fallback display for an already selected, source-witnessed helper only.
/// The caller establishes direct callable ownership against the pinned graph.
/// Prefer the most non-comment tokens, retaining source order on ties. This
/// question-independent display heuristic does not establish an AST body role.
/// Empty priority terms prevent this preview from becoming matching evidence.
pub(crate) fn preview_linked_code(
    windows: &[SourceQuerySite],
    comments: &[SourceQuerySite],
) -> Option<SelectedExcerpt> {
    if windows.len() > 16 || comments.len() > 64 {
        return None;
    }
    let (window, _) = windows
        .iter()
        .filter_map(|window| {
            if window.text.len() > 512
                || window.source.file.len() > 4096
                || !exact_shape(&window.source, &window.text)
            {
                return None;
            }
            let tokens = source_tokens(&window.source, &window.text, comments)?;
            (!tokens.is_empty()).then_some((window, tokens.len()))
        })
        .max_by(|(left, left_tokens), (right, right_tokens)| {
            left_tokens
                .cmp(right_tokens)
                .then_with(|| right.source.cmp(&left.source))
        })?;
    Some(SelectedExcerpt {
        excerpt: RankedSourceExcerpt {
            kind: RankedExcerptKind::LinkedCallablePreview,
            source: window.source.clone(),
            text: window.text.clone(),
            clipped: false,
            nested_scope: None,
        },
        terms: BTreeSet::new(),
    })
}

fn select_masked(
    kind: RankedExcerptKind,
    source: &SourceAnchor,
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
    excluded: &[SourceQuerySite],
) -> Option<SelectedExcerpt> {
    if max_bytes == 0 || text.len() > 4096 || !exact_shape(source, text) {
        return None;
    }
    let max_bytes = max_bytes.min(512);
    let tokens = source_tokens(source, text, excluded)?;
    let (start, end) = best_window(text, terms, max_bytes, &tokens)?;
    let retained = &text[start..end];
    let begin = advance((source.start_line, source.start_column), &text[..start])?;
    let finish = advance(begin, retained)?;
    let excerpt = RankedSourceExcerpt {
        kind,
        source: SourceAnchor {
            file: source.file.clone(),
            start_byte: source.start_byte.checked_add(start as u64)?,
            end_byte: source.start_byte.checked_add(end as u64)?,
            start_line: begin.0,
            start_column: begin.1,
            end_line: finish.0,
            end_column: finish.1,
        },
        text: retained.to_owned(),
        clipped: start > 0 || end < text.len(),
        nested_scope: None,
    };
    // Score only complete original-site tokens. Re-tokenizing the displayed
    // slice could turn a boundary prefix such as getaway -> get into evidence.
    let terms = tokens
        .into_iter()
        .filter(|token| start <= token.start && token.end <= end)
        .flat_map(|token| token.terms)
        .collect();
    Some(SelectedExcerpt { excerpt, terms })
}

/// Quotation selection uses the whole question, including action terms. This
/// affects displayed evidence only, never retrieval admission or node scores.
#[cfg(test)]
fn coverage(text: &str, groups: &[Vec<String>]) -> usize {
    let terms = compass_model::search::identifier_search_terms(text)
        .into_iter()
        .map(compass_model::canonical_code_token)
        .collect::<BTreeSet<_>>();
    coverage_terms(&terms, groups)
}

pub(crate) fn coverage_terms(terms: &BTreeSet<String>, groups: &[Vec<String>]) -> usize {
    groups
        .iter()
        .filter(|group| group.iter().any(|term| terms.contains(term)))
        .count()
}

/// Corpus statistics remain optional: zero/unknown frequency cannot make a
/// retained but unindexed word look maximally distinctive.
pub(crate) struct QueryTermGroup<'a> {
    pub variants: &'a [String],
    pub document_frequency: Option<u64>,
}

/// Smallest known positive corpus frequency among complete original-site
/// matches. This is quotation salience only, never a relevance probability.
pub(crate) fn rarest_document_frequency(
    terms: &BTreeSet<String>,
    groups: &[QueryTermGroup<'_>],
) -> u64 {
    groups
        .iter()
        .filter(|group| group.variants.iter().any(|variant| terms.contains(variant)))
        .filter_map(|group| group.document_frequency.filter(|frequency| *frequency > 0))
        .min()
        .unwrap_or(u64::MAX)
}

/// A retained code window can contain comments. Do not credit their terms as
/// code evidence when choosing between the exact comment and its enclosing
/// window. This scoring copy never replaces the displayed source quotation.
#[cfg(test)]
fn ranking_text(excerpt: &RankedSourceExcerpt, comments: &[SourceQuerySite]) -> String {
    if !matches!(
        excerpt.kind,
        RankedExcerptKind::CallableCode | RankedExcerptKind::NestedCallableCode
    ) {
        return excerpt.text.clone();
    }
    let mut text = String::with_capacity(excerpt.text.len());
    let mut cursor = 0;
    for comment in comments {
        if comment.source.file != excerpt.source.file
            || comment.source.end_byte <= excerpt.source.start_byte
            || comment.source.start_byte >= excerpt.source.end_byte
        {
            continue;
        }
        let start = comment
            .source
            .start_byte
            .saturating_sub(excerpt.source.start_byte)
            .min(excerpt.text.len() as u64) as usize;
        let end = comment
            .source
            .end_byte
            .saturating_sub(excerpt.source.start_byte)
            .min(excerpt.text.len() as u64) as usize;
        if start > cursor {
            let Some(retained) = excerpt.text.get(cursor..start) else {
                return String::new();
            };
            text.push_str(retained);
        }
        // Separate identifiers on either side of a removed comment.
        text.push(' ');
        cursor = cursor.max(end);
    }
    if let Some(retained) = excerpt.text.get(cursor..) {
        text.push_str(retained);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(text: &str) -> Option<SourceAnchor> {
        let finish = advance((3, 4), text)?;
        Some(SourceAnchor {
            file: "src/example.rs".to_owned(),
            start_byte: 20,
            end_byte: 20 + text.len() as u64,
            start_line: 3,
            start_column: 4,
            end_line: finish.0,
            end_column: finish.1,
        })
    }

    fn preview(
        source: &SourceAnchor,
        text: &str,
        comments: &[SourceQuerySite],
    ) -> Option<SelectedExcerpt> {
        preview_linked_code(
            &[SourceQuerySite {
                source: source.clone(),
                text: text.to_owned(),
            }],
            comments,
        )
    }

    #[test]
    fn helper_preview_prefers_inspection_context_over_a_parameter_list()
    -> Result<(), Box<dyn std::error::Error>> {
        let sites = ["(value)", "return value is not None"]
            .into_iter()
            .map(|text| {
                Ok(SourceQuerySite {
                    source: anchor(text).ok_or("source")?,
                    text: text.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
        let selected = preview_linked_code(&sites, &[]).ok_or("preview")?;
        assert_eq!(selected.excerpt.text, sites[1].text);
        assert!(selected.terms.is_empty());
        let mut later = sites[1].clone();
        later.source.start_byte += 100;
        later.source.end_byte += 100;
        later.source.start_line += 1;
        later.source.end_line += 1;
        let tied = preview_linked_code(&[later, sites[1].clone()], &[]).ok_or("tie")?;
        assert_eq!(tied.excerpt.source, sites[1].source);
        assert!(preview_linked_code(&vec![sites[0].clone(); 17], &[]).is_none());
        Ok(())
    }

    #[test]
    fn helper_preview_retains_bytes_without_claiming_matching_terms()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = "return café\n";
        let source = anchor(text).ok_or("source")?;
        let selected = preview(&source, text, &[]).ok_or("preview")?;
        assert!(selected.terms.is_empty());
        assert_eq!(
            selected.excerpt.kind,
            RankedExcerptKind::LinkedCallablePreview
        );
        assert_eq!(selected.excerpt.source, source);
        assert_eq!(selected.excerpt.text, text);
        assert!(!selected.excerpt.clipped);
        assert!(selected.excerpt.is_coherent());
        let mut incoherent = selected.excerpt;
        incoherent.nested_scope = Some(source);
        assert!(!incoherent.is_coherent());
        Ok(())
    }

    #[test]
    fn helper_preview_enforces_bounds_before_scanning_or_copying()
    -> Result<(), Box<dyn std::error::Error>> {
        let exact = format!("x{}", " ".repeat(511));
        let source = anchor(&exact).ok_or("source")?;
        assert!(preview(&source, &exact, &[]).is_some());
        let oversized = format!("{exact} ");
        assert!(preview(&anchor(&oversized).ok_or("source")?, &oversized, &[]).is_none());
        let mut bad = source.clone();
        bad.end_column += 1;
        assert!(preview(&bad, &exact, &[]).is_none());
        bad = source.clone();
        bad.file = "x".repeat(4097);
        assert!(preview(&bad, &exact, &[]).is_none());
        let comments = vec![
            SourceQuerySite {
                source,
                text: exact.clone()
            };
            65
        ];
        assert!(preview(&anchor(&exact).ok_or("source")?, &exact, &comments).is_none());
        Ok(())
    }

    #[test]
    fn helper_preview_requires_code_outside_owned_comments()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = "// only a comment";
        let source = anchor(text).ok_or("source")?;
        let comments = [SourceQuerySite {
            source: source.clone(),
            text: text.to_owned(),
        }];
        assert!(preview(&source, text, &comments).is_none());
        for text in ["", " (); {} ", "   "] {
            assert!(preview(&anchor(text).ok_or("source")?, text, &[]).is_none());
        }
        Ok(())
    }

    #[test]
    fn quoted_window_is_the_original_utf8_slice_with_exact_coordinates()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = format!(
            "// {}\r\n// HandlePayload guards the error.\n// tail",
            "é".repeat(400)
        );
        let source = anchor(&text).ok_or("source")?;
        let terms = BTreeSet::from(["payload".to_owned()]);
        let quote = select(
            RankedExcerptKind::CallableComment,
            &source,
            &text,
            &terms,
            100,
        )
        .ok_or("quote")?;
        assert!(quote.clipped);
        assert!(quote.text.contains("HandlePayload"));
        let start = usize::try_from(quote.source.start_byte - source.start_byte)?;
        let end = usize::try_from(quote.source.end_byte - source.start_byte)?;
        assert_eq!(quote.text, text[start..end]);
        assert_eq!(quote.source.start_line, 3);
        assert_eq!(quote.source.end_line, 5);
        assert!(exact_shape(&quote.source, &quote.text));
        Ok(())
    }

    #[test]
    fn later_coherent_query_evidence_beats_repeated_early_generic_terms()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = format!(
            "// protocol notices: {}\r\n// Reject malformed payload before decoding protocol frames.\n// {}",
            "é protocol ".repeat(70),
            "tail ".repeat(30),
        );
        let source = anchor(&text).ok_or("source")?;
        let terms = [
            "protocol",
            "reject",
            "malformed",
            "payload",
            "decoding",
            "frames",
        ]
        .into_iter()
        .map(str::to_owned)
        .map(compass_model::canonical_code_token)
        .collect();
        let quote = select(
            RankedExcerptKind::CallableComment,
            &source,
            &text,
            &terms,
            160,
        )
        .ok_or("quote")?;
        assert!(
            quote
                .text
                .contains("Reject malformed payload before decoding protocol frames")
        );
        assert!(quote.is_coherent());
        assert!(quote.clipped);
        assert!(quote.text.len() <= 160);
        let start = usize::try_from(quote.source.start_byte - source.start_byte)?;
        let end = usize::try_from(quote.source.end_byte - source.start_byte)?;
        assert_eq!(quote.text, text[start..end]);
        assert_eq!(
            advance((source.start_line, source.start_column), &text[..start]),
            Some((quote.source.start_line, quote.source.start_column))
        );
        Ok(())
    }

    #[test]
    fn separated_terms_fit_when_centering_would_drop_one() -> Result<(), Box<dyn std::error::Error>>
    {
        let text = format!(
            "// {}payload {} protocol.",
            "noise ".repeat(20),
            "x".repeat(90)
        );
        let source = anchor(&text).ok_or("source")?;
        let terms = BTreeSet::from(["payload".to_owned(), "protocol".to_owned()]);
        let quote = select(
            RankedExcerptKind::AstDocumentation,
            &source,
            &text,
            &terms,
            128,
        )
        .ok_or("quote")?;
        assert!(quote.text.contains("payload") && quote.text.contains("protocol"));
        assert!(quote.is_coherent());
        assert!(quote.text.len() <= 128);
        Ok(())
    }

    #[test]
    fn equal_query_term_coverage_keeps_earlier_source_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = format!(
            "// first payload protocol. {} // second payload protocol.",
            "noise ".repeat(70)
        );
        let source = anchor(&text).ok_or("source")?;
        let terms = BTreeSet::from(["payload".to_owned(), "protocol".to_owned()]);
        let quote = select(
            RankedExcerptKind::CallableComment,
            &source,
            &text,
            &terms,
            96,
        )
        .ok_or("quote")?;
        assert!(quote.text.contains("first payload protocol"));
        assert!(!quote.text.contains("second payload protocol"));
        assert!(quote.is_coherent());
        Ok(())
    }

    #[test]
    fn code_window_priority_does_not_borrow_a_comment_topic()
    -> Result<(), Box<dyn std::error::Error>> {
        let comment_text = "// Reject malformed payload decoding protocol.";
        let text = format!(
            "{comment_text}\n{}\nreturn reject(payload, protocol);",
            "filler ".repeat(90)
        );
        let source = anchor(&text).ok_or("source")?;
        let comment_source = anchor(comment_text).ok_or("comment source")?;
        let comments = [SourceQuerySite {
            source: comment_source,
            text: comment_text.to_owned(),
        }];
        let terms = ["reject", "malformed", "payload", "protocol"]
            .into_iter()
            .map(str::to_owned)
            .map(compass_model::canonical_code_token)
            .collect();
        let quote = select_code(
            RankedExcerptKind::CallableCode,
            &source,
            &text,
            &terms,
            128,
            &comments,
        )
        .ok_or("code quote")?;
        assert_eq!(
            coverage_terms(&quote.terms, &[vec!["malform".to_owned()]]),
            0
        );
        assert_eq!(
            coverage_terms(&quote.terms, &[vec!["reject".to_owned()]]),
            1
        );
        let quote = quote.excerpt;
        assert!(quote.text.contains("return reject(payload, protocol)"));
        assert!(!quote.text.contains("malformed"));
        assert!(quote.is_coherent());
        let comment_only = select_code(
            RankedExcerptKind::CallableCode,
            &comments[0].source,
            comment_text,
            &terms,
            128,
            &comments,
        );
        assert!(
            comment_only.is_none(),
            "comment-only text cannot supply code evidence"
        );
        Ok(())
    }

    #[test]
    fn quotation_priority_never_credits_clipped_identifier_prefix_or_suffix()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                format!("// payload {}getaway", " ".repeat(498)),
                512,
                "right",
            ),
            (format!("{}awayget payload.", " ".repeat(100)), 12, "left"),
        ];
        let terms = BTreeSet::from(["payload".to_owned(), "get".to_owned()]);
        let groups = vec![vec!["payload".to_owned()], vec!["get".to_owned()]];
        for (text, budget, side) in cases {
            let source = anchor(&text).ok_or("source")?;
            let selected = select_ranked(
                RankedExcerptKind::CallableComment,
                &source,
                &text,
                &terms,
                budget,
            )
            .ok_or("quote")?;
            assert!(selected.excerpt.is_coherent());
            if side == "right" {
                assert!(selected.excerpt.text.ends_with("get"));
            } else {
                assert!(selected.excerpt.text.starts_with("get"));
            }
            let start = usize::try_from(selected.excerpt.source.start_byte - source.start_byte)?;
            let end = usize::try_from(selected.excerpt.source.end_byte - source.start_byte)?;
            assert_eq!(selected.excerpt.text, text[start..end]);
            assert_eq!(
                coverage_terms(&selected.terms, &groups),
                1,
                "{side}: clipped token is not general evidence"
            );
            assert_eq!(
                coverage_terms(&selected.terms, &groups[1..]),
                0,
                "{side}: clipped token is not action evidence"
            );
        }
        Ok(())
    }

    #[test]
    fn complete_camel_snake_and_decomposed_tokens_keep_their_raw_source_spans()
    -> Result<(), Box<dyn std::error::Error>> {
        for text in ["// getPayload get_payload", "// get pa\u{301}yload"] {
            let source = anchor(text).ok_or("source")?;
            let terms = BTreeSet::from(["payload".to_owned(), "get".to_owned()]);
            let groups = vec![vec!["payload".to_owned()], vec!["get".to_owned()]];
            let selected = select_ranked(
                RankedExcerptKind::CallableComment,
                &source,
                text,
                &terms,
                512,
            )
            .ok_or("quote")?;
            assert_eq!(coverage_terms(&selected.terms, &groups), 2);
            assert_eq!(coverage_terms(&selected.terms, &groups[1..]), 1);
            assert_eq!(selected.excerpt.text, text);
            assert_eq!(selected.excerpt.source, source);
        }
        Ok(())
    }

    #[test]
    fn normalized_or_unmatched_text_is_not_reported_as_a_source_quote()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = "// Handle payload.";
        let source = anchor(text).ok_or("source")?;
        let terms = BTreeSet::from(["payload".to_owned()]);
        assert!(
            select(
                RankedExcerptKind::AstDocumentation,
                &source,
                "Handle payload.",
                &terms,
                100
            )
            .is_none()
        );
        let mut wrong = source.clone();
        wrong.end_column += 1;
        assert!(
            select(
                RankedExcerptKind::AstDocumentation,
                &wrong,
                text,
                &terms,
                100
            )
            .is_none()
        );
        assert!(
            select(
                RankedExcerptKind::CallableComment,
                &source,
                text,
                &BTreeSet::from(["cookie".to_owned()]),
                100
            )
            .is_none()
        );
        assert!(select(RankedExcerptKind::CallableComment, &source, text, &terms, 0).is_none());
        Ok(())
    }

    #[test]
    fn clipping_never_keeps_only_an_unmatched_identifier_prefix()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = format!("// {}Payload.", "x".repeat(800));
        let source = anchor(&text).ok_or("source")?;
        let terms = BTreeSet::from(["payload".to_owned()]);
        // The full identifier is a lexical match, but its displayed prefix is
        // not. Never quote unrelated bytes as matching source evidence.
        assert!(first_match(&text, &terms).is_some());
        assert!(
            select(
                RankedExcerptKind::CallableComment,
                &source,
                &text,
                &terms,
                100
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn quote_relevance_counts_concept_groups_once_and_retains_action_evidence() {
        let subject = ["diagnostic", "diagnostics"]
            .into_iter()
            .map(str::to_owned)
            .map(compass_model::canonical_code_token)
            .collect();
        let action = ["format", "formatted"]
            .into_iter()
            .map(str::to_owned)
            .map(compass_model::canonical_code_token)
            .collect();
        let groups = vec![subject, action];
        let parameter = "diagnostics: &Diagnostic";
        let implementation = "format!(\"{}\", diagnostic.message())";
        assert_eq!(super::coverage(parameter, &groups), 1);
        assert_eq!(super::coverage(implementation, &groups), 2);
        assert_eq!(super::coverage(parameter, &groups[1..]), 0);
        assert_eq!(super::coverage(implementation, &groups[1..]), 1);
    }

    #[test]
    fn enclosing_code_does_not_borrow_comment_terms_for_quote_priority()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = "value = value\n    # Handle payload normalization here.\n    return value";
        let source = anchor(text).ok_or("source")?;
        let comment_text = "# Handle payload normalization here.";
        let terms = BTreeSet::from(["handle".to_owned(), "payload".to_owned()]);
        let code =
            select(RankedExcerptKind::CallableCode, &source, text, &terms, 512).ok_or("code")?;
        let begin = text.find('#').ok_or("comment start")?;
        let point = advance((source.start_line, source.start_column), &text[..begin])
            .ok_or("comment point")?;
        let comment_source = SourceAnchor {
            file: source.file.clone(),
            start_byte: source.start_byte + begin as u64,
            end_byte: source.start_byte + (begin + comment_text.len()) as u64,
            start_line: point.0,
            start_column: point.1,
            end_line: point.0,
            end_column: point.1 + comment_text.len() as u32,
        };
        let comment = SourceQuerySite {
            source: comment_source,
            text: comment_text.to_owned(),
        };
        let groups = vec![vec!["handle".to_owned()], vec!["payload".to_owned()]];
        assert_eq!(coverage(&code.text, &groups), 2);
        assert_eq!(
            coverage(
                &ranking_text(&code, std::slice::from_ref(&comment)),
                &groups
            ),
            0
        );
        assert_eq!(code.text, text);
        let quote = select(
            RankedExcerptKind::CallableComment,
            &comment.source,
            &comment.text,
            &terms,
            512,
        )
        .ok_or("comment quote")?;
        assert_eq!(coverage(&ranking_text(&quote, &[comment]), &groups), 2);
        Ok(())
    }

    #[test]
    fn comment_mask_preserves_code_terms_and_separates_adjacent_identifiers()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = "handle/* payload */payload";
        let source = anchor(text).ok_or("source")?;
        let mut comment_source = source.clone();
        comment_source.start_byte += 6;
        comment_source.start_column += 6;
        comment_source.end_byte -= 7;
        comment_source.end_column -= 7;
        let comments = [SourceQuerySite {
            source: comment_source,
            text: "/* payload */".to_owned(),
        }];
        let quote = RankedSourceExcerpt {
            kind: RankedExcerptKind::CallableCode,
            source,
            text: text.to_owned(),
            clipped: false,
            nested_scope: None,
        };
        let groups = vec![vec!["handle".to_owned()], vec!["payload".to_owned()]];
        assert_eq!(ranking_text(&quote, &comments), "handle payload");
        assert_eq!(coverage(&ranking_text(&quote, &comments), &groups), 2);
        Ok(())
    }
}

#[cfg(test)]
mod salience_tests {
    use super::*;

    #[test]
    fn rarity_uses_complete_matching_groups_and_only_positive_known_frequencies() {
        let uncommon = vec!["uncommon".to_owned()];
        let common = vec!["search".to_owned(), "term".to_owned()];
        let unknown = vec!["absent".to_owned()];
        let zero = vec!["unindexed".to_owned()];
        let groups = [
            QueryTermGroup {
                variants: &uncommon,
                document_frequency: Some(1),
            },
            QueryTermGroup {
                variants: &common,
                document_frequency: Some(200),
            },
            QueryTermGroup {
                variants: &unknown,
                document_frequency: None,
            },
            QueryTermGroup {
                variants: &zero,
                document_frequency: Some(0),
            },
        ];
        let set = |values: &[&str]| {
            values
                .iter()
                .map(|v| (*v).to_owned())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            rarest_document_frequency(&set(&["search", "term"]), &groups),
            200
        );
        assert_eq!(rarest_document_frequency(&set(&["uncommon"]), &groups), 1);
        assert_eq!(
            rarest_document_frequency(&set(&["absent", "unindexed"]), &groups),
            u64::MAX
        );
        assert_eq!(
            rarest_document_frequency(&set(&["uncommonly"]), &groups),
            u64::MAX
        );
        assert_eq!(
            rarest_document_frequency(&set(&["search", "absent", "unindexed"]), &groups),
            200
        );
        let legacy = [QueryTermGroup {
            variants: &uncommon,
            document_frequency: None,
        }];
        assert_eq!(
            rarest_document_frequency(&set(&["uncommon"]), &legacy),
            u64::MAX
        );
    }
}
