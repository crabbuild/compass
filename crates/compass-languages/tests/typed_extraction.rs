use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use serde_json::{Map, Value, json};

#[test]
fn extraction_owns_flexible_raw_records() -> Result<(), serde_json::Error> {
    let extraction = Extraction {
        nodes: vec![RawNodeRecord {
            id: "function:example".to_owned(),
            attributes: Map::from_iter([
                ("label".to_owned(), json!("example")),
                ("type".to_owned(), json!("function")),
                ("producer_specific".to_owned(), json!({"nested": true})),
            ]),
        }],
        edges: vec![RawEdgeRecord {
            source: "function:example".to_owned(),
            target: "function:dependency".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("confidence".to_owned(), json!("EXTRACTED")),
            ]),
        }],
        ..Extraction::default()
    };

    assert_eq!(extraction.nodes[0].label(), "example");
    assert_eq!(extraction.nodes[0].string("type"), "function");
    assert_eq!(extraction.edges[0].string("relation"), "calls");

    let encoded = serde_json::to_value(&extraction)?;
    assert_eq!(
        encoded["nodes"][0]["producer_specific"]["nested"],
        Value::Bool(true)
    );
    assert_eq!(encoded["edges"][0]["relation"], "calls");

    let decoded: Extraction = serde_json::from_value(encoded)?;
    assert_eq!(decoded, extraction);
    Ok(())
}
