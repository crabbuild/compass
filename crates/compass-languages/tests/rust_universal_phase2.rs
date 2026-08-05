use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::path::Path;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, SemanticRole, validate_evidence,
};

#[test]
fn rust_phase2_emits_modules_traits_impls_import_trees_and_macros() -> Result<(), Box<dyn Error>> {
    let source = br#"
mod api {
    pub trait Render { fn render(&self); }
    pub struct Widget { pub value: u64 }
}
use crate::api::{self, Render as Renderer, Widget};
use external_crate::{Thing as ExternalThing, prelude::*};
type Current = Widget;
const LIMIT: u64 = 1;
macro_rules! local_macro { () => {}; }
impl Renderer for Widget { fn render(&self) { local_macro!(); } }
fn build(value: ExternalThing) -> Widget { local_macro!(); Widget { value: LIMIT } }
"#;
    let extraction = Engine::default().extract_source(Path::new("src/lib.rs"), source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let declarations = evidence
        .declarations
        .iter()
        .map(|declaration| {
            (
                declaration.kind.as_str(),
                declaration.qualified_name.as_str(),
            )
        })
        .collect::<HashSet<_>>();
    for expected in [
        ("module", "crate::api"),
        ("trait", "crate::api::Render"),
        ("struct", "crate::api::Widget"),
        ("field", "crate::api::Widget::value"),
        ("type_alias", "crate::Current"),
        ("constant", "crate::LIMIT"),
        ("macro", "crate::local_macro"),
        (
            "method",
            "<crate::api::Widget as crate::api::Render>::render",
        ),
        ("function", "crate::build"),
    ] {
        assert!(
            declarations.contains(&expected),
            "missing {expected:?}: {:#?}",
            evidence.declarations
        );
    }

    for (spelling, target, kind) in [
        ("api", "crate::api", BindingKind::Import),
        ("Renderer", "crate::api::Render", BindingKind::ImportAlias),
        ("Widget", "crate::api::Widget", BindingKind::Import),
        (
            "ExternalThing",
            "external_crate::Thing",
            BindingKind::ImportAlias,
        ),
        ("*", "external_crate::prelude", BindingKind::Import),
    ] {
        assert!(
            evidence.bindings.iter().any(|binding| {
                binding.spelling == spelling
                    && binding.qualified_target == target
                    && binding.kind == kind
                    && binding.range.start_byte < binding.range.end_byte
            }),
            "missing binding {spelling} -> {target}: {:#?}",
            evidence.bindings
        );
    }
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Implements
            && candidate.constraints.qualified_name.as_deref() == Some("crate::api::Render")
            && !candidate.constraints.allow_external
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.kind == BindingKind::Member
            && binding.spelling == "render"
            && binding.qualified_target == "<crate::api::Widget as crate::api::Render>::render"
            && binding.target_declaration_id.is_some()
    }));
    assert_eq!(
        evidence
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.role == SemanticRole::MacroInvocation
                    && occurrence.spelling == "local_macro"
            })
            .count(),
        2
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.constraints.qualified_name.as_deref() == Some("external_crate::Thing")
            && candidate.constraints.allow_external
    }));
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());
    Ok(())
}

#[test]
fn rust_phase2_preserves_repeated_unicode_occurrences_and_parser_diagnostics()
-> Result<(), Box<dyn Error>> {
    let source = "// λ keeps UTF-8 byte offsets honest\nfn target() {}\nfn caller() { target(); target(); }\n";
    let extraction =
        Engine::default().extract_source(Path::new("src/unicode.rs"), source.as_bytes())?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Unicode Rust evidence")?;
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].range, calls[1].range);
    for call in calls {
        let start = usize::try_from(call.range.start_byte)?;
        let end = usize::try_from(call.range.end_byte)?;
        assert_eq!(&source.as_bytes()[start..end], b"target");
    }

    let malformed = Engine::default().extract_source(
        Path::new("src/malformed.rs"),
        b"fn broken( { let value = 1;\n",
    )?;
    let malformed_evidence = malformed
        .semantic_evidence
        .as_ref()
        .ok_or("missing malformed Rust evidence")?;
    validate_evidence(malformed_evidence, EvidenceLimits::default())?;
    assert!(
        malformed_evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
    Ok(())
}

