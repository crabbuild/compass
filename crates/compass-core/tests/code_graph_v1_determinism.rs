use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{
    BuildOptions, BuildPurpose, GraphStorage, SemanticLayer, build_graph_with_semantic,
    build_local_graph,
};
use compass_files::{Cache, CacheKind, CacheOptions};
use compass_graph::{
    GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1, GraphSnapshotReader, SnapshotSelector, canonical_graph_json,
};
use compass_languages::Extraction;
use compass_model::code_graph::{
    EdgeKind, GraphDocument, NodeDetails, NodeKind, NodeRole, RouteStage,
};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, ResolutionState};
use compass_store::{
    STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore, StoreRef, local_sqlite_store_path,
};
use sha2::{Digest, Sha256};

const SOURCE: &str = r#"
pub struct Store;

impl Store {
    pub fn load(&self) -> usize { 1 }
}

pub fn handler(store: &Store) -> usize {
    store.load()
}
"#;

fn build(root: &Path) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let result = build_local_graph(&options)?;
    let path = result.output_dir.join("graph.json");
    let bytes = fs::read(&path)?;
    GraphDocument::load(&path)?;
    Ok((bytes, result.outputs_changed))
}

fn build_with_empty_semantic(root: &Path) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    options.purpose = BuildPurpose::Extract;
    let result = build_graph_with_semantic(
        &options,
        &SemanticLayer {
            fragment: serde_json::json!({
                "nodes": [],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 0,
                "output_tokens": 0,
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        },
    )?;
    let path = result.output_dir.join("graph.json");
    let bytes = fs::read(&path)?;
    GraphDocument::load(&path)?;
    Ok((bytes, result.outputs_changed))
}

fn build_clustered(root: &Path) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    options.purpose = BuildPurpose::Extract;
    let result = build_graph_with_semantic(
        &options,
        &SemanticLayer {
            fragment: serde_json::json!({
                "nodes": [],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 0,
                "output_tokens": 0,
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        },
    )?;
    let path = result.output_dir.join("graph.json");
    let bytes = fs::read(&path)?;
    GraphDocument::load(&path)?;
    Ok((bytes, result.outputs_changed))
}

fn cached_semantic_evidence_bytes(
    root: &Path,
    sources: &[&str],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut cache = Cache::open(root, CacheOptions::output_directory(None))?;
    let mut batches = Vec::with_capacity(sources.len());
    for source in sources {
        let path = root.join(source);
        let value = cache
            .load(&path, &CacheKind::Ast, None, false)?
            .ok_or_else(|| format!("missing AST cache entry for {source}"))?;
        let extraction: Extraction = serde_json::from_value(value)?;
        let batch = extraction
            .semantic_evidence
            .ok_or_else(|| format!("missing semantic evidence for {source}"))?;
        batches.push((source.to_string(), batch));
    }
    batches.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(serde_json::to_vec(&batches)?)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn clean_warm_restored_and_checkout_root_builds_are_byte_identical() -> Result<(), Box<dyn Error>> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    for root in [first.path(), second.path()] {
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/lib.rs"), SOURCE)?;
    }

    let (cold, cold_changed) = build(first.path())?;
    assert!(cold_changed);
    let (warm, warm_changed) = build(first.path())?;
    assert!(!warm_changed);
    assert_eq!(warm, cold);

    let (other_root, _) = build(second.path())?;
    assert_eq!(other_root, cold);

    fs::write(
        first.path().join("src/lib.rs"),
        format!("{SOURCE}\npub fn changed() {{}}\n"),
    )?;
    let (changed, changed_output) = build(first.path())?;
    assert!(changed_output);
    assert_ne!(changed, cold);

    fs::write(first.path().join("src/lib.rs"), SOURCE)?;
    let (restored, restored_output) = build(first.path())?;
    assert!(restored_output);
    assert_eq!(restored, cold);
    Ok(())
}

#[test]
fn build_publishes_a_reopenable_store_snapshot_matching_graph_json() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("src"))?;
    fs::write(directory.path().join("src/lib.rs"), SOURCE)?;

    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;
    options.graph_storage = GraphStorage::Sqlite;
    options.max_workers = Some(2);
    let result = build_local_graph(&options)?;
    assert!(result.timings.store_new_objects > 0);
    assert!(result.timings.store_write_transactions > 0);
    assert!(result.timings.store_bytes_written > 0);
    let graph_path = result.output_dir.join("graph.json");
    let graph = GraphDocument::load(&graph_path)?;
    let store_path = local_sqlite_store_path(&graph_path);
    let store = SqliteStore::open_read_only(&store_path)?;
    let reference: StoreRef =
        serde_json::from_slice(&fs::read(result.output_dir.join(STORE_REF_FILE_NAME))?)?;

    assert_ne!(store_path, result.output_dir.join(STORE_FILE_NAME));
    assert!(!result.output_dir.join(STORE_FILE_NAME).exists());
    assert!(store.read_snapshot().is_err());
    assert_eq!(
        reference,
        store.graph_snapshot_reference_for(&reference.snapshot_id, &reference.manifest_digest,)?
    );
    let retention = store
        .retention_metadata()?
        .ok_or("missing retention metadata")?;
    assert_eq!(retention.active_manifest_digest, reference.manifest_digest);
    let reader = GraphSnapshotReader::open_selector(
        &store,
        SnapshotSelector {
            schema: GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1.to_owned(),
            snapshot_id: reference.snapshot_id.clone(),
            manifest_digest: reference.manifest_digest.clone(),
        },
    )?;
    assert_eq!(reader.export_json_bytes()?, canonical_graph_json(&graph)?);
    assert_eq!(reference.snapshot_id, reader.selector().snapshot_id);
    assert_eq!(reference.manifest_digest, reader.selector().manifest_digest);
    assert!(
        store
            .discover_orphan_manifests(Default::default())?
            .is_empty()
    );
    Ok(())
}

