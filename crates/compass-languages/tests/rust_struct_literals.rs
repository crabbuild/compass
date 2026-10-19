use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, SemanticRole, UniversalEvidenceQualification,
    validate_evidence,
};

type Literal = (String, String, Option<String>);

fn literals(source: &str) -> Result<Vec<Literal>, Box<dyn Error>> {
    let evidence = Engine::default()
        .extract_source(Path::new("src/lib.rs"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Rust evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    assert_eq!(evidence.pipeline.version, 4);
    assert_eq!(
        evidence.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    let mut result = Vec::new();
    for occurrence in evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.context.as_deref() == Some("rust-struct-literal"))
    {
        assert_eq!(occurrence.role, SemanticRole::TypeReference);
        let candidates = evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 1);
        let candidate = candidates[0];
        assert_eq!(candidate.relation, CandidateRelation::References);
        assert!(candidate.constraints.module_or_package.is_none());
        assert!(candidate.constraints.scope_id.is_none());
        assert_eq!(
            candidate.constraints.allowed_target_kinds,
            ["enum_member", "struct", "type_alias"]
        );
        assert_eq!(
            candidate.source_declaration_id,
            occurrence.owner_declaration_id
        );
        let owner = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == occurrence.owner_declaration_id)
            .ok_or("missing literal owner")?;
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        result.push((
            owner.name.clone(),
            source
                .get(start..end)
                .ok_or("invalid literal range")?
                .to_owned(),
            candidate.constraints.qualified_name.clone(),
        ));
    }
    result.sort();
    Ok(result)
}

#[test]
fn direct_nested_and_self_literals_keep_their_exact_owners() -> Result<(), Box<dyn Error>> {
    let source = "// λ\nstruct Item { value: usize } impl Item { fn make() { let _ = Self { value: 1 }; } } fn outer() { let _ = Item { value: 2 }; fn inner() { let _ = Item { value: 3 }; } }";
    assert_eq!(
        literals(source)?,
        [
            ("inner".into(), "Item".into(), Some("crate::Item".into())),
            ("make".into(), "Self".into(), Some("crate::Item".into())),
            ("outer".into(), "Item".into(), Some("crate::Item".into())),
        ]
    );
    Ok(())
}

#[test]
fn generic_literals_anchor_the_nominal_path_not_type_arguments() -> Result<(), Box<dyn Error>> {
    let source = "struct Item<T> { value: T } fn make() { let _ = Item::<u8> { value: 1 }; }";
    assert_eq!(
        literals(source)?,
        [("make".into(), "Item".into(), Some("crate::Item".into()))]
    );
    Ok(())
}

#[test]
fn concrete_self_variant_keeps_the_enum_member_path() -> Result<(), Box<dyn Error>> {
    let source = "enum Event { Ready { value: usize } } impl Event { fn make() { let _ = Self::Ready { value: 1 }; } }";
    assert_eq!(
        literals(source)?,
        [(
            "make".into(),
            "Self::Ready".into(),
            Some("crate::Event::Ready".into())
        )]
    );
    Ok(())
}

#[test]
fn strings_patterns_macro_tokens_and_parser_recovery_do_not_emit_literal_contacts()
-> Result<(), Box<dyn Error>> {
    for source in [
        "struct Item { value: usize } fn make() { let _ = \"Item { value: 1 }\"; }",
        "struct Item { value: usize } fn make(item: Item) { let Item { value } = item; }",
        "struct Item { value: usize } fn make() { quote!(Item { value: 1 }); }",
        "struct Item { value: usize } fn make() { let _ = Item { value: }; }",
        "trait Make { fn make() { let _ = Self { value: 1 }; } }",
    ] {
        assert!(literals(source)?.is_empty(), "{source}");
    }
    Ok(())
}

#[test]
fn recovery_in_one_literal_does_not_erase_a_separate_valid_literal() -> Result<(), Box<dyn Error>> {
    for source in [
        "struct Item { value: usize } fn make() { let _ = Item { value: 1 }; let _ = Item { value: }; }",
        "struct Item { value: usize } fn make() { let _ = Item { value: }; let _ = Item { value: 1 }; }",
        "struct Item { value: usize } fn make() { let _ = Item { value: 1 };\nlet _ = Item { value: }; }",
        "struct Item { value: usize } fn make() { let _ = Item { value: };\nlet _ = Item { value: 1 }; }",
    ] {
        assert_eq!(
            literals(source)?,
            [("make".into(), "Item".into(), Some("crate::Item".into()))]
        );
    }
    Ok(())
}

#[test]
fn conflicting_imports_keep_an_unresolved_occurrence_and_diagnostic() -> Result<(), Box<dyn Error>>
{
    let source = "use crate::a::Item; use crate::b::Item; fn make() { let _ = Item { value: 1 }; }";
    let evidence = Engine::default()
        .extract_source(Path::new("src/lib.rs"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    let occurrence = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.context.as_deref() == Some("rust-struct-literal"))
        .ok_or("missing unresolved literal occurrence")?;
    assert_eq!(occurrence.role, SemanticRole::TypeReference);
    assert!(
        !evidence
            .candidates
            .iter()
            .any(|candidate| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
    );
    assert!(evidence.diagnostics.iter().any(|diagnostic| diagnostic.code
        == "ambiguous-struct-literal-type"
        && diagnostic.fact_id.as_ref() == Some(&occurrence.id)
        && diagnostic.range.as_ref() == Some(&occurrence.range)));
    Ok(())
}
