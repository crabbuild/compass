use std::fs;
use std::path::Path;
use std::sync::Arc;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, HierarchyConstraint,
    ProjectEvidenceIndex, ReceiverDispatchStrategy, Registry, SemanticEvidenceBatch, SemanticRole,
    validate_evidence,
};
use serde_json::{Value, json};
use tempfile::tempdir;

fn evidence(path: &str, source: &[u8]) -> SemanticEvidenceBatch {
    let absolute = Path::new("/repo").join(path);
    let mut engine = Engine::default();
    let extraction = engine
        .extract_source_combined(&absolute, path, source)
        .expect("extract Python fixture");
    let error = extraction.graph.error.clone();
    let batch = extraction
        .graph
        .semantic_evidence
        .unwrap_or_else(|| panic!("Python must publish universal evidence: {error:?}"));
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
    assert_eq!(snapshot["pipeline"]["version"], 1);
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
fn python_src_layout_uses_unique_static_project_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempdir()?;
    let root = directory.path();
    let source = root.join("src/acme/api.py");
    fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
    fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"acme\"\n[tool.setuptools.packages.find]\nwhere = [\"src\"]\n",
    )?;
    fs::write(&source, "def handler():\n    return None\n")?;
    let project = Arc::new(ProjectEvidenceIndex::build(
        root,
        std::slice::from_ref(&source),
    ));
    let mut engine = Engine::with_project_evidence(project);
    let batch = engine
        .extract_source_combined(
            &source,
            "src/acme/api.py",
            b"def handler():\n    return None\n",
        )?
        .graph
        .semantic_evidence
        .ok_or("missing Python evidence")?;
    let handler = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "handler")
        .ok_or("missing handler declaration")?;
    assert_eq!(handler.module_or_package.as_deref(), Some("acme.api"));
    assert_eq!(handler.qualified_name, "acme.api.handler");
    Ok(())
}

#[test]
fn python_stub_extension_uses_the_python_pipeline_and_module_suffix() {
    let spec = Registry::resolve(Path::new("pkg/api.pyi")).expect("Python stub registry entry");
    assert_eq!(spec.name, "python");
    let batch = evidence("pkg/api.pyi", b"def handler() -> None: ...\n");
    let handler = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "handler")
        .expect("stub handler declaration");
    assert_eq!(handler.module_or_package.as_deref(), Some("pkg.api"));
    assert_eq!(handler.qualified_name, "pkg.api.handler");
    let package = evidence("pkg/__init__.pyi", b"from .api import Handler\n");
    assert!(package.bindings.iter().any(|binding| {
        binding.kind == BindingKind::Reexport
            && binding.spelling == "Handler"
            && binding.qualified_target == "pkg.api.Handler"
    }));
}

#[test]
fn python_ambiguous_project_roots_retain_repository_identity_and_diagnostic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let root = directory.path();
    let source = root.join("src/acme/api.py");
    fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
    fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"acme\"\n[tool.setuptools.package-dir]\none = \"src\"\ntwo = \"src\"\n",
    )?;
    fs::write(&source, "def handler(): ...\n")?;
    let project = Arc::new(ProjectEvidenceIndex::build(
        root,
        std::slice::from_ref(&source),
    ));
    let mut engine = Engine::with_project_evidence(project);
    let batch = engine
        .extract_source_combined(&source, "src/acme/api.py", b"def handler(): ...\n")?
        .graph
        .semantic_evidence
        .ok_or("missing Python evidence")?;
    let handler = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "handler")
        .ok_or("missing handler declaration")?;
    assert_eq!(handler.module_or_package.as_deref(), Some("src.acme.api"));
    assert!(batch.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "python_module_identity_ambiguous"
            && diagnostic.message.contains("one.acme.api")
            && diagnostic.message.contains("two.acme.api")
    }));
    Ok(())
}

#[test]
fn python_shadowed_initializer_does_not_fabricate_receiver_dispatch() {
    let batch = evidence(
        "pkg/services.py",
        b"class Service:\n    def run(self):\n        return None\n\ndef execute(Service):\n    value = Service()\n    return value.run()\n",
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.target_spelling == "run" && candidate.constraints.hierarchy.is_some()
    }));
}

#[test]
fn python_call_shapes_are_exact_only_without_starred_arguments() {
    let batch = evidence(
        "pkg/calls.py",
        b"def target(value, count):\n    return None\n\ndef execute(args):\n    target(\"ready\", 2)\n    target(value=\"ready\", count=2)\n    target(*args)\n",
    );
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "target"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(2)
            && candidate.constraints.argument_types
                == [
                    Some("builtins.str".to_owned()),
                    Some("builtins.int".to_owned()),
                ]
    }));
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(2)
            && candidate.constraints.argument_types == [None, None]
    }));
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.argument_count.is_none()
            && candidate.constraints.argument_types.is_empty()
    }));
}