#[test]
fn empty_semantic_layer_does_not_reuse_output_after_cached_content_reversion()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), SOURCE)?;
    fs::write(
        root.join("model.php"),
        "<?php\nuse Illuminate\\Database\\Eloquent\\Model;\nclass Account extends Model {}\n",
    )?;

    let (clean, _) = build_with_empty_semantic(root)?;
    fs::write(
        root.join("src/lib.rs"),
        format!("{SOURCE}\npub fn temporary_edit() {{}}\n"),
    )?;
    let (edited, _) = build_with_empty_semantic(root)?;
    assert_ne!(edited, clean);

    fs::write(root.join("src/lib.rs"), SOURCE)?;
    let (restored, restored_output) = build_with_empty_semantic(root)?;
    assert!(restored_output);
    assert_eq!(restored, clean);
    Ok(())
}

#[test]
fn unrelated_incremental_edit_preserves_cached_framework_routes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for (relative, source) in [
        (
            "package.json",
            r#"{"dependencies":{"react-router-dom":"7.0.0"}}"#,
        ),
        (
            "src/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import AccountPage from "./AccountPage";
import UserPage from "./UserPage";
export const router = createBrowserRouter([
  { path: "/account", Component: AccountPage },
  { path: "/users/:id", Component: UserPage },
]);
"#,
        ),
        (
            "src/AccountPage.tsx",
            "export default function AccountPage() { return null; }\n",
        ),
        (
            "src/UserPage.tsx",
            "export default function UserPage() { return null; }\n",
        ),
        ("src/lib.rs", SOURCE),
    ] {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, source)?;
    }

    let (clean, _) = build(root)?;
    let clean_graph: GraphDocument = serde_json::from_slice(&clean)?;
    assert_eq!(
        clean_graph
            .links
            .iter()
            .filter(|edge| edge.kind == EdgeKind::RoutesTo)
            .count(),
        2
    );
    fs::write(
        root.join("src/lib.rs"),
        format!("{SOURCE}\npub fn temporary_edit() {{}}\n"),
    )?;
    let _ = build(root)?;
    fs::write(root.join("src/lib.rs"), SOURCE)?;
    let (restored, _) = build(root)?;
    assert_eq!(restored, clean);
    Ok(())
}

#[cfg(unix)]
#[test]
fn cached_file_symlink_keeps_its_logical_graph_identity() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), SOURCE)?;
    fs::write(
        root.join("CLAUDE.md"),
        "# Repro Guide\n\nSee [the source](src/lib.rs).\n",
    )?;
    symlink("CLAUDE.md", root.join("AGENTS.md"))?;

    let (cold, _) = build_clustered(root)?;
    let (warm, _) = build_clustered(root)?;

    assert_eq!(
        warm, cold,
        "loading a cached extraction must not collapse an in-repository file symlink into its target"
    );
    Ok(())
}

