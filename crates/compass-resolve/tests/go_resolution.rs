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
fn qualified_go_embeddings_bind_packages_without_cross_module_name_joins()
-> Result<(), Box<dyn Error>> {
    let agent_path = Path::new("cmd/agent/agent.go");
    let agent_source = b"package agent\n\ntype Agent interface { Run() }\n";
    let wrapper_path = Path::new("cmd/client/wrapper.go");
    let wrapper_source = br#"package client

import "example.com/project/cmd/agent"

type Wrapper interface {
    agent.Agent
}
"#;
    let context_path = Path::new("internal/contexts/context.go");
    let context_source = b"package contexts\n\ntype Context struct{}\n";
    let caller_path = Path::new("cmd/client/run.go");
    let caller_source = br#"package client

import "context"

func Run(ctx context.Context) {}
"#;
    let external_path = Path::new("cmd/external/wrapper.go");
    let external_source = br#"package external

import "other.example/agent"

type External interface {
    agent.Agent
}
"#;

    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(agent_path, agent_source)?,
        engine.extract_source(wrapper_path, wrapper_source)?,
        engine.extract_source(context_path, context_source)?,
        engine.extract_source(caller_path, caller_source)?,
        engine.extract_source(external_path, external_source)?,
    ];
    let sources = HashMap::from([
        (
            agent_path.to_string_lossy().into_owned(),
            String::from_utf8(agent_source.to_vec())?,
        ),
        (
            wrapper_path.to_string_lossy().into_owned(),
            String::from_utf8(wrapper_source.to_vec())?,
        ),
        (
            context_path.to_string_lossy().into_owned(),
            String::from_utf8(context_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
        (
            external_path.to_string_lossy().into_owned(),
            String::from_utf8(external_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, Path::new("."));

    let agent = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Agent" && node.string("source_file") == agent_path.to_string_lossy()
        })
        .ok_or("missing Agent definition")?;
    let embedding = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "embeds"
                && edge.string("source_file") == wrapper_path.to_string_lossy()
        })
        .ok_or("missing embedding")?;
    assert_eq!(
        embedding.target, agent.id,
        "nodes={:#?} embedding={embedding:#?}",
        resolved.nodes
    );

    let local_context = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Context"
                && node.string("source_file") == context_path.to_string_lossy()
        })
        .ok_or("missing repository Context")?;
    let external_context = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "context.Context"
                && node.string("source_file").is_empty()
        })
        .ok_or("missing qualified standard-library Context")?;
    assert_ne!(external_context.id, local_context.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references" && edge.target == external_context.id
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("source_file") != caller_path.to_string_lossy()
            || edge.string("relation") != "references"
            || edge.target != local_context.id
    }));
    let external_agent = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "other.example/agent.Agent")
        .ok_or("missing path-qualified external Agent")?;
    assert_ne!(external_agent.id, agent.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("source_file") == external_path.to_string_lossy()
            && edge.string("relation") == "embeds"
            && edge.target == external_agent.id
    }));
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
