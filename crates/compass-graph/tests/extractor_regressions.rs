use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, NodeKind};

type TestResult = Result<(), Box<dyn Error>>;

fn publish(
    path: &std::path::Path,
    root: &std::path::Path,
) -> Result<compass_model::code_graph::GraphDocument, Box<dyn Error>> {
    let extraction = Engine::default().extract(path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    Ok(normalize_document_v1(&flexible, root, "sha256:test", None)?)
}

fn has_no_publication_diagnostics(graph: &compass_model::code_graph::GraphDocument) {
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| { !diagnostic.code.starts_with("publication_") })
    );
}

#[test]
fn empty_bash_script_publishes_inventory_only_and_nonempty_scripts_keep_calls() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("script.sh");
    fs::write(&path, "")?;

    let graph = publish(&path, root)?;
    has_no_publication_diagnostics(&graph);
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].kind, NodeKind::File);
    assert!(graph.links.is_empty());

    fs::write(&path, "hello() { echo hello; }\nhello\n")?;
    let graph = publish(&path, root)?;
    has_no_publication_diagnostics(&graph);
    let function = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Function && node.name == "hello()")
        .ok_or("non-empty script function was not published")?;
    assert!(
        graph
            .links
            .iter()
            .any(|edge| { edge.kind == EdgeKind::Calls && edge.target == function.id })
    );
    Ok(())
}

#[test]
fn r_nested_function_identity_preserves_exact_names_scopes_calls_and_stability() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("helpers.R");
    let source = r#"left <- function() {
  helper <- function() { 1 }
  helper()
}
right <- function() {
  helper <- function() { 2 }
  helper()
}
outer <- function() {
  Foo <- function() { 3 }
  foo <- function() { 4 }
  format.result <- function() { 5 }
  format_result <- function() { 6 }
  Foo()
  foo()
  format.result()
  format_result()
}
"#;

    let graph_for = |contents: &str| -> Result<_, Box<dyn Error>> {
        fs::write(&path, contents)?;
        let graph = publish(&path, root)?;
        has_no_publication_diagnostics(&graph);
        let functions = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Function)
            .map(|node| (node.qualified_name.clone(), node.id.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(functions.len(), 9);
        Ok((graph, functions))
    };

    let (graph, functions) = graph_for(source)?;
    for qualified_name in [
        "left()",
        "left()::helper()",
        "right()",
        "right()::helper()",
        "outer()",
        "outer()::Foo()",
        "outer()::foo()",
        "outer()::format.result()",
        "outer()::format_result()",
    ] {
        assert!(
            functions.contains_key(qualified_name),
            "missing {qualified_name}"
        );
    }
    for (caller, callee) in [
        ("left()", "left()::helper()"),
        ("right()", "right()::helper()"),
        ("outer()", "outer()::Foo()"),
        ("outer()", "outer()::foo()"),
        ("outer()", "outer()::format.result()"),
        ("outer()", "outer()::format_result()"),
    ] {
        assert!(
            graph.links.iter().any(|edge| {
                edge.kind == EdgeKind::Calls
                    && edge.source == functions[caller]
                    && edge.target == functions[callee]
            }),
            "missing exact call {caller} -> {callee}"
        );
    }

    let shifted = format!("# stable identity\n{source}");
    let (_, shifted_functions) = graph_for(&shifted)?;
    assert_eq!(shifted_functions, functions);
    Ok(())
}
