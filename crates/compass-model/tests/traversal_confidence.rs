use std::error::Error;
use std::fs;

use compass_model::GraphDocument;
use serde_json::json;

#[test]
fn traversal_projection_preserves_weakest_confidence_and_deferred_state()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    fs::create_dir(directory.path().join("cache"))?;
    let links = [
        json!({"source":"a", "target":"b", "kind":"exports", "evidence":[{"confidence":"exact"},{"confidence":"inferred"}]}),
        json!({"source":"a", "target":"b", "kind":"exports", "evidence":[{"confidence":"inferred"},{"confidence":"exact"}]}),
        json!({"source":"a", "target":"b", "kind":"exports", "deferred":true, "evidence":[{"confidence":"exact"}]}),
        json!({"source":"a", "target":"b", "kind":"exports", "confidence":"EXTRACTED", "evidence":[{"confidence":"ambiguous"}]}),
    ];
    fs::write(
        &path,
        serde_json::to_vec(&json!({"nodes":[{"id":"a"},{"id":"b"}],"links":links}))?,
    )?;
    for _ in 0..2 {
        let graph = GraphDocument::load_for_traversal(&path)?;
        assert_eq!(graph.links[0].string("confidence"), "INFERRED");
        assert_eq!(graph.links[1].string("confidence"), "INFERRED");
        assert_eq!(graph.links[2].boolean("deferred"), Some(true));
        assert_eq!(graph.links[3].string("confidence"), "AMBIGUOUS");
    }
    // A cache bearing the previous format must be rebuilt, even when its
    // content signature still matches the authoritative graph.
    let cache = directory.path().join("cache/graph.json.traversal-v1.cache");
    let mut bytes = fs::read(&cache)?;
    bytes[..8].copy_from_slice(b"TRAILT06");
    fs::write(&cache, bytes)?;
    let graph = GraphDocument::load_for_traversal(&path)?;
    assert_eq!(graph.links[0].string("confidence"), "INFERRED");
    assert_eq!(graph.links[2].boolean("deferred"), Some(true));
    assert_eq!(&fs::read(cache)?[..8], b"TRAILT07");
    Ok(())
}
