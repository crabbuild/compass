use std::collections::BTreeSet;
use std::error::Error;

use compass_ir::{Capability, CoverageState, OperationKind};
use compass_languages::TreeSitterSyntaxProvider;
use compass_program::{FileInput, SyntaxProvider};

#[test]
fn rust_provider_emits_conservative_program_evidence() -> Result<(), Box<dyn Error>> {
    let source = br#"
fn work() {}
async fn load(state: &mut State) {
    state.value = 1;
    work();
    service.fetch().await;
    if state.ready { panic!("bad"); }
}
"#;
    let batch = TreeSitterSyntaxProvider::default()
        .analyze_file(FileInput {
            source_file: "src/lib.rs",
            language: "rust",
            source,
        })?
        .ok_or("missing Rust batch")?;
    let module = &batch.modules[0];
    assert_eq!(module.functions.len(), 2);
    let load = module
        .functions
        .iter()
        .find(|function| function.name == "load")
        .ok_or("missing load")?;
    let operations = load
        .blocks
        .iter()
        .flat_map(|block| &block.operations)
        .collect::<Vec<_>>();
    assert!(operations.iter().any(|operation| {
        matches!(
            &operation.kind,
            OperationKind::Call { callee, .. } if callee == "work"
        )
    }));
    assert!(operations.iter().any(|operation| {
        matches!(&operation.kind, OperationKind::Write { path } if path == "state.value")
    }));
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation.kind, OperationKind::Await))
    );
    assert!(matches!(
        load.coverage.get(&Capability::ControlFlow),
        Some(CoverageState::Partial { reasons })
            if reasons.iter().any(|reason| reason == "branch_sensitive_cfg")
    ));
    Ok(())
}

#[test]
fn typescript_family_and_unsupported_languages_dispatch() -> Result<(), Box<dyn Error>> {
    for (path, expected) in [
        ("sample.ts", "typescript"),
        ("sample.mts", "typescript"),
        ("sample.cts", "typescript"),
        ("sample.tsx", "tsx"),
        ("sample.js", "javascript"),
        ("sample.jsx", "javascript"),
        ("sample.mjs", "javascript"),
        ("sample.cjs", "javascript"),
    ] {
        let batch = TreeSitterSyntaxProvider::default()
            .analyze_file(FileInput {
                source_file: path,
                language: expected,
                source: b"const run = async () => { await work(); };",
            })?
            .ok_or("missing TypeScript-family batch")?;
        assert_eq!(batch.modules[0].language, expected);
        assert_eq!(batch.modules[0].functions.len(), 1);
    }
    assert!(
        TreeSitterSyntaxProvider::default()
            .analyze_file(FileInput {
                source_file: "main.go",
                language: "go",
                source: b"package main",
            })?
            .is_none()
    );
    Ok(())
}

#[test]
fn repeated_signatures_in_distinct_lexical_scopes_have_unique_syntax_symbols()
-> Result<(), Box<dyn Error>> {
    for (path, language, source) in [
        (
            "src/lib.rs",
            "rust",
            b"mod left { fn same() {} }\nmod right { fn same() {} }\n".as_slice(),
        ),
        (
            "src/app.ts",
            "typescript",
            b"namespace Left { function same() {} }\nnamespace Right { function same() {} }\n"
                .as_slice(),
        ),
    ] {
        let batch = TreeSitterSyntaxProvider::default()
            .analyze_file(FileInput {
                source_file: path,
                language,
                source,
            })?
            .ok_or("missing syntax batch")?;
        let functions = &batch.modules[0].functions;
        assert_eq!(functions.len(), 2);
        assert_eq!(
            functions
                .iter()
                .map(|function| function.symbol_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            functions.len()
        );
        compass_program::merge_evidence(vec![batch])?.validate()?;
    }
    Ok(())
}

#[test]
fn typescript_functions_cross_link_to_graph_nodes_and_rust_calls_have_real_callees()
-> Result<(), Box<dyn Error>> {
    let source = b"class Worker { run() { return work(); } }\nfunction top() { return work(); }\n";
    let batch = TreeSitterSyntaxProvider::default()
        .analyze_file(FileInput {
            source_file: "src/app.ts",
            language: "typescript",
            source,
        })?
        .ok_or("missing TypeScript batch")?;
    let worker = batch.modules[0]
        .functions
        .iter()
        .find(|function| function.name == "Worker.run")
        .ok_or("missing Worker.run")?;
    let worker_id =
        compass_languages::make_id(&[&compass_languages::make_id(&["src/app", "Worker"]), "run"]);
    assert_eq!(worker.graph_node_id.as_deref(), Some(worker_id.as_str()));
    let top = batch.modules[0]
        .functions
        .iter()
        .find(|function| function.name == "top")
        .ok_or("missing top")?;
    let top_id = compass_languages::make_id(&["src/app", "top"]);
    assert_eq!(top.graph_node_id.as_deref(), Some(top_id.as_str()));

    let rust = TreeSitterSyntaxProvider::default()
        .analyze_file(FileInput {
            source_file: "src/lib.rs",
            language: "rust",
            source: b"fn run(xs: Vec<i32>) { xs.iter().map(|_| work()).collect::<Vec<_>>(); }",
        })?
        .ok_or("missing Rust batch")?;
    let callees = rust.modules[0]
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.operations)
        .filter_map(|operation| match &operation.kind {
            OperationKind::Call { callee, .. } => Some(callee.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!callees.contains(&"_"), "callees: {callees:?}");
    assert!(callees.contains(&"map"), "callees: {callees:?}");
    assert!(callees.contains(&"collect"), "callees: {callees:?}");
    Ok(())
}
