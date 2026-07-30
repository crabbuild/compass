use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{
    BuildOptions, BuildPurpose, SemanticLayer, build_graph_with_semantic, build_local_graph,
};
use compass_model::code_graph::{
    EdgeKind, GraphDocument, NodeDetails, NodeKind, NodeRole, RouteStage,
};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, ResolutionState};

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
        edge.evidence
            .iter()
            .any(|evidence| evidence.origin == EvidenceOrigin::Convention)
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
