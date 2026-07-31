use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::Engine;
use compass_resolve::resolve;

#[test]
fn collection_resolution_preserves_each_rust_evidence_batch() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let left_source =
        b"struct Left {} impl Left { fn new() -> Self { Self {} } } fn left() { Left::new(); }";
    let right_source =
        b"struct Right {} impl Right { fn new() -> Self { Self {} } } fn right() { Right::new(); }";
    let left = engine.extract_source(Path::new("src/left.rs"), left_source)?;
    let right = engine.extract_source(Path::new("src/right.rs"), right_source)?;
    let sources = HashMap::from([
        (
            "src/left.rs".to_owned(),
            String::from_utf8(left_source.to_vec())?,
        ),
        (
            "src/right.rs".to_owned(),
            String::from_utf8(right_source.to_vec())?,
        ),
    ]);

    let merged = resolve(&[left, right], &sources);
    assert_eq!(merged.universal_evidence.len(), 2);
    let source_files = merged
        .universal_evidence
        .iter()
        .flat_map(|batch| batch.occurrences.iter())
        .map(|occurrence| occurrence.anchor.source_file.as_str())
        .collect::<Vec<_>>();
    assert_eq!(source_files, ["src/left.rs", "src/right.rs"]);
    assert!(
        merged
            .universal_evidence
            .iter()
            .all(|batch| batch.adapter_id == "compass.rust")
    );
    Ok(())
}
