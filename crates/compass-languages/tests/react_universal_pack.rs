use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex, RawFrameworkFact};
use tempfile::tempdir;

#[test]
fn react_client_modules_mark_private_components_as_client_boundaries() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/client.tsx");
    let source = br#""use client";
import * as React from 'react';

export function PublicCard() {
  return <PrivateRow />;
}

function PrivateRow() {
  return <span>row</span>;
}
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react":"19.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/client.tsx", source)
        .expect("React extraction");

    let boundaries = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain)
                if domain.framework == "react"
                    && domain.kind == "ui_role"
                    && domain.detail.get("role").and_then(|value| value.as_str())
                        == Some("client_boundary") =>
            {
                Some(domain.name.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(boundaries.len(), 2);
    assert!(boundaries.iter().any(|name| name == "PublicCard"));
    assert!(boundaries.iter().any(|name| name == "PrivateRow"));
}

#[test]
fn alternate_jsx_runtime_does_not_activate_react_from_an_incidental_dependency() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/card.tsx");
    let source = br#"export function Card() { return <article />; }"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react":"19.0.0","preact":"10.0.0"}}"#,
    )
    .expect("package manifest");
    fs::write(
        root.join("tsconfig.json"),
        br#"{"compilerOptions":{"jsx":"react-jsx","jsxImportSource":"preact"}}"#,
    )
    .expect("TypeScript configuration");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/card.tsx", source)
        .expect("TSX extraction");
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.framework == "react")
    }));
}

#[test]
fn npm_alias_to_preact_does_not_activate_react() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/card.tsx");
    let source = br#"import React from 'react';
export function Card() { return <article />; }"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react":"npm:preact@10.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/card.tsx", source)
        .expect("TSX extraction");
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.framework == "react")
    }));
}

#[test]
fn remix_workspace_dependency_activates_react_roles_in_nested_package() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("demos/bookstore/app/ui/card.tsx");
    let source = br#"export function Card() { return <article />; }"#;
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(
        root.join("demos/bookstore/package.json"),
        br#"{"dependencies":{"remix":"workspace:*"}}"#,
    )
    .expect("package manifest");
    fs::write(
        root.join("demos/bookstore/tsconfig.json"),
        br#"{"compilerOptions":{"jsx":"react-jsx","jsxImportSource":"remix/ui"}}"#,
    )
    .expect("TypeScript configuration");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let evidence = project.evidence_for(&source_path);
    assert!(
        evidence.has_dependency("remix"),
        "dependencies: {:?}",
        evidence.dependencies()
    );
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "demos/bookstore/app/ui/card.tsx", source)
        .expect("TSX extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.framework == "react")
    }));
}
