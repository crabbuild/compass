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
