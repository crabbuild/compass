#![allow(clippy::expect_used)]

use compass_languages::{
    AdapterIdentity, AdapterRegistry, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    Engine, EvidenceErrorCode, EvidenceLimits, EvidenceRange, Extraction, HierarchyConstraint,
    LanguageCapability, OccurrenceFact, ReceiverDispatchStrategy, RelationshipCandidate,
    ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
    UNIVERSAL_EVIDENCE_SCHEMA, UniversalAdapterProfile, validate_evidence,
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
            id: "compass.python".to_owned(),
            language: "python".to_owned(),
            version: 1,
            evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA.to_owned(),
            profile: UniversalAdapterProfile::UniversalCandidate,
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
            graph_node_id: "src_example_py_caller".to_owned(),
            kind: "function".to_owned(),
            name: "caller".to_owned(),
            qualified_name: "example.caller".to_owned(),
            module_or_package: Some("example".to_owned()),
            scope_id: None,
            signature: None,
            parameter_count: None,
            parameter_types: Vec::new(),
            direct_bases_complete: false,
            variadic: false,
            signature_hash: None,
            implementation_hash: None,
            source_hash: None,
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
            output_index: None,
            range: range(7, 13),
        }],
        occurrences: vec![OccurrenceFact {
            id: "occurrence:helper".to_owned(),
            language: "python".to_owned(),
            role: SemanticRole::Call,
            owner_declaration_id: "decl:caller".to_owned(),
            spelling: "helper".to_owned(),
            qualifier: None,
            context: None,
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
                exact_target_declaration_id: None,
                exact_language: Some("python".to_owned()),
                module_or_package: Some("tools".to_owned()),
                scope_id: Some("scope:caller".to_owned()),
                qualified_name: Some("tools.execute".to_owned()),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["function".to_owned()],
                hierarchy: None,
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
    let mut batch = valid_batch();
    batch.declarations[0].parameter_count = Some(1);
    batch.declarations[0].parameter_types = vec!["example.Request".to_owned()];
    batch.candidates[0].constraints.argument_count = Some(1);
    batch.candidates[0].constraints.argument_types = vec![Some("example.Request".to_owned())];
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid fixture");

    let encoded = serde_json::to_value(&batch).expect("serialize evidence");
    assert_eq!(encoded["adapter"]["language"], "python");
    assert_eq!(
        encoded["occurrences"][0]["ownerDeclarationId"],
        "decl:caller"
    );
    assert_eq!(encoded["occurrences"][0]["role"], "call");
    assert_eq!(
        encoded["declarations"][0]["parameterTypes"][0],
        "example.Request"
    );
    assert_eq!(
        encoded["candidates"][0]["constraints"]["argumentTypes"][0],
        "example.Request"
    );
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
fn output_positions_are_reserved_for_call_result_bindings() {
    let mut batch = valid_batch();
    batch.bindings[0].output_index = Some(0);
    assert_code(&batch, EvidenceErrorCode::InvalidFact);
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

    let mut missing_exact_target = valid_batch();
    missing_exact_target.candidates[0]
        .constraints
        .exact_target_declaration_id = Some("decl:missing".to_owned());
    assert_code(&missing_exact_target, EvidenceErrorCode::MissingReference);
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

    let mut undeclared_hierarchy = valid_batch();
    undeclared_hierarchy.candidates[0]
        .constraints
        .qualified_name = None;
    undeclared_hierarchy.candidates[0].constraints.hierarchy =
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "example.Owner".to_owned(),
            strategy: ReceiverDispatchStrategy::C3AfterReceiver,
        });
    assert_code(
        &undeclared_hierarchy,
        EvidenceErrorCode::UndeclaredCapability,
    );

    let mut invalid_hierarchy_relation = undeclared_hierarchy;
    invalid_hierarchy_relation
        .adapter
        .capabilities
        .push(LanguageCapability::HierarchyDispatch);
    invalid_hierarchy_relation.candidates[0]
        .constraints
        .hierarchy = Some(HierarchyConstraint::DirectBase {
        base_set_complete: true,
    });
    assert_code(&invalid_hierarchy_relation, EvidenceErrorCode::InvalidFact);
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

    let mut typed = valid_batch();
    typed.candidates[0].constraints.argument_count = Some(1);
    typed.candidates[0].constraints.argument_types = vec![Some("java.lang.String".to_owned())];
    let limits = EvidenceLimits {
        callable_types_per_fact: 0,
        ..EvidenceLimits::default()
    };
    let error = validate_evidence(&typed, limits).expect_err("callable-type limit");
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);
}

