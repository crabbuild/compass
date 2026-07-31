use std::error::Error;

use compass_languages::{
    CandidateRelation, Engine, LanguageCapability, SemanticRole, UniversalAdapterProfile,
};

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
    external_crate::NoCollision::launch();
}
"#;
    let source = source_text.as_bytes();
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;

    assert_eq!(evidence.adapter.id, "compass.rust");
    assert_eq!(evidence.adapter.version, 1);
    assert_eq!(
        evidence.adapter.evidence_schema,
        "compass.languages.evidence/1"
    );
    assert_eq!(
        evidence.adapter.profile,
        UniversalAdapterProfile::UniversalCandidate
    );
    for capability in [
        LanguageCapability::Namespaces,
        LanguageCapability::Traits,
        LanguageCapability::ImplOwnership,
        LanguageCapability::Macros,
        LanguageCapability::Imports,
        LanguageCapability::Calls,
        LanguageCapability::ExternalReferences,
    ] {
        assert!(evidence.adapter.capabilities.contains(&capability));
    }
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());

    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 5, "occurrences={calls:#?}");
    assert_eq!(
        calls
            .iter()
            .filter(|occurrence| occurrence.qualifier.as_deref() == Some("Alpha"))
            .count(),
        2
    );
    for occurrence in &calls {
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        let qualifier = occurrence.qualifier.as_deref().ok_or("missing qualifier")?;
        assert_eq!(
            std::str::from_utf8(&source[start..end])?,
            format!("{qualifier}::{}", occurrence.spelling)
        );
    }

    let alpha_calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.constraints.qualified_name.as_deref()
                    == Some("crate::qualified::Alpha::new")
        })
        .count();
    assert_eq!(alpha_calls, 2);
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "launch"
            && candidate.constraints.qualified_name.as_deref()
                == Some("external_crate::NoCollision::launch")
            && candidate.constraints.allow_external
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.target_spelling == "new"
            && candidate.constraints.qualified_name.as_deref()
                == Some("crate::qualified::Beta::new")
            && !candidate.constraints.allow_external
    }));
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
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let candidate = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "new"
        })
        .ok_or("missing namespaced candidate")?;
    assert_eq!(
        candidate.constraints.qualified_name.as_deref(),
        Some("other::Item::new")
    );
    assert!(candidate.constraints.allow_external);
    assert_ne!(
        candidate.constraints.qualified_name.as_deref(),
        Some("crate::namespaced::Item::new")
    );
    Ok(())
}
