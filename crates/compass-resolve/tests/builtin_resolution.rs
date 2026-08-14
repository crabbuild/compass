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
        edge.string("language") == "java"
            && edge.string("extractor") == "compass.resolve.java.universal"
            && edge.string("resolution_rule") == "explicit-binding"
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

#[test]
fn typescript_builtin_calls_and_members_do_not_publish_external_hubs() -> Result<(), Box<dyn Error>>
{
    let path = Path::new("src/builtins.ts");
    let source = br#"
export function normalize(input: unknown) {
    const text = String(input);
    const count = Number(input);
    console.log(text);
    Promise.resolve(count);
    return new Date();
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing TypeScript semantic evidence")?;

    assert!(evidence.candidates.iter().all(|candidate| {
        !matches!(
            candidate.relation,
            compass_languages::CandidateRelation::Calls
                | compass_languages::CandidateRelation::Constructs
                | compass_languages::CandidateRelation::AccessesMember
        ) || !matches!(
            candidate.target_spelling.as_str(),
            "String" | "Number" | "log" | "resolve" | "Date"
        )
    }));

    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    assert!(
        call_edges(&resolved).is_empty(),
        "edges={:?}",
        resolved.edges
    );
    assert!(resolved.nodes.iter().all(|node| {
        node.attributes
            .get("module")
            .and_then(serde_json::Value::as_str)
            != Some("javascript.global")
    }));
    Ok(())
}

#[test]
fn typescript_source_shadowing_of_builtin_name_remains_resolvable() -> Result<(), Box<dyn Error>> {
    let path = Path::new("src/caller.ts");
    let source = b"export function String(value: number) { return value; }\n\
                   export function caller() { return String(7); }\n";
    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let target = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "String()"
                && node.string("source_file").replace('\\', "/") == "src/caller.ts"
        })
        .ok_or("missing source-defined String")?;

    assert!(
        call_edges(&resolved)
            .iter()
            .any(|edge| edge.target == target.id),
        "nodes={:?} edges={:?}",
        resolved.nodes,
        resolved.edges
    );
    assert!(resolved.nodes.iter().all(|node| {
        node.attributes
            .get("module")
            .and_then(serde_json::Value::as_str)
            != Some("javascript.global")
    }));
    Ok(())
}

#[test]
fn swift_builtin_globals_are_dropped_but_same_file_shadowing_is_kept() -> Result<(), Box<dyn Error>>
{
    let path = Path::new("Sources/Calls.swift");
    let source = br#"
struct Payload: Codable, Sendable {
    let data: Data
}
func print() {}
func projectCall() {}
func run(identifier: UUID) {
    print()
    projectCall()
    _ = Data()
    _ = UUID()
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let raw_names = extracted
        .raw_calls
        .iter()
        .flatten()
        .map(|call| call.callee.as_str())
        .collect::<Vec<_>>();

    assert!(!raw_names.contains(&"Data"));
    assert!(!raw_names.contains(&"UUID"));
    assert!(
        extracted
            .nodes
            .iter()
            .all(|node| { !matches!(node.label(), "Codable" | "Sendable" | "Data" | "UUID") })
    );
    let call_targets = call_edges(&extracted)
        .into_iter()
        .filter_map(|edge| extracted.nodes.iter().find(|node| node.id == edge.target))
        .map(compass_languages::RawNodeRecord::label)
        .collect::<Vec<_>>();
    assert!(call_targets.contains(&"print()"));
    assert!(call_targets.contains(&"projectCall()"));
    Ok(())
}
