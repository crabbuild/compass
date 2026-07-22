use std::error::Error;
use std::fs;

use compass_global::{GlobalError, GlobalPaths, global_add, global_list, global_remove};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

fn paths(root: &std::path::Path) -> GlobalPaths {
    let directory = root.join("global");
    GlobalPaths {
        graph: directory.join("global-graph.json"),
        manifest: directory.join("global-manifest.json"),
        directory,
    }
}

fn write_graph(
    path: &std::path::Path,
    local_id: &str,
    label: &str,
    external_label: &str,
) -> Result<(), Box<dyn Error>> {
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "directed":true,
            "multigraph":true,
            "graph":{"project":"雪😀"},
            "nodes":[
                {"id":local_id,"label":label,"source_file":format!("src/{local_id}.rs")},
                {"id":"external","label":external_label,"source_file":""}
            ],
            "links":[
                {"source":local_id,"target":"external","relation":"imports","key":"one"},
                {"source":"external","target":local_id,"relation":"documents","key":"two"},
                {"source":local_id,"target":local_id,"relation":"self","key":"self"}
            ]
        }))?,
    )?;
    Ok(())
}

#[test]
fn global_store_recovers_manifests_replaces_repos_and_deduplicates_external_nodes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = paths(directory.path());
    fs::create_dir_all(&paths.directory)?;
    fs::write(&paths.manifest, "not json")?;
    let recovered = global_list(&paths);
    assert_eq!(recovered.warnings.len(), 1);
    assert_eq!(recovered.value["version"], 1);
    assert!(fs::read_dir(&paths.directory)?.any(|entry| {
        entry
            .ok()
            .is_some_and(|entry| entry.file_name().to_string_lossy().contains(".corrupt."))
    }));

    let first = directory.path().join("first.json");
    let second = directory.path().join("second.json");
    write_graph(&first, "one", "One", "Shared API")?;
    write_graph(&second, "two", "雪😀", "Shared API")?;
    let now = OffsetDateTime::UNIX_EPOCH + Duration::microseconds(123_456);
    let added = global_add(&paths, &first, "repo", now)?;
    assert_eq!(added.nodes_added, 2);
    assert_eq!(added.nodes_removed, 0);
    assert!(!added.skipped);
    let skipped = global_add(&paths, &first, "repo", now)?;
    assert!(skipped.skipped);

    let replaced = global_add(&paths, &second, "repo", now)?;
    assert_eq!(replaced.nodes_removed, 2);
    assert_eq!(replaced.warnings.len(), 1);
    assert!(replaced.warnings[0].contains("previously pointed"));
    let third = global_add(&paths, &first, "other", now)?;
    assert_eq!(third.nodes_added, 1, "external node is reused globally");

    let graph: Value = serde_json::from_slice(&fs::read(&paths.graph)?)?;
    assert_eq!(graph["directed"], false);
    assert_eq!(graph["multigraph"], false);
    let nodes = graph["nodes"].as_array().ok_or("nodes")?;
    assert_eq!(nodes.len(), 3);
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node["label"] == "Shared API")
            .count(),
        1
    );
    assert!(graph["links"].as_array().is_some_and(|links| {
        !links.iter().any(|edge| edge.get("key").is_some())
            && !links.iter().any(|edge| edge["source"] == edge["target"])
    }));
    let encoded = fs::read_to_string(&paths.graph)?;
    assert!(encoded.contains("\\u96ea"));
    assert!(encoded.contains("\\ud83d\\ude00"));
    let manifest = global_list(&paths);
    assert!(manifest.warnings.is_empty());
    assert!(
        manifest.value["repos"]["repo"]["added_at"]
            .as_str()
            .is_some_and(|value| value.contains(".123456+00:00"))
    );

    let (removed, warnings) = global_remove(&paths, "repo")?;
    assert_eq!(removed, 1);
    assert!(warnings.is_empty());
    let after_first_remove: Value = serde_json::from_slice(&fs::read(&paths.graph)?)?;
    assert!(after_first_remove["nodes"].as_array().is_some_and(|nodes| {
        nodes.iter().any(|node| node["label"] == "Shared API")
            && nodes.iter().any(|node| node["id"] == "other::one")
    }));
    let (removed, _) = global_remove(&paths, "other")?;
    assert_eq!(
        removed, 2,
        "last owner removes its node and orphaned external"
    );
    assert!(matches!(
        global_remove(&paths, "missing"),
        Err(GlobalError::UnknownRepo(_))
    ));
    Ok(())
}

#[test]
fn global_store_reports_missing_invalid_and_unwritable_inputs() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = paths(directory.path());
    assert!(matches!(
        global_add(
            &paths,
            &directory.path().join("missing.json"),
            "repo",
            OffsetDateTime::UNIX_EPOCH
        ),
        Err(GlobalError::GraphNotFound(_))
    ));

    let wrong_extension = directory.path().join("graph.txt");
    fs::write(&wrong_extension, "{}")?;
    assert!(matches!(
        global_add(&paths, &wrong_extension, "repo", OffsetDateTime::UNIX_EPOCH),
        Err(GlobalError::Graph { .. })
    ));
    let corrupt = directory.path().join("corrupt.json");
    fs::write(&corrupt, "not json")?;
    assert!(matches!(
        global_add(&paths, &corrupt, "repo", OffsetDateTime::UNIX_EPOCH),
        Err(GlobalError::Graph { .. })
    ));

    let valid = directory.path().join("valid.json");
    write_graph(&valid, "one", "One", "API")?;
    let blocked = directory.path().join("blocked");
    fs::write(&blocked, "file where directory is required")?;
    let blocked_paths = GlobalPaths {
        graph: blocked.join("graph.json"),
        manifest: blocked.join("manifest.json"),
        directory: blocked,
    };
    assert!(matches!(
        global_add(&blocked_paths, &valid, "repo", OffsetDateTime::UNIX_EPOCH),
        Err(GlobalError::Read { .. })
    ));
    Ok(())
}
