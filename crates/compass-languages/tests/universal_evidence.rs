use compass_languages::{
    AdapterIdentity, AdapterRegistry, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    EvidenceErrorCode, EvidenceLimits, EvidenceRange, Extraction, LanguageCapability,
    OccurrenceFact, RelationshipCandidate, ResolutionConstraint, ScopeFact, SemanticEvidenceBatch,
    SemanticRole, validate_evidence,
};

fn range(start: u64, end: u64) -> EvidenceRange {
    EvidenceRange {
        source_file: "src/example.py".to_owned(),
        start_byte: start,
        end_byte: end,
        start_line: 1,
        start_column: u32::try_from(start).expect("small fixture offset"),
        end_line: 1,
        end_column: u32::try_from(end).expect("small fixture offset"),
    }
}

fn valid_batch() -> SemanticEvidenceBatch {
    SemanticEvidenceBatch {
        adapter: AdapterIdentity {
            language: "python".to_owned(),
            producer: "tree-sitter-python".to_owned(),
            capabilities: vec![
                LanguageCapability::Declarations,
                LanguageCapability::LexicalScopes,
                LanguageCapability::Imports,
                LanguageCapability::Aliases,
                LanguageCapability::Calls,
                LanguageCapability::ExternalReferences,
            ],
        },
        declarations: vec![DeclarationFact {
            id: "decl:caller".to_owned(),
            language: "python".to_owned(),
            kind: "function".to_owned(),
            name: "caller".to_owned(),
            qualified_name: "example.caller".to_owned(),
            module_or_package: Some("example".to_owned()),
            scope_id: None,
            range: range(0, 6),
        }],
        scopes: vec![ScopeFact {
            id: "scope:caller".to_owned(),
            language: "python".to_owned(),
            kind: "function".to_owned(),
            owner_declaration_id: Some("decl:caller".to_owned()),
            parent_scope_id: None,
            range: range(0, 20),
        }],
        bindings: vec![BindingFact {
            id: "binding:helper".to_owned(),
            language: "python".to_owned(),
            kind: BindingKind::ImportAlias,
            spelling: "helper".to_owned(),
            qualified_target: "tools.execute".to_owned(),
            target_declaration_id: None,
            scope_id: Some("scope:caller".to_owned()),
            range: range(7, 13),
        }],
        occurrences: vec![OccurrenceFact {
            id: "occurrence:helper".to_owned(),
            language: "python".to_owned(),
            role: SemanticRole::Call,
            owner_declaration_id: "decl:caller".to_owned(),
            spelling: "helper".to_owned(),
            qualifier: None,
            scope_id: Some("scope:caller".to_owned()),
            range: range(14, 20),
        }],
        candidates: vec![RelationshipCandidate {
            id: "candidate:helper".to_owned(),
            language: "python".to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: "decl:caller".to_owned(),
            occurrence_id: Some("occurrence:helper".to_owned()),
            binding_id: Some("binding:helper".to_owned()),
            target_spelling: "helper".to_owned(),
            constraints: ResolutionConstraint {
                exact_language: Some("python".to_owned()),
                module_or_package: Some("tools".to_owned()),
                scope_id: Some("scope:caller".to_owned()),
                qualified_name: Some("tools.execute".to_owned()),
                allowed_target_kinds: vec!["function".to_owned()],
                allow_external: true,
            },
        }],
        diagnostics: Vec::new(),
    }
}

fn assert_code(batch: &SemanticEvidenceBatch, code: EvidenceErrorCode) {
    let error = validate_evidence(batch, EvidenceLimits::default())
        .expect_err("fixture must fail validation");
    assert_eq!(error.code, code, "{error}");
}

#[test]
fn evidence_round_trips_with_closed_camel_case_schema() {
    let batch = valid_batch();
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid fixture");

    let encoded = serde_json::to_value(&batch).expect("serialize evidence");
    assert_eq!(encoded["adapter"]["language"], "python");
    assert_eq!(
        encoded["occurrences"][0]["ownerDeclarationId"],
        "decl:caller"
    );
    assert_eq!(encoded["occurrences"][0]["role"], "call");
    assert_eq!(
        serde_json::from_value::<SemanticEvidenceBatch>(encoded).expect("deserialize evidence"),
        batch
    );
}

