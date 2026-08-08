use std::error::Error;

use compass_languages::{
    AdapterIdentity, BindingFact, BindingKind, CandidateRelation, DeclarationFact, EvidenceRange,
    LanguageCapability, OccurrenceFact, RelationshipCandidate, ResolutionConstraint, ScopeFact,
    SemanticEvidenceBatch, SemanticRole, UNIVERSAL_EVIDENCE_SCHEMA, UniversalAdapterProfile,
};
use compass_resolve::evidence::{
    ResolutionDecision, ResolutionRule, UniversalResolutionIndex, UniversalResolutionLimits,
};

const LANGUAGE: &str = "python";
const CALLER_ID: &str = "decl:caller";
const CALLER_SCOPE_ID: &str = "scope:caller";
const CANDIDATE_ID: &str = "candidate:target";

fn range(start: u64, end: u64) -> EvidenceRange {
    EvidenceRange {
        source_file: "src/example.py".to_owned(),
        start_byte: start,
        end_byte: end,
        start_line: 1,
        start_column: u32::try_from(start).unwrap_or_default(),
        end_line: 1,
        end_column: u32::try_from(end).unwrap_or_default(),
    }
}

fn declaration(
    id: &str,
    graph_node_id: &str,
    name: &str,
    qualified_name: &str,
    scope_id: Option<&str>,
    start: u64,
) -> DeclarationFact {
    DeclarationFact {
        id: id.to_owned(),
        language: LANGUAGE.to_owned(),
        graph_node_id: graph_node_id.to_owned(),
        kind: "function".to_owned(),
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        namespace: None,
        module_or_package: Some("example".to_owned()),
        scope_id: scope_id.map(str::to_owned),
        signature: None,
        parameter_count: None,
        parameter_types: Vec::new(),
        direct_bases_complete: false,
        variadic: false,
        signature_hash: None,
        implementation_hash: None,
        source_hash: None,
        definition_start_byte: None,
        range: range(start, start.saturating_add(6)),
    }
}

fn binding(id: &str, target: &str, target_declaration_id: Option<&str>) -> BindingFact {
    BindingFact {
        id: id.to_owned(),
        language: LANGUAGE.to_owned(),
        kind: BindingKind::ImportAlias,
        spelling: "target".to_owned(),
        qualified_target: target.to_owned(),
        namespace: None,
        type_only: false,
        target_declaration_id: target_declaration_id.map(str::to_owned),
        scope_id: Some(CALLER_SCOPE_ID.to_owned()),
        output_index: None,
        result_type_qualified_name: None,
        receiver_binding_id: None,
        fallback_binding_id: None,
        range: range(60, 66),
    }
}

fn batch() -> SemanticEvidenceBatch {
    SemanticEvidenceBatch {
        adapter: AdapterIdentity {
            id: "compass.python.contract-test".to_owned(),
            language: LANGUAGE.to_owned(),
            dialect: None,
            version: 1,
            evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA.to_owned(),
            profile: UniversalAdapterProfile::UniversalCandidate,
            producer: "contract-test".to_owned(),
            capabilities: vec![
                LanguageCapability::Declarations,
                LanguageCapability::LexicalScopes,
                LanguageCapability::Imports,
                LanguageCapability::Aliases,
                LanguageCapability::Calls,
                LanguageCapability::ExternalReferences,
            ],
        },
        declarations: vec![declaration(
            CALLER_ID,
            "node:caller",
            "caller",
            "example.caller",
            None,
            0,
        )],
        scopes: vec![ScopeFact {
            id: CALLER_SCOPE_ID.to_owned(),
            language: LANGUAGE.to_owned(),
            kind: "function".to_owned(),
            owner_declaration_id: Some(CALLER_ID.to_owned()),
            parent_scope_id: None,
            range: range(0, 100),
        }],
        bindings: Vec::new(),
        occurrences: vec![OccurrenceFact {
            id: "occurrence:target".to_owned(),
            language: LANGUAGE.to_owned(),
            role: SemanticRole::Call,
            owner_declaration_id: CALLER_ID.to_owned(),
            spelling: "target".to_owned(),
            qualifier: None,
            context: None,
            scope_id: Some(CALLER_SCOPE_ID.to_owned()),
            range: range(80, 86),
        }],
        candidates: vec![RelationshipCandidate {
            id: CANDIDATE_ID.to_owned(),
            language: LANGUAGE.to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: CALLER_ID.to_owned(),
            occurrence_id: Some("occurrence:target".to_owned()),
            binding_id: None,
            target_spelling: "target".to_owned(),
            constraints: ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some(LANGUAGE.to_owned()),
                module_or_package: None,
                scope_id: Some(CALLER_SCOPE_ID.to_owned()),
                qualified_name: None,
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["function".to_owned()],
                hierarchy: None,
                allow_external: false,
            },
        }],
        diagnostics: Vec::new(),
    }
}

fn index(
    batch: &SemanticEvidenceBatch,
    limits: UniversalResolutionLimits,
) -> Result<UniversalResolutionIndex, Box<dyn Error>> {
    Ok(UniversalResolutionIndex::new(
        std::slice::from_ref(batch),
        limits,
    )?)
}

fn assert_resolved(decision: ResolutionDecision, expected_id: &str, expected_rule: ResolutionRule) {
    assert_eq!(
        decision,
        ResolutionDecision::Resolved {
            declaration_id: expected_id.to_owned(),
            evidence: compass_resolve::evidence::ResolutionEvidence {
                rule: expected_rule,
                candidate_count: 1,
            },
        }
    );
}

