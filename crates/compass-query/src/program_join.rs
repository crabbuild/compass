use std::collections::HashMap;

use compass_ir::ProgramBundle;
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, ResolutionState, SourceAnchor,
};
use compass_model::query_contract::{
    CodeQueryResponse, QueryDiagnostic, QueryDiagnosticCode, QueryEvidence, QueryEvidenceLayer,
};

pub fn join_program_evidence(response: &mut CodeQueryResponse, program: Option<&ProgramBundle>) {
    let Some(program) = program else {
        response.diagnostics.push(QueryDiagnostic {
            code: QueryDiagnosticCode::ProgramUnavailable,
            message: "Program IR enrichment is not available".to_owned(),
            node_id: None,
            path: None,
        });
        response.sort_stable();
        return;
    };
    let mut nodes = response
        .nodes
        .iter_mut()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>();
    for module in &program.modules {
        for function in &module.functions {
            let Some(graph_node_id) = function.graph_node_id.as_deref() else {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::ProgramOrphan,
                    message: format!(
                        "Program symbol {} has no structural graph identity",
                        function.symbol_id
                    ),
                    node_id: None,
                    path: Some(module.source_file.clone()),
                });
                continue;
            };
            let Some(node) = nodes.get_mut(graph_node_id) else {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::ProgramOrphan,
                    message: format!(
                        "Program symbol {} references absent graph node {graph_node_id}",
                        function.symbol_id
                    ),
                    node_id: Some(graph_node_id.to_owned()),
                    path: Some(module.source_file.clone()),
                });
                continue;
            };
            if node.name.trim_end_matches("()").trim_start_matches('.')
                != function.name.trim_end_matches("()").trim_start_matches('.')
            {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::ProgramConflict,
                    message: format!(
                        "Program name {} contradicts structural name {}",
                        function.name, node.name
                    ),
                    node_id: Some(graph_node_id.to_owned()),
                    path: Some(module.source_file.clone()),
                });
            }
            let structural = node.source.as_ref();
            node.evidence.push(QueryEvidence {
                layer: QueryEvidenceLayer::ProgramIr,
                origin: EvidenceOrigin::Artifact,
                extractor: "compass.program_ir".to_owned(),
                confidence: EvidenceConfidence::Exact,
                anchor: Some(SourceAnchor {
                    file: function.anchor.source_file.clone(),
                    start_byte: function.anchor.start_byte,
                    end_byte: function.anchor.end_byte,
                    start_line: structural.map_or(1, |anchor| anchor.start_line),
                    start_column: structural.map_or(0, |anchor| anchor.start_column),
                    end_line: structural.map_or(1, |anchor| anchor.end_line),
                    end_column: structural.map_or(0, |anchor| anchor.end_column),
                }),
                rule: Some("graph_node_id".to_owned()),
                wiring_site: None,
                resolution: ResolutionState::Exact,
            });
        }
    }
    for node in nodes.into_values() {
        node.evidence.sort_by(|left, right| {
            evidence_layer_key(left.layer)
                .cmp(&evidence_layer_key(right.layer))
                .then_with(|| left.extractor.cmp(&right.extractor))
        });
    }
    response.sort_stable();
}

const fn evidence_layer_key(layer: QueryEvidenceLayer) -> u8 {
    match layer {
        QueryEvidenceLayer::StructuralGraph => 0,
        QueryEvidenceLayer::ProgramIr => 1,
    }
}
