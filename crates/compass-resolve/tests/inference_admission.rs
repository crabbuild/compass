use std::collections::HashMap;

use compass_languages::{
    AdapterIdentity, CandidateRelation, DeclarationFact, EvidenceRange, Extraction,
    LanguageCapability, OccurrenceFact, RelationshipCandidate, ResolutionConstraint, ScopeFact,
    SemanticEvidenceBatch, SemanticRole, UNIVERSAL_EVIDENCE_SCHEMA, UniversalAdapterProfile,
};
use compass_resolve::{ResolutionAdmission, resolve_prevalidated_owned_with_root_at_inference};

fn range(start: u64, end: u64) -> EvidenceRange {
    EvidenceRange {
        source_file: "src/lib.py".to_owned(),
        start_byte: start,
        end_byte: end,
        start_line: 1,
        start_column: u32::try_from(start).unwrap_or_default(),
        end_line: 1,
        end_column: u32::try_from(end).unwrap_or_default(),
    }
}

fn external_call_batch() -> SemanticEvidenceBatch {
    SemanticEvidenceBatch {
        adapter: AdapterIdentity {
            id: "compass.python.inference-admission-test".to_owned(),
            language: "python".to_owned(),
            dialect: None,
            version: 1,
            evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA.to_owned(),
            profile: UniversalAdapterProfile::UniversalCandidate,
            producer: "inference-admission-test".to_owned(),
            capabilities: vec![
                LanguageCapability::Declarations,
                LanguageCapability::LexicalScopes,
                LanguageCapability::Calls,
                LanguageCapability::ExternalReferences,
            ],
        },
        declarations: vec![DeclarationFact {
            id: "decl:caller".to_owned(),
            language: "python".to_owned(),
            graph_node_id: "node:caller".to_owned(),
            kind: "function".to_owned(),
            name: "caller".to_owned(),
            qualified_name: "lib.caller".to_owned(),
            namespace: None,
            module_or_package: Some("lib".to_owned()),
            scope_id: Some("scope:caller".to_owned()),
            signature: None,
            parameter_count: None,
            parameter_types: Vec::new(),
            direct_bases_complete: false,
            variadic: false,
            signature_hash: None,
            implementation_hash: None,
            source_hash: None,
            definition_start_byte: None,
            range: range(0, 10),
        }],
        scopes: vec![ScopeFact {
            id: "scope:caller".to_owned(),
            language: "python".to_owned(),
            kind: "function".to_owned(),
            owner_declaration_id: Some("decl:caller".to_owned()),
            parent_scope_id: None,
            range: range(0, 100),
        }],
        bindings: Vec::new(),
        occurrences: vec![OccurrenceFact {
            id: "occurrence:external".to_owned(),
            language: "python".to_owned(),
            role: SemanticRole::Call,
            owner_declaration_id: "decl:caller".to_owned(),
            spelling: "execute".to_owned(),
            qualifier: Some("vendor.Service".to_owned()),
            context: Some("call".to_owned()),
            scope_id: Some("scope:caller".to_owned()),
            range: range(20, 27),
        }],
        candidates: vec![RelationshipCandidate {
            id: "candidate:external".to_owned(),
            language: "python".to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: "decl:caller".to_owned(),
            occurrence_id: Some("occurrence:external".to_owned()),
            binding_id: None,
            target_spelling: "execute".to_owned(),
            constraints: ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some("python".to_owned()),
                module_or_package: None,
                scope_id: Some("scope:caller".to_owned()),
                qualified_name: Some("vendor.Service.execute".to_owned()),
                argument_count: Some(0),
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["function".to_owned(), "method".to_owned()],
                hierarchy: None,
                allow_external: true,
            },
        }],
        diagnostics: Vec::new(),
    }
}

#[test]
fn low_admission_never_materializes_qualified_external_inference() {
    let extraction = Extraction {
        semantic_evidence: Some(external_call_batch()),
        ..Extraction::default()
    };
    let sources = HashMap::from([("src/lib.py".to_owned(), String::new())]);

    let low = resolve_prevalidated_owned_with_root_at_inference(
        vec![extraction.clone()],
        &sources,
        std::path::Path::new("."),
        ResolutionAdmission::Low,
    );
    let max = resolve_prevalidated_owned_with_root_at_inference(
        vec![extraction],
        &sources,
        std::path::Path::new("."),
        ResolutionAdmission::Max,
    );

    assert!(low.nodes.iter().all(|node| {
        node.attributes
            .get("placeholder")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    }));
    assert!(low.edges.iter().all(|edge| {
        edge.attributes
            .get("confidence")
            .and_then(serde_json::Value::as_str)
            != Some("INFERRED")
    }));
    assert!(max.nodes.iter().any(|node| {
        node.attributes
            .get("placeholder")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }));
    assert!(max.edges.iter().any(|edge| {
        edge.attributes
            .get("resolution_rule")
            .and_then(serde_json::Value::as_str)
            == Some("qualified-external")
    }));
}
