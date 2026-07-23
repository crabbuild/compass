use std::collections::BTreeMap;

use compass_ir::{
    BasicBlock, Capability, CoverageState, EvidenceRecord, FunctionIr, IrError, ModuleIr,
    Operation, OperationKind, ProgramBundle, ProviderDescriptor, ProviderKind, SourceAnchor,
    Terminator,
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn bundle() -> ProgramBundle {
    let call_evidence = EvidenceRecord {
        id: C.to_owned(),
        provider_id: "syntax:src/lib.rs".to_owned(),
        source_file: Some("src/lib.rs".to_owned()),
        capability: Capability::CallResolution,
        detail: "unique same-module target".to_owned(),
    };
    let anchor = SourceAnchor {
        source_file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 32,
    };
    let mut coverage = BTreeMap::new();
    coverage.insert(Capability::Syntax, CoverageState::Complete);
    coverage.insert(
        Capability::CallResolution,
        CoverageState::Partial {
            reasons: vec!["dynamic_dispatch".to_owned()],
        },
    );
    ProgramBundle {
        schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
        providers: vec![ProviderDescriptor {
            id: "syntax:src/lib.rs".to_owned(),
            kind: ProviderKind::Syntax,
            version: "tree-sitter-rust/1".to_owned(),
            scope: "src/lib.rs".to_owned(),
            input_digest: A.to_owned(),
            configuration_digest: B.to_owned(),
        }],
        evidence: vec![call_evidence],
        modules: vec![ModuleIr {
            source_file: "src/lib.rs".to_owned(),
            language: "rust".to_owned(),
            source_digest: A.to_owned(),
            graph_node_id: None,
            functions: vec![FunctionIr {
                symbol_id: "rust:src/lib.rs:run".to_owned(),
                name: "run".to_owned(),
                graph_node_id: None,
                signature_digest: B.to_owned(),
                body_digest: C.to_owned(),
                anchor: anchor.clone(),
                parameters: Vec::new(),
                return_type: None,
                blocks: vec![BasicBlock {
                    id: 0,
                    operations: vec![Operation {
                        ordinal: 0,
                        anchor: SourceAnchor {
                            source_file: "src/lib.rs".to_owned(),
                            start_byte: 12,
                            end_byte: 18,
                        },
                        evidence: vec![C.to_owned()],
                        kind: OperationKind::Call {
                            callee: "work".to_owned(),
                            callee_anchor: SourceAnchor {
                                source_file: "src/lib.rs".to_owned(),
                                start_byte: 12,
                                end_byte: 16,
                            },
                            resolved_symbols: vec!["rust:src/lib.rs:work".to_owned()],
                            receiver_type: None,
                        },
                    }],
                    terminator: Terminator::Return { value: None },
                    evidence: Vec::new(),
                }],
                coverage: coverage.clone(),
                evidence: vec![C.to_owned()],
            }],
            coverage,
            evidence: vec![C.to_owned()],
        }],
    }
}

#[test]
fn canonical_bytes_ignore_set_order() -> Result<(), IrError> {
    let first = bundle();
    let mut second = first.clone();
    second.modules.reverse();
    second.providers.reverse();
    second.evidence.reverse();
    second.modules[0].functions.reverse();
    second.modules[0].evidence.push(C.to_owned());
    assert_eq!(first.canonical_bytes()?, second.canonical_bytes()?);
    assert_eq!(first.digest()?, second.digest()?);
    assert!(
        first
            .canonical_bytes()?
            .windows(compass_ir::PROGRAM_SCHEMA.len())
            .any(|window| window == compass_ir::PROGRAM_SCHEMA.as_bytes())
    );
    Ok(())
}

#[test]
fn validation_rejects_absolute_paths_and_unknown_evidence() {
    let mut absolute = bundle();
    absolute.modules[0].source_file = "/tmp/src/lib.rs".to_owned();
    assert!(matches!(absolute.validate(), Err(IrError::InvalidPath(_))));

    let mut unknown = bundle();
    unknown.modules[0].evidence = vec![A.to_owned()];
    assert!(matches!(
        unknown.validate(),
        Err(IrError::UnknownEvidence(_))
    ));
}

#[test]
fn validation_rejects_duplicate_provider_and_evidence_ids() {
    let mut providers = bundle();
    providers.providers.push(providers.providers[0].clone());
    assert!(matches!(
        providers.validate(),
        Err(IrError::DuplicateProvider(_))
    ));

    let mut evidence = bundle();
    evidence.evidence.push(evidence.evidence[0].clone());
    assert!(matches!(
        evidence.validate(),
        Err(IrError::DuplicateEvidence(_))
    ));
}

#[test]
fn schema_two_uses_four_states_and_schema_one_remains_readable() {
    let mut current = bundle();
    current.modules[0].coverage.insert(
        Capability::Types,
        CoverageState::Indeterminate {
            reasons: vec!["type_evidence_insufficient".to_owned()],
        },
    );
    current.modules[0].coverage.insert(
        Capability::Contracts,
        CoverageState::Failed {
            reasons: vec!["contract_analyzer_failed".to_owned()],
        },
    );
    assert!(current.validate().is_ok());

    current.modules[0].coverage.insert(
        Capability::DataFlow,
        CoverageState::Unavailable {
            reasons: vec!["legacy_only".to_owned()],
        },
    );
    assert!(matches!(
        current.validate(),
        Err(IrError::InvalidCoverage { .. })
    ));

    let mut legacy = bundle();
    legacy.schema = compass_ir::PROGRAM_SCHEMA_V1.to_owned();
    legacy.modules[0].coverage.insert(
        Capability::DataFlow,
        CoverageState::Unavailable {
            reasons: vec!["legacy_unavailable".to_owned()],
        },
    );
    assert!(legacy.validate().is_ok());
    legacy.modules[0].coverage.insert(
        Capability::Contracts,
        CoverageState::Failed {
            reasons: vec!["not_supported_in_schema_one".to_owned()],
        },
    );
    assert!(matches!(
        legacy.validate(),
        Err(IrError::InvalidCoverage { .. })
    ));
}
