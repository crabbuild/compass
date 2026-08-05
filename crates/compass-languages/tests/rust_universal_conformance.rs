use std::error::Error;

use compass_languages::{
    CandidateRelation, Engine, HierarchyConstraint, LanguageCapability, SemanticRole,
    UniversalAdapterProfile,
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
    assert_eq!(evidence.adapter.version, 6);
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
        LanguageCapability::HierarchyDispatch,
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

#[test]
fn inherited_associated_types_emit_complete_trait_hierarchy_evidence() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("inherited.rs");
    let source = br#"trait Consumer<Item>: Send + Sized { type Reducer; }
trait UnindexedConsumer<I>: Consumer<I> {
    fn to_reducer(&self) -> Self::Reducer;
}
struct ConcreteReducer;
struct ItemConsumer;
impl<T: Send> Consumer<T> for ItemConsumer { type Reducer = ConcreteReducer; }
impl<T: Send> UnindexedConsumer<T> for ItemConsumer {
    fn to_reducer(&self) -> Self::Reducer { ConcreteReducer }
}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let unindexed = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::inherited::UnindexedConsumer")
        .ok_or("missing UnindexedConsumer declaration")?;
    assert!(unindexed.direct_bases_complete);
    let direct_bases = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.source_declaration_id == unindexed.id
                && candidate.relation == CandidateRelation::Extends
        })
        .collect::<Vec<_>>();
    assert_eq!(direct_bases.len(), 1, "direct bases={direct_bases:#?}");
    assert_eq!(direct_bases[0].target_spelling, "Consumer");
    assert_eq!(
        direct_bases[0].constraints.hierarchy,
        Some(HierarchyConstraint::DirectBase {
            base_set_complete: true,
        })
    );

    let method = evidence
        .declarations
        .iter()
        .find(|declaration| {
            declaration.qualified_name
                == "<crate::inherited::ItemConsumer as crate::inherited::UnindexedConsumer>::to_reducer"
        })
        .ok_or("missing UnindexedConsumer implementation method")?;
    let receiver = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::inherited::ItemConsumer")
        .ok_or("missing ItemConsumer declaration")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == method.id
            && candidate.relation == CandidateRelation::Returns
            && candidate.target_spelling == "Reducer"
            && candidate.constraints.hierarchy
                == Some(HierarchyConstraint::RustAssociatedType {
                    receiver_declaration_id: receiver.id.clone(),
                    receiver_qualified_name: "crate::inherited::ItemConsumer".to_owned(),
                    trait_qualified_name: "crate::inherited::UnindexedConsumer".to_owned(),
                })
    }));
    Ok(())
}
