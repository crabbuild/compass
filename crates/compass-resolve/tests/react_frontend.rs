use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex};
use compass_resolve::resolve_with_root;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn react_project_publishes_roles_and_occurrence_preserving_renders() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"name":"fixture","dependencies":{"react":"19.0.0","react-dom":"19.0.0"}}"#,
    )
    .expect("package manifest");
    let source_path = root.join("src/App.tsx");
    let source = br#"import { useState } from "react";

export function Button() { return <button />; }
export function useOrders() { return useState(false); }
export function useNotHook() { return helper(); }
function helper() { return null; }
export function App() {
  const [open, setOpen] = useState(false);
  return <><Button /><Button /></>;
}
"#;
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/App.tsx", source)
        .expect("react extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.kind == "ui_role"
                    && domain.detail.get("role").and_then(Value::as_str)
                        == Some("ui_component")
        )
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.kind == "ui_role"
                    && domain.name == "useOrders"
                    && domain.detail.get("role").and_then(Value::as_str) == Some("hook")
        )
    }));
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.kind == "ui_role"
                    && domain.name == "useNotHook"
                    && domain.detail.get("role").and_then(Value::as_str) == Some("hook")
        )
    }));

    let sources = HashMap::from([(
        "src/App.tsx".to_owned(),
        String::from_utf8(source.to_vec()).expect("utf8 fixture"),
    )]);
    let resolved = resolve_with_root(&[extraction.clone()], &sources, root);
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);

    let render_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "renders")
        .collect::<Vec<_>>();
    assert_eq!(render_edges.len(), 2, "two Button JSX occurrences");
    assert!(render_edges.iter().all(|edge| {
        edge.string("context") == "jsx"
            && edge.string("render_kind") == "jsx"
            && edge.string("source_file") == "src/App.tsx"
    }));
    let mut anchors = render_edges
        .iter()
        .map(|edge| {
            (
                edge.string("start_byte").parse::<usize>().expect("start"),
                edge.string("end_byte").parse::<usize>().expect("end"),
            )
        })
        .collect::<Vec<_>>();
    anchors.sort_unstable();
    let first = source
        .windows(b"<Button />".len())
        .position(|window| window == b"<Button />")
        .expect("first Button occurrence");
    let second = source
        .windows(b"<Button />".len())
        .enumerate()
        .find_map(|(index, window)| (index > first && window == b"<Button />").then_some(index))
        .expect("second Button occurrence");
    assert_eq!(
        anchors,
        vec![
            (first + 1, first + 1 + b"Button".len()),
            (second + 1, second + 1 + b"Button".len()),
        ]
    );
    let button_id = render_edges[0].target.clone();
    let button = resolved
        .nodes
        .iter()
        .find(|node| node.id == button_id)
        .expect("render target node");
    assert!(
        button
            .attributes
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|roles| roles.iter().any(|role| role == "ui_component"))
    );
    let resolved_again = resolve_with_root(&[extraction], &sources, root);
    assert_eq!(
        serde_json::to_vec(&resolved).expect("serialize first resolution"),
        serde_json::to_vec(&resolved_again).expect("serialize second resolution")
    );
}

#[test]
fn jsx_in_a_non_react_runtime_does_not_activate_react_semantics() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"name":"fixture","dependencies":{"preact":"10.0.0"}}"#,
    )
    .expect("package manifest");
    let source_path = root.join("src/App.tsx");
    let source = br#"export function App() { return <div>not React</div>; }
"#;
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/App.tsx", source)
        .expect("preact extraction");
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain) if domain.kind == "ui_role"
        )
    }));
}

#[test]
fn react_client_directives_mark_module_components_and_server_directives_stay_export_scoped() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"name":"fixture","dependencies":{"react":"19.0.0"}}"#,
    )
    .expect("package manifest");
    let client_path = root.join("src/Client.tsx");
    let client_source = br#"'use client';
export function Client() { return <div />; }
function Private() { return <span />; }
"#;
    fs::create_dir_all(client_path.parent().expect("source parent")).expect("source directory");
    fs::write(&client_path, client_source).expect("client source");

    let server_path = root.join("src/actions.ts");
    let server_source = br#"'use server';
export async function save() { return true; }
function helper() { return false; }
"#;
    fs::write(&server_path, server_source).expect("server source");

    let project = ProjectEvidenceIndex::build(root, &[client_path.clone(), server_path.clone()]);
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let client = engine
        .extract_source_graph_only(&client_path, "src/Client.tsx", client_source)
        .expect("client extraction");
    assert!(client.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.name == "Client"
                    && domain.detail.get("role").and_then(Value::as_str)
                        == Some("client_component")
        )
    }));
    assert!(client.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.name == "Private"
                    && domain.detail.get("role").and_then(Value::as_str)
                        == Some("client_component")
        )
    }));

    let server = engine
        .extract_source_graph_only(&server_path, "src/actions.ts", server_source)
        .expect("server extraction");
    assert!(server.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.name == "save"
                    && domain.detail.get("role").and_then(Value::as_str)
                        == Some("server_function")
        )
    }));
    assert!(!server.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Domain(domain)
                if domain.name == "helper"
                    && domain.detail.get("role").and_then(Value::as_str)
                        == Some("server_function")
        )
    }));
}

#[test]
fn react_factory_calls_publish_create_element_and_root_renders() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react":"19.0.0","react-dom":"19.0.0"}}"#,
    )
    .expect("manifest");
    let path = root.join("src/App.tsx");
    let source = br#"import React, { lazy } from 'react';
import { createRoot } from 'react-dom/client';
function Widget() { return null; }
export function App() { return React.createElement(Widget, { value: 1 }); }
const LazyWidget = lazy(() => import('./Widget'));
createRoot(document.getElementById('root')!).render(<App />);
"#;
    fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    fs::write(&path, source).expect("source");
    let widget_path = root.join("src/Widget.tsx");
    let widget_source = b"export default function Widget() { return null; }\n";
    fs::write(&widget_path, widget_source).expect("widget source");
    let project = ProjectEvidenceIndex::build(root, &[path.clone(), widget_path.clone()]);
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/App.tsx", source)
        .expect("extract");
    let widget_extraction = engine
        .extract_source_graph_only(&widget_path, "src/Widget.tsx", widget_source)
        .expect("widget extract");
    let resolved = resolve_with_root(
        &[extraction, widget_extraction],
        &HashMap::from([
            (
                "src/App.tsx".to_owned(),
                String::from_utf8(source.to_vec()).unwrap(),
            ),
            (
                "src/Widget.tsx".to_owned(),
                String::from_utf8(widget_source.to_vec()).unwrap(),
            ),
        ]),
        root,
    );
    let renders = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "renders")
        .collect::<Vec<_>>();
    assert!(
        renders
            .iter()
            .any(|edge| edge.string("render_kind") == "create_element")
    );
    assert!(
        renders
            .iter()
            .any(|edge| edge.string("render_kind") == "root")
    );
    assert!(
        renders
            .iter()
            .any(|edge| edge.string("render_kind") == "lazy")
    );
}