#[test]
fn callable_type_vectors_must_match_their_source_arity() {
    let mut declaration = valid_batch();
    declaration.declarations[0].parameter_count = Some(1);
    declaration.declarations[0].parameter_types =
        vec!["java.lang.String".to_owned(), "java.lang.Object".to_owned()];
    assert_code(&declaration, EvidenceErrorCode::InvalidFact);

    let mut candidate = valid_batch();
    candidate.candidates[0].constraints.argument_count = Some(1);
    candidate.candidates[0].constraints.argument_types = vec![Some("int".to_owned()), None];
    assert_code(&candidate, EvidenceErrorCode::InvalidFact);
}

#[test]
fn complete_direct_base_sets_are_reserved_for_java_types() {
    let mut batch = valid_batch();
    batch.declarations[0].direct_bases_complete = true;
    assert_code(&batch, EvidenceErrorCode::InvalidFact);
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
        ["go", "java", "python", "rust"]
    );
    assert!(
        profiles
            .iter()
            .all(|profile| !profile.capabilities.is_empty())
    );
    assert!(profiles.iter().all(|profile| {
        !profile.id.is_empty()
            && profile.version > 0
            && profile.evidence_schema == UNIVERSAL_EVIDENCE_SCHEMA
    }));
    assert_eq!(
        AdapterRegistry::universal_profile("go").map(|profile| profile.version),
        Some(3)
    );
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        profiles.len()
    );
    assert!(profiles.iter().all(|profile| {
        profile
            .capabilities
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    }));
    assert_eq!(
        AdapterRegistry::universal_profile("java")
            .map(|profile| (profile.version, profile.profile)),
        Some((3, UniversalAdapterProfile::UniversalCandidate))
    );
    assert_eq!(
        AdapterRegistry::universal_profile("rust").map(|profile| profile.version),
        Some(2)
    );
}

#[test]
fn empty_hard_cut_sources_emit_zero_width_file_inventory_evidence() {
    for (path, source_file, language) in [
        ("/repo/pkg/__init__.py", "pkg/__init__.py", "python"),
        ("/repo/pkg/empty.go", "pkg/empty.go", "go"),
        ("/repo/pkg/empty.rs", "pkg/empty.rs", "rust"),
    ] {
        let mut engine = Engine::default();
        let evidence = engine
            .extract_source_combined(std::path::Path::new(path), source_file, b"")
            .expect("extract empty hard-cut source")
            .graph
            .semantic_evidence
            .expect("empty source evidence");
        validate_evidence(&evidence, EvidenceLimits::default()).expect("valid empty evidence");

        assert_eq!(evidence.adapter.language, language);
        assert_eq!(evidence.declarations.len(), 1);
        assert_eq!(evidence.declarations[0].kind, "file");
        assert_eq!(evidence.declarations[0].range.source_file, source_file);
        assert_eq!(evidence.declarations[0].range.start_byte, 0);
        assert_eq!(evidence.declarations[0].range.end_byte, 0);
        assert_eq!(evidence.scopes.len(), 1);
        assert_eq!(evidence.scopes[0].kind, "module");
        assert_eq!(evidence.scopes[0].range, evidence.declarations[0].range);
    }
}

