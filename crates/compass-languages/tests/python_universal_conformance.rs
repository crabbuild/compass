use std::path::Path;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, HierarchyConstraint,
    ReceiverDispatchStrategy, Registry, SemanticEvidenceBatch, SemanticRole, validate_evidence,
};
use serde_json::{Value, json};

fn evidence(path: &str, source: &[u8]) -> SemanticEvidenceBatch {
    let absolute = Path::new("/repo").join(path);
    let mut engine = Engine::default();
    let batch = engine
        .extract_source_combined(&absolute, path, source)
        .expect("extract Python fixture")
        .graph
        .semantic_evidence
        .expect("Python must publish universal evidence");
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid Python evidence");
    batch
}

fn selected_snapshot(batch: &SemanticEvidenceBatch) -> Value {
    let declarations = batch
        .declarations
        .iter()
        .filter(|fact| matches!(fact.name.as_str(), "Service" | "run" | "worker"))
        .map(|fact| {
            json!({
                "kind": fact.kind,
                "name": fact.name,
                "qualifiedName": fact.qualified_name,
                "module": fact.module_or_package,
                "range": [fact.range.start_byte, fact.range.end_byte],
            })
        })
        .collect::<Vec<_>>();
    let bindings = batch
        .bindings
        .iter()
        .filter(|fact| matches!(fact.spelling.as_str(), "Parent" | "mark" | "Worker"))
        .map(|fact| {
            json!({
                "kind": fact.kind,
                "spelling": fact.spelling,
                "target": fact.qualified_target,
                "range": [fact.range.start_byte, fact.range.end_byte],
            })
        })
        .collect::<Vec<_>>();
    let occurrences = batch
        .occurrences
        .iter()
        .filter(|fact| {
            matches!(
                fact.role,
                SemanticRole::Decorator
                    | SemanticRole::BaseType
                    | SemanticRole::Annotation
                    | SemanticRole::Construction
                    | SemanticRole::Call
            )
        })
        .map(|fact| {
            json!({
                "role": fact.role,
                "spelling": fact.spelling,
                "qualifier": fact.qualifier,
                "range": [fact.range.start_byte, fact.range.end_byte],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "pipeline": {
            "id": batch.pipeline.id,
            "version": batch.pipeline.version,
            "schema": batch.pipeline.evidence_schema,
        },
        "declarations": declarations,
        "bindings": bindings,
        "occurrences": occurrences,
    })
}

#[test]
fn python_exact_evidence_snapshot_covers_identity_imports_aliases_and_calls() {
    let source = br#"from pkg.base import Base as Parent
from pkg.helpers import decorate as mark
from pkg.worker import Worker

@mark
class Service(Parent):
    def run(self, value: Input) -> Output:
        worker = Worker()
        worker.execute(value)
        mark(value)
        mark(value)
"#;
    let batch = evidence("src/acme/service.py", source);
    let snapshot = selected_snapshot(&batch);

    assert_eq!(snapshot["pipeline"]["id"], "compass.python");
    assert_eq!(snapshot["pipeline"]["version"], 11);
    assert_eq!(
        snapshot["pipeline"]["schema"],
        "compass.languages.evidence/2"
    );
    assert!(snapshot["declarations"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["kind"] == "class"
                && item["qualifiedName"] == "src.acme.service.Service"
                && item["module"] == "src.acme.service"
        })
    }));
    assert!(batch.bindings.iter().any(|binding| {
        binding.kind == BindingKind::ImportAlias
            && binding.spelling == "Parent"
            && binding.qualified_target == "pkg.base.Base"
    }));
    assert!(batch.bindings.iter().any(|binding| {
        binding.kind == BindingKind::ImportAlias
            && binding.spelling == "mark"
            && binding.qualified_target == "pkg.helpers.decorate"
    }));
    assert!(batch.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::BaseType && occurrence.spelling == "Parent"
    }));
    assert!(batch.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Decorator && occurrence.spelling == "mark"
    }));
    assert!(batch.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Annotation && occurrence.spelling == "Input"
    }));
    let repeated = batch
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call && occurrence.spelling == "mark")
        .collect::<Vec<_>>();
    assert_eq!(repeated.len(), 2);
    assert_ne!(repeated[0].range, repeated[1].range);
}

#[test]
fn python_receiver_ownership_and_c3_snapshot_is_source_proven() {
    let batch = evidence(
        "pkg/models.py",
        b"class Base:\n    def save(self):\n        return None\n\nclass Model(Base):\n    def persist(self):\n        return self.save()\n\nclass Child(Model):\n    def persist(self):\n        return super().persist()\n",
    );
    let save = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "save"
        })
        .expect("self.save candidate");
    assert_eq!(
        save.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.models.Model".to_owned(),
            strategy: ReceiverDispatchStrategy::C3FromReceiver,
        })
    );
    let super_call = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "persist"
                && candidate.constraints.hierarchy.is_some()
        })
        .expect("super().persist candidate");
    assert_eq!(
        super_call.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.models.Child".to_owned(),
            strategy: ReceiverDispatchStrategy::C3AfterReceiver,
        })
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn python_malformed_source_and_fact_order_are_deterministic() {
    let source = b"def broken(value:\n    helper(value)\n";
    let first = evidence("src/broken.py", source);
    let second = evidence("src/broken.py", source);
    assert_eq!(first, second);
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
}

#[test]
fn python_src_layout_is_characterized_as_an_established_gap() {
    let batch = evidence("src/acme/api.py", b"def handler():\n    return None\n");
    let handler = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "handler")
        .expect("handler declaration");
    assert_eq!(handler.module_or_package.as_deref(), Some("src.acme.api"));
    assert_eq!(handler.qualified_name, "src.acme.api.handler");
}

#[test]
fn python_stub_extension_is_characterized_as_an_established_gap() {
    assert!(Registry::resolve(Path::new("pkg/api.pyi")).is_none());
}

#[test]
fn python_shadowed_initializer_is_characterized_as_an_established_gap() {
    let batch = evidence(
        "pkg/services.py",
        b"class Service:\n    def run(self):\n        return None\n\ndef execute(Service):\n    value = Service()\n    return value.run()\n",
    );
    let call = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run" && candidate.constraints.hierarchy.is_some()
        })
        .expect("established producer currently invents receiver evidence for the shadowed class");
    assert_eq!(
        call.constraints.hierarchy,
        Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name: "pkg.services.Service".to_owned(),
            strategy: ReceiverDispatchStrategy::C3FromReceiver,
        })
    );
}