#[test]
fn exact_source_declaration_outranks_an_explicit_binding() -> Result<(), Box<dyn Error>> {
    let mut evidence = batch();
    evidence.declarations.extend([
        declaration(
            "decl:exact",
            "node:exact",
            "target",
            "example.exact",
            None,
            20,
        ),
        declaration(
            "decl:bound",
            "node:bound",
            "target",
            "example.bound",
            None,
            30,
        ),
    ]);
    evidence.bindings.push(binding(
        "binding:target",
        "example.bound",
        Some("decl:bound"),
    ));
    evidence.candidates[0].binding_id = Some("binding:target".to_owned());
    evidence.candidates[0]
        .constraints
        .exact_target_declaration_id = Some("decl:exact".to_owned());

    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    assert_resolved(
        resolver.resolve(CANDIDATE_ID),
        "decl:exact",
        ResolutionRule::ExactSourceDeclaration,
    );
    Ok(())
}

#[test]
fn explicit_binding_outranks_a_same_spelled_lexical_declaration() -> Result<(), Box<dyn Error>> {
    let mut evidence = batch();
    evidence.declarations.extend([
        declaration(
            "decl:bound",
            "node:bound",
            "target",
            "example.bound",
            None,
            20,
        ),
        declaration(
            "decl:lexical",
            "node:lexical",
            "target",
            "example.caller.target",
            Some(CALLER_SCOPE_ID),
            30,
        ),
    ]);
    evidence.bindings.push(binding(
        "binding:target",
        "example.bound",
        Some("decl:bound"),
    ));
    evidence.candidates[0].binding_id = Some("binding:target".to_owned());

    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    assert_resolved(
        resolver.resolve(CANDIDATE_ID),
        "decl:bound",
        ResolutionRule::ExplicitBinding,
    );
    Ok(())
}

#[test]
fn one_lexical_declaration_resolves_exactly() -> Result<(), Box<dyn Error>> {
    let mut evidence = batch();
    evidence.declarations.push(declaration(
        "decl:lexical",
        "node:lexical",
        "target",
        "example.caller.target",
        Some(CALLER_SCOPE_ID),
        20,
    ));

    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    assert_resolved(
        resolver.resolve(CANDIDATE_ID),
        "decl:lexical",
        ResolutionRule::ExactLexicalDeclaration,
    );
    Ok(())
}

#[test]
fn competing_lexical_declarations_remain_ambiguous_at_the_lookup_limit()
-> Result<(), Box<dyn Error>> {
    let mut evidence = batch();
    evidence.declarations.extend([
        declaration(
            "decl:first",
            "node:first",
            "target",
            "example.caller.target.first",
            Some(CALLER_SCOPE_ID),
            20,
        ),
        declaration(
            "decl:second",
            "node:second",
            "target",
            "example.caller.target.second",
            Some(CALLER_SCOPE_ID),
            30,
        ),
    ]);

    let resolver = index(
        &evidence,
        UniversalResolutionLimits {
            candidates_per_lookup: 1,
            ..UniversalResolutionLimits::default()
        },
    )?;
    assert_eq!(
        resolver.resolve(CANDIDATE_ID),
        ResolutionDecision::Ambiguous { candidate_count: 2 }
    );
    Ok(())
}

#[test]
fn qualified_external_requires_explicit_permission() -> Result<(), Box<dyn Error>> {
    let mut evidence = batch();
    evidence.candidates[0].constraints.scope_id = None;
    evidence.candidates[0].constraints.qualified_name = Some("vendor.target".to_owned());
    evidence.candidates[0].constraints.allow_external = true;

    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    assert_eq!(
        resolver.resolve(CANDIDATE_ID),
        ResolutionDecision::QualifiedExternal {
            qualified_name: "vendor.target".to_owned(),
            evidence: compass_resolve::evidence::ResolutionEvidence {
                rule: ResolutionRule::QualifiedExternal,
                candidate_count: 0,
            },
        }
    );

    evidence.candidates[0].constraints.allow_external = false;
    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    assert_eq!(
        resolver.resolve(CANDIDATE_ID),
        ResolutionDecision::Unresolved
    );
    Ok(())
}

#[test]
fn declaration_input_order_does_not_change_the_decision() -> Result<(), Box<dyn Error>> {
    let mut first = batch();
    first.declarations.push(declaration(
        "decl:lexical",
        "node:lexical",
        "target",
        "example.caller.target",
        Some(CALLER_SCOPE_ID),
        20,
    ));
    let mut second = first.clone();
    second.declarations.reverse();

    let first = index(&first, UniversalResolutionLimits::default())?;
    let second = index(&second, UniversalResolutionLimits::default())?;
    assert_eq!(first.resolve(CANDIDATE_ID), second.resolve(CANDIDATE_ID));
    assert_eq!(first.candidate_ids(), second.candidate_ids());
    Ok(())
}

#[test]
fn materialization_preserves_the_decision_rule_and_occurrence_anchor() -> Result<(), Box<dyn Error>>
{
    let mut evidence = batch();
    evidence.declarations.push(declaration(
        "decl:exact",
        "node:exact",
        "target",
        "example.target",
        None,
        20,
    ));
    evidence.candidates[0]
        .constraints
        .exact_target_declaration_id = Some("decl:exact".to_owned());

    let resolver = index(&evidence, UniversalResolutionLimits::default())?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    resolver.materialize(&mut nodes, &mut edges);

    let edge = edges
        .iter()
        .find(|edge| edge.source == "node:caller" && edge.target == "node:exact")
        .ok_or("missing materialized exact-source edge")?;
    assert_eq!(edge.string("relation"), "calls");
    assert_eq!(edge.string("resolution_rule"), "exact-source-declaration");
    assert_eq!(
        edge.attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64),
        Some(80)
    );
    assert_eq!(
        edge.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64),
        Some(86)
    );
    Ok(())
}
