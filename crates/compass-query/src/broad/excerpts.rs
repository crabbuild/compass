//! Exact retained source quotations. Callers must establish ownership first.
use compass_model::provenance::SourceAnchor;
use compass_model::source_query::SourceQuerySite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedExcerptKind {
    CallableComment,
    CallableCode,
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

/// Choose an exact, UTF-8-safe window around the first matching identifier term.
/// No match means no quotation. Source coordinates describe retained bytes only.
pub(crate) fn select(
    kind: RankedExcerptKind,
    source: &SourceAnchor,
    text: &str,
    terms: &BTreeSet<String>,
    max_bytes: usize,
) -> Option<RankedSourceExcerpt> {
    if !exact_shape(source, text) || max_bytes == 0 || text.len() > 4096 {
        return None;
    }
    let max_bytes = max_bytes.min(512);
    let matched = first_match(text, terms)?;
    let mut start = matched.saturating_sub(max_bytes / 3);
    while !text.is_char_boundary(start) {
        start = start.checked_sub(1)?;
    }
    let mut end = start.saturating_add(max_bytes).min(text.len());
    while !text.is_char_boundary(end) {
        end = end.checked_sub(1)?;
    }
    if start >= end || matched >= end {
        return None;
    }
    let retained = &text[start..end];
    // A matched identifier subword may lie outside this clipped window.
    // Retained bytes must themselves support the quotation's relevance.
    first_match(retained, terms)?;
    let begin = advance((source.start_line, source.start_column), &text[..start])?;
    let finish = advance(begin, retained)?;
    Some(RankedSourceExcerpt {
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
    })
}

/// Quotation selection uses the whole question, including action terms. This
/// affects displayed evidence only, never retrieval admission or node scores.
pub(crate) fn coverage(text: &str, groups: &[Vec<String>]) -> usize {
    let terms = compass_model::search::identifier_search_terms(text)
        .into_iter()
        .map(compass_model::canonical_code_token)
        .collect::<BTreeSet<_>>();
    groups
        .iter()
        .filter(|group| group.iter().any(|term| terms.contains(term)))
        .count()
}

/// A retained code window can contain comments. Do not credit their terms as
/// code evidence when choosing between the exact comment and its enclosing
/// window. This scoring copy never replaces the displayed source quotation.
pub(crate) fn ranking_text(excerpt: &RankedSourceExcerpt, comments: &[SourceQuerySite]) -> String {
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
