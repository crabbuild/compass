use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};

const SOURCE: &str =
    include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/VarargsDemo.java");

#[test]
fn java_array_parameter_receivers_remain_unresolved_instead_of_naming_element_types()
-> Result<(), Box<dyn Error>> {
    let source = br#"package audit;
class Item { public Item clone() { return this; } }
class Use {
    void spread(Item... values) { values.clone(); }
    void fixed(Item[] values) { values.clone(); }
    void suffix(Item values[]) { values.clone(); }
}
"#;
    let extraction = Engine::default().extract_source(Path::new("Use.java"), source)?;
    let evidence = extraction.semantic_evidence.ok_or("missing evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    let calls = evidence
        .candidates
        .iter()
        .filter(|c| c.relation == CandidateRelation::Calls && c.target_spelling == "clone")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    for call in calls {
        assert!(call.constraints.qualified_name.is_none());
        assert!(!call.constraints.allow_external);
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|o| Some(&o.id) == call.occurrence_id.as_ref())
            .ok_or("missing clone occurrence")?;
        assert_eq!(occurrence.qualifier.as_deref(), Some("values"));
        assert_eq!(
            source.get(
                usize::try_from(occurrence.range.start_byte)?
                    ..usize::try_from(occurrence.range.end_byte)?
            ),
            Some(b"clone".as_slice())
        );
    }
    Ok(())
}

#[test]
fn java_spread_parameter_ast_and_signatures() -> Result<(), Box<dyn Error>> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_language_pack::get_language("java")?)?;
    let tree = parser
        .parse(SOURCE, None)
        .ok_or("missing Java syntax tree")?;
    assert!(!tree.root_node().has_error());
    assert!(
        tree.root_node().to_sexp().contains(
            "(spread_parameter (type_identifier) (variable_declarator name: (identifier)))"
        )
    );
    let extraction =
        Engine::default().extract_source(Path::new("VarargsDemo.java"), SOURCE.as_bytes())?;
    let evidence = extraction
        .semantic_evidence
        .ok_or("missing Java evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    for (line, signature, types, count, variadic) in [
        (4, "join(String...)", vec!["java.lang.String[]"], 1, true),
        (5, "join(int...)", vec!["int[]"], 1, true),
        (6, "join(boolean)", vec!["boolean"], 1, false),
        (
            7,
            "prefixed(String,String...)",
            vec!["java.lang.String", "java.lang.String[]"],
            2,
            true,
        ),
        (
            8,
            "arrays(String[],int[])",
            vec!["java.lang.String[]", "int[]"],
            2,
            false,
        ),
    ] {
        let declaration = evidence
            .declarations
            .iter()
            .find(|d| d.kind == "method" && d.range.start_line == line)
            .ok_or("missing method")?;
        assert_eq!(declaration.signature.as_deref(), Some(signature));
        assert_eq!(declaration.parameter_types, types);
        assert_eq!(declaration.parameter_count, Some(count));
        assert_eq!(declaration.variadic, variadic);
    }
    Ok(())
}

#[test]
fn java_varargs_preserves_dimensions_receivers_and_parameter_value_types()
-> Result<(), Box<dyn Error>> {
    let source =
        include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/ExtendedVarargs.java");
    let extraction =
        Engine::default().extract_source(Path::new("ExtendedVarargs.java"), source.as_bytes())?;
    let evidence = extraction.semantic_evidence.ok_or("missing evidence")?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    for (name, signature, parameter_type) in [
        (
            "spreadForward",
            "spreadForward(String...)",
            "java.lang.String[]",
        ),
        (
            "trailingDimensions",
            "trailingDimensions(String[])",
            "java.lang.String[]",
        ),
        ("receiver", "receiver(String...)", "java.lang.String[]"),
        ("modified", "modified(String...)", "java.lang.String[]"),
        ("arrays", "arrays(String[]...)", "java.lang.String[][]"),
        ("generic", "generic(java.util.List...)", "java.util.List[]"),
    ] {
        let declaration = evidence
            .declarations
            .iter()
            .find(|d| d.name == name)
            .ok_or("missing declaration")?;
        assert_eq!(declaration.signature.as_deref(), Some(signature));
        assert_eq!(declaration.parameter_types, [parameter_type]);
        assert_eq!(declaration.parameter_count, Some(1));
    }
    for (name, argument_type) in [
        ("arrayArgument", "java.lang.String[]"),
        ("matrixArgument", "java.lang.String[][]"),
        ("matrixInitializer", "java.lang.String[][]"),
        ("spreadForward", "java.lang.String[]"),
        ("trailingDimensions", "java.lang.String[]"),
        ("receiver", "java.lang.String[]"),
        ("modified", "java.lang.String[]"),
        ("arrays", "java.lang.String[][]"),
    ] {
        let declaration = evidence
            .declarations
            .iter()
            .find(|d| d.name == name)
            .ok_or("missing caller")?;
        let call = evidence
            .candidates
            .iter()
            .find(|c| c.source_declaration_id == declaration.id && c.target_spelling == "shape")
            .ok_or("missing shape call")?;
        assert_eq!(
            call.constraints.argument_types,
            [Some(argument_type.to_owned())],
            "{name}"
        );
    }
    Ok(())
}