#[test]
fn python_emits_direct_source_grounded_evidence() {
    let source = br#"from tools.runner import execute as run

@run
class Derived(Base):
    def handle(self, value: Input) -> Output:
        run(value)
        run(value)
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/src/example.py"),
            "src/example.py",
            source,
        )
        .expect("extract python");
    assert!(
        extraction.graph.raw_calls.is_none(),
        "hard-cut Python extraction must not publish replaced raw call facts"
    );
    assert!(
        extraction.graph.nodes.is_empty() && extraction.graph.edges.is_empty(),
        "hard-cut Python extraction must not construct its replaced raw graph"
    );
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");

    assert_eq!(evidence.adapter.language, "python");
    assert!(
        evidence
            .bindings
            .iter()
            .any(|binding| binding.spelling == "run"
                && binding.qualified_target == "tools.runner.execute")
    );
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Decorator && occurrence.spelling == "run"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::BaseType && occurrence.spelling == "Base"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Annotation && occurrence.spelling == "Input"
    }));
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call && occurrence.spelling == "run")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].range.start_byte, calls[1].range.start_byte);
    for occurrence in calls {
        let start = usize::try_from(occurrence.range.start_byte).expect("fixture offset");
        let end = usize::try_from(occurrence.range.end_byte).expect("fixture offset");
        assert_eq!(&source[start..end], b"run");
    }
}

#[test]
fn python_local_class_receivers_emit_bounded_hierarchy_dispatch() {
    fn call_candidate(source: &[u8]) -> RelationshipCandidate {
        let mut engine = Engine::default();
        engine
            .extract_source_combined(
                std::path::Path::new("/repo/pkg/checks.py"),
                "pkg/checks.py",
                source,
            )
            .expect("extract python")
            .graph
            .semantic_evidence
            .expect("python universal evidence")
            .candidates
            .into_iter()
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "check"
            })
            .expect("Model.check call candidate")
    }

    let candidate = call_candidate(
        b"class Base:\n    @classmethod\n    def check(cls):\n        return []\ndef verify():\n    class Model(Base):\n        pass\n    return Model.check()\n",
    );
    assert_eq!(
        candidate.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.checks.verify::Model".to_owned(),
            strategy: ReceiverDispatchStrategy::C3FromReceiver,
        })
    );

    let rebound = call_candidate(
        b"class Base:\n    @classmethod\n    def check(cls):\n        return []\ndef verify(replacement):\n    class Model(Base):\n        pass\n    Model = replacement\n    return Model.check()\n",
    );
    assert_eq!(rebound.constraints.hierarchy, None);
}

#[test]
fn python_bound_method_receivers_emit_hierarchy_dispatch_unless_rebound() {
    fn call_candidate(source: &[u8]) -> RelationshipCandidate {
        let mut engine = Engine::default();
        engine
            .extract_source_combined(
                std::path::Path::new("/repo/pkg/models.py"),
                "pkg/models.py",
                source,
            )
            .expect("extract python")
            .graph
            .semantic_evidence
            .expect("python universal evidence")
            .candidates
            .into_iter()
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "check"
            })
            .expect("self.check call candidate")
    }

    let candidate =
        call_candidate(b"class Model:\n    def verify(self):\n        return self.check()\n");
    assert_eq!(
        candidate.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.models.Model".to_owned(),
            strategy: ReceiverDispatchStrategy::C3FromReceiver,
        })
    );

    let captured = call_candidate(
        b"class Model:\n    def verify(self, values):\n        return [self.check() for value in values]\n",
    );
    assert_eq!(
        captured.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.models.Model".to_owned(),
            strategy: ReceiverDispatchStrategy::C3FromReceiver,
        })
    );

    let mut engine = Engine::default();
    let rebound = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/models.py"),
            "pkg/models.py",
        b"class Model:\n    def verify(self, replacement):\n        self = replacement\n        return self.check()\n",
        )
        .expect("extract rebound receiver")
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    assert!(rebound.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::Calls || candidate.target_spelling != "check"
    }));

    let mut engine = Engine::default();
    let shadowed = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/models.py"),
            "pkg/models.py",
            b"class Model:\n    def verify(self, replacements):\n        return [self.check() for self in replacements]\n",
        )
        .expect("extract shadowed comprehension receiver")
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    assert!(shadowed.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::Calls || candidate.target_spelling != "check"
    }));
}

