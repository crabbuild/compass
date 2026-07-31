use compass_languages::{
    AdapterIdentity, AdapterRegistry, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    Engine, EvidenceErrorCode, EvidenceLimits, EvidenceRange, Extraction, HierarchyConstraint,
    LanguageCapability, OccurrenceFact, ReceiverDispatchStrategy, RelationshipCandidate,
    ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole, file_stem, make_id,
    validate_evidence,
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
            graph_node_id: "src_example_py_caller".to_owned(),
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
                exact_target_declaration_id: None,
                exact_language: Some("python".to_owned()),
                module_or_package: Some("tools".to_owned()),
                scope_id: Some("scope:caller".to_owned()),
                qualified_name: Some("tools.execute".to_owned()),
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
fn python_emits_ordered_direct_bases_and_receiver_dispatch_constraints() {
    let source = br#"class Root:
    def run(self):
        return None

class Mixin:
    pass

class Child(Mixin, Root):
    class Product:
        pass

    def local(self):
        return None

    @classmethod
    def make(cls):
        cls.local()

    def run(self):
        self.local()
        self.Product()
        super().run()
"#;
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(
            std::path::Path::new("/repo/pkg/models.py"),
            "pkg/models.py",
            source,
        )
        .expect("extract python");
    let evidence = extraction
        .graph
        .semantic_evidence
        .expect("python universal evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid hierarchy evidence");

    assert!(
        evidence
            .adapter
            .capabilities
            .contains(&LanguageCapability::HierarchyDispatch)
    );
    let mut bases = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Extends
                && matches!(
                    candidate.constraints.hierarchy.as_ref(),
                    Some(HierarchyConstraint::DirectBase {
                        base_set_complete: true
                    })
                )
        })
        .map(|candidate| {
            (
                evidence
                    .occurrences
                    .iter()
                    .find(|occurrence| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
                    .expect("base occurrence")
                    .range
                    .start_byte,
                candidate
                    .constraints
                    .qualified_name
                    .clone()
                    .expect("exact base identity"),
            )
        })
        .collect::<Vec<_>>();
    bases.sort_unstable();
    assert_eq!(
        bases
            .into_iter()
            .map(|(_, qualified)| qualified)
            .collect::<Vec<_>>(),
        ["pkg.models.Mixin", "pkg.models.Root"]
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.target_spelling == "run"
            && matches!(
                candidate.constraints.hierarchy.as_ref(),
                Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name,
                    strategy: ReceiverDispatchStrategy::C3AfterReceiver,
                }) if receiver_qualified_name == "pkg.models.Child"
            )
            && candidate.constraints.qualified_name.is_none()
            && !candidate.constraints.allow_external
    }));
    for qualifier in ["self", "cls"] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.target_spelling == "local"
                && candidate
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| evidence.occurrences.iter().find(|fact| fact.id == id))
                    .and_then(|fact| fact.qualifier.as_deref())
                    == Some(qualifier)
                && matches!(
                    candidate.constraints.hierarchy.as_ref(),
                    Some(HierarchyConstraint::ReceiverDispatch {
                        receiver_qualified_name,
                        strategy: ReceiverDispatchStrategy::C3FromReceiver,
                    }) if receiver_qualified_name == "pkg.models.Child"
                )
                && candidate.constraints.qualified_name.is_none()
                && !candidate.constraints.allow_external
        }));
    }
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Constructs
            && candidate.target_spelling == "Product"
            && matches!(
                candidate.constraints.hierarchy.as_ref(),
                Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name,
                    strategy: ReceiverDispatchStrategy::C3FromReceiver,
                }) if receiver_qualified_name == "pkg.models.Child"
            )
    }));
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
fn python_runtime_declarations_have_exact_lexical_owners_and_aligned_graph_ids() {
    let source = br#"def outer(flag):
    if flag:
        def duplicate():
            return 1
    else:
        def duplicate():
            return 2
    def inner():
        from helpers import execute
        return execute()
    class Runtime:
        def run(self):
            return inner()
    return Runtime
"#;
    let path = std::path::Path::new("pkg/runtime.py");
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(path, "pkg/runtime.py", source)
        .expect("extract nested Python declarations");
    let graph = extraction.graph;
    assert_eq!(graph.error, None);

    let stem = file_stem(path);
    let outer_id = make_id(&[&stem, "outer"]);
    let duplicate_id = make_id(&[&outer_id, "duplicate"]);
    let duplicate_overload_id = make_id(&[&duplicate_id, "overload", "6"]);
    let inner_id = make_id(&[&outer_id, "inner"]);
    let runtime_id = make_id(&[&outer_id, "Runtime"]);
    let run_id = make_id(&[&runtime_id, "run"]);

    for (id, label) in [
        (&outer_id, "outer()"),
        (&duplicate_id, "duplicate()"),
        (&duplicate_overload_id, "duplicate()"),
        (&inner_id, "inner()"),
        (&runtime_id, "Runtime"),
        (&run_id, ".run()"),
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == *id && node.label() == label),
            "missing source declaration {id} ({label})"
        );
    }
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == runtime_id)
            .expect("runtime class")
            .string("qualified_name"),
        "outer::Runtime"
    );
    for (source, target, relation) in [
        (&outer_id, &duplicate_id, "contains"),
        (&outer_id, &duplicate_overload_id, "contains"),
        (&outer_id, &inner_id, "contains"),
        (&outer_id, &runtime_id, "contains"),
        (&runtime_id, &run_id, "method"),
    ] {
        assert!(graph.edges.iter().any(|edge| {
            edge.source == *source && edge.target == *target && edge.string("relation") == relation
        }));
    }

    let evidence = graph
        .semantic_evidence
        .as_ref()
        .expect("Python universal evidence");
    validate_evidence(evidence, EvidenceLimits::default()).expect("valid nested evidence");
    let declaration = |graph_id: &str| {
        evidence
            .declarations
            .iter()
            .find(|declaration| declaration.graph_node_id == graph_id)
            .unwrap_or_else(|| panic!("missing evidence declaration for {graph_id}"))
    };
    let outer = declaration(&outer_id);
    let duplicate = declaration(&duplicate_id);
    let duplicate_overload = declaration(&duplicate_overload_id);
    let inner = declaration(&inner_id);
    let runtime = declaration(&runtime_id);
    let run = declaration(&run_id);
    assert_eq!(inner.qualified_name, "pkg.runtime.outer::inner");
    assert_eq!(runtime.qualified_name, "pkg.runtime.outer::Runtime");
    assert_eq!(run.qualified_name, "pkg.runtime.outer::Runtime::run");
    for child in [duplicate, duplicate_overload, inner, runtime] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.source_declaration_id == outer.id
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(child.id.as_str())
        }));
    }

    let inner_scope = evidence
        .scopes
        .iter()
        .find(|scope| scope.owner_declaration_id.as_deref() == Some(inner.id.as_str()))
        .expect("inner scope");
    assert!(
        evidence.scopes.iter().any(|scope| {
            scope.owner_declaration_id.as_deref() == Some(inner.id.as_str())
                && scope.parent_scope_id.as_deref()
                    == evidence
                        .scopes
                        .iter()
                        .find(|scope| {
                            scope.owner_declaration_id.as_deref() == Some(outer.id.as_str())
                        })
                        .map(|scope| scope.id.as_str())
        }),
        "inner declaration is not lexically scoped to outer"
    );
    let binding = evidence
        .bindings
        .iter()
        .find(|binding| binding.spelling == "execute")
        .expect("nested import binding");
    assert_eq!(binding.scope_id.as_deref(), Some(inner_scope.id.as_str()));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.source_declaration_id == inner.id
            && candidate.binding_id.as_deref() == Some(binding.id.as_str())
    }));
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