#[test]
fn rust_phase2_emits_direct_test_relationship_candidates() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/tests.rs"),
        b"fn target() {}\n#[test]\nfn target_works() { target(); }\n",
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    assert!(
        evidence
            .adapter
            .capabilities
            .contains(&compass_languages::LanguageCapability::Tests)
    );
    let test = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "target_works")
        .ok_or("missing test declaration")?;
    let call = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.role == SemanticRole::Call && occurrence.spelling == "target")
        .ok_or("missing test call")?;
    for relation in [CandidateRelation::Calls, CandidateRelation::Tests] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == relation
                && candidate.source_declaration_id == test.id
                && candidate.occurrence_id.as_deref() == Some(call.id.as_str())
        }));
    }
    Ok(())
}

#[test]
fn rust_phase2_distinguishes_same_named_module_and_function_candidates()
-> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("benches/components/mod.rs"),
        b"mod insert_simple;\nfn insert_simple() {}\n",
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let candidates = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.constraints.qualified_name.as_deref() == Some("crate::insert_simple")
        })
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 2, "candidates={candidates:#?}");
    assert_ne!(candidates[0].id, candidates[1].id);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.constraints.allowed_target_kinds.join(","))
            .collect::<HashSet<_>>(),
        HashSet::from(["function".to_owned(), "module".to_owned()])
    );
    Ok(())
}

#[test]
fn rust_phase2_preserves_nested_function_declarations_and_calls() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        b"fn join() { fn call() {} call(); call(); }\n",
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let outer = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::join")
        .ok_or("missing outer function")?;
    let nested = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::join::call")
        .ok_or("missing nested function")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == outer.id
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(nested.id.as_str())
    }));
    assert_eq!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.source_declaration_id == outer.id
                    && candidate.target_spelling == "call"
            })
            .count(),
        2
    );
    Ok(())
}

#[test]
fn rust_phase2_attaches_local_wildcards_to_lowercase_call_candidates() -> Result<(), Box<dyn Error>>
{
    let extraction = Engine::default().extract_source(
        Path::new("src/join/test.rs"),
        b"use super::*;\nfn invokes() { join(); }\n",
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let call = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "join"
        })
        .ok_or("missing lowercase call candidate")?;
    let binding = call
        .binding_id
        .as_deref()
        .and_then(|binding_id| {
            evidence
                .bindings
                .iter()
                .find(|binding| binding.id == binding_id)
        })
        .ok_or("lowercase call has no wildcard binding")?;
    assert_eq!(binding.spelling, "*");
    assert_eq!(binding.qualified_target, "crate::join");
    Ok(())
}

#[test]
fn rust_phase2_declares_scoped_type_parameters_and_targets_their_uses() -> Result<(), Box<dyn Error>>
{
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"struct Wrapper<T: Clone> { value: T }
fn identity<T: Send>(value: T) -> T { value }
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let wrapper = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::Wrapper")
        .ok_or("missing Wrapper")?;
    let identity = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::identity")
        .ok_or("missing identity")?;
    let parameters = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "parameter" && declaration.name == "T")
        .collect::<Vec<_>>();
    assert_eq!(parameters.len(), 2);
    assert_ne!(parameters[0].qualified_name, parameters[1].qualified_name);
    for (owner, expected_prefix) in [(wrapper, "crate::Wrapper"), (identity, "crate::identity")] {
        let parameter = parameters
            .iter()
            .copied()
            .find(|parameter| parameter.qualified_name.starts_with(expected_prefix))
            .ok_or("missing owner-scoped type parameter")?;
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.source_declaration_id == owner.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(parameter.id.as_str())
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::References
                    | CandidateRelation::TypeOf
                    | CandidateRelation::Returns
            ) && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parameter.id.as_str())
        }));
    }
    Ok(())
}

