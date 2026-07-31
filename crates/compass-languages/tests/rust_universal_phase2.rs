use std::collections::HashSet;
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
