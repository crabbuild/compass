use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

use compass_graph::{Communities, GodNode};
use compass_model::GraphDocument;
use compass_output::{WikiOptions, export_wiki};
use serde_json::json;

#[test]
fn directed_wiki_preserves_incoming_outgoing_and_self_loop_evidence() -> Result<(), Box<dyn Error>>
{
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": true,
        "graph": {},
        "nodes": [
            {"id":"caller","label":"Caller","source_file":"src/caller.rs"},
            {"id":"external","label":"External","source_file":"src/external.rs"},
            {"id":"target","label":"Target","source_file":"src/target.rs"},
            {"id":"dependency","label":"Dependency","source_file":"src/dependency.rs"}
        ],
        "links": [
            {"source":"caller","target":"target","relation":"calls","confidence":"EXTRACTED"},
            {"source":"caller","target":"target","relation":"calls","confidence":"EXTRACTED"},
            {"source":"external","target":"target","relation":"calls","confidence":"INFERRED"},
            {"source":"target","target":"dependency","relation":"uses","confidence":"EXTRACTED"},
            {"source":"target","target":"target","relation":"recurs","confidence":"EXTRACTED"}
        ]
    }))?;
    let communities = Communities::from([
        (0, vec!["target".to_owned()]),
        (1, vec!["caller".to_owned(), "dependency".to_owned()]),
        (2, vec!["external".to_owned()]),
    ]);
    let labels = BTreeMap::from([
        (0, "Core".to_owned()),
        (1, "Runtime".to_owned()),
        (2, "Boundary".to_owned()),
    ]);
    let gods = vec![GodNode {
        id: "target".to_owned(),
        label: "Target".to_owned(),
        degree: 5,
    }];
    let options = WikiOptions {
        community_labels: Some(&labels),
        cohesion: None,
        god_nodes: Some(&gods),
    };
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;

    export_wiki(&document, &communities, first.path(), &options)?;
    export_wiki(&document, &communities, second.path(), &options)?;

    let community = fs::read_to_string(first.path().join("Core.md"))?;
    assert!(community.contains("[Runtime](Runtime.md) (3 shared connections)"));
    assert!(community.contains("[Boundary](Boundary.md) (1 shared connections)"));
    assert!(community.contains("- EXTRACTED: 4 (80%)"));
    assert!(community.contains("- INFERRED: 1 (20%)"));

    let god = fs::read_to_string(first.path().join("Target.md"))?;
    assert_eq!(god.matches("- ← Caller `EXTRACTED`").count(), 2);
    assert!(god.contains("- ← External `INFERRED`"));
    assert!(god.contains("- → Dependency `EXTRACTED`"));
    assert!(god.contains("- ↻ [Target](Target.md) `EXTRACTED`"));

    assert_eq!(
        directory_markdown(first.path())?,
        directory_markdown(second.path())?
    );
    Ok(())
}

#[test]
fn undirected_wiki_keeps_directionless_connection_rendering() -> Result<(), Box<dyn Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": false,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {"id":"alpha","label":"Alpha","source_file":"alpha.rs"},
            {"id":"beta","label":"Beta","source_file":"beta.rs"}
        ],
        "links": [
            {"source":"beta","target":"alpha","relation":"related","confidence":"EXTRACTED"}
        ]
    }))?;
    let communities = Communities::from([(0, vec!["alpha".to_owned(), "beta".to_owned()])]);
    let gods = vec![GodNode {
        id: "alpha".to_owned(),
        label: "Alpha".to_owned(),
        degree: 1,
    }];
    let directory = tempfile::tempdir()?;

    export_wiki(
        &document,
        &communities,
        directory.path(),
        &WikiOptions {
            community_labels: None,
            cohesion: None,
            god_nodes: Some(&gods),
        },
    )?;

    let god = fs::read_to_string(directory.path().join("Alpha.md"))?;
    assert!(god.contains("- Beta `EXTRACTED`"));
    assert!(!god.contains("← Beta"));
    assert!(!god.contains("→ Beta"));
    Ok(())
}

fn directory_markdown(path: &std::path::Path) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut files = fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let content = fs::read_to_string(entry.path())?;
            Ok((name, content))
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}
