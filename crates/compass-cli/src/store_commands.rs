use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use compass_files::BuildGuard;
use compass_graph::{GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1, GraphSnapshotReader, SnapshotSelector};
use compass_store::{
    STORE_FILE_NAME, STORE_REF_FILE_NAME, STORE_SCHEMA_V1, SqliteStore, StoreRef,
    local_sqlite_store_path,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Outcome;

const BACKUP_SCHEMA_V1: &str = "compass.store.backup/1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    schema: String,
    store_schema: String,
    adapter: String,
    graph_digest: String,
    store_digest: String,
    snapshot_id: String,
    manifest_digest: String,
    store_reference: StoreRef,
}

pub(crate) fn command(args: &[String]) -> Outcome {
    let operation = args.first().map(String::as_str).unwrap_or("status");
    let result = match operation {
        "status" => status(args),
        "validate" => validate(args),
        "backup" => backup(args),
        "restore" => restore(args),
        _ => Err("usage: compass store <status|validate|backup|restore> [OPTIONS]".to_owned()),
    };
    match result {
        Ok(value) => match option(args, "--format").unwrap_or("text") {
            "json" => match serde_json::to_string_pretty(&value) {
                Ok(output) => Outcome::success(output),
                Err(error) => Outcome::failure(format!("error: {error}")),
            },
            "text" => Outcome::success(render_text(&value)),
            value => Outcome::failure(format!(
                "error: --format must be text or json (found {value})"
            )),
        },
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn status(args: &[String]) -> Result<Value, String> {
    let output = output_root(args)?;
    let graph_path = output.join("graph.json");
    let store_path = local_sqlite_store_path(&graph_path);
    let reference_path = output.join(STORE_REF_FILE_NAME);
    let mut graph = if graph_path.is_file() {
        Some(graph_status(&graph_path)?)
    } else {
        None
    };

    let store = if store_path.is_file() {
        match SqliteStore::open_read_only(&store_path) {
            Ok(store) => match validate_store(&store, graph.as_mut(), &graph_path) {
                Ok((reference, snapshot_id, manifest_digest)) => Some(json!({
                    "present": true,
                    "valid": true,
                    "bytes": fs::metadata(&store_path).map(|metadata| metadata.len()).unwrap_or(0),
                    "sha256": digest_file(&store_path)?,
                    "adapter": "sqlite",
                    "storeSchema": STORE_SCHEMA_V1,
                    "snapshotId": snapshot_id,
                    "manifestDigest": manifest_digest,
                    "reference": reference,
                })),
                Err(error) => Some(json!({
                    "present": true,
                    "valid": false,
                    "adapter": "sqlite",
                    "error": error,
                })),
            },
            Err(error) => Some(json!({
                "present": true,
                "valid": false,
                "adapter": "sqlite",
                "error": error.to_string(),
            })),
        }
    } else {
        None
    };

    let reference = if reference_path.is_file() {
        let bytes =
            fs::read(&reference_path).map_err(|error| format!("read store.ref: {error}"))?;
        match serde_json::from_slice::<StoreRef>(&bytes) {
            Ok(reference) => match reference.validate() {
                Ok(()) => json!({ "present": true, "valid": true, "value": reference }),
                Err(error) => json!({
                    "present": true,
                    "valid": false,
                    "error": error.to_string(),
                }),
            },
            Err(error) => json!({
                "present": true,
                "valid": false,
                "error": error.to_string(),
            }),
        }
    } else {
        json!({ "present": false })
    };

    Ok(json!({
        "schema": "compass.store.status/1",
        "output": output,
        "graphJson": graph.unwrap_or_else(|| json!({ "present": false })),
        "store": store.unwrap_or_else(|| json!({ "present": false })),
        "storeRef": reference,
        "rebuildCommand": "compass update --force",
    }))
}

fn validate(args: &[String]) -> Result<Value, String> {
    let output = output_root(args)?;
    let graph_path = output.join("graph.json");
    let store_path = local_sqlite_store_path(&graph_path);
    if !store_path.is_file() {
        return Err(format!(
            "store sidecar is missing: {}",
            store_path.display()
        ));
    }
    let mut graph = if graph_path.is_file() {
        Some(graph_status(&graph_path)?)
    } else {
        None
    };
    let store = SqliteStore::open_read_only(&store_path).map_err(|error| error.to_string())?;
    let (reference, snapshot_id, manifest_digest) =
        validate_store(&store, graph.as_mut(), &graph_path)?;
    let reference_path = output.join(STORE_REF_FILE_NAME);
    if !reference_path.is_file() {
        return Err(format!(
            "store.ref is missing: {}",
            reference_path.display()
        ));
    }
    let on_disk: StoreRef = serde_json::from_slice(
        &fs::read(&reference_path).map_err(|error| format!("read store.ref: {error}"))?,
    )
    .map_err(|error| format!("decode store.ref: {error}"))?;
    on_disk.validate().map_err(|error| error.to_string())?;
    if on_disk != reference {
        return Err("store.ref does not match the active SQLite snapshot".to_owned());
    }
    Ok(json!({
        "schema": "compass.store.validation/1",
        "valid": true,
        "output": output,
        "graphJson": graph.unwrap_or_else(|| json!({ "present": false })),
        "adapter": "sqlite",
        "storeSchema": STORE_SCHEMA_V1,
        "snapshotId": snapshot_id,
        "manifestDigest": manifest_digest,
        "storeReference": reference,
    }))
}

fn backup(args: &[String]) -> Result<Value, String> {
    let output = output_root(args)?;
    let destination = option(args, "--output")
        .map(PathBuf::from)
        .ok_or_else(|| "store backup requires --output DIR".to_owned())?;
    if destination.exists() {
        return Err(format!(
            "backup destination already exists: {}",
            destination.display()
        ));
    }
    let graph_path = output.join("graph.json");
    let store_path = local_sqlite_store_path(&graph_path);
    let reference_path = output.join(STORE_REF_FILE_NAME);
    let mut graph_value = graph_status(&graph_path)?;
    let reference_bytes =
        fs::read(&reference_path).map_err(|error| format!("read store.ref: {error}"))?;
    let reference: StoreRef = serde_json::from_slice(&reference_bytes)
        .map_err(|error| format!("decode store.ref: {error}"))?;
    reference.validate().map_err(|error| error.to_string())?;
    let store = SqliteStore::open(&store_path).map_err(|error| error.to_string())?;
    let (_, snapshot_id, manifest_digest) =
        validate_store(&store, Some(&mut graph_value), &graph_path)?;
    if reference.snapshot_id != snapshot_id || reference.manifest_digest != manifest_digest {
        return Err("store.ref does not match the active snapshot".to_owned());
    }

    fs::create_dir_all(&destination)
        .map_err(|error| format!("create backup destination: {error}"))?;
    let result = (|| {
        store
            .backup_to(destination.join(STORE_FILE_NAME))
            .map_err(|error| error.to_string())?;
        fs::copy(&graph_path, destination.join("graph.json"))
            .map_err(|error| format!("copy graph.json: {error}"))?;
        fs::copy(&reference_path, destination.join(STORE_REF_FILE_NAME))
            .map_err(|error| format!("copy store.ref: {error}"))?;
        let manifest = BackupManifest {
            schema: BACKUP_SCHEMA_V1.to_owned(),
            store_schema: STORE_SCHEMA_V1.to_owned(),
            adapter: "sqlite".to_owned(),
            graph_digest: graph_value
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| "graph status is missing its digest".to_owned())?
                .to_owned(),
            store_digest: digest_file(&destination.join(STORE_FILE_NAME))?,
            snapshot_id,
            manifest_digest,
            store_reference: reference,
        };
        fs::write(
            destination.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("write backup manifest: {error}"))?;
        Ok::<_, String>(manifest)
    })();
    match result {
        Ok(manifest) => Ok(json!({
            "schema": BACKUP_SCHEMA_V1,
            "backup": destination,
            "manifest": manifest,
        })),
        Err(error) => {
            let _ = fs::remove_dir_all(&destination);
            Err(error)
        }
    }
}

fn restore(args: &[String]) -> Result<Value, String> {
    let source = option(args, "--from")
        .map(PathBuf::from)
        .ok_or_else(|| "store restore requires --from DIR".to_owned())?;
    let destination = option(args, "--into")
        .map(PathBuf::from)
        .ok_or_else(|| "store restore requires --into DIR".to_owned())?;
    if destination.exists()
        && fs::read_dir(&destination)
            .map_err(|error| format!("inspect restore destination: {error}"))?
            .next()
            .is_some()
    {
        return Err(format!(
            "restore destination must be new or empty: {}",
            destination.display()
        ));
    }
    let manifest: BackupManifest = serde_json::from_slice(
        &fs::read(source.join("manifest.json"))
            .map_err(|error| format!("read backup manifest: {error}"))?,
    )
    .map_err(|error| format!("decode backup manifest: {error}"))?;
    if manifest.schema != BACKUP_SCHEMA_V1
        || manifest.store_schema != STORE_SCHEMA_V1
        || manifest.adapter != "sqlite"
    {
        return Err("backup uses an unsupported Compass store format".to_owned());
    }
    manifest
        .store_reference
        .validate()
        .map_err(|error| error.to_string())?;
    let graph_path = source.join("graph.json");
    let mut graph_value = graph_status(&graph_path)?;
    if graph_value.get("sha256").and_then(Value::as_str) != Some(&manifest.graph_digest) {
        return Err("backup graph digest does not match manifest".to_owned());
    }
    let backup_store = source.join(STORE_FILE_NAME);
    let store = SqliteStore::open_read_only(&backup_store).map_err(|error| error.to_string())?;
    validate_store(&store, Some(&mut graph_value), &graph_path)?;
    if digest_file(&backup_store)? != manifest.store_digest {
        return Err("backup store digest does not match manifest".to_owned());
    }
    let backup_reference: StoreRef = serde_json::from_slice(
        &fs::read(source.join(STORE_REF_FILE_NAME))
            .map_err(|error| format!("read backup store.ref: {error}"))?,
    )
    .map_err(|error| format!("decode backup store.ref: {error}"))?;
    if backup_reference != manifest.store_reference {
        return Err("backup store.ref does not match manifest".to_owned());
    }

    fs::create_dir_all(&destination)
        .map_err(|error| format!("create restore destination: {error}"))?;
    let result = (|| {
        SqliteStore::restore_from(&backup_store, destination.join(STORE_FILE_NAME))
            .map_err(|error| error.to_string())?;
        fs::copy(&graph_path, destination.join("graph.json"))
            .map_err(|error| format!("restore graph.json: {error}"))?;
        fs::copy(
            source.join(STORE_REF_FILE_NAME),
            destination.join(STORE_REF_FILE_NAME),
        )
        .map_err(|error| format!("restore store.ref: {error}"))?;
        validate(&["validate".to_owned(), destination.display().to_string()])?;
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    Ok(json!({
        "schema": "compass.store.restore/1",
        "restored": destination,
        "snapshotId": manifest.snapshot_id,
        "graphDigest": manifest.graph_digest,
    }))
}

fn validate_store(
    store: &SqliteStore,
    graph: Option<&mut Value>,
    graph_path: &Path,
) -> Result<(StoreRef, String, String), String> {
    let reference_path = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STORE_REF_FILE_NAME);
    let reference: StoreRef = serde_json::from_slice(
        &fs::read(&reference_path)
            .map_err(|error| format!("read {}: {error}", reference_path.display()))?,
    )
    .map_err(|error| format!("decode store.ref: {error}"))?;
    reference.validate().map_err(|error| error.to_string())?;
    let reader = GraphSnapshotReader::open_selector(
        store,
        SnapshotSelector {
            schema: GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1.to_owned(),
            snapshot_id: reference.snapshot_id.clone(),
            manifest_digest: reference.manifest_digest.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    let manifest = reader.manifest();
    reader
        .validate_integrity()
        .map_err(|error| error.to_string())?;
    if let Some(graph) = graph {
        let graph_bytes = graph
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "graph status is missing its byte count".to_owned())?;
        let graph_digest = graph
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| "graph status is missing its digest".to_owned())?;
        if graph_bytes != manifest.graph_bytes || graph_digest != manifest.graph_digest {
            return Err(format!(
                "store manifest does not match {}",
                graph_path.display()
            ));
        }
        graph["nodes"] = json!(manifest.node_count);
        graph["edges"] = json!(manifest.edge_count);
    }
    let actual = store
        .graph_snapshot_reference_for(&reference.snapshot_id, &reference.manifest_digest)
        .map_err(|error| error.to_string())?;
    if actual != reference {
        return Err("store.ref does not match the selected immutable snapshot".to_owned());
    }
    let manifest_digest = reference.manifest_digest.clone();
    Ok((reference, manifest.snapshot_id.clone(), manifest_digest))
}

fn graph_status(path: &Path) -> Result<Value, String> {
    let bytes = fs::metadata(path)
        .map_err(|error| format!("inspect {}: {error}", path.display()))?
        .len();
    Ok(json!({
        "present": true,
        "bytes": bytes,
        "sha256": digest_file(path)?,
    }))
}

fn output_root(args: &[String]) -> Result<PathBuf, String> {
    let candidate = positional(args)
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("compass-out"));
    if candidate.file_name().and_then(|name| name.to_str()) == Some("graph.json") {
        let graph = BuildGuard::resolve_requested_artifact(&candidate)
            .map_err(|error| error.to_string())?;
        return Ok(graph.parent().unwrap_or(Path::new(".")).to_path_buf());
    }
    let output_container = if candidate.join("compass-out").is_dir() {
        candidate.join("compass-out")
    } else {
        candidate.clone()
    };
    if output_container.join("current-snapshot").is_file()
        || output_container.join("snapshots").is_dir()
    {
        return BuildGuard::resolve_current_snapshot_directory(&output_container)
            .map_err(|error| error.to_string());
    }
    if candidate.join("graph.json").is_file() || candidate.join(STORE_FILE_NAME).is_file() {
        return Ok(candidate);
    }
    BuildGuard::resolve_current_snapshot_directory(&output_container)
        .map_err(|error| error.to_string())
}

fn positional(args: &[String]) -> Vec<String> {
    let value_options = ["--format", "--output", "--from", "--into"];
    let mut values = Vec::new();
    let mut skip = false;
    for argument in args.iter().skip(1) {
        if skip {
            skip = false;
        } else if value_options.contains(&argument.as_str()) {
            skip = true;
        } else if !argument.starts_with("--") {
            values.push(argument.clone());
        }
    }
    values
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().enumerate().find_map(|(index, argument)| {
        if argument == name {
            args.get(index + 1).map(String::as_str)
        } else {
            argument
                .strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn digest_file(path: &Path) -> Result<String, String> {
    let mut reader =
        File::open(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn render_text(value: &Value) -> String {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let output = value
        .get("output")
        .or_else(|| value.get("backup"))
        .or_else(|| value.get("restored"))
        .map(Value::to_string)
        .unwrap_or_default();
    let valid = value.get("valid").and_then(Value::as_bool);
    match valid {
        Some(valid) => format!("{schema}: valid={valid} {output}"),
        None => format!("{schema}: {output}"),
    }
}
