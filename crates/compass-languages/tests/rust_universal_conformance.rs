use std::error::Error;

use compass_languages::{Engine, OccurrenceRole};

#[test]
fn qualified_calls_keep_owner_identity_repetition_and_unicode_ranges() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("qualified.rs");
    let source_text = r#"
// The lambda keeps byte and character offsets different: λ
struct Alpha {}
struct Beta {}
impl Alpha { fn new() -> Self { Self {} } }
impl Beta { fn new() -> Self { Self {} } }
fn build() {
    Alpha::new();
    Beta::new();
    Alpha::new();
    External::new();
}
"#;
    let source = source_text.as_bytes();
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .universal_evidence
        .first()
        .ok_or("missing universal evidence")?;
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == OccurrenceRole::Call)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4, "occurrences={calls:#?}");
    assert_eq!(
        calls
            .iter()
            .filter(|occurrence| occurrence.qualifier.as_deref() == Some("Alpha"))
            .count(),
        2
    );
    for occurrence in &calls {
        let start = usize::try_from(occurrence.anchor.start_byte)?;
        let end = usize::try_from(occurrence.anchor.end_byte)?;
        let qualifier = occurrence.qualifier.as_deref().ok_or("missing qualifier")?;
        assert_eq!(
            std::str::from_utf8(&source[start..end])?,
            format!("{qualifier}::new()")
        );
    }

    let alpha = extraction
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name")
                .starts_with("impl Alpha::new(")
        })
        .ok_or("missing Alpha::new")?;
    let beta = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name").starts_with("impl Beta::new("))
        .ok_or("missing Beta::new")?;
    let alpha_calls = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == alpha.id)
        .count();
    let beta_calls = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == beta.id)
        .count();
    assert_eq!(
        alpha_calls, 2,
        "alpha={alpha:#?} nodes={:#?} edges={:#?}",
        extraction.nodes, extraction.edges
    );
    assert_eq!(
        beta_calls, 1,
        "beta={beta:#?} nodes={:#?} edges={:#?}",
        extraction.nodes, extraction.edges
    );
    let external = evidence
        .relationship_candidates
        .iter()
        .find(|candidate| candidate.qualifier.as_deref() == Some("External"))
        .ok_or("missing external candidate")?;
    assert!(external.external_identity);
    Ok(())
}

#[test]
fn unresolved_namespaced_calls_never_fall_back_to_terminal_method_names()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("namespaced.rs");
    let source = br#"
struct Item {}
impl Item { fn new() -> Self { Self {} } }
fn build() { other::Item::new(); }
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let local_new = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name").starts_with("impl Item::new("))
        .ok_or("missing Item::new")?;
    assert!(
        extraction
            .edges
            .iter()
            .all(|edge| { edge.string("relation") != "calls" || edge.target != local_new.id })
    );
    let candidate = extraction
        .universal_evidence
        .first()
        .ok_or("missing universal evidence")?
        .relationship_candidates
        .iter()
        .find(|candidate| candidate.spelling == "new")
        .ok_or("missing namespaced candidate")?;
    assert_eq!(candidate.qualifier.as_deref(), Some("other::Item"));
    assert!(candidate.external_identity);
    Ok(())
}
