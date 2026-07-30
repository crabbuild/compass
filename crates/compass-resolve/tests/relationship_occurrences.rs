use std::collections::HashMap;
use std::error::Error;
use std::fs;

use compass_languages::Engine;

#[test]
fn deferred_calls_retain_each_exact_producer_stamped_occurrence() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let caller_path = directory.path().join("caller.rs");
    let callee_path = directory.path().join("callee.rs");
    let caller_source = "fn caller(){callee();callee();}\n";
    let callee_source = "fn callee(){}\n";
    fs::write(&caller_path, caller_source)?;
    fs::write(&callee_path, callee_source)?;

    let mut engine = Engine::default();
    let caller = engine.extract(&caller_path)?;
    let callee = engine.extract(&callee_path)?;
    let sources = HashMap::from([
        (
            caller_path.to_string_lossy().into_owned(),
            caller_source.to_owned(),
        ),
        (
            callee_path.to_string_lossy().into_owned(),
            callee_source.to_owned(),
        ),
    ]);
    let resolved =
        compass_resolve::resolve_with_root(&[caller, callee], &sources, directory.path());
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .collect::<Vec<_>>();
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

    assert_eq!(sites, [(Some(12), Some(20)), (Some(21), Some(29))]);
    assert!(calls.iter().all(|edge| {
        edge.string("language") == "rust" && edge.string("extractor") == "compass.languages.rust"
    }));
    Ok(())
}
