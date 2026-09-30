use std::error::Error;
use std::fs;

use compass_model::{Graph, GraphDocument};
use compass_query::explanation_source;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SOURCE: &str = "fn run() {\n    body();\n}\n";

fn graph(digest: Option<Value>) -> Result<Graph, Box<dyn Error>> {
    let mut node = json!({
        "id": "n:run", "name": "run", "kind": "function",
        "source": {
            "file": "lib.rs", "startByte": 0, "endByte": SOURCE.len(),
            "startLine": 1, "endLine": 3, "startColumn": 0, "endColumn": 1
        },
        "details": {"type": "symbol", "data": {}}
    });
    if let Some(digest) = digest {
        node["details"]["data"]["sourceDigest"] = digest;
    }
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true, "multigraph": true, "graph": {},
        "nodes": [node], "links": []
    }))?;
    Ok(Graph::from_document(document)?)
}

#[test]
fn missing_digest_returns_explicitly_unverified_current_source() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = graph(None)?;
    for source in [SOURCE.to_owned(), SOURCE.replace("body", "next")] {
        fs::write(root.path().join("lib.rs"), &source)?;
        let excerpt = explanation_source(&graph, "n:run", root.path(), 4096)?;
        assert_eq!(excerpt.source, source);
        assert!(!excerpt.digest_verified);
        assert!(!excerpt.truncated);
        let bounded = explanation_source(&graph, "n:run", root.path(), 5)?;
        assert_eq!(bounded.source, "fn ru");
        assert!(!bounded.digest_verified);
        assert!(bounded.truncated);
    }
    Ok(())
}

#[test]
fn stored_digest_verifies_the_entire_span_even_when_only_a_prefix_is_returned()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let digest = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    for digest in [digest.clone(), format!("sha256:{digest}")] {
        let graph = graph(Some(json!(digest)))?;
        fs::write(root.path().join("lib.rs"), SOURCE)?;
        let excerpt = explanation_source(&graph, "n:run", root.path(), 5)?;
        assert_eq!(excerpt.source, "fn ru");
        assert!(excerpt.digest_verified);
        assert!(excerpt.truncated);
        fs::write(root.path().join("lib.rs"), SOURCE.replace("body", "next"))?;
        let error = explanation_source(&graph, "n:run", root.path(), 5)
            .err()
            .ok_or("stale source was accepted")?;
        assert!(error.to_string().contains("does not match"));
    }
    Ok(())
}

#[test]
fn malformed_digest_cannot_fall_back_to_unverified_source() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("lib.rs"), SOURCE)?;
    for digest in [
        json!(null),
        json!(false),
        json!(7),
        json!([]),
        json!({}),
        json!(""),
        json!("sha256:bad"),
    ] {
        let graph = graph(Some(digest))?;
        assert!(explanation_source(&graph, "n:run", root.path(), 4096).is_err());
    }
    Ok(())
}

#[test]
fn absent_digest_keeps_missing_file_and_span_errors() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = graph(None)?;
    assert!(explanation_source(&graph, "n:run", root.path(), 4096).is_err());
    fs::write(root.path().join("lib.rs"), "short")?;
    assert!(explanation_source(&graph, "n:run", root.path(), 4096).is_err());
    assert!(explanation_source(&graph, "missing", root.path(), 4096).is_err());
    Ok(())
}
