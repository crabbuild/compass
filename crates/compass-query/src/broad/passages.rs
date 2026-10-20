//! Query-local comment lengths for already indexed matches, not new evidence.
use super::*;
use compass_model::source_query::SourceQuerySite;

/// Caller establishes exact AST ownership. Independently verify the retained
/// comment field fits its existing byte/term bounds; unrelated code-window or
/// identifier clipping cannot turn complete comment passages into long prose.
/// Reuse group variants without renormalizing canonical concept keys.
pub(super) fn comment_lengths(
    comments: &[SourceQuerySite],
    groups: &[(String, Vec<String>, f64, u64)],
    check: &dyn Fn() -> Result<(), QueryError>,
) -> Result<BTreeMap<String, usize>, QueryError> {
    use compass_model::search::{SEARCH_FIELD_MAX_BYTES, SEARCH_FIELD_MAX_TERMS};
    if comments.len() > 64 || groups.len() > 32 {
        return Ok(BTreeMap::new());
    }
    let mut lengths = BTreeMap::<String, usize>::new();
    let mut total_bytes = 0_usize;
    let mut field_terms = BTreeSet::new();
    for site in comments {
        check()?;
        total_bytes = total_bytes.saturating_add(site.text.len());
        // A site at its retention ceiling may end in a partial raw token.
        if site.text.len() >= 4096 || total_bytes > SEARCH_FIELD_MAX_BYTES {
            return Ok(BTreeMap::new());
        }
        let terms = identifier_search_terms(&site.text)
            .into_iter()
            .map(compass_model::canonical_code_token)
            .collect::<BTreeSet<_>>();
        field_terms.extend(terms.iter().cloned());
        if field_terms.len() > SEARCH_FIELD_MAX_TERMS {
            return Ok(BTreeMap::new());
        }
        for (term, variants, _, _) in groups {
            if variants.iter().any(|variant| terms.contains(variant)) {
                lengths
                    .entry(term.clone())
                    .and_modify(|length| *length = (*length).min(terms.len()))
                    .or_insert(terms.len());
            }
        }
    }
    Ok(lengths)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn site(text: String) -> SourceQuerySite {
        SourceQuerySite {
            source: SourceAnchor {
                file: "sample.rs".to_owned(),
                start_byte: 0,
                end_byte: text.len() as u64,
                start_line: 1,
                end_line: 1,
                start_column: 0,
                end_column: text.len() as u32,
            },
            text,
        }
    }
    #[test]
    fn incomplete_comment_fields_retain_the_original_normalizer()
    -> Result<(), Box<dyn std::error::Error>> {
        let groups = vec![("rare".to_owned(), vec!["rare".to_owned()], 1.0, 1)];
        assert_eq!(
            comment_lengths(&[site("rare topic".to_owned())], &groups, &|| Ok(()))?.get("rare"),
            Some(&2)
        );
        for comments in [
            vec![site("rare ".repeat(900))],
            (0..65).map(|_| site("rare".to_owned())).collect(),
            (0..3).map(|_| site("rare ".repeat(600))).collect(),
            vec![site(format!(
                "rare {}",
                (0..600).map(|n| format!("x{n} ")).collect::<String>()
            ))],
        ] {
            assert!(comment_lengths(&comments, &groups, &|| Ok(()))?.is_empty());
        }
        let error = comment_lengths(&[site("rare".to_owned())], &groups, &|| {
            Err(QueryError::new(
                QueryErrorKind::Cancelled,
                "test_cancelled",
                "cancelled",
            ))
        });
        assert!(matches!(error,Err(e) if e.kind()==QueryErrorKind::Cancelled));
        Ok(())
    }
}