#[test]
fn rust_phase2_keeps_impl_and_method_type_parameters_lexically_distinct()
-> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"trait Marker {}
struct Wrapper<T> { value: T }
impl<T: Marker> Wrapper<T> {
    fn convert<U: Marker>(&self, value: U) -> T { self.value }
}
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let type_parameters = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "parameter")
        .collect::<Vec<_>>();
    assert_eq!(type_parameters.len(), 3);
    assert_eq!(
        type_parameters
            .iter()
            .map(|parameter| parameter.qualified_name.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let implementation_parameter = type_parameters
        .iter()
        .copied()
        .find(|parameter| {
            parameter.name == "T" && parameter.qualified_name.contains("<impl<T: Marker>")
        })
        .ok_or("missing implementation type parameter")?;
    let method_parameter = type_parameters
        .iter()
        .copied()
        .find(|parameter| parameter.name == "U")
        .ok_or("missing method type parameter")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Returns
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(implementation_parameter.id.as_str())
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(method_parameter.id.as_str())
    }));
    Ok(())
}

#[test]
fn rust_phase2_scopes_associated_types_and_resolves_self_returns_exactly()
-> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"trait Produce {
    type Output;
    fn produce() -> Self::Output;
}
struct Alpha;
struct Beta;
struct AlphaOutput;
struct BetaOutput;
struct Iter;
impl Produce for Alpha {
    type Output = AlphaOutput;
    fn produce() -> Self::Output { AlphaOutput }
}
impl Produce for Beta {
    type Output = BetaOutput;
    fn produce() -> Self::Output { BetaOutput }
}
trait Iterate { type Iter; fn iter() -> Self::Iter; }
impl Iterate for Alpha { type Iter = Iter; fn iter() -> Self::Iter { Iter } }
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let associated_types = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "type_alias" && declaration.name == "Output")
        .collect::<Vec<_>>();
    assert_eq!(associated_types.len(), 3);
    assert_eq!(
        associated_types
            .iter()
            .map(|declaration| declaration.qualified_name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "<impl Produce for Alpha>::Output",
            "<impl Produce for Beta>::Output",
            "crate::Produce::Output",
        ])
    );

    for (owner, concrete) in [("Alpha", "AlphaOutput"), ("Beta", "BetaOutput")] {
        let associated = associated_types
            .iter()
            .copied()
            .find(|declaration| {
                declaration
                    .qualified_name
                    .contains(&format!("for {owner}>"))
            })
            .ok_or("missing impl-scoped associated type")?;
        let method = evidence
            .declarations
            .iter()
            .find(|declaration| {
                declaration.kind == "method"
                    && declaration
                        .qualified_name
                        .starts_with(&format!("<crate::{owner} as crate::Produce>"))
            })
            .ok_or("missing impl method")?;
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == method.id
                && candidate.relation == CandidateRelation::Returns
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(associated.id.as_str())
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == associated.id
                && candidate.relation == CandidateRelation::References
                && candidate.constraints.qualified_name.as_deref()
                    == Some(format!("crate::{concrete}").as_str())
        }));
    }
    let iteration_alias = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "<impl Iterate for Alpha>::Iter")
        .ok_or("missing same-named associated type")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == iteration_alias.id
            && candidate.relation == CandidateRelation::References
            && candidate.constraints.qualified_name.as_deref() == Some("crate::Iter")
    }));
    Ok(())
}

#[test]
fn rust_phase2_publishes_type_lifetime_and_const_generic_parameters() -> Result<(), Box<dyn Error>>
{
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        b"struct Buffer<'a, T, const N: usize> { value: &'a [T; N] }\n",
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let buffer = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::Buffer")
        .ok_or("missing Buffer")?;
    let parameters = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "parameter")
        .collect::<Vec<_>>();
    assert_eq!(
        parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["'a", "N", "T"])
    );
    for parameter in parameters {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.source_declaration_id == buffer.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(parameter.id.as_str())
        }));
        assert!(evidence.candidates.iter().any(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::References | CandidateRelation::TypeOf
            ) && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parameter.id.as_str())
        }));
    }
    Ok(())
}