#[test]
fn empty_python_modules_publish_stable_file_and_import_facts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("pkg"))?;
    fs::write(root.join("pkg/__init__.py"), b"")?;
    fs::write(root.join("pkg/__main__.py"), b"")?;
    fs::write(root.join("consumer.py"), "import pkg.__main__\n")?;

    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let first = build_local_graph(&options)?;
    let first_path = first.output_dir.join("graph.json");
    let first_bytes = fs::read(&first_path)?;
    let first_graph = GraphDocument::load(&first_path)?;
    for source_file in ["pkg/__init__.py", "pkg/__main__.py"] {
        let file_nodes = first_graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File && node.source_file() == Some(source_file))
            .collect::<Vec<_>>();
        assert_eq!(file_nodes.len(), 1);
        assert!(file_nodes[0].evidence.iter().any(|evidence| {
            evidence.origin == EvidenceOrigin::Convention
                && evidence.rule.as_deref() == Some("empty-file-inventory")
        }));
    }
    let main_module = first_graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::File && node.source_file() == Some("pkg/__main__.py"))
        .ok_or("missing pkg/__main__.py file node")?;
    assert!(first_graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Imports
            && edge.target == main_module.id
            && first_graph
                .nodes
                .iter()
                .any(|node| node.id == edge.source && node.source_file() == Some("consumer.py"))
    }));

    options.force = true;
    let forced = build_local_graph(&options)?;
    let forced_bytes = fs::read(forced.output_dir.join("graph.json"))?;
    assert_eq!(forced_bytes, first_bytes);
    Ok(())
}

#[test]
fn python_and_go_cold_and_cached_rebuilds_have_identical_graph_and_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for (relative, source) in [
        (
            "python/pkg/base.py",
            "class Base:\n    def run(self):\n        return 1\n",
        ),
        (
            "python/pkg/__init__.py",
            "from .base import Base as ExportedBase\n",
        ),
        (
            "python/app.py",
            "from pkg import ExportedBase\n\
             class Worker(ExportedBase):\n\
                 def execute(self):\n\
                     return self.run()\n",
        ),
        ("go/go.mod", "module example.com/project\n\ngo 1.22\n"),
        (
            "go/model/task.go",
            "package model\n\
             type Task struct{}\n\
             func (task *Task) Run() {}\n",
        ),
        (
            "go/cmd/agent/main.go",
            "package agent\n\
             import \"example.com/project/model\"\n\
             func Execute(task *model.Task) { task.Run() }\n",
        ),
    ] {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, source)?;
    }
    let evidence_sources = [
        "go/cmd/agent/main.go",
        "go/model/task.go",
        "python/app.py",
        "python/pkg/__init__.py",
        "python/pkg/base.py",
    ];

    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let cold = build_local_graph(&options)?;
    assert!(cold.files_extracted >= evidence_sources.len());
    let cold_graph_bytes = fs::read(cold.output_dir.join("graph.json"))?;
    let cold_graph = GraphDocument::load(&cold.output_dir.join("graph.json"))?;
    let cold_evidence = cached_semantic_evidence_bytes(root, &evidence_sources)?;
    let cold_digest = sha256(&cold_graph_bytes);

    options.force = true;
    options.reuse_cache_on_force = true;
    let warm = build_local_graph(&options)?;
    assert_eq!(warm.files_extracted, 0);
    assert!(warm.files_cached >= evidence_sources.len());
    let warm_graph_bytes = fs::read(warm.output_dir.join("graph.json"))?;
    let warm_graph = GraphDocument::load(&warm.output_dir.join("graph.json"))?;
    let warm_evidence = cached_semantic_evidence_bytes(root, &evidence_sources)?;
    let warm_digest = sha256(&warm_graph_bytes);

    assert_eq!(warm_graph_bytes, cold_graph_bytes);
    assert_eq!(warm_evidence, cold_evidence);
    assert_eq!(warm_digest, cold_digest);
    assert_eq!(
        warm_graph.graph.build.generation_id,
        cold_graph.graph.build.generation_id
    );
    Ok(())
}

#[test]
fn unchanged_clustered_extract_reuses_verified_generation() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), SOURCE)?;

    let (cold, cold_changed) = build_clustered(root)?;
    let (warm, warm_changed) = build_clustered(root)?;

    assert!(cold_changed);
    assert!(!warm_changed);
    assert_eq!(warm, cold);
    Ok(())
}

