use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, build_local_graph};
use compass_model::code_graph::{
    EdgeKind, GraphDocument, NodeDetails, NodeKind, NodeRole, RouteStage,
};
use compass_model::provenance::ResolutionState;

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
import { AdminPage as Screen } from "./AdminPage";
export const router = createBrowserRouter([{ path: "/admin", Component: Screen }]);
"#,
        ),
        (
            "src/admin/AdminPage.tsx",
            "export function AdminPage() { return null; }\n",
        ),
        (
            "src/public/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import { PublicPage as Screen } from "./PublicPage";
export const router = createBrowserRouter([{ path: "/public", Component: Screen }]);
"#,
        ),
        (
            "src/public/PublicPage.tsx",
            "export function PublicPage() { return null; }\n",
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
    let conflict_route = graph
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.details.as_ref(),
                Some(NodeDetails::Route(details)) if details.path == "/conflict"
            )
        })
        .ok_or("missing conflict route")?;
    let conflict_targets = graph
        .links
        .iter()
        .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.source == conflict_route.id)
        .filter_map(|edge| graph.nodes.iter().find(|node| node.id == edge.target))
        .map(|node| node.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        conflict_targets,
        BTreeSet::from(["firstHandler", "secondHandler"])
    );
    Ok(())
}