#[test]
fn python_class_callable_aliases_emit_source_proven_member_bindings() {
    let source = b"def helper():\n    return None\n\nclass UsesHelper:\n    helper_alias = helper\n\n    def run(self):\n        return self.helper_alias()\n\nclass ReboundAlias:\n    helper_alias = helper\n    helper_alias = object()\n\nclass ShadowedTarget:\n    helper = object()\n    helper_alias = helper\n\nhelper = replacement\n\nclass ReboundTarget:\n    helper_alias = helper\n";
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/models.py"),
            "pkg/models.py",
            source,
        )
        .expect("extract python")
        .graph
        .semantic_evidence
        .expect("python universal evidence");

    let aliases = evidence
        .bindings
        .iter()
        .filter(|binding| binding.kind == BindingKind::Member && binding.spelling == "helper_alias")
        .collect::<Vec<_>>();
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].qualified_target, "pkg.models.helper");
    let owner = aliases[0]
        .scope_id
        .as_deref()
        .and_then(|scope_id| evidence.scopes.iter().find(|scope| scope.id == scope_id))
        .and_then(|scope| scope.owner_declaration_id.as_deref())
        .and_then(|owner_id| {
            evidence
                .declarations
                .iter()
                .find(|declaration| declaration.id == owner_id)
        })
        .expect("member binding owner");
    assert_eq!(owner.qualified_name, "pkg.models.UsesHelper");
}

#[test]
fn universal_declarations_preserve_signature_and_implementation_change_metadata() {
    fn function_declaration(source: &[u8]) -> DeclarationFact {
        let mut engine = Engine::default();
        engine
            .extract_source_combined(
                std::path::Path::new("/repo/src/example.py"),
                "src/example.py",
                source,
            )
            .expect("extract python")
            .graph
            .semantic_evidence
            .expect("python universal evidence")
            .declarations
            .into_iter()
            .find(|declaration| declaration.kind == "function")
            .expect("function declaration")
    }

    let original = function_declaration(b"def value() -> int:\n    return 1\n");
    let changed = function_declaration(b"def value() -> int:\n    return 2\n");

    assert_eq!(original.signature.as_deref(), Some("def value() -> int"));
    assert_eq!(original.signature_hash, changed.signature_hash);
    assert_ne!(original.implementation_hash, changed.implementation_hash);
    assert_ne!(original.source_hash, changed.source_hash);
    for digest in [
        original.signature_hash.as_deref(),
        original.implementation_hash.as_deref(),
        original.source_hash.as_deref(),
    ] {
        assert_eq!(digest.expect("declaration digest").len(), 64);
    }
    let serialized = serde_json::to_value(original).expect("serialize declaration metadata");
    assert!(serialized.get("signatureHash").is_some());
    assert!(serialized.get("implementationHash").is_some());
    assert!(serialized.get("sourceHash").is_some());
}

#[test]
fn python_imports_are_ast_grounded_and_ignore_inline_comments() {
    let source = br#"from django.contrib.postgres.aggregates import (
    StringAgg,  # RemovedInDjango70Warning.
)
from . import PostgreSQLTestCase
import tools.runner as runner
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/tests/postgres_tests/example.py"),
            "tests/postgres_tests/example.py",
            source,
        )
        .expect("extract python");
    assert_eq!(extraction.graph.error, None);
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");

    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "StringAgg"
            && binding.qualified_target == "django.contrib.postgres.aggregates.StringAgg"
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "PostgreSQLTestCase"
            && binding.qualified_target == "tests.postgres_tests.PostgreSQLTestCase"
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "runner" && binding.qualified_target == "tools.runner"
    }));
    assert!(
        evidence
            .bindings
            .iter()
            .all(|binding| !binding.spelling.contains('#')
                && !binding.qualified_target.contains('#'))
    );
}