#[test]
fn edit_restore_does_not_preserve_recomputable_heuristic_facts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), SOURCE)?;
    fs::write(
        root.join("MissingReference.csproj"),
        r#"<Project>
  <ItemGroup>
    <ProjectReference Include="absent/Dependency.csproj" />
  </ItemGroup>
</Project>
"#,
    )?;

    let (clean, _) = build(root)?;
    fs::write(
        root.join("src/lib.rs"),
        format!("{SOURCE}\npub fn temporary_edit() {{}}\n"),
    )?;
    let (edited, _) = build(root)?;
    assert_ne!(edited, clean);

    fs::write(root.join("src/lib.rs"), SOURCE)?;
    let (restored, _) = build(root)?;
    let clean_graph: GraphDocument = serde_json::from_slice(&clean)?;
    let restored_graph: GraphDocument = serde_json::from_slice(&restored)?;
    assert_eq!(restored_graph.nodes.len(), clean_graph.nodes.len());
    assert_eq!(restored_graph.links.len(), clean_graph.links.len());
    assert_eq!(
        restored_graph.graph.coverage.len(),
        clean_graph.graph.coverage.len()
    );
    assert_eq!(
        restored_graph.graph.diagnostics.len(),
        clean_graph.graph.diagnostics.len()
    );
    assert_eq!(restored_graph.graph.build, clean_graph.graph.build);
    assert_eq!(restored_graph.graph.files, clean_graph.graph.files);
    assert_eq!(restored_graph.graph.coverage, clean_graph.graph.coverage);
    assert_eq!(
        restored_graph.graph.diagnostics,
        clean_graph.graph.diagnostics
    );
    assert_eq!(restored_graph.nodes, clean_graph.nodes);
    assert_eq!(restored_graph.links, clean_graph.links);
    assert!(
        restored == clean,
        "recomputable resolver and graph heuristics must not feed back from the prior graph"
    );
    Ok(())
}