#[test]
fn rust_phase2_preserves_impl_self_constructors_and_public_wildcards() -> Result<(), Box<dyn Error>>
{
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"
pub use external::prelude::*;
struct Widget(u64);
impl Widget {
    fn first(&self) { self.second(); (*self).second(); }
    fn second(&self) {}
    fn new() -> Self { Self(0) }
}
impl Default for Widget {
    fn default() -> Self { Self::new() }
}
fn build() -> Widget { Widget(1) }
fn external() { External::load(); }
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let self_calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "second"
        })
        .collect::<Vec<_>>();
    assert_eq!(self_calls.len(), 2);
    assert!(self_calls.iter().all(|candidate| {
        candidate.constraints.qualified_name.as_deref() == Some("crate::Widget::second")
            && !candidate.constraints.allow_external
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "new"
            && candidate.constraints.qualified_name.as_deref() == Some("crate::Widget::new")
            && !candidate.constraints.allow_external
    }));

    let constructor = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "Widget"
        })
        .ok_or("missing tuple-struct constructor")?;
    assert!(
        constructor
            .constraints
            .allowed_target_kinds
            .contains(&"struct".to_owned())
    );
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "*"
            && binding.qualified_target == "external::prelude"
            && binding.kind == BindingKind::Reexport
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Reexports && candidate.target_spelling == "prelude"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "load"
            && candidate.binding_id.is_some()
            && candidate.constraints.qualified_name.as_deref() == Some("External::load")
            && candidate.constraints.allow_external
    }));
    Ok(())
}

#[test]
fn rust_phase2_scopes_trait_impl_methods_and_emits_enum_payload_references()
-> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"
trait Execute { fn execute(&self); }
struct Local;
struct Remote;
impl Execute for Local { fn execute(&self) {} }
impl Execute for Remote { fn execute(&self) {} }
enum Event { Local(Local), Remote { value: Remote } }
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    for owner in ["Local", "Remote"] {
        let method = evidence
            .declarations
            .iter()
            .find(|declaration| {
                declaration.kind == "method"
                    && declaration
                        .qualified_name
                        .starts_with(&format!("<crate::{owner} as "))
            })
            .ok_or("missing trait impl method")?;
        let containment = evidence
            .candidates
            .iter()
            .find(|candidate| {
                candidate.relation == CandidateRelation::Contains
                    && candidate.constraints.qualified_name.as_deref()
                        == Some(method.qualified_name.as_str())
            })
            .ok_or("missing trait impl containment")?;
        assert!(evidence.scopes.iter().any(|scope| {
            scope.id == method.scope_id.as_deref().unwrap_or_default() && scope.kind == "impl"
        }));
        assert_eq!(
            containment.constraints.qualified_name.as_deref(),
            Some(method.qualified_name.as_str())
        );
    }

    let event = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::Event")
        .ok_or("missing enum declaration")?;
    for payload in ["crate::Local", "crate::Remote"] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::References
                && candidate.source_declaration_id == event.id
                && candidate.constraints.qualified_name.as_deref() == Some(payload)
        }));
    }
    Ok(())
}

#[test]
fn rust_generic_impl_bounds_resolve_direct_and_field_receivers() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"
trait Render { fn render(&self); }
struct Wrapper<T> { value: T }
impl<T> Wrapper<T>
where
    T: Render,
{
    fn invoke(&self, value: T) { value.render(); self.value.render(); }
}
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;

    let calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "render"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "calls={calls:#?}");
    assert!(
        calls.iter().all(|candidate| {
            candidate.constraints.qualified_name.as_deref() == Some("crate::Render::render")
                && !candidate.constraints.allow_external
        }),
        "calls={calls:#?}"
    );
    Ok(())
}

#[test]
fn rust_struct_generic_bounds_flow_into_inherent_impls() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/lib.rs"),
        br#"
trait Render { fn render(&self); }
struct Wrapper<T: Render> { value: T }
impl<T> Wrapper<T> {
    fn invoke(&self, value: T) { value.render(); self.value.render(); }
}
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "render"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "calls={calls:#?}");
    assert!(
        calls.iter().all(|candidate| {
            candidate.constraints.qualified_name.as_deref() == Some("crate::Render::render")
                && !candidate.constraints.allow_external
        }),
        "calls={calls:#?}"
    );
    Ok(())
}
