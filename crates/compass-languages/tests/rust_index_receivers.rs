use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, SemanticRole};

#[test]
fn indexed_field_calls_preserve_element_owner_and_each_occurrence() -> Result<(), Box<dyn Error>> {
    let source = b"struct Entry;\nimpl Entry { fn close(&mut self) {} }\nstruct Walker { entries: Vec<Entry>, cursor: usize }\nimpl Walker { fn close(&mut self) {} fn advance(&mut self) {\n    self.entries[self.cursor].close();\n    self.entries[0].close();\n} }\n";
    let extraction = Engine::default().extract_source(Path::new("src/lib.rs"), source)?;
    let evidence = extraction
        .semantic_evidence
        .ok_or("missing Rust evidence")?;
    let calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "close"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(
            call.constraints.qualified_name.as_deref(),
            Some("crate::Entry::close"),
            "{call:#?}"
        );
        assert!(!call.constraints.allow_external);
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|o| Some(&o.id) == call.occurrence_id.as_ref())
            .ok_or("missing occurrence")?;
        assert_eq!(occurrence.role, SemanticRole::Call);
        let raw = std::str::from_utf8(
            &source[usize::try_from(occurrence.range.start_byte)?
                ..usize::try_from(occurrence.range.end_byte)?],
        )?;
        assert!(
            matches!(
                raw,
                "self.entries[self.cursor].close" | "self.entries[0].close"
            ),
            "{raw}"
        );
    }
    Ok(())
}

