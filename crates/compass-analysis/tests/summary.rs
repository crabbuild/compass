use std::collections::BTreeMap;

use compass_analysis::{affected_summaries, analyze};
use compass_ir::{
    BasicBlock, Capability, CoverageState, EvidenceRecord, FunctionIr, ModuleIr, Operation,
    OperationKind, ProgramBundle, ProviderDescriptor, ProviderKind, SourceAnchor, Terminator,
    hex_sha256,
};

const EVIDENCE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn program(target: Option<&str>, body: &[u8]) -> ProgramBundle {
    let source = "src/lib.rs";
    let anchor = SourceAnchor {
        source_file: source.to_owned(),
        start_byte: 0,
        end_byte: 30,
    };
    let mut coverage = BTreeMap::new();
    coverage.insert(Capability::Syntax, CoverageState::Complete);
    ProgramBundle {
        schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
        providers: vec![ProviderDescriptor {
            id: "syntax:src/lib.rs".to_owned(),
            kind: ProviderKind::Syntax,
            version: "1".to_owned(),
            scope: source.to_owned(),
            input_digest: hex_sha256(body),
            configuration_digest: hex_sha256(b"config"),
        }],
        evidence: vec![EvidenceRecord {
            id: EVIDENCE.to_owned(),
            provider_id: "syntax:src/lib.rs".to_owned(),
            source_file: Some(source.to_owned()),
            capability: if target.is_some() {
                Capability::CallResolution
            } else {
                Capability::Syntax
            },
            detail: "call".to_owned(),
        }],
        modules: vec![ModuleIr {
            source_file: source.to_owned(),
            language: "rust".to_owned(),
            source_digest: hex_sha256(body),
            graph_node_id: None,
            functions: vec![FunctionIr {
                symbol_id: "run".to_owned(),
                name: "run".to_owned(),
                graph_node_id: None,
                signature_digest: hex_sha256(b"fn run()"),
                body_digest: hex_sha256(body),
                anchor,
                parameters: Vec::new(),
                return_type: None,
                blocks: vec![BasicBlock {
                    id: 0,
                    operations: vec![
                        Operation {
                            ordinal: 0,
                            anchor: SourceAnchor {
                                source_file: source.to_owned(),
                                start_byte: 10,
                                end_byte: 14,
                            },
                            evidence: vec![EVIDENCE.to_owned()],
                            kind: OperationKind::Call {
                                callee: "work".to_owned(),
                                callee_anchor: SourceAnchor {
                                    source_file: source.to_owned(),
                                    start_byte: 10,
                                    end_byte: 14,
                                },
                                resolved_symbols: target.into_iter().map(str::to_owned).collect(),
                                receiver_type: None,
                            },
                        },
                        Operation {
                            ordinal: 1,
                            anchor: SourceAnchor {
                                source_file: source.to_owned(),
                                start_byte: 15,
                                end_byte: 20,
                            },
                            evidence: vec![EVIDENCE.to_owned()],
                            kind: OperationKind::Write {
                                path: "state".to_owned(),
                            },
                        },
                        Operation {
                            ordinal: 2,
                            anchor: SourceAnchor {
                                source_file: source.to_owned(),
                                start_byte: 21,
                                end_byte: 26,
                            },
                            evidence: vec![EVIDENCE.to_owned()],
                            kind: OperationKind::Await,
                        },
                    ],
                    terminator: Terminator::Return { value: None },
                    evidence: Vec::new(),
                }],
                coverage: coverage.clone(),
                evidence: vec![EVIDENCE.to_owned()],
            }],
            coverage,
            evidence: vec![EVIDENCE.to_owned()],
        }],
    }
}

#[test]
fn summaries_capture_behavior_and_reverse_calls() -> Result<(), Box<dyn std::error::Error>> {
    let bundle = analyze(program(Some("work"), b"first"))?;
    assert_eq!(bundle.summaries[0].resolved_calls, ["work"]);
    assert_eq!(bundle.summaries[0].writes, ["state"]);
    assert_eq!(bundle.summaries[0].effects, ["await"]);
    assert_eq!(bundle.reverse_calls["work"], ["run"]);
    assert_eq!(
        bundle.canonical_bytes()?,
        bundle.canonicalized().canonical_bytes()?
    );
    Ok(())
}

#[test]
fn invalidation_tracks_changed_callers() -> Result<(), Box<dyn std::error::Error>> {
    let previous = analyze(program(Some("work"), b"first"))?;
    let current = program(Some("work"), b"second");
    assert_eq!(
        affected_summaries(&previous, &current)?,
        ["run".to_owned()].into()
    );
    Ok(())
}
