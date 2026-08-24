use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{NodeDetails, NodeKind};

#[test]
fn repeated_markdown_headings_use_stable_hierarchical_identities() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("docs/cookbook.md");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = r#"# Cookbook
## Recipe 1
### Problem
First problem.
## Recipe 2
### Problem
Second problem.
"#;
    fs::write(&path, source)?;

    let identities = |path: &std::path::Path| -> Result<BTreeMap<String, String>, Box<dyn Error>> {
        let extraction = Engine::default().extract(path)?;
        let flexible = build_from_extraction(&extraction, true, Some(root));
        let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
        let uris = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Resource && node.name == "Problem")
            .map(|node| {
                let uri = match node.details.as_ref() {
                    Some(NodeDetails::Resource(resource)) => resource.uri.clone(),
                    _ => None,
                };
                (node.qualified_name.clone(), uri)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            uris,
            BTreeMap::from([
                (
                    "Cookbook::Recipe 1::Problem".to_owned(),
                    Some("#problem".to_owned())
                ),
                (
                    "Cookbook::Recipe 2::Problem".to_owned(),
                    Some("#problem-1".to_owned())
                ),
            ])
        );
        Ok(graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Resource && node.name == "Problem")
            .map(|node| (node.qualified_name.clone(), node.id.clone()))
            .collect())
    };

    let before = identities(&path)?;
    assert_eq!(
        before.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Cookbook::Recipe 1::Problem", "Cookbook::Recipe 2::Problem"]
    );

    fs::write(&path, format!("Introductory text.\n\n{source}"))?;
    let after = identities(&path)?;
    assert_eq!(after, before);
    Ok(())
}