#[test]
fn python_package_wildcard_reexports_emit_bounded_source_evidence() {
    let source = b"from django.db.models.fields import *  # public facade\n";
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/django/db/models/__init__.py"),
            "django/db/models/__init__.py",
            source,
        )
        .expect("extract python");
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");

    let binding = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "*")
        .expect("wildcard reexport binding");
    assert_eq!(binding.kind, BindingKind::Reexport);
    assert_eq!(binding.qualified_target, "django.db.models.fields");
    assert_eq!(binding.range.start_byte, 36);
    assert_eq!(binding.range.end_byte, 37);
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Reexports
            && candidate.binding_id.as_deref() == Some(binding.id.as_str())
            && candidate.constraints.qualified_name.as_deref() == Some("django.db.models.fields")
    }));
}

#[test]
fn python_partial_aliases_emit_exact_callable_declarations_and_references() {
    let source = br#"from functools import partial

def _route(value, *, Pattern):
    return Pattern(value)

route = partial(_route, Pattern=str)
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/routes.py"),
            "pkg/routes.py",
            source,
        )
        .expect("extract python")
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");

    let alias = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.routes.route")
        .expect("partial alias declaration");
    assert_eq!(alias.kind, "function");
    let target = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.routes._route")
        .expect("underlying function declaration");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.source_declaration_id == alias.id
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(target.id.as_str())
            && !candidate.constraints.allow_external
    }));
}

#[test]
fn python_partial_aliases_fail_closed_for_dynamic_or_shadowed_factories() {
    for source in [
        br#"from functools import partial
def _route(value):
    return value
route = partial(factory(), Pattern=str)
"#
        .as_slice(),
        br#"from functools import partial
def _route(value):
    return value
partial = factory
route = partial(_route, Pattern=str)
"#
        .as_slice(),
        br#"from functools import partial
def _route(value):
    return value
if enabled:
    route = partial(_route, Pattern=str)
"#
        .as_slice(),
        br#"from functools import partial
def _route(value):
    return value
def _route(value, extra):
    return value, extra
route = partial(_route, Pattern=str)
"#
        .as_slice(),
    ] {
        let mut engine = Engine::default();
        let evidence = engine
            .extract_source_combined(
                std::path::Path::new("/repo/pkg/routes.py"),
                "pkg/routes.py",
                source,
            )
            .expect("extract python")
            .graph
            .semantic_evidence
            .expect("python universal evidence");
        validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");
        assert!(
            evidence
                .declarations
                .iter()
                .all(
                    |declaration| declaration.qualified_name != "pkg.routes.route"
                        || declaration.kind != "function"
                )
        );
    }
}

#[test]
fn python_unique_module_variables_emit_exact_identity_and_initializer_type() {
    let source = br#"class Service:
    pass

singleton = Service()
constant = 7

def factory():
    return Service()

product = factory()

def local_shadow():
    singleton = object()
    return singleton
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/state.py"),
            "pkg/state.py",
            source,
        )
        .expect("extract python")
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");

    let singleton = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.state.singleton")
        .expect("module singleton declaration");
    let constant = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.state.constant")
        .expect("module constant declaration");
    let product = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.state.product")
        .expect("module product declaration");
    let service = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "pkg.state.Service")
        .expect("initializer class declaration");
    assert_eq!(singleton.kind, "variable");
    assert_eq!(constant.kind, "variable");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::TypeOf
            && candidate.source_declaration_id == singleton.id
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(service.id.as_str())
            && !candidate.constraints.allow_external
    }));
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::TypeOf
            || candidate.source_declaration_id != constant.id
    }));
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::TypeOf
            || candidate.source_declaration_id != product.id
    }));
}

