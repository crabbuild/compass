use std::error::Error;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, HierarchyConstraint, LanguageCapability, SemanticRole,
    UniversalEvidenceQualification,
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

    assert_eq!(evidence.pipeline.id, "compass.rust");
    assert_eq!(evidence.pipeline.version, 15);
    assert_eq!(
        evidence.pipeline.evidence_schema,
        "compass.languages.evidence/2"
    );
    assert_eq!(
        evidence.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
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
        assert!(evidence.pipeline.capabilities.contains(&capability));
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
fn source_proven_method_results_bind_the_next_call_receiver() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("method_chain.rs");
    let source = br#"struct Input;
struct Output;
impl Input { fn transform(self) -> Output { Output } }
impl Output { fn finish(self) {} }
fn local(input: Input) { input.transform().finish(); }
fn unknown<T>(input: T) { input.transform().finish(); }
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let bindings = evidence
        .bindings
        .iter()
        .filter(|binding| binding.kind == BindingKind::CallResult)
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 2, "bindings={bindings:#?}");
    let binding = bindings
        .iter()
        .find(|binding| binding.result_type_qualified_name.is_some())
        .ok_or("missing exact local call-result binding")?;
    assert_eq!(binding.spelling, "input.transform()");
    assert_eq!(
        binding.qualified_target,
        "crate::method_chain::Input::transform"
    );
    assert_eq!(
        binding.result_type_qualified_name.as_deref(),
        Some("crate::method_chain::Output")
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "finish"
            && candidate.binding_id.as_deref() == Some(binding.id.as_str())
    }));
    assert!(bindings.iter().any(|binding| {
        binding.result_type_qualified_name.is_none()
            && binding.qualified_target == "crate::method_chain::unknown::<T>::transform"
    }));
    Ok(())
}

#[test]
fn raw_pointer_returns_are_not_retyped_as_the_pointee_receiver() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("pointer_chain.rs");
    let source = br#"struct Worker;
impl Worker { fn current() -> *const Worker { std::ptr::null() } }
fn run() { unsafe { Worker::current().as_ref(); } }
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let current_result = evidence
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == BindingKind::CallResult && binding.spelling == "Worker::current()"
        })
        .ok_or("missing current call-result binding")?;
    assert_eq!(current_result.result_type_qualified_name, None);
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
fn blanket_implementations_use_the_exact_scoped_type_parameter_owner() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("blanket_impl.rs");
    let source = br#"trait Render {}
impl<T> Render for T {}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let parameter = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "<impl<T> Render for T>::<T>")
        .ok_or("missing blanket implementation parameter")?;
    let implementation = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.source_declaration_id == parameter.id
                && candidate.relation == CandidateRelation::Implements
                && candidate.target_spelling == "Render"
        })
        .ok_or("missing blanket implementation candidate")?;
    assert_eq!(
        implementation.constraints.qualified_name.as_deref(),
        Some("crate::blanket_impl::Render")
    );
    let occurrence = evidence
        .occurrences
        .iter()
        .find(|occurrence| implementation.occurrence_id.as_deref() == Some(occurrence.id.as_str()))
        .ok_or("missing blanket implementation occurrence")?;
    let start = usize::try_from(occurrence.range.start_byte)?;
    let end = usize::try_from(occurrence.range.end_byte)?;
    assert_eq!(&source[start..end], b"Render");
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.source_declaration_id != parameter.id
            || candidate.relation != CandidateRelation::References
            || candidate.target_spelling != "Render"
    }));
    Ok(())
}

#[test]
fn implementation_trait_arguments_emit_exact_implementer_owned_references()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("impl_args.rs");
    let source = br#"trait Convert<T> {}
struct Input;
struct Output;
impl Convert<Input> for Output {}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let output = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::impl_args::Output")
        .ok_or("missing Output declaration")?;
    let reference = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.source_declaration_id == output.id
                && candidate.relation == CandidateRelation::References
                && candidate.target_spelling == "Input"
        })
        .ok_or("missing implementation argument reference")?;
    assert_eq!(
        reference.constraints.qualified_name.as_deref(),
        Some("crate::impl_args::Input")
    );
    let occurrence = evidence
        .occurrences
        .iter()
        .find(|occurrence| reference.occurrence_id.as_deref() == Some(occurrence.id.as_str()))
        .ok_or("missing implementation argument occurrence")?;
    let start = usize::try_from(occurrence.range.start_byte)?;
    let end = usize::try_from(occurrence.range.end_byte)?;
    assert_eq!(&source[start..end], b"Input");
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.source_declaration_id != output.id
            || candidate.relation != CandidateRelation::References
            || candidate.target_spelling != "Convert"
    }));
    Ok(())
}

#[test]
fn malformed_implementation_trait_arguments_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed_impl_args.rs");
    let source = br#"trait Convert<T> {}
struct Input;
struct Output;
    impl Convert<Input +> for Output {}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing malformed Rust semantic evidence")?;
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::References || candidate.target_spelling != "Input"
    }));
    Ok(())
}

#[test]
fn malformed_blanket_implementations_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed_blanket_impl.rs");
    let source = br#"trait Render<T> {}
impl<T> Render<T +> for T {}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing malformed Rust semantic evidence")?;
    let parameter = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "<impl<T> Render<T +> for T>::<T>")
        .ok_or("parser recovery did not preserve the implementation parameter")?;
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.source_declaration_id != parameter.id
            || candidate.relation != CandidateRelation::Implements
    }));
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
