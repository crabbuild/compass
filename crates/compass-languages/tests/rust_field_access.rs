use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, SemanticRole, validate_evidence,
};

type FieldTarget = (String, Option<String>);

fn targets(source: &str) -> Result<Vec<FieldTarget>, Box<dyn Error>> {
    let evidence = Engine::default()
        .extract_source(Path::new("src/lib.rs"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Rust evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    let mut result = Vec::new();
    for candidate in &evidence.candidates {
        if candidate.relation != CandidateRelation::AccessesMember {
            continue;
        }
        assert_eq!(candidate.constraints.allowed_target_kinds, ["field"]);
        assert!(!candidate.constraints.allow_external);
        assert!(candidate.binding_id.is_none());
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|o| Some(&o.id) == candidate.occurrence_id.as_ref())
            .ok_or("missing member occurrence")?;
        assert_eq!(occurrence.role, SemanticRole::MemberAccess);
        assert!(occurrence.qualifier.is_some());
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        let spelling = source
            .get(start..end)
            .ok_or("invalid member source range")?;
        assert_eq!(spelling, candidate.target_spelling);
        result.push((
            spelling.to_owned(),
            candidate.constraints.qualified_name.clone(),
        ));
    }
    result.sort();
    Ok(result)
}

#[test]
fn self_field_uses_keep_each_occurrence_and_skip_method_selectors() -> Result<(), Box<dyn Error>> {
    let source = "// λ\nstruct State { value: usize } impl State { fn get(&self) -> usize { self.value } fn bump(&mut self) { self.value = self.value + 1; self.get(); } }";
    assert_eq!(
        targets(source)?,
        vec![("value".into(), Some("crate::State::value".into())); 3]
    );
    Ok(())
}

#[test]
fn typed_parameters_nested_fields_and_indexes_keep_nominal_owners() -> Result<(), Box<dyn Error>> {
    let source = "struct Item { value: usize } struct State { items: Vec<Item> } fn run(state: &State, item: &Item) { let _ = item.value; let _ = state.items[0].value; }";
    assert_eq!(
        targets(source)?,
        vec![
            ("items".into(), Some("crate::State::items".into())),
            ("value".into(), Some("crate::Item::value".into())),
            ("value".into(), Some("crate::Item::value".into())),
        ]
    );
    Ok(())
}

#[test]
fn unknown_and_shadowed_receivers_do_not_inherit_outer_types() -> Result<(), Box<dyn Error>> {
    let source = "struct Item { value: usize } fn run(item: &Item) { { let item = unknown(); let _ = item.value; } let _ = item.value; for item in unknown() { let _ = item.value; } let _ = (|item| item.value)(unknown()); }";
    assert_eq!(
        targets(source)?,
        vec![
            ("value".into(), None),
            ("value".into(), None),
            ("value".into(), None),
            ("value".into(), Some("crate::Item::value".into())),
        ]
    );
    Ok(())
}

#[test]
fn ambiguous_imports_unknown_index_and_raw_pointers_remain_unresolved() -> Result<(), Box<dyn Error>>
{
    for source in [
        "use crate::a::Item; use crate::b::Item; fn run(item: &Item) { let _ = item.value; }",
        "struct Item { value: usize } fn run(items: Vec<Item>, index: Unknown) { let _ = items[index].value; }",
        "struct Item { value: usize } fn run(item: *const Item) { let _ = item.value; }",
    ] {
        assert_eq!(targets(source)?, vec![("value".into(), None)], "{source}");
    }
    Ok(())
}

#[test]
fn receiver_wrappers_and_trait_impl_self_keep_field_identity() -> Result<(), Box<dyn Error>> {
    let source = "struct Item { value: usize } trait Read { fn read(&self) -> usize; } impl Read for Item { fn read(&self) -> usize { (&self).value } }";
    assert_eq!(
        targets(source)?,
        vec![("value".into(), Some("crate::Item::value".into()))]
    );
    Ok(())
}

#[test]
fn bounded_receiver_syntax_never_falls_back_to_a_field_name() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "struct Item {{ value: usize }} fn run(item: Item) {{ let _ = {}item{}.value; }}",
        "(".repeat(40),
        ")".repeat(40)
    );
    assert_eq!(targets(&source)?, vec![("value".into(), None)]);
    Ok(())
}

#[test]
fn generic_method_selectors_are_not_fields_but_callable_fields_are_contacts()
-> Result<(), Box<dyn Error>> {
    let source = "struct Item { callback: fn() } impl Item { fn method<T>(&self) {} fn run(&self) { self.method::<usize>(); (self.callback)(); } }";
    assert_eq!(
        targets(source)?,
        vec![("callback".into(), Some("crate::Item::callback".into()))]
    );
    Ok(())
}