#[test]
fn python_module_variables_fail_closed_for_competing_or_conditional_bindings() {
    for source in [
        br#"class Service:
    pass
singleton = Service()
singleton = Service()
"#
        .as_slice(),
        br#"class Service:
    pass
if enabled:
    singleton = Service()
"#
        .as_slice(),
        br#"class Service:
    pass
singleton = Service()
singleton += replacement
"#
        .as_slice(),
        br#"class Service:
    pass
singleton = Service()
del singleton
"#
        .as_slice(),
        br#"class Service:
    pass
singleton = Service()
del singleton, other
"#
        .as_slice(),
        br#"class Service:
    pass
singleton = Service()
def configure(default=(singleton := replacement)):
    return default
"#
        .as_slice(),
        br#"from other import singleton
class Service:
    pass
singleton = Service()
"#
        .as_slice(),
    ] {
        let mut engine = Engine::default();
        let evidence = engine
            .extract_source_combined(
                std::path::Path::new("/repo/pkg/state.py"),
                "pkg/state.py",
                source,
            )
            .expect("extract python")
            .graph
            .semantic_evidence
            .expect("python universal evidence");
        validate_evidence(&evidence, EvidenceLimits::default()).expect("valid python evidence");
        assert!(
            evidence
                .declarations
                .iter()
                .all(|declaration| declaration.qualified_name != "pkg.state.singleton")
        );
    }
}

#[test]
fn python_dynamic_bases_and_nested_initializer_imports_fail_closed() {
    let source = br#"from framework import factory

class Dynamic(factory(Base)):
    def load(self):
        from vendor import helper
        return helper()
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/__init__.py"),
            "pkg/__init__.py",
            source,
        )
        .expect("extract python");
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("python universal evidence");

    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.relation != CandidateRelation::Extends
            || !matches!(candidate.target_spelling.as_str(), "factory" | "Base")
    }));
    assert!(
        evidence.bindings.iter().any(|binding| {
            binding.spelling == "factory" && binding.kind == BindingKind::Reexport
        })
    );
    assert!(
        evidence
            .bindings
            .iter()
            .any(|binding| { binding.spelling == "helper" && binding.kind == BindingKind::Import })
    );
}

#[test]
fn python_preindexes_imports_without_violating_same_scope_execution_order() {
    let source = br#"def before():
    callback()
    return callback

EARLY = [callback]
from tools.runner import execute as callback
LATE = [callback]
import pkg.api
import other.tools as alias

def dotted():
    pkg.api.execute()
    alias.execute()
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(std::path::Path::new("/repo/app.py"), "app.py", source)
        .expect("extract python")
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid Python evidence");

    let callback = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "callback")
        .expect("callback import binding");
    assert_eq!(callback.qualified_target, "tools.runner.execute");
    let package = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "pkg")
        .expect("top-level dotted package binding");
    assert_eq!(package.kind, BindingKind::Import);
    assert_eq!(package.qualified_target, "pkg");
    let alias = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "alias")
        .expect("dotted import alias");
    assert_eq!(alias.qualified_target, "other.tools");

    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "callback"
            && candidate.binding_id.as_deref() == Some(&callback.id)
            && candidate.constraints.qualified_name.as_deref() == Some("tools.runner.execute")
    }));
    for qualified in ["pkg.api.execute", "other.tools.execute"] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.constraints.qualified_name.as_deref() == Some(qualified)
        }));
    }
    let callback_references = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::CallableReference && occurrence.spelling == "callback"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        callback_references.len(),
        2,
        "the deferred function body and post-import collection are valid; the pre-import module use is not"
    );
    assert_eq!(
        callback_references
            .iter()
            .map(|occurrence| occurrence.context.as_deref())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([Some("collection"), Some("return")])
    );
    for reference in callback_references {
        let candidates = evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.occurrence_id.as_deref() == Some(&reference.id))
            .collect::<Vec<_>>();
        assert!(candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::References
                && candidate.constraints.allow_external
        }));
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.relation != CandidateRelation::IndirectCalls)
        );
    }
}