#[test]
fn production_pipeline_preserves_framework_domain_kinds_and_route_targets()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for (relative, source) in [
        (
            "package.json",
            r#"{"dependencies":{"nuxt":"4.0.0","@nestjs/common":"11.0.0","react-router-dom":"7.0.0"}}"#,
        ),
        (
            "src/orders.ts",
            r#"import { Controller } from '@nestjs/common';
import { EventPattern, MessagePattern } from '@nestjs/microservices';
@Controller()
export class OrdersConsumer {
  @MessagePattern('orders.created')
  handleCreated() {}
  @EventPattern('orders.cancelled')
  handleCancelled() {}
}
"#,
        ),
        (
            "src/OrderEvents.java",
            r#"import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.amqp.rabbit.annotation.RabbitListener;
class OrderEvents {
  @KafkaListener(topics = "orders.created")
  public void consume(String event) {}
  @RabbitListener(queues = "orders.queue")
  public void consumeQueue(String event) {}
}
"#,
        ),
        (
            "src/jobs.py",
            r#"from celery import shared_task
@shared_task
def refresh_inventory():
    pass
"#,
        ),
        (
            "src/admin/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import Screen from "./AdminPage";
export const router = createBrowserRouter([{ path: "/admin", Component: Screen }]);
"#,
        ),
        (
            "src/admin/AdminPage.tsx",
            "export default function AdminPage() { return null; }\n",
        ),
        (
            "src/public/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import Screen from "./PublicPage";
export const router = createBrowserRouter([{ path: "/public", Component: Screen }]);
"#,
        ),
        (
            "src/public/PublicPage.tsx",
            "export default function PublicPage() { return null; }\n",
        ),
        (
            "nuxt/middleware/auth.ts",
            "export default defineNuxtRouteMiddleware(() => {});\n",
        ),
        (
            "src/server.ts",
            r#"import express from "express";
const app = express();
app.get("/staged", authenticate, missingMiddleware, show);
app.get("/conflict", firstHandler);
app.get("/conflict", secondHandler);
function authenticate() {}
function show() {}
function firstHandler() {}
function secondHandler() {}
"#,
        ),
    ] {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, source)?;
    }

    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let result = build_local_graph(&options)?;
    let graph = GraphDocument::load(&result.output_dir.join("graph.json"))?;

    for kind in [
        NodeKind::Event,
        NodeKind::Message,
        NodeKind::Topic,
        NodeKind::Queue,
        NodeKind::Job,
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == kind),
            "missing {kind:?}"
        );
    }
    for (route_path, target_source) in [
        ("/admin", "src/admin/AdminPage.tsx"),
        ("/public", "src/public/PublicPage.tsx"),
    ] {
        let route = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Route
                    && node.details.as_ref().is_some_and(|details| {
                        matches!(
                            details,
                            compass_model::code_graph::NodeDetails::Route(details)
                                if details.path == route_path
                        )
                    })
            })
            .ok_or_else(|| format!("missing route {route_path}"))?;
        let target = graph
            .links
            .iter()
            .find(|edge| edge.kind == EdgeKind::RoutesTo && edge.source == route.id)
            .and_then(|edge| graph.nodes.iter().find(|node| node.id == edge.target))
            .ok_or_else(|| format!("missing route target for {route_path}"))?;
        assert_eq!(
            target.source.as_ref().map(|source| source.file.as_str()),
            Some(target_source)
        );
    }
    assert!(graph.nodes.iter().any(|node| {
        node.roles.contains(&NodeRole::Middleware)
            && node
                .source
                .as_ref()
                .is_some_and(|source| source.file == "nuxt/middleware/auth.ts")
    }));
    let staged_route = graph
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.details.as_ref(),
                Some(NodeDetails::Route(details)) if details.path == "/staged"
            )
        })
        .ok_or("missing staged route")?;
    let Some(NodeDetails::Route(staged)) = staged_route.details.as_ref() else {
        return Err("missing staged route details".into());
    };
    assert_eq!(
        staged
            .stages
            .iter()
            .map(|stage| (
                stage.stage,
                stage.position,
                stage.reference.as_str(),
                stage.resolution,
                stage.target.is_some(),
                stage.candidates.len(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                RouteStage::Middleware,
                0,
                "authenticate",
                ResolutionState::Exact,
                true,
                1,
            ),
            (
                RouteStage::Middleware,
                1,
                "missingMiddleware",
                ResolutionState::Unresolved,
                false,
                0,
            ),
            (
                RouteStage::Handler,
                2,
                "show",
                ResolutionState::Exact,
                true,
                1,
            ),
        ]
    );
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| { edge.kind == EdgeKind::RoutesTo && edge.source == staged_route.id })
            .count(),
        2
    );
    let conflict_routes = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.details.as_ref(),
                Some(NodeDetails::Route(details)) if details.path == "/conflict"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(conflict_routes.len(), 2);
    let mut conflict_references = BTreeSet::new();
    for route in conflict_routes {
        let anchor = route.source.as_ref().ok_or("missing conflict anchor")?;
        let Some(NodeDetails::Route(details)) = route.details.as_ref() else {
            return Err("missing conflict route details".into());
        };
        let [stage] = details.stages.as_slice() else {
            return Err("conflict route must have one handler stage".into());
        };
        assert_eq!(stage.stage, RouteStage::Handler);
        assert_eq!(stage.position, 0);
        assert_eq!(stage.resolution, ResolutionState::Exact);
        assert_eq!(stage.candidates.len(), 1);
        let target = stage
            .target
            .as_deref()
            .ok_or("missing conflict stage target")?;
        assert_eq!(stage.candidates[0].node_id, target);
        conflict_references.insert(stage.reference.as_str());

        let edges = graph
            .links
            .iter()
            .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.source == route.id)
            .collect::<Vec<_>>();
        let [edge] = edges.as_slice() else {
            return Err("conflict route must have one authoritative edge".into());
        };
        assert_eq!(edge.target, target);
        assert_eq!(edge.relationship_site.as_ref(), Some(anchor));
        assert!(
            edge.evidence
                .iter()
                .all(|evidence| evidence.confidence == EvidenceConfidence::Exact)
        );
        let target_node = graph
            .nodes
            .iter()
            .find(|node| node.id == target)
            .ok_or("missing conflict target node")?;
        assert_eq!(
            target_node.name.trim_end_matches("()"),
            stage.reference.as_str()
        );
    }
    assert_eq!(
        conflict_references,
        BTreeSet::from(["firstHandler", "secondHandler"])
    );
    Ok(())
}

