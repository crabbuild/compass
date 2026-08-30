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

fn is_external_node(node: &compass_languages::RawNodeRecord) -> bool {
    node.attributes
        .get("external")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
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
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.constraints.module_or_package.as_deref() == Some("javascript.global")
            && candidate.constraints.allow_external
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
    assert!(extracted.raw_calls.is_none());
    assert!(
        extracted
            .nodes
            .iter()
            .all(|node| { !matches!(node.label(), "Codable" | "Sendable" | "Data" | "UUID") })
    );
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let call_targets = call_edges(&resolved)
        .into_iter()
        .filter_map(|edge| resolved.nodes.iter().find(|node| node.id == edge.target))
        .map(compass_languages::RawNodeRecord::label)
        .collect::<Vec<_>>();
    assert!(call_targets.contains(&"print()"));
    assert!(call_targets.contains(&"projectCall()"));
    Ok(())
}

#[test]
fn rust_prelude_and_primitive_calls_do_not_publish_external_hubs() -> Result<(), Box<dyn Error>> {
    let path = Path::new("src/builtins.rs");
    let source = br#"
fn normalize(value: i64) {
    let _ = Vec::new();
    let _ = String::from("value");
    let _ = Box::new(value);
    let _ = u32::try_from(value);
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
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
        !is_external_node(node)
            || !["Vec::new", "String::from", "Box::new", "u32::try_from"]
                .contains(&node.string("qualified_name").as_str())
    }));
    Ok(())
}

#[test]
fn rust_source_shadowing_of_prelude_name_remains_resolvable() -> Result<(), Box<dyn Error>> {
    let path = Path::new("src/shadowed.rs");
    let source = br#"
struct Vec;
impl Vec {
    fn new() -> Self { Vec }
}
fn caller() { let _ = Vec::new(); }
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(
        call_edges(&resolved).iter().any(|edge| {
            resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && node.string("qualified_name").ends_with("::Vec::new")
                    && node.string("source_file").replace('\\', "/") == "src/shadowed.rs"
            })
        }),
        "nodes={:?} edges={:?}",
        resolved.nodes,
        resolved.edges
    );
    Ok(())
}

