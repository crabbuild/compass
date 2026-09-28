use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, SemanticRole, validate_evidence,
};

type Target = (String, Option<String>);
fn targets(source: &str) -> Result<Vec<Target>, Box<dyn Error>> {
    let evidence = Engine::default()
        .extract_source(Path::new("Box.java"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Java evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    let mut result = Vec::new();
    for candidate in &evidence.candidates {
        if candidate.relation != CandidateRelation::AccessesMember {
            continue;
        }
        assert!(
            candidate
                .constraints
                .allowed_target_kinds
                .iter()
                .any(|kind| kind == "field")
        );
        assert!(
            candidate
                .constraints
                .allowed_target_kinds
                .iter()
                .all(|kind| matches!(kind.as_str(), "field" | "enum_member"))
        );
        assert!(!candidate.constraints.allow_external);
        assert!(candidate.binding_id.is_none());
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|o| Some(&o.id) == candidate.occurrence_id.as_ref())
            .ok_or("missing occurrence")?;
        assert_eq!(occurrence.role, SemanticRole::MemberAccess);
        assert!(occurrence.qualifier.is_some());
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert_eq!(
            source.get(start..end),
            Some(candidate.target_spelling.as_str())
        );
        result.push((
            candidate.target_spelling.clone(),
            candidate.constraints.qualified_name.clone(),
        ));
    }
    result.sort();
    Ok(result)
}
fn field(name: &str, owner: &str) -> Target {
    (name.into(), Some(format!("p.{owner}::{name}")))
}

#[test]
fn own_fields_preserve_occurrences_and_parameter_shadowing() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Box { int value; Box(int value) { this.value = value; } int get() { return value + this.value; } int argument(int value) { return value; } }";
    assert_eq!(targets(source)?, vec![field("value", "Box"); 3]);
    Ok(())
}
#[test]
fn local_scope_exit_restores_field_binding() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Box { int value; void run() { sink(value); { int value = 1; sink(value); } sink(value); for (int value = 0; value < 1; value++) { sink(value); } sink(value); } void sink(int x) {} }";
    assert_eq!(targets(source)?, vec![field("value", "Box"); 3]);
    Ok(())
}
#[test]
fn lexical_receivers_do_not_use_shadowed_or_out_of_scope_types() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { Cell item; void run(Cell param) { sink(param.value); sink(item.value); { var item = unknown(); sink(item.value); } sink(item.value); } }";
    let mut expected = vec![
        field("item", "Box"),
        field("item", "Box"),
        field("value", "Cell"),
        field("value", "Cell"),
        field("value", "Cell"),
        ("value".into(), None),
    ];
    expected.sort();
    assert_eq!(targets(source)?, expected);
    Ok(())
}
#[test]
fn lambdas_keep_this_but_parameters_catch_and_resources_shadow_fields() -> Result<(), Box<dyn Error>>
{
    let source = "package p; class Box { int value; Object failure; Token resource; void run() { F f = value -> value; G g = () -> this.value; try {} catch (Exception failure) { sink(failure); } sink(failure); try (Token resource = new Token()) { sink(resource.value); } catch (Exception e) { sink(resource.value); } } } class Token { int value; }";
    let mut expected = vec![
        field("value", "Box"),
        field("failure", "Box"),
        field("resource", "Box"),
        field("value", "Token"),
        field("value", "Token"),
    ];
    expected.sort();
    assert_eq!(targets(source)?, expected);
    Ok(())
}
#[test]
fn declaration_type_label_and_method_names_are_not_field_occurrences() -> Result<(), Box<dyn Error>>
{
    let source = "package p; class Box { int value; int value() { return 1; } void run() { value(); value: while (true) { break value; } } }";
    assert!(targets(source)?.is_empty());
    Ok(())
}
#[test]
fn nested_this_and_unknown_class_boundaries_are_distinct() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Box { int value; class Inner { int value; int run() { return value + Box.this.value; } } void run() { class Local { int value; int get() { return this.value; } } F f = new F() { int value; int get() { return this.value; } }; } }";
    let mut expected = vec![
        field("value", "Box"),
        field("value", "Box::Inner"),
        ("value".into(), None),
        ("value".into(), None),
    ];
    expected.sort();
    assert_eq!(targets(source)?, expected);
    Ok(())
}
#[test]
fn typed_cast_array_and_generic_bound_receivers_retain_nominal_fields() -> Result<(), Box<dyn Error>>
{
    let source = "package p; class Cell { int value; } class Box { Cell[] items; int run(Object unknown, Cell item) { return items[0].value + ((Cell) unknown).value + item.value; } <T extends Cell> int generic(T item) { return item.value; } }";
    let mut expected = vec![field("items", "Box")];
    expected.extend(vec![field("value", "Cell"); 4]);
    expected.sort();
    assert_eq!(targets(source)?, expected);
    Ok(())
}
#[test]
fn ambiguous_imports_and_unknown_receivers_do_not_invent_field_targets()
-> Result<(), Box<dyn Error>> {
    for source in [
        "package p; import a.Cell; import b.Cell; class Box { int run(Cell item) { return item.value; } }",
        "package p; class Box { int run() { return unknown().value; } }",
    ] {
        assert_eq!(targets(source)?, vec![("value".into(), None)], "{source}");
    }
    Ok(())
}

