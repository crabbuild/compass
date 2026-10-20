use std::{collections::HashMap, error::Error};

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::NodeKind;
use compass_model::provenance::EvidenceOrigin;
use compass_resolve::resolve_with_root;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    extensions: Vec<String>,
    source: String,
    definition: String,
}

#[test]
fn initializer_source_envelopes_preserve_identity_and_relationship_records()
-> Result<(), Box<dyn Error>> {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../fixtures/callable_initializer_definitions.json"
    ))?;
    for (index, case) in cases.iter().enumerate() {
        for extension in &case.extensions {
            for seed in ["alpha", "zeta"] {
                let file = format!("{seed}_{index}.{extension}");
                let directory = tempfile::tempdir()?;
                let path = directory.path().join(&file);
                std::fs::write(&path, &case.source)?;
                let extraction = Engine::default().extract_source_graph_only(
                    &path,
                    &file,
                    case.source.as_bytes(),
                )?;
                assert!(extraction.error.is_none(), "{file}: {:?}", extraction.error);
                let sources =
                    HashMap::from([(path.to_string_lossy().into_owned(), case.source.clone())]);
                let resolved = resolve_with_root(&[extraction], &sources, directory.path());
                let mut previous = resolved.clone();
                for node in &mut previous.nodes {
                    node.attributes.remove("source_anchor");
                    node.attributes
                        .remove(compass_model::provenance::NODE_PROVENANCE_ANCHOR_ATTRIBUTE);
                }
                let publish = |raw: compass_languages::Extraction| -> Result<_, Box<dyn Error>> {
                    let evidence = BuildEvidence::from_extraction(
                        directory.path(),
                        &raw,
                        "sha256:initializer-fixture",
                    )?;
                    let graph = normalize_v1(raw, evidence)?;
                    compass_model::validate_code_graph(&graph)?;
                    Ok(graph)
                };
                let graph = publish(resolved)?;
                let previous = publish(previous)?;
                let targets = graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.kind == NodeKind::Function
                            && node.name.trim_end_matches("()") == "opaque"
                    })
                    .collect::<Vec<_>>();
                assert_eq!(targets.len(), 1, "{file}: ambiguous fixture declaration");
                let target = targets[0];
                let source = target.source.as_ref().ok_or("missing callable source")?;
                let start = case
                    .source
                    .find(&case.definition)
                    .ok_or("missing definition")?;
                assert_eq!(source.start_byte as usize, start, "{file}");
                assert_eq!(
                    source.end_byte as usize,
                    start + case.definition.len(),
                    "{file}: missing initializer body"
                );
                assert_eq!(
                    &case.source[source.start_byte as usize..source.end_byte as usize],
                    case.definition,
                    "{file}"
                );
                assert!(
                    target
                        .evidence
                        .iter()
                        .any(|proof| proof.origin == EvidenceOrigin::Ast
                            && proof
                                .anchors
                                .iter()
                                .any(|anchor| anchor.start_byte as usize == start
                                    && anchor.end_byte as usize == start + "opaque".len())),
                    "{file}: binding provenance changed"
                );
                assert_eq!(
                    graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
                    previous
                        .nodes
                        .iter()
                        .map(|node| &node.id)
                        .collect::<Vec<_>>(),
                    "{file}: definition envelope changed stable identities"
                );
                assert_eq!(
                    graph.links, previous.links,
                    "{file}: definition envelope changed relationship occurrences"
                );
            }
        }
    }
    Ok(())
}