#[test]
fn rust_imported_shadowing_of_prelude_name_remains_resolvable() -> Result<(), Box<dyn Error>> {
    let type_path = Path::new("src/names.rs");
    let type_source = br#"
pub struct Vec;
impl Vec {
    pub fn new() -> Self { Vec }
}
"#;
    let caller_path = Path::new("src/lib.rs");
    let caller_source = br#"
mod names;
use crate::names::Vec;
pub fn caller() { let _ = Vec::new(); }
"#;
    let mut engine = Engine::default();
    let type_extraction = engine.extract_source(type_path, type_source)?;
    let caller_extraction = engine.extract_source(caller_path, caller_source)?;
    let sources = HashMap::from([
        (
            type_path.to_string_lossy().into_owned(),
            String::from_utf8(type_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve(&[type_extraction, caller_extraction], &sources);

    assert!(
        call_edges(&resolved).iter().any(|edge| {
            resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && node.string("qualified_name").ends_with("::names::Vec::new")
                    && node.string("source_file").replace('\\', "/") == "src/names.rs"
            })
        }),
        "nodes={:?} edges={:?}",
        resolved.nodes,
        resolved.edges
    );
    Ok(())
}

#[test]
fn java_lang_types_calls_and_constructions_do_not_publish_external_hubs()
-> Result<(), Box<dyn Error>> {
    let path = Path::new("src/Builtins.java");
    let source = br#"
class Builtins {
    void normalize(Object value) {
        String.valueOf(value);
        Math.max(1, 2);
        new String();
    }
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(
        resolved.nodes.iter().all(|node| {
            !is_external_node(node) || !node.string("qualified_name").starts_with("java.lang.")
        }),
        "nodes={:?}",
        resolved.nodes
    );
    assert!(
        call_edges(&resolved).is_empty(),
        "edges={:?}",
        resolved.edges
    );
    Ok(())
}

#[test]
fn java_source_shadowing_of_java_lang_name_remains_resolvable() -> Result<(), Box<dyn Error>> {
    let path = Path::new("src/Shadowed.java");
    let source = br#"
class String {
    static String valueOf(Object value) { return new String(); }
}
class Caller {
    void run() { String.valueOf(null); }
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(
        call_edges(&resolved).iter().any(|edge| {
            resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && node.string("qualified_name").ends_with(".String::valueOf")
                    && node.string("source_file").replace('\\', "/") == "src/Shadowed.java"
            })
        }),
        "nodes={:?} edges={:?}",
        resolved.nodes,
        resolved.edges
    );
    Ok(())
}

#[test]
fn java_same_package_shadowing_of_java_lang_name_remains_resolvable() -> Result<(), Box<dyn Error>>
{
    let type_path = Path::new("src/example/String.java");
    let type_source = br#"
package example;
class String {
    static String valueOf(Object value) { return new String(); }
}
"#;
    let caller_path = Path::new("src/example/Caller.java");
    let caller_source = br#"
package example;
class Caller {
    void run() { String.valueOf(null); }
}
"#;
    let mut engine = Engine::default();
    let type_extraction = engine.extract_source(type_path, type_source)?;
    let caller_extraction = engine.extract_source(caller_path, caller_source)?;
    let sources = HashMap::from([
        (
            type_path.to_string_lossy().into_owned(),
            String::from_utf8(type_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve(&[type_extraction, caller_extraction], &sources);

    assert!(
        call_edges(&resolved).iter().any(|edge| {
            resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && node.string("qualified_name") == "example.String::valueOf"
                    && node.string("source_file").replace('\\', "/") == "src/example/String.java"
            })
        }),
        "nodes={:?} edges={:?}",
        resolved.nodes,
        resolved.edges
    );
    Ok(())
}

#[test]
fn python_builtins_are_suppressed_but_source_shadowing_is_resolved() -> Result<(), Box<dyn Error>> {
    let builtin_path = Path::new("src/builtins.py");
    let builtin_source = b"def caller(values):\n    return len(list(map(str, values)))\n";
    let shadow_path = Path::new("src/shadowed.py");
    let shadow_source = b"def len(value):\n    return 7\n\ndef caller():\n    return len([])\n";
    let mut engine = Engine::default();
    let builtin = engine.extract_source(builtin_path, builtin_source)?;
    let shadow = engine.extract_source(shadow_path, shadow_source)?;
    let sources = HashMap::from([
        (
            builtin_path.to_string_lossy().into_owned(),
            String::from_utf8(builtin_source.to_vec())?,
        ),
        (
            shadow_path.to_string_lossy().into_owned(),
            String::from_utf8(shadow_source.to_vec())?,
        ),
    ]);

    let builtin_resolved = compass_resolve::resolve(&[builtin], &sources);
    assert!(call_edges(&builtin_resolved).is_empty());
    let shadow_resolved = compass_resolve::resolve(&[shadow], &sources);
    assert!(
        call_edges(&shadow_resolved).iter().any(|edge| {
            shadow_resolved
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.label() == "len()")
        }),
        "nodes={:?} edges={:?}",
        shadow_resolved.nodes,
        shadow_resolved.edges
    );
    Ok(())
}

#[test]
fn go_predeclared_calls_are_suppressed_but_source_shadowing_is_resolved()
-> Result<(), Box<dyn Error>> {
    let builtin_path = Path::new("src/builtins.go");
    let builtin_source =
        b"package sample\nfunc caller(values []int) int { return len(append(values, 1)) }\n";
    let shadow_path = Path::new("src/shadowed.go");
    let shadow_source = b"package sample\nfunc len(values []int) int { return 7 }\nfunc caller() int { return len(nil) }\n";
    let mut engine = Engine::default();
    let builtin = engine.extract_source(builtin_path, builtin_source)?;
    let shadow = engine.extract_source(shadow_path, shadow_source)?;
    let sources = HashMap::from([
        (
            builtin_path.to_string_lossy().into_owned(),
            String::from_utf8(builtin_source.to_vec())?,
        ),
        (
            shadow_path.to_string_lossy().into_owned(),
            String::from_utf8(shadow_source.to_vec())?,
        ),
    ]);

    let builtin_resolved = compass_resolve::resolve(&[builtin], &sources);
    assert!(call_edges(&builtin_resolved).is_empty());
    let shadow_resolved = compass_resolve::resolve(&[shadow], &sources);
    assert!(
        call_edges(&shadow_resolved).iter().any(|edge| {
            shadow_resolved
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.label() == "len()")
        }),
        "nodes={:?} edges={:?}",
        shadow_resolved.nodes,
        shadow_resolved.edges
    );
    Ok(())
}

#[test]
fn javascript_expanded_builtin_globals_do_not_publish_external_hubs() -> Result<(), Box<dyn Error>>
{
    let path = Path::new("src/builtins.js");
    let source =
        b"export function run(value) { Uint8Array(value); setTimeout(run, 0); fetch(value); }\n";
    let extracted = Engine::default().extract_source(path, source)?;
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