#[test]
fn force_update_is_a_clean_byte_identical_rebuild() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for (relative, source) in [
        (
            "project/urls.py",
            r#"from django.urls import path
from . import views
urlpatterns = [path("health/", views.health, name="health")]
"#,
        ),
        (
            "project/views.py",
            "def health(request):\n    return \"ok\"\n",
        ),
        ("guide.md", "# Service guide\n\n[Routes](project/urls.py)\n"),
    ] {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, source)?;
    }

    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let clean = build_local_graph(&options)?;
    let clean_path = clean.output_dir.join("graph.json");
    let clean_bytes = fs::read(&clean_path)?;
    let clean_graph = GraphDocument::load(&clean_path)?;
    assert!(clean_graph.nodes.iter().any(|node| {
        node.kind == NodeKind::Resource
            && node.source_file() == Some("guide.md")
            && node
                .evidence
                .iter()
                .any(|evidence| evidence.origin == EvidenceOrigin::Artifact)
    }));
    assert!(
        clean_graph
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Route)
    );
    assert!(clean_graph.links.iter().any(|edge| {
        edge.kind == compass_model::code_graph::EdgeKind::RoutesTo
            && edge
                .evidence
                .iter()
                .any(|evidence| evidence.origin == EvidenceOrigin::Config)
    }));

    options.force = true;
    let forced = build_local_graph(&options)?;
    let forced_path = forced.output_dir.join("graph.json");
    let forced_bytes = fs::read(&forced_path)?;
    let forced_graph = GraphDocument::load(&forced_path)?;

    assert_eq!(forced_bytes, clean_bytes);
    assert_eq!(forced_graph.graph.files, clean_graph.graph.files);
    assert_eq!(forced_graph.nodes, clean_graph.nodes);
    assert_eq!(forced_graph.links, clean_graph.links);
    assert!(forced_graph.links.iter().all(|edge| {
        edge.evidence
            .iter()
            .all(|evidence| evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap"))
    }));
    Ok(())
}

#[test]
fn force_extract_with_cache_reuse_has_no_prior_published_semantic_input()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> usize { 42 }\n")?;
    fs::write(
        root.join("README.md"),
        "# Example\n\nThe implementation is in [Rust](src/lib.rs).\n",
    )?;

    let mut options = BuildOptions::new(root);
    options.purpose = BuildPurpose::Extract;
    options.no_cluster = true;
    options.no_viz = true;
    options.program_analysis = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());

    let clean = build_local_graph(&options)?;
    let clean_path = clean.output_dir.join("graph.json");
    let clean_bytes = fs::read(&clean_path)?;
    let clean_graph = GraphDocument::load(&clean_path)?;
    let clean_document_coverage = clean_graph
        .graph
        .coverage
        .iter()
        .filter(|coverage| coverage.capability == "node:document")
        .cloned()
        .collect::<Vec<_>>();
    assert!(!clean_document_coverage.is_empty());
    assert!(clean_graph.nodes.iter().any(|node| {
        node.kind == NodeKind::Resource
            && node.source_file() == Some("README.md")
            && node
                .evidence
                .iter()
                .any(|evidence| evidence.origin == EvidenceOrigin::Artifact)
    }));

    options.force = true;
    options.reuse_cache_on_force = true;
    let forced = build_local_graph(&options)?;
    let forced_path = forced.output_dir.join("graph.json");
    let forced_bytes = fs::read(&forced_path)?;
    let forced_graph = GraphDocument::load(&forced_path)?;
    let forced_document_coverage = forced_graph
        .graph
        .coverage
        .iter()
        .filter(|coverage| coverage.capability == "node:document")
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(forced.files_considered, clean.files_considered);
    assert_eq!(forced.files_extracted, 0);
    assert_eq!(forced.files_cached, clean.files_considered);
    assert_eq!(forced.program_syntax_analyzed, 0);
    assert!(forced.program_syntax_reused > 0);
    assert_eq!(forced_bytes, clean_bytes);
    assert_eq!(forced_graph.graph.files, clean_graph.graph.files);
    assert_eq!(forced_graph.nodes, clean_graph.nodes);
    assert_eq!(forced_graph.links, clean_graph.links);
    assert_eq!(forced_document_coverage, clean_document_coverage);
    assert!(forced_graph.links.iter().all(|edge| {
        edge.evidence
            .iter()
            .all(|evidence| evidence.rule.as_deref() != Some("incremental-ast-endpoint-remap"))
    }));
    Ok(())
}
