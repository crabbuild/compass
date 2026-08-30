use compass_agent_graph::EffectiveGraph;
use compass_query::{EffectiveGraphEngine, GraphEngine};

#[test]
fn effective_engine_preserves_exact_composition_identity_and_is_read_only()
-> Result<(), Box<dyn std::error::Error>> {
    let effective = serde_json::from_str::<EffectiveGraph>(include_str!(
        "../../../fixtures/contracts/agent-graph/effective-v1.json"
    ))?;
    let identity = effective.effective_identity.as_str().to_owned();
    let engine = EffectiveGraphEngine::from_effective(effective)?;
    assert_eq!(engine.graph_identity(), identity);
    assert_eq!(engine.graph().graph.schema, "compass.graph/1");
    assert_eq!(
        engine.effective().composition_version,
        "compass.agent-graph.composition/1"
    );
    Ok(())
}