fn close_targets(source: &str) -> Result<Vec<Option<String>>, Box<dyn Error>> {
    let evidence = Engine::default()
        .extract_source(Path::new("src/lib.rs"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Rust evidence")?;
    Ok(evidence
        .candidates
        .iter()
        .filter(|call| call.relation == CandidateRelation::Calls && call.target_spelling == "close")
        .map(|call| call.constraints.qualified_name.clone())
        .collect())
}

#[test]
fn standard_sequences_and_nested_field_indexes_retain_the_element_type()
-> Result<(), Box<dyn Error>> {
    for (ty, expression) in [
        ("Vec<Entry>", "entries[0]"),
        ("std::vec::Vec<Entry>", "entries[0usize]"),
        ("alloc::vec::Vec<Entry>", "entries[0]"),
        ("&[Entry]", "entries[0]"),
        ("&mut [Entry; 4]", "entries[0]"),
        ("[[Entry; 2]; 4]", "entries[0][1]"),
        ("Vec<Vec<Entry>>", "entries[0][1]"),
        ("Box<[Entry]>", "entries[0]"),
        ("std::sync::Arc<Vec<Entry>>", "entries[0]"),
        ("Vec<Entry>", "(entries[0])"),
        ("Vec<Entry>", "(&entries[0])"),
        ("Vec<Entry>", "(&mut entries[0])"),
    ] {
        let source = format!(
            "struct Entry; impl Entry {{ fn close(&self) {{}} }} fn run(mut entries: {ty}) {{ {expression}.close(); }}"
        );
        assert_eq!(
            close_targets(&source)?,
            vec![Some("crate::Entry::close".to_owned())],
            "{source}"
        );
    }
    let source = "struct Entry; impl Entry { fn close(&self) {} } struct Slot { entry: Entry } struct Store { slots: Vec<Slot> } fn run(store: &Store, n: usize) { store.slots[n].entry.close(); }";
    assert_eq!(
        close_targets(source)?,
        vec![Some("crate::Entry::close".to_owned())]
    );
    Ok(())
}

#[test]
fn indexed_field_type_names_use_the_field_declaration_scope() -> Result<(), Box<dyn Error>> {
    let source = "mod model { pub struct Entry; impl Entry { pub fn close(&self) {} } pub struct Store { pub entries: Vec<Entry> } } struct Entry; impl Entry { fn close(&self) {} } fn run(store: &model::Store) { store.entries[0].close(); }";
    assert_eq!(
        close_targets(source)?,
        vec![Some("crate::model::Entry::close".to_owned())]
    );
    Ok(())
}

#[test]
fn custom_containers_and_shadowed_vec_never_imply_the_generic_argument()
-> Result<(), Box<dyn Error>> {
    for prefix in [
        "struct Vec<T>(T);",
        "use crate::custom::Vec;",
        "use crate::custom::*;",
        "use crate::one::*; use crate::two::*;",
        "#![no_implicit_prelude]",
        "#![no_std]",
        "#[cfg_attr(feature = \"bare\", no_implicit_prelude)] mod other {}",
    ] {
        let source = format!(
            "{prefix} struct Entry; impl Entry {{ fn close(&self) {{}} }} fn run(entries: Vec<Entry>) {{ entries[0].close(); }}"
        );
        assert_eq!(close_targets(&source)?, vec![None], "{source}");
    }
    for source in [
        "struct Entry; impl Entry { fn close(&self) {} } struct Other; struct Custom<T>(T); impl<T> std::ops::Index<usize> for Custom<T> { type Output = Other; fn index(&self, _: usize) -> &Other { todo!() } } fn run(entries: Custom<Entry>) { entries[0].close(); }",
        "fn run<Vec>(entries: Vec) { entries[0].close(); }",
        "struct Holder<Vec> { entries: Vec } impl<Vec> Holder<Vec> { fn run(&self) { self.entries[0].close(); } }",
    ] {
        assert_eq!(close_targets(source)?, vec![None], "{source}");
    }
    Ok(())
}

#[test]
fn ranges_unknown_indexes_and_raw_pointers_do_not_claim_element_methods()
-> Result<(), Box<dyn Error>> {
    for (ty, index_ty, index) in [
        ("Vec<Entry>", "usize", ".."),
        ("Vec<Entry>", "usize", "0..1"),
        ("Vec<Entry>", "std::ops::Range<usize>", "n"),
        ("Vec<Entry>", "Unknown", "n"),
        ("Vec<Entry>", "u32", "n"),
        ("Vec<Entry>", "usize", "&n"),
        ("Vec<Entry>", "usize", "(&n)"),
        ("Vec<Entry>", "&usize", "n"),
        ("Vec<Entry>", "usize", "0u32"),
        ("*const Vec<Entry>", "usize", "n"),
        ("*mut [Entry; 4]", "usize", "n"),
    ] {
        let source = format!(
            "struct Entry; impl Entry {{ fn close(&self) {{}} }} fn run(entries: {ty}, n: {index_ty}) {{ entries[{index}].close(); }}"
        );
        assert_eq!(close_targets(&source)?, vec![None], "{source}");
    }
    Ok(())
}

#[test]
fn indexed_local_receivers_and_indexes_respect_lexical_shadowing() -> Result<(), Box<dyn Error>> {
    let source = "struct Entry; impl Entry { fn close(&self) {} } fn run(entries: Vec<Entry>, n: usize) { { let entries = unknown(); entries[n].close(); } { let n = unknown(); entries[n].close(); } entries[n].close(); }";
    let targets = close_targets(source)?;
    assert_eq!(targets.iter().filter(|t| t.is_none()).count(), 2);
    assert_eq!(
        targets
            .iter()
            .filter(|t| t.as_deref() == Some("crate::Entry::close"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn explicit_vec_alias_and_element_alias_are_resolved() -> Result<(), Box<dyn Error>> {
    let source = "use std::vec::Vec as Sequence; use crate::api::Entry as Item; fn run(entries: Sequence<Item>) { entries[0].close(); }";
    assert_eq!(
        close_targets(source)?,
        vec![Some("crate::api::Entry::close".to_owned())]
    );
    Ok(())
}

#[test]
fn excessive_receiver_depth_remains_unresolved() -> Result<(), Box<dyn Error>> {
    let expression = format!("{}entries[0]{}", "(".repeat(40), ")".repeat(40));
    let source = format!("struct Entry; fn run(entries: Vec<Entry>) {{ {expression}.close(); }}");
    assert_eq!(close_targets(&source)?, vec![None]);
    Ok(())
}

#[test]
fn ambiguous_element_imports_do_not_select_a_target() -> Result<(), Box<dyn Error>> {
    let source = "use crate::a::Entry; use crate::b::Entry; fn run(entries: Vec<Entry>) { entries[0].close(); }";
    assert_eq!(close_targets(source)?, vec![None]);
    Ok(())
}

#[test]
fn nested_element_paths_preserve_every_module_segment() -> Result<(), Box<dyn Error>> {
    for (import, ty) in [
        ("", "api::nested::Entry"),
        ("use crate::api as a;", "a::nested::Entry"),
    ] {
        let source = format!(
            "mod api {{ pub struct Entry; impl Entry {{ pub fn close(&self) {{}} }} pub mod nested {{ pub struct Entry; impl Entry {{ pub fn close(&self) {{}} }} }} }} {import} fn run(entries: Vec<{ty}>, entry: &{ty}) {{ entries[0].close(); entry.close(); }}"
        );
        assert_eq!(
            close_targets(&source)?,
            vec![Some("crate::api::nested::Entry::close".to_owned()); 2],
            "{source}"
        );
    }
    let source = "use std as standard; struct Entry; impl Entry { fn close(&self) {} } fn run(entries: standard::vec::Vec<Entry>) { entries[0].close(); }";
    assert_eq!(
        close_targets(source)?,
        vec![Some("crate::Entry::close".to_owned())]
    );
    Ok(())
}

#[test]
fn conditional_field_layouts_do_not_choose_one_element_type() -> Result<(), Box<dyn Error>> {
    let source = "struct A; impl A { fn close(&self) {} } struct B; impl B { fn close(&self) {} } #[cfg(feature = \"a\")] struct Store { entries: Vec<A> } #[cfg(not(feature = \"a\"))] struct Store { entries: Vec<B> } impl Store { fn run(&self) { self.entries[0].close(); } }";
    assert_eq!(close_targets(source)?, vec![None]);
    Ok(())
}