#[test]
fn python_parameter_declarations_and_callable_shapes_preserve_method_semantics() {
    let batch = evidence(
        "pkg/service.py",
        b"class Service:\n    def run(self, value: str, count: int):\n        return None\n\n    @staticmethod\n    def build(value: str):\n        return Service()\n\n    @classmethod\n    def from_value(cls, value: str):\n        return cls()\n\n    def fallback(self, value: str = \"ready\"):\n        return value\n\n    def variadic(self, value: str, *extra: str):\n        return value\n\n    def dynamic(self, **values: str):\n        return values\n",
    );
    let declaration = |name: &str| {
        batch
            .declarations
            .iter()
            .find(|declaration| declaration.kind == "method" && declaration.name == name)
            .unwrap_or_else(|| panic!("missing method {name}"))
    };
    assert_eq!(declaration("run").parameter_count, Some(2));
    assert_eq!(declaration("build").parameter_count, Some(1));
    assert_eq!(declaration("from_value").parameter_count, Some(1));
    assert_eq!(declaration("fallback").parameter_count, None);
    assert_eq!(declaration("variadic").parameter_count, Some(2));
    assert!(declaration("variadic").variadic);
    assert_eq!(declaration("dynamic").parameter_count, None);
    assert!(declaration("dynamic").variadic);
    let parameter_names = batch
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "parameter")
        .map(|declaration| declaration.name.as_str())
        .collect::<Vec<_>>();
    for expected in ["self", "value", "count", "cls", "extra", "values"] {
        assert!(parameter_names.contains(&expected), "missing {expected}");
    }
}

#[test]
fn python_exact_annotations_publish_canonical_parameter_typeof_and_returns_evidence() {
    let batch = evidence(
        "pkg/typed.py",
        b"from typing import Annotated, Any, Optional\nfrom domain.models import Input, Output\n\nclass Container:\n    field: Input\n\ndef handle(value: Annotated[Optional[\"Input\"], \"payload\"], items: list[Input]) -> Output:\n    local: Output = Output()\n    return local\n\ndef dynamic(value: Any) -> Any:\n    return value\n",
    );
    let handle = batch
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "handle")
        .expect("handle declaration");
    assert_eq!(handle.parameter_count, Some(2));
    assert_eq!(
        handle.parameter_types,
        [
            "domain.models.Input | builtins.NoneType",
            "builtins.list[domain.models.Input]",
        ]
    );
    let parameters = batch
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "parameter"
                && declaration
                    .qualified_name
                    .starts_with(&handle.qualified_name)
        })
        .collect::<Vec<_>>();
    assert_eq!(parameters.len(), 2);
    assert!(parameters.iter().all(|parameter| {
        batch.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == parameter.id
                && candidate.relation == CandidateRelation::TypeOf
                && candidate.constraints.qualified_name.is_some()
        })
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == handle.id
            && candidate.relation == CandidateRelation::Returns
            && candidate.constraints.qualified_name.as_deref() == Some("domain.models.Output")
    }));
    for (kind, name, target) in [
        ("field", "field", "domain.models.Input"),
        ("variable", "local", "domain.models.Output"),
    ] {
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| declaration.kind == kind && declaration.name == name)
            .unwrap_or_else(|| panic!("missing {kind} {name}"));
        assert!(batch.candidates.iter().any(|candidate| {
            candidate.source_declaration_id == declaration.id
                && candidate.relation == CandidateRelation::TypeOf
                && candidate.constraints.qualified_name.as_deref() == Some(target)
        }));
    }
    let dynamic = batch
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "dynamic")
        .expect("dynamic declaration");
    assert!(dynamic.parameter_types.is_empty());
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == dynamic.id
            && candidate.relation == CandidateRelation::Returns
    }));
}

#[test]
fn python_exact_call_result_annotation_drives_receiver_without_terminal_fallback() {
    let batch = evidence(
        "pkg/results.py",
        b"class Service:\n    def run(self):\n        return None\n\ndef create() -> Service:\n    return Service()\n\ndef execute():\n    value = create()\n    return value.run()\n",
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.target_spelling == "run"
            && candidate.constraints.hierarchy
                == Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: "pkg.results.Service".to_owned(),
                    strategy: ReceiverDispatchStrategy::C3FromReceiver,
                })
    }));
    assert!(batch.bindings.iter().any(|binding| {
        binding.kind == BindingKind::CallResult
            && binding.spelling == "value"
            && binding.qualified_target == "pkg.results.Service"
    }));

    let ambiguous = evidence(
        "pkg/ambiguous_results.py",
        b"class First:\n    def run(self): ...\n\nclass Second:\n    def run(self): ...\n\ndef create() -> First: ...\ndef create() -> Second: ...\n\ndef execute():\n    value = create()\n    return value.run()\n",
    );
    assert!(!ambiguous.candidates.iter().any(|candidate| {
        candidate.target_spelling == "run" && candidate.constraints.hierarchy.is_some()
    }));
}
