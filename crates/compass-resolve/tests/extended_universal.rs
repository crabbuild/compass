use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::Engine;

#[test]
fn extended_universal_languages_never_resolve_same_named_jvm_types_by_family()
-> Result<(), Box<dyn Error>> {
    let scala_source = "class Shared { def run(): Unit = () }\n";
    let caller_source = "class Caller { def call(): Unit = Shared().run() }\n";
    let java_source = "class Shared { void run() {} }\n";
    let scala_path = Path::new("src/Shared.scala");
    let caller_path = Path::new("src/Caller.scala");
    let java_path = Path::new("src/Shared.java");
    let mut engine = Engine::default();
    let scala = engine.extract_source_graph_only(
        scala_path,
        scala_path.to_str().unwrap_or_default(),
        scala_source.as_bytes(),
    )?;
    let caller = engine.extract_source_graph_only(
        caller_path,
        caller_path.to_str().unwrap_or_default(),
        caller_source.as_bytes(),
    )?;
    let java = engine.extract_source_graph_only(
        java_path,
        java_path.to_str().unwrap_or_default(),
        java_source.as_bytes(),
    )?;
    let sources = HashMap::from([
        (
            scala_path.to_string_lossy().into_owned(),
            scala_source.to_owned(),
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            caller_source.to_owned(),
        ),
        (
            java_path.to_string_lossy().into_owned(),
            java_source.to_owned(),
        ),
    ]);
    let resolved =
        compass_resolve::resolve_with_root(&[scala, caller, java], &sources, Path::new("."));

    assert!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("source_file") == caller_path.to_string_lossy()
                    && matches!(
                        edge.string("relation").as_str(),
                        "calls" | "constructs" | "references"
                    )
            })
            .all(|edge| {
                let target_source = resolved
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .map(|node| node.string("source_file"));
                target_source.is_none_or(|source| source.ends_with(".scala"))
            })
    );
    Ok(())
}

#[test]
fn every_extended_language_keeps_same_named_foreign_types_unresolved() -> Result<(), Box<dyn Error>>
{
    let cases = [
        (
            "swift",
            "src/swift/Caller.swift",
            "struct Shared {}\nstruct Caller { func call() { _ = Shared() } }\n",
            "src/java/Shared.java",
            "class Shared {}\n",
        ),
        (
            "dart",
            "src/dart/caller.dart",
            "class Shared {}\nclass Caller { void call() { Shared(); } }\n",
            "src/scala/Shared.scala",
            "class Shared\n",
        ),
        (
            "scala",
            "src/scala/Caller.scala",
            "class Shared\nclass Caller { def call(): Unit = new Shared() }\n",
            "src/java/Shared.java",
            "class Shared {}\n",
        ),
        (
            "groovy",
            "src/groovy/Caller.groovy",
            "class Shared {}\nclass Caller { void call() { new Shared() } }\n",
            "src/kotlin/Shared.kt",
            "class Shared\n",
        ),
    ];

    for (language, caller_path, caller_source, foreign_path, foreign_source) in cases {
        let caller_path = Path::new(caller_path);
        let foreign_path = Path::new(foreign_path);
        let mut engine = Engine::default();
        let caller = engine.extract_source_graph_only(
            caller_path,
            caller_path.to_str().unwrap_or_default(),
            caller_source.as_bytes(),
        )?;
        let foreign = engine.extract_source_graph_only(
            foreign_path,
            foreign_path.to_str().unwrap_or_default(),
            foreign_source.as_bytes(),
        )?;
        let sources = HashMap::from([
            (
                caller_path.to_string_lossy().into_owned(),
                caller_source.to_owned(),
            ),
            (
                foreign_path.to_string_lossy().into_owned(),
                foreign_source.to_owned(),
            ),
        ]);
        let resolved =
            compass_resolve::resolve_with_root(&[caller, foreign], &sources, Path::new("."));
        let semantic_edges = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("source_file") == caller_path.to_string_lossy()
                    && matches!(
                        edge.string("relation").as_str(),
                        "calls" | "constructs" | "references" | "extends" | "implements"
                    )
            })
            .collect::<Vec<_>>();
        assert!(
            !semantic_edges.is_empty(),
            "{language}: caller did not emit a semantic edge"
        );
        assert!(
            semantic_edges.iter().all(|edge| {
                resolved
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .is_none_or(|node| node.string("source_file") != foreign_path.to_string_lossy())
            }),
            "{language}: resolved a target from a foreign language file"
        );
    }
    Ok(())
}
