use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};
use compass_resolve::resolve;

#[test]
fn java_zero_argument_varargs_does_not_treat_missing_parameter_types_as_empty_parameters()
-> Result<(), Box<dyn Error>> {
    let source =
        include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/ExtendedVarargs.java");
    let mut extraction =
        Engine::default().extract_source(Path::new("ExtendedVarargs.java"), source.as_bytes())?;
    let evidence = extraction
        .semantic_evidence
        .as_mut()
        .ok_or("missing evidence")?;
    let unknown = evidence
        .declarations
        .iter_mut()
        .find(|d| d.signature.as_deref() == Some("choose(Object...)"))
        .ok_or("missing overload")?;
    unknown.parameter_types.clear();
    // Canonical parameter types are optional evidence. An absent vector does
    // not turn this one-parameter declaration into a zero-parameter overload.
    validate_evidence(evidence, EvidenceLimits::default())?;
    let sources = HashMap::from([("ExtendedVarargs.java".to_owned(), source.to_owned())]);
    let resolved = resolve(&[extraction], &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let caller = resolved
        .nodes
        .iter()
        .find(|n| n.string("qualified_name") == "audit.ExtendedVarargs::mostSpecificEmpty")
        .ok_or("missing caller")?;
    assert!(
        !resolved
            .edges
            .iter()
            .any(|e| e.source == caller.id && e.string("rule").starts_with("universal-call-")),
        "missing canonical types must retain ambiguity for zero-argument varargs"
    );
    Ok(())
}

#[test]
fn java_varargs_keeps_invocation_phases_occurrences_and_unknown_targets()
-> Result<(), Box<dyn Error>> {
    let source =
        include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/ExtendedVarargs.java");
    let inputs = [
        ("ExtendedVarargs.java", source),
        (
            "VarargsDemo.java",
            include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/VarargsDemo.java"),
        ),
    ];
    let sources = inputs
        .into_iter()
        .map(|(p, s)| (p.to_owned(), s.to_owned()))
        .collect::<HashMap<_, _>>();
    let mut engine = Engine::default();
    let mut extractions = inputs
        .into_iter()
        .map(|(p, s)| engine.extract_source(Path::new(p), s.as_bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = extractions[0]
        .semantic_evidence
        .as_ref()
        .ok_or("missing evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let unknown = evidence
        .declarations
        .iter()
        .find(|d| d.name == "unknown")
        .ok_or("missing unknown caller")?;
    let unknown_call = evidence
        .candidates
        .iter()
        .filter(|c| {
            c.source_declaration_id == unknown.id
                && c.relation == CandidateRelation::Calls
                && c.target_spelling == "choose"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unknown_call.len(),
        1,
        "retain the unresolved source occurrence"
    );
    assert_eq!(unknown_call[0].constraints.argument_types, [None]);
    let repeated = evidence
        .declarations
        .iter()
        .find(|d| d.name == "repeated")
        .ok_or("missing repeated caller")?;
    let repeated_calls = evidence
        .candidates
        .iter()
        .filter(|c| {
            c.source_declaration_id == repeated.id
                && c.relation == CandidateRelation::Calls
                && c.target_spelling == "strings"
        })
        .collect::<Vec<_>>();
    assert_eq!(repeated_calls.len(), 2);
    assert_ne!(
        repeated_calls[0].occurrence_id,
        repeated_calls[1].occurrence_id
    );
    for candidate in &evidence.candidates {
        if candidate.relation != CandidateRelation::Calls {
            continue;
        }
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|o| Some(&o.id) == candidate.occurrence_id.as_ref())
            .ok_or("missing occurrence")?;
        assert_eq!(
            source.get(
                usize::try_from(occurrence.range.start_byte)?
                    ..usize::try_from(occurrence.range.end_byte)?
            ),
            Some(candidate.target_spelling.as_str())
        );
    }
    let resolved = resolve(&extractions, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let mut calls = BTreeMap::<String, Vec<String>>::new();
    for edge in &resolved.edges {
        if edge.string("relation") != "calls" || !edge.string("rule").starts_with("universal-call-")
        {
            continue;
        }
        let caller = resolved
            .nodes
            .iter()
            .find(|n| n.id == edge.source)
            .ok_or("missing caller")?;
        let target = resolved
            .nodes
            .iter()
            .find(|n| n.id == edge.target)
            .ok_or("missing target")?;
        let name = caller.string("qualified_name");
        if let Some(name) = name.strip_prefix("audit.ExtendedVarargs::")
            && target
                .string("qualified_name")
                .starts_with("audit.ExtendedVarargs::")
        {
            assert_eq!(edge.string("_origin"), "ast");
            assert_eq!(edge.string("confidence"), "EXTRACTED");
            calls
                .entry(name.to_owned())
                .or_default()
                .push(target.string("signature"));
        }
    }
    let expected = [
        ("empty", "strings(String...)"),
        ("mostSpecificEmpty", "choose(String...)"),
        ("mostSpecificMany", "choose(String...)"),
        ("nullArray", "choose(String...)"),
        ("fixedBeforeExpanded", "phase(Object)"),
        ("wideningBeforeBoxing", "widen(long)"),
        ("boxingBeforeExpanded", "loose(Integer)"),
        ("primitiveSpecific", "primitive(int...)"),
        ("prefixEmpty", "prefixed(String,int...)"),
        ("arrayArgument", "shape(String[])"),
        ("matrixArgument", "shape(String[][])"),
        ("matrixInitializer", "shape(String[][])"),
        ("spreadForward", "shape(String[])"),
        ("trailingDimensions", "shape(String[])"),
        ("receiver", "shape(String[])"),
        ("modified", "shape(String[])"),
        ("arrays", "shape(String[][])"),
    ]
    .into_iter()
    .map(|(name, signature)| (name.to_owned(), vec![signature.to_owned()]))
    .chain([(
        "repeated".to_owned(),
        vec!["strings(String...)".to_owned(); 2],
    )])
    .collect::<BTreeMap<_, _>>();
    // unknown's nested method-result argument has no proven type: retaining
    // ambiguity is correct even though javac knows the String return type.
    assert_eq!(calls, expected);
    extractions.reverse();
    let reversed = resolve(&extractions, &sources);
    assert!(reversed.error.is_none(), "{:?}", reversed.error);
    let canonical = |graph: &compass_languages::Extraction| {
        let mut edges = graph
            .edges
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?;
        edges.sort();
        Ok::<_, serde_json::Error>(edges)
    };
    assert_eq!(canonical(&resolved)?, canonical(&reversed)?);
    Ok(())
}
