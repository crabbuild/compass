use std::fs;
use std::path::{Path, PathBuf};

use compass_files::BuildGuard;
use compass_graph::{
    GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1, GraphSnapshotReader, SnapshotSelector, canonical_graph_json,
};
use compass_model::code_graph::GraphDocument;
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
    let graph = if graph_path.is_file() {
        let bytes = fs::read(&graph_path).map_err(|error| format!("read graph.json: {error}"))?;
        let document = GraphDocument::load(&graph_path).map_err(|error| error.to_string())?;
        Some(graph_status(&bytes, &document))
    } else {
        None
    };

    let store = if store_path.is_file() {
        match SqliteStore::open_read_only(&store_path) {
            Ok(store) => match validate_store(&store, graph.as_ref(), &graph_path) {
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
    let graph = if graph_path.is_file() {
        let bytes = fs::read(&graph_path).map_err(|error| format!("read graph.json: {error}"))?;
        let document = GraphDocument::load(&graph_path).map_err(|error| error.to_string())?;
        Some(graph_status(&bytes, &document))
    } else {
        None
    };
    let store = SqliteStore::open_read_only(&store_path).map_err(|error| error.to_string())?;
    let (reference, snapshot_id, manifest_digest) =
        validate_store(&store, graph.as_ref(), &graph_path)?;
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
    let graph_bytes = fs::read(&graph_path).map_err(|error| format!("read graph.json: {error}"))?;
    let graph = GraphDocument::load(&graph_path).map_err(|error| error.to_string())?;
    let reference_bytes =
        fs::read(&reference_path).map_err(|error| format!("read store.ref: {error}"))?;
    let reference: StoreRef = serde_json::from_slice(&reference_bytes)
        .map_err(|error| format!("decode store.ref: {error}"))?;
    reference.validate().map_err(|error| error.to_string())?;
    let store = SqliteStore::open(&store_path).map_err(|error| error.to_string())?;
    let graph_value = graph_status(&graph_bytes, &graph);
    let (_, snapshot_id, manifest_digest) =
        validate_store(&store, Some(&graph_value), &graph_path)?;
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
            graph_digest: digest(&graph_bytes),
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
    let graph_bytes = fs::read(source.join("graph.json"))
        .map_err(|error| format!("read backup graph.json: {error}"))?;
    if digest(&graph_bytes) != manifest.graph_digest {
        return Err("backup graph digest does not match manifest".to_owned());
    }
    let graph_path = source.join("graph.json");
    let graph = GraphDocument::load(&graph_path).map_err(|error| error.to_string())?;
    let backup_store = source.join(STORE_FILE_NAME);
    let store = SqliteStore::open_read_only(&backup_store).map_err(|error| error.to_string())?;
    validate_store(
        &store,
        Some(&graph_status(&graph_bytes, &graph)),
        &graph_path,
    )?;
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
        fs::write(destination.join("graph.json"), &graph_bytes)
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
    graph: Option<&Value>,
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
    let exported = reader
        .export_json_bytes()
        .map_err(|error| error.to_string())?;
    if let Some(graph) = graph {
        let graph_bytes =
            fs::read(graph_path).map_err(|error| format!("read graph.json: {error}"))?;
        let expected = graph_bytes_from_status(graph, &graph_bytes)?;
        if exported != expected {
            return Err("store export is not byte-identical to graph.json".to_owned());
        }
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

fn graph_status(bytes: &[u8], graph: &GraphDocument) -> Value {
    json!({
        "present": true,
        "bytes": bytes.len(),
        "sha256": digest(bytes),
        "nodes": graph.nodes.len(),
        "edges": graph.links.len(),
    })
}

fn graph_bytes_from_status(graph: &Value, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let expected = graph
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "graph status is missing its digest".to_owned())?;
    if expected != digest(bytes) {
        return Err("graph status digest changed during validation".to_owned());
    }
    let document = serde_json::from_slice::<GraphDocument>(bytes)
        .map_err(|error| format!("decode graph.json: {error}"))?;
    canonical_graph_json(&document).map_err(|error| error.to_string())
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
    if candidate.join("graph.json").is_file() || candidate.join(STORE_FILE_NAME).is_file() {
        return Ok(candidate);
    }
    let output_container = if candidate.join("compass-out").is_dir() {
        candidate.join("compass-out")
    } else {
        candidate
    };
    BuildGuard::resolve_active_directory(&output_container).map_err(|error| error.to_string())
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

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_file(path: &Path) -> Result<String, String> {
    Ok(digest(&fs::read(path).map_err(|error| {
        format!("read {}: {error}", path.display())
    })?))
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
