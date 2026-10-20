use std::collections::HashMap;
use std::error::Error;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::EdgeKind;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_resolve::resolve_with_root;
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    kind: String,
    line: u32,
}

#[derive(Deserialize)]
struct Case {
    prefix: String,
    kind: String,
    owner: String,
    target: String,
    relation: String,
    expected: Option<Expected>,
    typescript: String,
    javascript: String,
}

fn terminal(name: &str) -> &str {
    name.trim_start_matches('.').trim_end_matches("()")
}

fn check_graphs(ambiguous: bool) -> Result<(), Box<dyn Error>> {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../fixtures/typescript_member_scopes.json"
    ))?;
    let directory = tempfile::tempdir()?;
    let mut engine = Engine::default();
    for case in cases {
        if (case.kind == "ambiguous") != ambiguous {
            continue;
        }
        for (extension, source) in [("ts", &case.typescript), ("js", &case.javascript)] {
            for seed in 0..24 {
                let file = format!("{}{:02}.{extension}", case.prefix, seed);
                let path = directory.path().join(&file);
                std::fs::write(&path, source)?;
                let extraction =
                    engine.extract_source_graph_only(&path, &file, source.as_bytes())?;
                assert!(extraction.error.is_none(), "{file}: {:?}", extraction.error);
                let sources =
                    HashMap::from([(path.to_string_lossy().into_owned(), source.clone())]);
                let resolved = resolve_with_root(&[extraction], &sources, directory.path());
                let evidence = BuildEvidence::from_extraction(
                    directory.path(),
                    &resolved,
                    "sha256:primary-member-scope-fixture",
                )?;
                let graph = normalize_v1(resolved, evidence)?;
                compass_model::validate_code_graph(&graph)?;
                let kind = match case.relation.as_str() {
                    "calls" => EdgeKind::Calls,
                    "accesses_member" => EdgeKind::References,
                    _ => return Err("unsupported fixture relation".into()),
                };
                let matches = graph
                    .links
                    .iter()
                    .filter(|edge| edge.kind == kind)
                    .filter_map(|edge| {
                        let owner = graph.nodes.iter().find(|node| node.id == edge.source)?;
                        let target = graph.nodes.iter().find(|node| node.id == edge.target)?;
                        (terminal(&owner.name) == case.owner
                            && terminal(&target.name) == case.target)
                            .then_some((owner, edge, target))
                    })
                    .collect::<Vec<_>>();
                let Some(expected) = &case.expected else {
                    assert!(matches.is_empty(), "{file}: invented accessor target");
                    continue;
                };
                assert_eq!(matches.len(), 1, "{file}: occurrence multiplicity");
                let (owner, edge, target) = matches[0];
                assert_eq!(target.kind.as_str(), expected.kind, "{file}");
                let target_source = target.source.as_ref().ok_or("missing target source")?;
                assert_eq!(target_source.file, file);
                assert_eq!(target_source.start_line, expected.line, "{file}");
                let site = edge
                    .relationship_site
                    .as_ref()
                    .ok_or("missing occurrence")?;
                let owner_source = owner.source.as_ref().ok_or("missing owner source")?;
                assert_eq!(site.file, file);
                assert!(owner_source.start_byte <= site.start_byte);
                assert!(site.end_byte <= owner_source.end_byte);
                let snippet = source
                    .get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?)
                    .ok_or("invalid occurrence bytes")?;
                assert!(snippet.contains(&case.target), "{file}: {snippet}");
                assert!(edge.occurrence_rule.is_some());
                assert!(edge.evidence.iter().any(|proof| {
                    proof.origin == EvidenceOrigin::Ast
                        && proof.confidence == EvidenceConfidence::Exact
                }));
            }
        }
    }
    Ok(())
}

#[test]
fn ambiguous_accessors_never_publish_hash_selected_references() -> Result<(), Box<dyn Error>> {
    check_graphs(true)
}

#[test]
fn primary_member_scopes_preserve_published_call_and_reference_proofs() -> Result<(), Box<dyn Error>>
{
    check_graphs(false)
}
