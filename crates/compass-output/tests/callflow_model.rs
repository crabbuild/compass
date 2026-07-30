use std::collections::BTreeMap;
use std::error::Error;

use compass_model::GraphDocument;
use compass_output::{
    CALLFLOW_VIEWER_SCHEMA, CallflowOptions, CallflowSection, callflow_view_model,
};
use serde_json::json;

#[test]
fn complete_model_preserves_cross_section_calls_and_edge_coverage() -> Result<(), Box<dyn Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": [
            {"id":"api","label":"api","community":0,"source_file":"src/api.rs"},
            {"id":"helper","label":"helper","community":0,"source_file":"src/helper.rs"},
            {"id":"store","label":"store","community":1,"source_file":"src/store.rs"},
            {"id":"orphan","label":"orphan","source_file":"src/orphan.rs"}
        ],
        "links": [
            {"source":"api","target":"helper","relation":"calls","confidence":"EXTRACTED"},
            {"source":"api","target":"store","relation":"calls","confidence":"INFERRED"},
            {"source":"orphan","target":"api","relation":"references","confidence":"AMBIGUOUS"}
        ]
    }))?;
    let communities = BTreeMap::from([
        (0, vec!["api".to_owned(), "helper".to_owned()]),
        (1, vec!["store".to_owned()]),
    ]);
    let sections = vec![
        CallflowSection {
            id: "overview".to_owned(),
            name: "Overview".to_owned(),
            communities: vec![],
        },
        CallflowSection {
            id: "api".to_owned(),
            name: "API".to_owned(),
            communities: vec!["0".to_owned()],
        },
        CallflowSection {
            id: "storage".to_owned(),
            name: "Storage".to_owned(),
            communities: vec!["1".to_owned()],
        },
    ];

    let model = callflow_view_model(
        &document,
        &communities,
        &CallflowOptions {
            sections: Some(&sections),
            project_name: "Fixture",
            ..CallflowOptions::default()
        },
    )?;

    assert_eq!(model.schema, CALLFLOW_VIEWER_SCHEMA);
    assert_eq!(model.coverage.internal, 1);
    assert_eq!(model.coverage.cross_section, 1);
    assert_eq!(model.coverage.unassigned, 1);
    assert_eq!(
        model.coverage.internal + model.coverage.cross_section + model.coverage.unassigned,
        document.links.len()
    );
    assert_eq!(model.cross_section_calls.len(), 1);
    let call = &model.cross_section_calls[0];
    assert_eq!(call.source, "api");
    assert_eq!(call.target, "store");
    assert_eq!(call.source_section, "api");
    assert_eq!(call.target_section, "storage");
    assert_eq!(call.relation, "calls");
    assert_eq!(call.confidence, "inferred");
    assert_eq!(model.sections[1].node_count, 2);
    assert_eq!(model.sections[1].internal_call_count, 1);
    Ok(())
}

#[test]
fn source_scopes_are_classified_without_discarding_nodes() -> Result<(), Box<dyn Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": [
            {"id":"prod","community":0,"source_file":"src/lib.rs"},
            {"id":"test","community":0,"source_file":"tests/test_api.py"},
            {"id":"generated","community":0,"source_file":"generated/schema.rs"},
            {"id":"vendor","community":0,"source_file":"vendor/pkg/lib.rs"},
            {"id":"unknown","community":0}
        ],
        "links": []
    }))?;
    let communities = BTreeMap::from([(
        0,
        vec![
            "prod".to_owned(),
            "test".to_owned(),
            "generated".to_owned(),
            "vendor".to_owned(),
            "unknown".to_owned(),
        ],
    )]);
    let sections = vec![CallflowSection {
        id: "core".to_owned(),
        name: "Core".to_owned(),
        communities: vec!["0".to_owned()],
    }];

    let model = callflow_view_model(
        &document,
        &communities,
        &CallflowOptions {
            sections: Some(&sections),
            ..CallflowOptions::default()
        },
    )?;
    let scopes = model.sections[1]
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                serde_json::to_value(&node.scope)
                    .expect("scope serializes")
                    .as_str()
                    .expect("scope is text")
                    .to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(scopes.get("prod").map(String::as_str), Some("production"));
    assert_eq!(scopes.get("test").map(String::as_str), Some("test"));
    assert_eq!(
        scopes.get("generated").map(String::as_str),
        Some("generated")
    );
    assert_eq!(scopes.get("vendor").map(String::as_str), Some("vendor"));
    assert_eq!(scopes.get("unknown").map(String::as_str), Some("unknown"));
    Ok(())
}
