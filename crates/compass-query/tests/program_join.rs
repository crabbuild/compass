use compass_ir::ProgramBundle;
use compass_model::code_graph::NodeKind;
use compass_model::query_contract::{
    CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, QueryDiagnosticCode,
    QueryEvidenceLayer, QueryNode,
};
use compass_query::join_program_evidence;

fn response() -> CodeQueryResponse {
    let mut response =
        CodeQueryResponse::empty(CodeQueryOperation::Callers, CodeQueryLimits::default());
    response.nodes.push(QueryNode {
        id: "graph:show".to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: "show".to_owned(),
        qualified_name: "api.show".to_owned(),
        language: Some("python".to_owned()),
        framework: None,
        source: None,
        details: None,
        evidence: Vec::new(),
    });
    response
}

fn program(name: &str, graph_node_id: Option<&str>) -> Result<ProgramBundle, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "schema": "http://crab.build/compass/v1",
        "providers": [],
        "evidence": [],
        "modules": [{
            "source_file": "api.py",
            "language": "python",
            "source_digest": "sha256:source",
            "functions": [{
                "symbol_id": "program:show",
                "name": name,
                "graph_node_id": graph_node_id,
                "signature_digest": "sha256:signature",
                "body_digest": "sha256:body",
                "visibility": "public",
                "execution_mode": "sync",
                "is_test": false,
                "anchor": {"source_file":"api.py","start_byte":0,"end_byte":10},
                "blocks": []
            }]
        }]
    }))
}

#[test]
fn program_join_is_additive_and_uses_only_graph_node_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let mut response = response();
    join_program_evidence(&mut response, Some(&program("show", Some("graph:show"))?));
    assert!(
        response.nodes[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.layer == QueryEvidenceLayer::ProgramIr })
    );
    assert!(response.diagnostics.is_empty());

    let structural = response.nodes[0].clone();
    join_program_evidence(
        &mut response,
        Some(&program("contradiction", Some("graph:show"))?),
    );
    assert_eq!(response.nodes[0].id, structural.id);
    assert_eq!(response.nodes[0].name, structural.name);
    assert!(
        response
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::ProgramConflict })
    );

    join_program_evidence(&mut response, Some(&program("orphan", None)?));
    assert!(
        response
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::ProgramOrphan })
    );
    Ok(())
}

#[test]
fn missing_program_is_a_successful_typed_diagnostic() {
    let mut response = response();
    join_program_evidence(&mut response, None);
    assert!(
        response
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::ProgramUnavailable })
    );
}
