use std::collections::BTreeMap;

use compass_ir::{
    BasicBlock, Capability, CoverageState, FunctionIr, ModuleIr, Operation, OperationKind,
    ProviderDescriptor, ProviderKind, SourceAnchor, Terminator, hex_sha256,
};
use compass_program::{
    EvidenceBatch, EvidenceFact, FactKind, ProjectAnalyzer, ProjectInput, ProviderError,
    evidence_record, merge_evidence,
};

fn descriptor(id: &str, kind: ProviderKind) -> ProviderDescriptor {
    ProviderDescriptor {
        id: id.to_owned(),
        kind,
        version: "1".to_owned(),
        scope: if id.starts_with("syntax") {
            "src/lib.rs".to_owned()
        } else {
            "repository".to_owned()
        },
        input_digest: hex_sha256(id.as_bytes()),
        configuration_digest: hex_sha256(b"config"),
    }
}

fn syntax_batch() -> EvidenceBatch {
    let descriptor = descriptor("syntax:src/lib.rs", ProviderKind::Syntax);
    let anchor = SourceAnchor {
        source_file: "src/lib.rs".to_owned(),
        start_byte: 11,
        end_byte: 15,
    };
    let record = evidence_record(
        &descriptor.id,
        Some("src/lib.rs"),
        Capability::Syntax,
        "syntax call",
        Some(&anchor),
        "call",
        "work",
    );
    let mut coverage = BTreeMap::new();
    coverage.insert(Capability::Syntax, CoverageState::Complete);
    coverage.insert(
        Capability::CallResolution,
        CoverageState::Unavailable {
            reasons: vec!["compiler_semantics_unavailable".to_owned()],
        },
    );
    EvidenceBatch {
        descriptor,
        evidence: vec![record.clone()],
        modules: vec![ModuleIr {
            source_file: "src/lib.rs".to_owned(),
            language: "rust".to_owned(),
            source_digest: hex_sha256(b"fn run() { work(); }"),
            graph_node_id: None,
            functions: vec![FunctionIr {
                symbol_id: "rust:src/lib.rs:run".to_owned(),
                name: "run".to_owned(),
                graph_node_id: None,
                signature_digest: hex_sha256(b"fn run()"),
                body_digest: hex_sha256(b"{ work(); }"),
                anchor: SourceAnchor {
                    source_file: "src/lib.rs".to_owned(),
                    start_byte: 0,
                    end_byte: 20,
                },
                parameters: Vec::new(),
                return_type: None,
                blocks: vec![BasicBlock {
                    id: 0,
                    operations: vec![Operation {
                        ordinal: 0,
                        anchor: anchor.clone(),
                        evidence: vec![record.id.clone()],
                        kind: OperationKind::Call {
                            callee: "work".to_owned(),
                            callee_anchor: anchor,
                            resolved_symbols: Vec::new(),
                            receiver_type: None,
                        },
                    }],
                    terminator: Terminator::Return { value: None },
                    evidence: Vec::new(),
                }],
                coverage: coverage.clone(),
                evidence: vec![record.id.clone()],
            }],
            coverage: coverage.clone(),
            evidence: vec![record.id],
        }],
        facts: Vec::new(),
        coverage: BTreeMap::from([("src/lib.rs".to_owned(), coverage)]),
    }
}

fn resolution_batch(id: &str, target: &str) -> EvidenceBatch {
    let descriptor = descriptor(id, ProviderKind::Artifact);
    let anchor = SourceAnchor {
        source_file: "src/lib.rs".to_owned(),
        start_byte: 11,
        end_byte: 15,
    };
    let record = evidence_record(
        &descriptor.id,
        Some("src/lib.rs"),
        Capability::CallResolution,
        format!("resolved to {target}"),
        Some(&anchor),
        "call_resolution",
        target,
    );
    EvidenceBatch {
        descriptor,
        evidence: vec![record.clone()],
        modules: Vec::new(),
        facts: vec![EvidenceFact {
            evidence_id: record.id,
            capability: Capability::CallResolution,
            anchor,
            kind: FactKind::CallResolution {
                target: target.to_owned(),
            },
        }],
        coverage: BTreeMap::new(),
    }
}

#[test]
fn merge_is_order_independent_and_enriches_calls() -> Result<(), Box<dyn std::error::Error>> {
    let syntax = syntax_batch();
    let artifact = resolution_batch("scip:a", "rust:src/lib.rs:work");
    let first = merge_evidence(vec![syntax.clone(), artifact.clone()])?;
    let second = merge_evidence(vec![artifact, syntax])?;
    assert_eq!(first.canonical_bytes()?, second.canonical_bytes()?);
    let OperationKind::Call {
        resolved_symbols, ..
    } = &first.modules[0].functions[0].blocks[0].operations[0].kind
    else {
        return Err("expected call".into());
    };
    assert_eq!(resolved_symbols, &["rust:src/lib.rs:work"]);
    Ok(())
}

#[test]
fn merge_preserves_conflicting_targets() -> Result<(), Box<dyn std::error::Error>> {
    let bundle = merge_evidence(vec![
        syntax_batch(),
        resolution_batch("scip:a", "target:a"),
        resolution_batch("project:a", "target:b"),
    ])?;
    let function = &bundle.modules[0].functions[0];
    let OperationKind::Call {
        resolved_symbols, ..
    } = &function.blocks[0].operations[0].kind
    else {
        return Err("expected call".into());
    };
    assert_eq!(resolved_symbols, &["target:a", "target:b"]);
    assert!(matches!(
        function.coverage.get(&Capability::CallResolution),
        Some(CoverageState::Partial { reasons })
            if reasons.iter().any(|reason| reason == "provider_conflict")
    ));
    Ok(())
}

struct FakeProject;

impl ProjectAnalyzer for FakeProject {
    fn descriptor(
        &self,
        repository_digest: &str,
        build_context_digest: &str,
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "project:fake".to_owned(),
            kind: ProviderKind::Project,
            version: "1".to_owned(),
            scope: "repository".to_owned(),
            input_digest: repository_digest.to_owned(),
            configuration_digest: build_context_digest.to_owned(),
        }
    }

    fn analyze_project(&self, input: ProjectInput<'_>) -> Result<EvidenceBatch, ProviderError> {
        Ok(EvidenceBatch {
            descriptor: self.descriptor(input.repository_digest, input.build_context_digest),
            evidence: Vec::new(),
            modules: Vec::new(),
            facts: Vec::new(),
            coverage: BTreeMap::new(),
        })
    }
}

#[test]
fn project_analyzer_contract_is_in_memory() -> Result<(), ProviderError> {
    let batch = FakeProject.analyze_project(ProjectInput {
        repository_digest: &hex_sha256(b"repo"),
        build_context_digest: &hex_sha256(b"context"),
        files: &[],
    })?;
    assert_eq!(batch.descriptor.kind, ProviderKind::Project);
    Ok(())
}