#[test]
fn unknown_fields_are_rejected_at_nested_boundaries() {
    let mut encoded = serde_json::to_value(valid_batch()).expect("serialize evidence");
    encoded["adapter"]["compatibilityMode"] = serde_json::json!(true);
    let error =
        serde_json::from_value::<SemanticEvidenceBatch>(encoded).expect_err("unknown field");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn extraction_omits_absent_evidence_and_round_trips_present_evidence() {
    let mut extraction = Extraction::default();
    let absent = serde_json::to_value(&extraction).expect("serialize extraction");
    assert!(absent.get("semantic_evidence").is_none());

    extraction.semantic_evidence = Some(valid_batch());
    let encoded = serde_json::to_value(&extraction).expect("serialize extraction with evidence");
    assert!(encoded.get("semantic_evidence").is_some());
    let decoded: Extraction = serde_json::from_value(encoded).expect("deserialize extraction");
    assert_eq!(decoded.semantic_evidence, extraction.semantic_evidence);
}

#[test]
fn invalid_paths_and_ranges_are_rejected() {
    let mut unsafe_path = valid_batch();
    unsafe_path.declarations[0].range.source_file = "../example.py".to_owned();
    assert_code(&unsafe_path, EvidenceErrorCode::InvalidPath);

    let mut empty_range = valid_batch();
    empty_range.declarations[0].range.end_byte = 0;
    assert_code(&empty_range, EvidenceErrorCode::InvalidRange);
}

#[test]
fn duplicate_ids_and_all_dangling_reference_kinds_are_rejected() {
    let mut duplicate = valid_batch();
    duplicate.bindings[0].id = "scope:caller".to_owned();
    assert_code(&duplicate, EvidenceErrorCode::DuplicateId);

    let mut missing_scope = valid_batch();
    missing_scope.bindings[0].scope_id = Some("scope:missing".to_owned());
    assert_code(&missing_scope, EvidenceErrorCode::MissingReference);

    let mut missing_declaration = valid_batch();
    missing_declaration.occurrences[0].owner_declaration_id = "decl:missing".to_owned();
    assert_code(&missing_declaration, EvidenceErrorCode::MissingReference);

    let mut missing_binding = valid_batch();
    missing_binding.candidates[0].binding_id = Some("binding:missing".to_owned());
    assert_code(&missing_binding, EvidenceErrorCode::MissingReference);

    let mut missing_occurrence = valid_batch();
    missing_occurrence.candidates[0].occurrence_id = Some("occurrence:missing".to_owned());
    assert_code(&missing_occurrence, EvidenceErrorCode::MissingReference);
}

#[test]
fn behavioral_candidates_require_occurrences() {
    let mut batch = valid_batch();
    batch.candidates[0].occurrence_id = None;
    assert_code(&batch, EvidenceErrorCode::MissingOccurrence);
}

#[test]
fn capabilities_and_language_constraints_fail_closed() {
    let mut undeclared = valid_batch();
    undeclared
        .adapter
        .capabilities
        .retain(|capability| *capability != LanguageCapability::Calls);
    assert_code(&undeclared, EvidenceErrorCode::UndeclaredCapability);

    let mut cross_language = valid_batch();
    cross_language.candidates[0].constraints.exact_language = Some("ruby".to_owned());
    assert_code(&cross_language, EvidenceErrorCode::LanguageMismatch);

    let mut undeclared_external = valid_batch();
    undeclared_external
        .adapter
        .capabilities
        .retain(|capability| *capability != LanguageCapability::ExternalReferences);
    assert_code(
        &undeclared_external,
        EvidenceErrorCode::UndeclaredCapability,
    );
}

#[test]
fn every_resource_boundary_is_enforced() {
    let batch = valid_batch();
    let limits = EvidenceLimits {
        declarations: 0,
        ..EvidenceLimits::default()
    };
    let error = validate_evidence(&batch, limits).expect_err("declaration limit");
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);

    let limits = EvidenceLimits {
        allowed_target_kinds_per_candidate: 0,
        ..EvidenceLimits::default()
    };
    let error = validate_evidence(&batch, limits).expect_err("target-kind limit");
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);
}

#[test]
fn first_fact_error_is_independent_of_input_order() {
    let mut first = valid_batch();
    let mut later = first.declarations[0].clone();
    later.id = "z-invalid".to_owned();
    later.range.end_byte = later.range.start_byte;
    let mut earlier = first.declarations[0].clone();
    earlier.id = "a-invalid".to_owned();
    earlier.range.end_byte = earlier.range.start_byte;
    first.declarations.extend([later.clone(), earlier.clone()]);

    let mut second = valid_batch();
    second.declarations.extend([earlier, later]);
    second.declarations.reverse();

    let first_error =
        validate_evidence(&first, EvidenceLimits::default()).expect_err("invalid ranges");
    let second_error =
        validate_evidence(&second, EvidenceLimits::default()).expect_err("invalid ranges");
    assert_eq!(first_error, second_error);
    assert!(first_error.message.contains("a-invalid"));
}

#[test]
fn universal_adapter_profiles_are_unique_sorted_and_truthful() {
    AdapterRegistry::validate().expect("production registry must be valid");
    let profiles = AdapterRegistry::universal_profiles();
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.language)
            .collect::<Vec<_>>(),
        ["go", "python"]
    );
    assert!(
        profiles
            .iter()
            .all(|profile| !profile.capabilities.is_empty())
    );
    assert!(profiles.iter().all(|profile| {
        profile
            .capabilities
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    }));
    assert!(AdapterRegistry::universal_profile("java").is_none());
    assert!(AdapterRegistry::universal_profile("rust").is_none());
}
