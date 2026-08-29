use std::error::Error;
use std::fs;
use std::process::Command;

use compass_files::BuildGuard;
use compass_graph::{
    GRAPH_SNAPSHOT_ACTIVE_KEY, GRAPH_SNAPSHOT_CATALOG_PARTITION, GRAPH_SNAPSHOT_OBJECT_PARTITION,
    GraphSnapshotReader, SnapshotSelector, graph_snapshot_manifest_key,
};
use compass_store::{
    Key, MAX_GRAPH_BYTES, NamespaceId, PartitionKey, SqliteStore, Store, StoreRef, WriteCondition,
    local_sqlite_store_path,
};
use sha2::{Digest, Sha256};

#[test]
fn oversized_snapshot_limit_error_names_scope_remedies_and_exits_one() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("sample.rs"), "pub fn sample() {}\n")?;
    let built = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["update", ".", "--code-only", "--no-viz"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        built.status.success(),
        "initial build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let output_root = root.path().join("compass-out");
    let graph_path = BuildGuard::resolve_artifact(&output_root, "graph.json")?;
    let store_path = local_sqlite_store_path(&graph_path);
    let store = SqliteStore::open(&store_path)?;
    let namespace = NamespaceId::graph();
    let catalog = PartitionKey::new(GRAPH_SNAPSHOT_CATALOG_PARTITION)?;
    let objects = PartitionKey::new(GRAPH_SNAPSHOT_OBJECT_PARTITION)?;
    let active = Key::new(GRAPH_SNAPSHOT_ACTIVE_KEY)?;
    let active_entry = store
        .get(&namespace, &catalog, &active)?
        .ok_or("store has no active graph snapshot")?;
    let mut selector: SnapshotSelector = serde_json::from_slice(&active_entry.value)?;
    let reader = GraphSnapshotReader::open_selector(&store, selector.clone())?;
    let mut manifest = reader.manifest().clone();
    manifest.graph_bytes = MAX_GRAPH_BYTES as u64 + 1;
    drop(reader);
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_digest = format!("{:x}", Sha256::digest(&manifest_bytes));
    let manifest_key = graph_snapshot_manifest_key(&manifest_digest)?;
    store.put(
        &namespace,
        &objects,
        &manifest_key,
        &manifest_bytes,
        WriteCondition::Missing,
    )?;

    selector.manifest_digest = manifest_digest;
    let selector_bytes = serde_json::to_vec(&selector)?;
    store.put(
        &namespace,
        &catalog,
        &active,
        &selector_bytes,
        WriteCondition::Version(active_entry.version),
    )?;
    store.checkpoint()?;
    drop(store);

    let reference_path = graph_path
        .parent()
        .ok_or("resolved graph has no parent directory")?
        .join("store.ref");
    let mut reference: StoreRef = serde_json::from_slice(&fs::read(&reference_path)?)?;
    reference.manifest_digest = selector.manifest_digest;
    fs::write(&reference_path, serde_json::to_vec_pretty(&reference)?)?;

    let queried = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["search", "sample", "--graph"])
        .arg(&graph_path)
        .args(["--engine", "store"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert_eq!(queried.status.code(), Some(1));
    assert!(queried.stdout.is_empty());
    let stderr = String::from_utf8(queried.stderr)?;
    assert!(stderr.contains("snapshot limit exceeded"), "{stderr}");
    assert!(stderr.contains("--exclude <pattern>"), "{stderr}");
    assert!(stderr.contains(".compassignore"), "{stderr}");
    assert!(stderr.contains("COMPASS_MAX_GRAPH_BYTES"), "{stderr}");
    Ok(())
}