#[test]
fn simple_pattern_branches_and_abrupt_guards_keep_flow_scope() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { Cell item; int positive(Object obj) { if (obj instanceof Cell item) { return item.value; } return item.value; } int negative(Object obj) { if (!(obj instanceof Cell item)) return 0; return item.value; } }";
    let mut expected = vec![field("item", "Box")];
    expected.extend(vec![field("value", "Cell"); 3]);
    expected.sort();
    assert_eq!(targets(source)?, expected);
    Ok(())
}
#[test]
fn complex_pattern_flow_blocks_outer_field_fallback() -> Result<(), Box<dyn Error>> {
    for source in [
        "package p; class Cell { int value; } class Box { Cell item; int run(Object obj) { if (!(obj instanceof Cell item)) { log(); return 0; } return item.value; } }",
        "package p; class Cell { int value; } class Box { Cell item; int run(Object obj) { if (obj instanceof Cell item && item.value > 0) return 1; return 0; } }",
    ] {
        assert_eq!(targets(source)?, vec![("value".into(), None)]);
    }
    Ok(())
}
#[test]
fn static_nested_instances_are_distinct_from_static_callable_contexts() -> Result<(), Box<dyn Error>>
{
    let source = "package p; class Box { int value; static class Inner { int value; int own() { return this.value + value; } static int invalid() { return value; } } }";
    assert_eq!(targets(source)?, vec![field("value", "Box::Inner"); 2]);
    Ok(())
}
#[test]
fn unregistered_local_callable_does_not_attribute_typed_access_to_outer_method()
-> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { void run() { class Local { int get(Cell item) { return item.value; } } } }";
    assert_eq!(targets(source)?, vec![("value".into(), None)]);
    Ok(())
}
#[test]
fn inherited_members_never_fall_back_to_an_enclosing_field() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Box { int value; class Inner extends Unknown { int get() { return value; } } }";
    assert!(targets(source)?.is_empty());
    Ok(())
}
#[test]
fn receiver_depth_exhaustion_is_unresolved() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "package p; class Cell {{ int value; }} class Box {{ int get(Cell item) {{ return {}item{}.value; }} }}",
        "(".repeat(30),
        ")".repeat(30)
    );
    assert_eq!(targets(&source)?, vec![("value".into(), None)]);
    Ok(())
}

#[test]
fn local_type_declaration_shadows_a_same_named_top_level_type() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { int run() { class Cell { int value; } Cell item = new Cell(); return item.value; } }";
    assert_eq!(targets(source)?, vec![("value".into(), None)]);
    Ok(())
}
#[test]
fn multi_catch_and_intersection_bounds_remain_unknown() -> Result<(), Box<dyn Error>> {
    for source in [
        "package p; class Box { int run() { try {} catch (First | Second item) { return item.value; } } }",
        "package p; class Box { <T extends First & Second> int run(T item) { return item.value; } }",
    ] {
        assert_eq!(targets(source)?, vec![("value".into(), None)]);
    }
    Ok(())
}

#[test]
fn enhanced_loop_binding_starts_after_the_iterable_expression() -> Result<(), Box<dyn Error>> {
    let source = "package p; // λ\nclass Cell { int value; } class Box { Cell[] items; void run() { for (Cell items : items) { sink(items.value); } } }";
    assert_eq!(
        targets(source)?,
        vec![field("items", "Box"), field("value", "Cell")]
    );
    Ok(())
}
#[test]
fn declared_field_types_keep_their_declaration_scope() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { Cell item; class Inner { class Cell { int value; } int run() { return item.value; } } }";
    assert_eq!(
        targets(source)?,
        vec![field("item", "Box"), field("value", "Cell")]
    );
    Ok(())
}

#[test]
fn unregistered_class_initializer_never_uses_enclosing_callable_ownership()
-> Result<(), Box<dyn Error>> {
    let source = "package p; class Cell { int value; } class Box { void run() { class Local { int value = new Cell().value; } } }";
    assert_eq!(targets(source)?, vec![("value".into(), None)]);
    Ok(())
}

#[test]
fn member_type_shadows_same_named_import_for_field_receivers() -> Result<(), Box<dyn Error>> {
    let source = "package p; import remote.Cell; class Box { class Cell { int value; } int run(Cell item) { return item.value; } }";
    assert_eq!(targets(source)?, vec![field("value", "Box::Cell")]);
    Ok(())
}
#[test]
fn local_type_prefix_shadows_a_package_in_qualified_type_syntax() -> Result<(), Box<dyn Error>> {
    let source = "package p; class Box { class remote { class Cell { int value; } } int run(remote.Cell item) { return item.value; } }";
    assert_eq!(targets(source)?, vec![field("value", "Box::remote::Cell")]);
    Ok(())
}

#[test]
fn shadowed_package_prefix_cannot_supply_missing_nested_types() -> Result<(), Box<dyn Error>> {
    for source in [
        "package p; class Box { class remote {} int run(remote.Cell item) { return item.value; } }",
        "package p; class Box<remote> { int run(remote.Cell item) { return item.value; } }",
    ] {
        assert_eq!(targets(source)?, vec![("value".into(), None)]);
    }
    Ok(())
}
