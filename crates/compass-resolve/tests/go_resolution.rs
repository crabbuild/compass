use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, RawNodeRecord};
use serde_json::{Map, Value};

#[test]
fn cross_file_go_receiver_methods_use_the_declared_type_owner() -> Result<(), Box<dyn Error>> {
    let declaration_path = Path::new("cmd/agent/vogon.go");
    let declaration_source = b"package agent\n\ntype Agent struct{}\n";
    let methods_path = Path::new("cmd/agent/hooks.go");
    let methods_source = br#"package agent

func (a *Agent) Prepare() {}
func (a *Agent) Finish() {}
"#;

    let mut engine = Engine::default();
    let declaration = engine.extract_source(declaration_path, declaration_source)?;
    let methods = engine.extract_source(methods_path, methods_source)?;
    let sources = HashMap::from([
        (
            declaration_path.to_string_lossy().into_owned(),
            String::from_utf8(declaration_source.to_vec())?,
        ),
        (
            methods_path.to_string_lossy().into_owned(),
            String::from_utf8(methods_source.to_vec())?,
        ),
    ]);

    let resolved =
        compass_resolve::resolve_with_root(&[declaration, methods], &sources, Path::new("."));
    let owners = resolved
        .nodes
        .iter()
        .filter(|node| node.label() == "Agent")
        .collect::<Vec<_>>();

    assert_eq!(
        owners.len(),
        1,
        "receiver placeholders must be rebound before source-ID disambiguation: {:?}",
        resolved.nodes
    );
    let owner_id = &owners[0].id;
    let method_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "method")
        .collect::<Vec<_>>();
    assert_eq!(method_edges.len(), 2, "edges={:?}", resolved.edges);
    assert!(
        method_edges.iter().all(|edge| &edge.source == owner_id),
        "every receiver method must be owned by the declared type: {method_edges:?}"
    );
    assert_eq!(
        owners[0].string("source_file"),
        declaration_path.to_string_lossy()
    );
    Ok(())
}

#[test]
fn go_local_callback_is_not_exported_for_cross_file_resolution() -> Result<(), Box<dyn Error>> {
    let path = Path::new("internal/pushqueue/pushqueue.go");
    let source = br#"package pushqueue

func acquire() (func(), error) { return func() {}, nil }

func enqueue() {
    release, err := acquire()
    _ = err
    defer release()
}
"#;

    let extracted = Engine::default().extract_source(path, source)?;
    let raw_calls = extracted.raw_calls.as_deref().unwrap_or_default();
    assert!(
        raw_calls.iter().all(|call| call.callee != "release"),
        "lexically bound callbacks must stay within their callable scope: {raw_calls:?}"
    );
    Ok(())
}

#[test]
fn go_local_binding_only_shadows_calls_after_its_declaration() -> Result<(), Box<dyn Error>> {
    let path = Path::new("internal/pushqueue/shadow.go");
    let source = br#"package pushqueue

func release() {}
func enqueue() {
    release()
    release := func() {}
    release()
}
"#;

    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let release_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && resolved.nodes.iter().any(|node| {
                    node.id == edge.target
                        && node.label() == "release()"
                        && node.string("source_file") == path.to_string_lossy()
                })
        })
        .collect::<Vec<_>>();

    assert_eq!(
        release_calls.len(),
        1,
        "only the call before the short declaration targets the package function: {:?}",
        resolved.edges
    );
    assert_eq!(release_calls[0].string("source_location"), "L5");
    Ok(())
}

#[test]
fn generic_call_resolution_never_targets_file_nodes() -> Result<(), Box<dyn Error>> {
    let caller_path = Path::new("internal/pushqueue/pushqueue.go");
    let caller_source = b"package pushqueue\n\nfunc enqueue() { release() }\n";
    let mut extracted = Engine::default().extract_source(caller_path, caller_source)?;
    extracted.nodes.push(RawNodeRecord {
        id: "mise_tasks_release_file".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), Value::String("release".to_owned())),
            (
                "source_file".to_owned(),
                Value::String("mise-tasks/release".to_owned()),
            ),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("symbol_kind".to_owned(), Value::String("file".to_owned())),
        ]),
    });
    let sources = HashMap::from([(
        caller_path.to_string_lossy().into_owned(),
        String::from_utf8(caller_source.to_vec())?,
    )]);

    let resolved = compass_resolve::resolve_with_root(&[extracted], &sources, Path::new("."));
    assert!(
        resolved.edges.iter().all(|edge| {
            edge.string("relation") != "calls" || edge.target != "mise_tasks_release_file"
        }),
        "file nodes must never be selected as callable targets: {:?}",
        resolved.edges
    );
    Ok(())
}
