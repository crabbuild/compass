use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, SemanticRole, validate_evidence,
};

#[derive(Debug)]
struct Contact {
    owner: String,
    target: Option<String>,
    kinds: Vec<String>,
    start: usize,
}

fn contacts(source: &str) -> Result<Vec<Contact>, Box<dyn Error>> {
    let evidence = Engine::default()
        .extract_source(Path::new("State.java"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Java evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    let mut contacts = Vec::new();
    for candidate in &evidence.candidates {
        if candidate.relation != CandidateRelation::AccessesMember {
            continue;
        }
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|site| Some(&site.id) == candidate.occurrence_id.as_ref())
            .ok_or("missing member occurrence")?;
        let owner = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == candidate.source_declaration_id)
            .ok_or("missing source declaration")?;
        assert_eq!(occurrence.role, SemanticRole::MemberAccess);
        assert_eq!(occurrence.owner_declaration_id, owner.id);
        assert!(!candidate.constraints.allow_external);
        assert!(candidate.binding_id.is_none());
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert_eq!(
            source.get(start..end),
            Some(candidate.target_spelling.as_str())
        );
        contacts.push(Contact {
            owner: owner.qualified_name.clone(),
            target: candidate.constraints.qualified_name.clone(),
            kinds: candidate.constraints.allowed_target_kinds.clone(),
            start,
        });
    }
    contacts.sort_by_key(|contact| contact.start);
    Ok(contacts)
}

fn pairs(contacts: &[Contact]) -> Vec<(&str, Option<&str>)> {
    contacts
        .iter()
        .map(|contact| (contact.owner.as_str(), contact.target.as_deref()))
        .collect()
}

#[test]
fn qualified_enum_access_allows_enum_members_and_preserves_the_member_token()
-> Result<(), Box<dyn Error>> {
    let source =
        "package p; // λ\nenum State { A, B } class Box { State run() { return State.A; } }";
    let actual = contacts(source)?;
    assert_eq!(pairs(&actual), [("p.Box::run", Some("p.State::A"))]);
    assert!(actual[0].kinds.iter().any(|kind| kind == "enum_member"));
    assert_eq!(actual[0].start, source.find("State.A").ok_or("access")? + 6);
    Ok(())
}

#[test]
fn unqualified_enum_values_are_static_and_parameters_shadow_them() -> Result<(), Box<dyn Error>> {
    let source = "package p; enum State { A, B; State next() { return B; } State shadow(State B) { return B; } static State first() { return A; } }";
    let actual = contacts(source)?;
    assert_eq!(
        pairs(&actual),
        [
            ("p.State::next", Some("p.State::B")),
            ("p.State::first", Some("p.State::A")),
        ]
    );
    assert!(
        actual
            .iter()
            .all(|c| c.kinds.iter().any(|k| k == "enum_member"))
    );
    Ok(())
}

#[test]
fn registered_enum_constant_bodies_keep_their_method_owners() -> Result<(), Box<dyn Error>> {
    let source =
        "package p; enum State { A { State next() { return B; } }, B; State next() { return A; } }";
    assert_eq!(
        pairs(&contacts(source)?),
        [
            ("p.State::A::next", Some("p.State::B")),
            ("p.State::next", Some("p.State::A")),
        ]
    );
    Ok(())
}

#[test]
fn constant_body_fields_shadow_enum_values_and_this_keeps_the_body_owner()
-> Result<(), Box<dyn Error>> {
    let source = "package p; enum State { A { int B = 1; int size() { return B + this.B; } }, B; State next() { return B; } }";
    assert_eq!(
        pairs(&contacts(source)?),
        [
            ("p.State::A::size", Some("p.State::A::B")),
            ("p.State::A::size", Some("p.State::A::B")),
            ("p.State::next", Some("p.State::B")),
        ]
    );
    Ok(())
}

#[test]
fn enum_switch_labels_follow_the_selector_instead_of_a_same_named_field()
-> Result<(), Box<dyn Error>> {
    let source = "package p; enum State { A, B } class Box { static final int A = 1; int run(State value) { switch (value) { case A: return 1; case B: return 2; default: return 0; } } }";
    let actual = contacts(source)?;
    assert_eq!(
        pairs(&actual),
        [
            ("p.Box::run", Some("p.State::A")),
            ("p.Box::run", Some("p.State::B")),
        ]
    );
    assert!(
        actual
            .iter()
            .all(|contact| contact.kinds == ["enum_member"])
    );
    Ok(())
}

#[test]
fn unknown_switch_expressions_do_not_supply_a_convenient_enum_owner() -> Result<(), Box<dyn Error>>
{
    let source = "package p; enum State { A } class Box { int A; int run() { switch (unknown()) { case A: return 1; default: return 0; } } }";
    let actual = contacts(source)?;
    assert_eq!(pairs(&actual), [("p.Box::run", None)]);
    assert_eq!(actual[0].kinds, ["enum_member"]);
    Ok(())
}

#[test]
fn value_local_type_and_type_parameter_shadowing_do_not_select_the_enum()
-> Result<(), Box<dyn Error>> {
    for source in [
        "package p; enum State { A } class Box { Object run(Object State) { return State.A; } }",
        "package p; enum State { A } class Box { void run() { class State {} sink(State.A); } }",
        "package p; enum State { A } class Box { <State> Object run() { return State.A; } }",
    ] {
        let actual = contacts(source)?;
        assert!(
            actual
                .iter()
                .all(|contact| contact.target.as_deref() != Some("p.State::A")),
            "{source}: {actual:#?}"
        );
    }
    Ok(())
}

#[test]
fn unregistered_anonymous_and_local_classes_do_not_borrow_an_enum_owner()
-> Result<(), Box<dyn Error>> {
    for source in [
        "package p; enum State { A; void run() { Object value = new Object() { State get() { return A; } }; } }",
        "package p; enum State { A; void run() { class Local { State get() { return A; } } } }",
    ] {
        assert!(contacts(source)?.is_empty(), "{source}");
    }
    Ok(())
}

#[test]
fn enum_body_field_initializers_keep_the_field_as_source() -> Result<(), Box<dyn Error>> {
    let source = "package p; enum State { A { int B = 1; int next = B; }, B; }";
    assert_eq!(
        pairs(&contacts(source)?),
        [("p.State::A::next", Some("p.State::A::B"))]
    );
    Ok(())
}

#[test]
fn duplicate_enum_values_and_interface_hiding_remain_unknown() -> Result<(), Box<dyn Error>> {
    for source in [
        "package p; enum State { A, A; State next() { return A; } }",
        "package p; enum State implements Mask { A; Object next() { return State.A; } } interface Mask { Object State = null; }",
    ] {
        assert!(
            contacts(source)?.iter().all(|c| c.target.is_none()),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn nested_enum_receiver_precedes_a_same_named_import() -> Result<(), Box<dyn Error>> {
    let source = "package p; import remote.State; class Box { enum State { A } State run() { return State.A; } }";
    assert_eq!(
        pairs(&contacts(source)?),
        [("p.Box::run", Some("p.Box::State::A"))]
    );
    Ok(())
}
