use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::Engine;

fn call_edges(
    extraction: &compass_languages::Extraction,
) -> Vec<&compass_languages::RawEdgeRecord> {
    extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .collect()
}

#[test]
fn deferred_java_builtin_collision_keeps_repeated_exact_call_sites() -> Result<(), Box<dyn Error>> {
    let path = Path::new("src/Collisions.java");
    let source = br#"
class Collisions {
    void open() {}
    void caller() {
        this.open();
        this.open();
    }
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let calls = call_edges(&resolved);

    assert_eq!(calls.len(), 2, "edges={:?}", resolved.edges);
    let mut sites = calls
        .iter()
        .map(|edge| {
            (
                edge.attributes
                    .get("start_byte")
                    .and_then(|value| value.as_u64()),
                edge.attributes
                    .get("end_byte")
                    .and_then(|value| value.as_u64()),
            )
        })
        .collect::<Vec<_>>();
    sites.sort_unstable();
    assert!(
        sites
            .iter()
            .all(|(start, end)| matches!((start, end), (Some(start), Some(end)) if start < end))
    );
    assert_ne!(sites[0], sites[1], "repeated occurrences were coalesced");
    assert!(calls.iter().all(|edge| {
        edge.string("language") == "java" && edge.string("extractor") == "compass.languages.java"
    }));
    Ok(())
}

#[test]
fn deferred_resolution_does_not_cross_language_families_for_builtin_spellings()
-> Result<(), Box<dyn Error>> {
    let rust_path = Path::new("src/caller.rs");
    let rust_source = b"fn caller(){ open(); }\n";
    let java_path = Path::new("src/Open.java");
    let java_source = b"class Open { static void open() {} }\n";
    let mut engine = Engine::default();
    let rust = engine.extract_source(rust_path, rust_source)?;
    let java = engine.extract_source(java_path, java_source)?;
    let sources = HashMap::from([
        (
            rust_path.to_string_lossy().into_owned(),
            String::from_utf8(rust_source.to_vec())?,
        ),
        (
            java_path.to_string_lossy().into_owned(),
            String::from_utf8(java_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve(&[rust, java], &sources);

    assert!(
        call_edges(&resolved).is_empty(),
        "cross-language call was invented: {:?}",
        resolved.edges
    );
    Ok(())
}
