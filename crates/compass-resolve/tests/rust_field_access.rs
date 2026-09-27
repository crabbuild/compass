use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_resolve::resolve;

fn extract(file: &str, source: &str) -> Result<Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(file), source.as_bytes())?)
}

#[test]
fn cross_file_field_accesses_publish_exact_occurrences_and_preserve_order()
-> Result<(), Box<dyn Error>> {
    let model = "pub struct Item { pub value: usize }\n";
    let caller =
        "use crate::model::Item;\nfn run(item: &mut Item) {\n    item.value = item.value + 1;\n}\n";
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(directory.path().join("src/model.rs"), model)?;
    std::fs::write(directory.path().join("src/lib.rs"), caller)?;
    let sources = HashMap::from([
        ("src/model.rs".into(), model.into()),
        ("src/lib.rs".into(), caller.into()),
    ]);
    let inputs = vec![
        extract("src/model.rs", model)?,
        extract("src/lib.rs", caller)?,
    ];
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let build = BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:field-access")?;
    let graph = normalize_v1(resolved, build)?;
    let run = graph
        .nodes
        .iter()
        .find(|n| n.name == "run()")
        .ok_or("missing run")?;
    let field = graph
        .nodes
        .iter()
        .find(|n| n.name == "value" && n.kind == NodeKind::Field)
        .ok_or("missing field")?;
    let edges = graph
        .links
        .iter()
        .filter(|e| e.source == run.id && e.target == field.id)
        .collect::<Vec<_>>();
    assert_eq!(edges.len(), 2, "{:#?}", graph.links);
    let mut starts = BTreeSet::new();
    for edge in edges {
        assert_eq!(edge.kind, EdgeKind::References);
        assert!(
            edge.occurrence_rule
                .as_ref()
                .is_some_and(|rule| rule.as_str().starts_with("universal-member-access-"))
        );
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing occurrence")?;
        assert_eq!(site.file, "src/lib.rs");
        assert_eq!(site.start_line, 3);
        let start = usize::try_from(site.start_byte)?;
        let end = usize::try_from(site.end_byte)?;
        assert_eq!(caller.get(start..end), Some("value"));
        assert!(starts.insert(start));
        assert!(edge.evidence.iter().any(|e| e.anchors.contains(site)));
    }
    let mut reversed = inputs;
    reversed.reverse();
    let second = resolve(&reversed, &sources);
    let build = BuildEvidence::from_extraction(directory.path(), &second, "sha256:field-access")?;
    let second = normalize_v1(second, build)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&second.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&second.links)?
    );
    Ok(())
}

#[test]
fn shadowed_unknown_and_duplicate_field_targets_never_publish_a_convenient_match()
-> Result<(), Box<dyn Error>> {
    for source in [
        "struct Item { value: usize } fn run(item: &Item) { let item = unknown(); let _ = item.value; }",
        "struct Item { value: usize } fn run(item: Unknown) { let _ = item.value; }",
        "struct Item { value: usize, value: usize } fn run(item: &Item) { let _ = item.value; }",
        "struct Item { value: usize } fn run(item: &Item) { let _ = (|item| item.value)(unknown()); }",
        "mod other { pub struct Item { pub value: usize } } use other::*; fn run(item: Unknown) { let _ = item.value; }",
    ] {
        let resolved = resolve(
            &[extract("src/lib.rs", source)?],
            &HashMap::from([("src/lib.rs".into(), source.into())]),
        );
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        assert!(
            !resolved
                .edges
                .iter()
                .any(|e| e.string("relation") == "accesses"),
            "{source}: {:#?}",
            resolved.edges
        );
    }
    Ok(())
}

#[test]
fn nested_receivers_publish_field_contacts_but_never_method_contacts() -> Result<(), Box<dyn Error>>
{
    let source = "struct Item { value: usize } impl Item { fn get(&self) -> usize { self.value } } struct State { item: Item } impl State { fn run(&self) { self.item.get(); } }";
    let resolved = resolve(
        &[extract("src/lib.rs", source)?],
        &HashMap::from([("src/lib.rs".into(), source.into())]),
    );
    let fields = resolved
        .nodes
        .iter()
        .filter(|n| n.string("symbol_kind") == "field")
        .map(|n| n.id.as_str())
        .collect::<BTreeSet<_>>();
    let accesses = resolved
        .edges
        .iter()
        .filter(|e| e.string("relation") == "accesses")
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 2, "{:#?}", resolved.edges);
    for edge in accesses {
        assert!(fields.contains(edge.target.as_str()));
    }
    assert!(
        resolved
            .edges
            .iter()
            .any(|e| e.string("relation") == "calls")
    );
    Ok(())
}