#[test]
fn go_emits_packages_receivers_embeddings_and_exact_calls() {
    let source = br#"package sample

import alias "example.com/tools"

type Base struct{}
type Derived struct {
    Base
    Client alias.Client
}

func (d *Derived) Handle(value alias.Input) alias.Output {
    d.Handle(value)
    alias.Run(value)
    alias.Run(value)
    return alias.Output{}
}
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go");
    assert!(
        extraction.graph.raw_calls.is_none(),
        "hard-cut Go extraction must not publish replaced raw call facts"
    );
    assert!(
        extraction.graph.nodes.is_empty() && extraction.graph.edges.is_empty(),
        "hard-cut Go extraction must not construct its replaced raw graph"
    );
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid go evidence");

    assert_eq!(evidence.adapter.language, "go");
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "alias" && binding.qualified_target == "example.com/tools"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Receiver && occurrence.spelling == "Derived"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Embedding && occurrence.spelling == "Base"
    }));
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.spelling == "Run")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|occurrence| occurrence.qualifier.as_deref() == Some("alias"))
    );
    let receiver_binding = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "d")
        .expect("typed receiver binding");
    assert_eq!(receiver_binding.kind, BindingKind::LocalAlias);
    assert_eq!(receiver_binding.qualified_target, "sample.Derived");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.target_spelling == "Handle"
            && candidate.binding_id.as_deref() == Some(&receiver_binding.id)
            && candidate.constraints.qualified_name.as_deref() == Some("sample.Derived::Handle")
    }));
}

#[test]
fn go_emits_direct_and_grouped_aliases_with_closure_signature_references() {
    let source = br#"package sample

import "example.com/types"

type Direct = types.Token
type (
    Grouped = types.Skill
)

type Event struct {
    Token *Direct
    Skill Grouped
}

func use() {
    _ = func(value types.Input) {}
}
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go")
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid go evidence");

    for name in ["Direct", "Grouped"] {
        assert!(
            evidence.declarations.iter().any(|declaration| {
                declaration.name == name && declaration.kind == "type_alias"
            })
        );
    }
    for (target, qualifier) in [
        ("Token", Some("types")),
        ("Skill", Some("types")),
        ("Direct", None),
        ("Grouped", None),
        ("Input", Some("types")),
    ] {
        assert!(evidence.occurrences.iter().any(|occurrence| {
            occurrence.role == SemanticRole::TypeReference
                && occurrence.spelling == target
                && occurrence.qualifier.as_deref() == qualifier
        }));
    }
    for (member, target) in [("Token", "sample.Direct"), ("Skill", "sample.Grouped")] {
        assert!(evidence.bindings.iter().any(|binding| {
            binding.kind == BindingKind::Member
                && binding.spelling == member
                && binding.qualified_target == target
        }));
    }
}

#[test]
fn go_embeddings_require_a_declared_type_owner() {
    let source = br#"package sample

type Base struct{}
type Declared struct {
    Base
}

func run() {
    local := struct {
        Base
    }{}
    _ = local
}
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go");
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    let embeddings = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Embeds)
        .collect::<Vec<_>>();

    assert_eq!(embeddings.len(), 1);
    let owner = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.id == embeddings[0].source_declaration_id)
        .expect("embedding owner");
    assert_eq!(owner.name, "Declared");
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.target_spelling != "Base" || candidate.relation != CandidateRelation::References
    }));
}

#[test]
fn go_calls_respect_block_lifetimes_and_captured_callback_bindings() {
    let source = br#"package sample

func target() {}

func caller(callback func()) {
    {
        target := func() {}
        target()
    }
    target()
    invoke := func() {
        callback()
    }
    invoke()
}
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go")
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid go evidence");

    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call)
        .collect::<Vec<_>>();
    assert_eq!(
        calls
            .iter()
            .filter(|occurrence| occurrence.spelling == "target")
            .count(),
        1,
        "the block-local callback must not suppress the later package function call"
    );
    assert!(
        calls
            .iter()
            .all(|occurrence| !matches!(occurrence.spelling.as_str(), "callback" | "invoke")),
        "parameters, captures, and function-valued locals must not resolve as package calls"
    );
}

