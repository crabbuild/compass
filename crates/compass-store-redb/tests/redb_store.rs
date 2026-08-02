use std::error::Error;
use std::fs;
use std::sync::Arc;
use std::thread;

use compass_graph::{GraphSnapshotBuilder, GraphSnapshotReader, canonical_graph_json};
use compass_model::code_graph::{BuildMetadata, GraphDocument};
use compass_store::test_support::assert_store_contract;
use compass_store::{
    Key, NamespaceId, PartitionKey, ScanLimits, SqliteStore, Store, StoreError, WriteCondition,
};
use compass_store_redb::{REDB_FILE_NAME, RedbStore};

fn empty_graph() -> GraphDocument {
    GraphDocument::empty_v1(BuildMetadata {
        builder_version: "redb-test".to_owned(),
        schema_fingerprint: "redb-test-schema".to_owned(),
        source_tree_digest: "redb-test-tree".to_owned(),
        configuration_digest: "redb-test-config".to_owned(),
        generation_id: "redb-test-generation".to_owned(),
        source_commit: None,
    })
}

#[test]
fn redb_passes_the_shared_store_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path().join(REDB_FILE_NAME))?;
    assert_store_contract(&store)?;
    assert!(store.capabilities().durable_acknowledgements);
    Ok(())
}

#[test]
fn redb_reopens_with_identical_values_and_read_only_writes_fail() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join(REDB_FILE_NAME);
    let namespace = NamespaceId::new(b"tenant")?;
    let partition = PartitionKey::new(b"objects")?;
    let key = Key::new(b"one")?;
    {
        let store = RedbStore::open(&path)?;
        store.put(
            &namespace,
            &partition,
            &key,
            b"value",
            WriteCondition::Missing,
        )?;
    }
    let reopened = RedbStore::open_read_only(&path)?;
    let reopened_entry = reopened
        .get(&namespace, &partition, &key)?
        .ok_or("reopened redb value is missing")?;
    assert_eq!(reopened_entry.value, b"value");
    assert!(matches!(
        reopened.put(
            &namespace,
            &partition,
            &key,
            b"changed",
            WriteCondition::Any,
        ),
        Err(StoreError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn redb_composite_ordering_handles_namespace_partition_and_binary_key_boundaries()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path().join(REDB_FILE_NAME))?;
    let namespace = NamespaceId::new([0x00, 0xff])?;
    let partition = PartitionKey::new([0x01, 0x00])?;
    for key in [
        vec![0_u8],
        vec![0, 1],
        vec![0xff],
        vec![0xff, 0],
        vec![0xff, 0xff],
    ] {
        let key_id = Key::new(&key)?;
        store.put(
            &namespace,
            &partition,
            &key_id,
            key.as_slice(),
            WriteCondition::Missing,
        )?;
    }
    let page = store.scan(
        &namespace,
        &partition,
        &Default::default(),
        ScanLimits::default(),
        None,
    )?;
    let keys = page
        .entries
        .into_iter()
        .map(|entry| entry.key)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            vec![0],
            vec![0, 1],
            vec![0xff],
            vec![0xff, 0],
            vec![0xff, 0xff],
        ]
    );
    Ok(())
}

#[test]
fn redb_writer_gate_reports_bounded_backpressure() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join(REDB_FILE_NAME);
    let store = Arc::new(RedbStore::open(&path)?);
    let namespace = NamespaceId::new(b"tenant")?;
    let partition = PartitionKey::new(b"records")?;
    let mut writers = Vec::new();
    for index in 0..8_u8 {
        let store = Arc::clone(&store);
        let namespace = namespace.clone();
        let partition = partition.clone();
        writers.push(thread::spawn(move || {
            let key = Key::new([index])?;
            store
                .put(
                    &namespace,
                    &partition,
                    &key,
                    b"value",
                    WriteCondition::Missing,
                )
                .map(|_| ())
        }));
    }
    let mut backpressure = 0;
    for writer in writers {
        match writer.join().map_err(|_| "writer thread panicked")? {
            Ok(()) => {}
            Err(StoreError::Backend { .. }) => backpressure += 1,
            Err(error) => return Err(error.into()),
        }
    }
    assert!(backpressure < 8);
    Ok(())
}

#[test]
fn redb_database_file_can_be_backed_up_after_reopen() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join(REDB_FILE_NAME);
    let backup = directory.path().join("backup.redb");
    let namespace = NamespaceId::new(b"tenant")?;
    let partition = PartitionKey::new(b"records")?;
    let key = Key::new(b"one")?;
    {
        let store = RedbStore::open(&path)?;
        store.put(
            &namespace,
            &partition,
            &key,
            b"value",
            WriteCondition::Missing,
        )?;
    }
    fs::copy(&path, &backup)?;
    let restored = RedbStore::open_read_only(&backup)?;
    let restored_entry = restored
        .get(&namespace, &partition, &key)?
        .ok_or("restored redb value is missing")?;
    assert_eq!(restored_entry.value, b"value");
    Ok(())
}

#[test]
fn redb_graph_snapshots_match_sqlite_identity_and_export() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let redb = RedbStore::open(directory.path().join(REDB_FILE_NAME))?;
    let sqlite = SqliteStore::open(directory.path().join("sqlite.db"))?;
    let graph = empty_graph();
    let builder = GraphSnapshotBuilder::new();
    let redb_prepared = builder.prepare(&redb, &graph)?;
    let sqlite_prepared = builder.prepare(&sqlite, &graph)?;
    assert_eq!(redb_prepared.manifest, sqlite_prepared.manifest);
    assert_eq!(
        redb_prepared.manifest_digest,
        sqlite_prepared.manifest_digest
    );
    builder.activate(&redb, &redb_prepared)?;
    builder.activate(&sqlite, &sqlite_prepared)?;
    let redb_reader = GraphSnapshotReader::open_active(&redb)?.ok_or("redb snapshot missing")?;
    let sqlite_reader =
        GraphSnapshotReader::open_active(&sqlite)?.ok_or("sqlite snapshot missing")?;
    assert_eq!(redb_reader.manifest(), sqlite_reader.manifest());
    assert_eq!(
        redb_reader.export_json_bytes()?,
        sqlite_reader.export_json_bytes()?
    );
    assert_eq!(
        redb_reader.export_json_bytes()?,
        canonical_graph_json(&graph)?
    );
    Ok(())
}
