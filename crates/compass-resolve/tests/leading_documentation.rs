use std::collections::HashMap;
use std::error::Error;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::EdgeKind;
use compass_model::provenance::{LEADING_DOCUMENTATION_RULE, is_proven_leading_documentation};
use compass_resolve::resolve_with_root;

#[test]
fn leading_documents_survive_real_extraction_resolution_and_strict_publication()
-> Result<(), Box<dyn Error>> {
    for (file, source, expected) in [
        (
            "src/lib.rs",
            "/// Retains protocol error context.\npub fn work() {}\n",
            1,
        ),
        (
            "sample.go",
            "package sample\n// Retains protocol error context.\nfunc Work() {}\n",
            1,
        ),
        (
            "sample.ts",
            "/** Retains protocol error context. */\nexport function work() {}\n",
            1,
        ),
        (
            "sample.ts",
            "/** Number overload. */\nexport function convert(input: number): number;\n/** String overload. */\nexport function convert(input: string): string;\n/** Retains protocol error context. */\nexport function convert(input: unknown): unknown { return input; }\n",
            3,
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join(file);
        std::fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
        std::fs::write(&path, source)?;
        let extraction = Engine::default().extract(&path)?;
        let resolved = resolve_with_root(
            &[extraction],
            &HashMap::from([(path.to_string_lossy().into_owned(), source.to_owned())]),
            directory.path(),
        );
        let build = BuildEvidence::from_extraction(
            directory.path(),
            &resolved,
            "sha256:leading-documentation-test",
        )?;
        let graph = normalize_v1(resolved, build)?;
        compass_model::validate_code_graph(&graph)?;
        let edges = graph
            .links
            .iter()
            .filter(|edge| {
                edge.occurrence_rule.as_ref().map(|rule| rule.as_str())
                    == Some(LEADING_DOCUMENTATION_RULE)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            edges.len(),
            expected,
            "{file}: {}",
            serde_json::to_string(&graph)?
        );
        for edge in edges {
            assert_eq!(edge.kind, EdgeKind::Documents);
            let document = graph
                .nodes
                .iter()
                .find(|node| node.id == edge.source)
                .ok_or("missing document")?;
            let owner = graph
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .ok_or("missing owner")?;
            assert!(is_proven_leading_documentation(document, owner, edge));
            let anchor = document.source.as_ref().ok_or("missing excerpt")?;
            assert_eq!(anchor.file, file);
            let excerpt = source
                .get(usize::try_from(anchor.start_byte)?..usize::try_from(anchor.end_byte)?)
                .ok_or("invalid excerpt")?;
            let Some(compass_model::code_graph::NodeDetails::Resource(details)) = &document.details
            else {
                return Err("not a resource".into());
            };
            assert_eq!(details.content.as_deref(), Some(excerpt));
            assert!(
                owner
                    .source
                    .as_ref()
                    .is_some_and(|definition| definition.start_byte >= anchor.end_byte)
            );
        }
    }
    Ok(())
}

#[test]
fn excerpt_limits_publish_a_specific_omission_warning() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("sample.rs");
    let source = format!("/** {} */\npub fn work() {{}}\n", "x".repeat(4096));
    std::fs::write(&path, &source)?;
    let extraction = Engine::default().extract(&path)?;
    let resolved = resolve_with_root(
        &[extraction],
        &HashMap::from([(path.to_string_lossy().into_owned(), source)]),
        directory.path(),
    );
    let build = BuildEvidence::from_extraction(
        directory.path(),
        &resolved,
        "sha256:omitted-documentation-test",
    )?;
    let graph = normalize_v1(resolved, build)?;
    assert!(graph.nodes.iter().any(|node| node.name == "work()"));
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .any(
                |diagnostic| diagnostic.code == "leading_documentation_omission_summary"
                    && diagnostic.message.contains("1 extraction warning records")
            )
    );
    assert!(graph.links.iter().all(
        |edge| edge.occurrence_rule.as_ref().map(|rule| rule.as_str())
            != Some(LEADING_DOCUMENTATION_RULE)
    ));
    Ok(())
}

#[test]
fn missing_or_ambiguous_definition_scope_omits_docs_with_warning() -> Result<(), Box<dyn Error>> {
    for ambiguous in [false, true] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("sample.rs");
        let source = "/// Protocol diagnostics parsed here.\npub fn work() {}\n";
        std::fs::write(&path, source)?;
        let mut extraction = Engine::default().extract(&path)?;
        let batch = extraction.semantic_evidence.as_mut().ok_or("no evidence")?;
        let owner = batch
            .declarations
            .iter()
            .find(|declaration| declaration.name == "work")
            .ok_or("no work")?
            .id
            .clone();
        if ambiguous {
            let mut duplicate = batch
                .scopes
                .iter()
                .find(|scope| scope.owner_declaration_id.as_deref() == Some(&owner))
                .ok_or("no definition scope")?
                .clone();
            duplicate.id.push_str(":duplicate");
            batch.scopes.push(duplicate);
        } else {
            batch
                .scopes
                .retain(|scope| scope.owner_declaration_id.as_deref() != Some(&owner));
        }
        let resolved = resolve_with_root(
            &[extraction],
            &HashMap::from([(path.to_string_lossy().into_owned(), source.to_owned())]),
            directory.path(),
        );
        let build = BuildEvidence::from_extraction(
            directory.path(),
            &resolved,
            "sha256:missing-doc-owner-test",
        )?;
        let graph = normalize_v1(resolved, build)?;
        assert!(graph.nodes.iter().any(|node| node.name == "work()"));
        assert!(
            graph
                .graph
                .diagnostics
                .iter()
                .any(
                    |diagnostic| diagnostic.code == "leading_documentation_omission_summary"
                        && diagnostic.message.contains("1 excerpts lack a unique")
                )
        );
        assert!(graph.links.iter().all(|edge| {
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str())
                != Some(LEADING_DOCUMENTATION_RULE)
        }));
    }
    Ok(())
}