#[test]
fn go_calls_use_lexical_composite_and_method_return_receiver_types() {
    let source = br#"package sample

type Bucket struct{}
type Cursor struct{}
type Stats struct{}
type Options struct{}
type Command struct{ Options }

func (b *Bucket) write() {}
func (b *Bucket) Cursor() *Cursor { return &Cursor{} }
func (b *Bucket) Open() (*Cursor, error) { return &Cursor{}, nil }
func (c *Cursor) node() {}
func (s *Stats) Add() {}
func (o *Options) AddFlags() {}

func create(b *Bucket) {
    bucket := Bucket{}
    bucket.write()
    c := b.Cursor()
    c.node()
    opened, err := b.Open()
    _ = err
    opened.node()
    var stats Stats
    stats.Add()
    var command Command
    command.Options.AddFlags()
}
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go")
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid go evidence");

    for (spelling, qualified_name) in [
        ("write", "sample.Bucket::write"),
        ("node", "sample.Cursor::node"),
        ("Add", "sample.Stats::Add"),
        ("AddFlags", "sample.Options::AddFlags"),
    ] {
        assert!(
            evidence.candidates.iter().any(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == spelling
                    && candidate.constraints.qualified_name.as_deref() == Some(qualified_name)
            }),
            "missing {qualified_name}: {:#?}",
            evidence.candidates
        );
    }
    let cursor = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "sample.Bucket::Cursor")
        .expect("Cursor method declaration");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == cursor.id
            && candidate.relation == CandidateRelation::Returns
            && candidate.target_spelling == "Cursor"
            && candidate.constraints.qualified_name.as_deref() == Some("sample.Cursor")
    }));
    let call_result = evidence
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == BindingKind::CallResult
                && binding.spelling == "c"
                && binding.qualified_target == "sample.Bucket::Cursor"
        })
        .expect("method call-result binding");
    assert_eq!(call_result.output_index, None);
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "node"
            && candidate.binding_id.as_deref() == Some(call_result.id.as_str())
    }));
    let positional_result = evidence
        .bindings
        .iter()
        .find(|binding| binding.kind == BindingKind::CallResult && binding.spelling == "opened")
        .expect("positional method call-result binding");
    assert_eq!(positional_result.output_index, Some(0));
}

#[test]
fn go_calls_follow_type_assertion_receiver_types() {
    let source = br#"package sample

type Command struct{}

func (c *Command) Execute() {}

func invoke(value interface{}) {
    command := value.(*Command)
    command.Execute()
}
"#;
    let mut engine = Engine::default();
    let evidence = engine
        .extract_source_combined(
            std::path::Path::new("/repo/sample/example.go"),
            "sample/example.go",
            source,
        )
        .expect("extract go")
        .graph
        .semantic_evidence
        .expect("go universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid go evidence");

    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "Execute"
            && candidate.constraints.qualified_name.as_deref() == Some("sample.Command::Execute")
    }));
}

#[test]
fn direct_adapter_ids_and_partial_diagnostics_are_deterministic() {
    let path = std::path::Path::new("/repo/src/example.py");
    let source_file = "src/example.py";
    let source = b"def broken(value:\n    helper(value)\n";
    let mut first_engine = Engine::default();
    let first = first_engine
        .extract_source_combined(path, source_file, source)
        .expect("first extract")
        .graph
        .semantic_evidence
        .expect("first evidence");
    let mut second_engine = Engine::default();
    let second = second_engine
        .extract_source_combined(path, source_file, source)
        .expect("second extract")
        .graph
        .semantic_evidence
        .expect("second evidence");
    assert_eq!(first, second);
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
}
