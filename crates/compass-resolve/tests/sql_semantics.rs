use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::extract_sql_content;
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::EvidenceConfidence;
use compass_model::provenance::EvidenceOrigin;

#[test]
fn sql_text_facts_publish_with_artifact_provenance_and_exact_original_anchors()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V3__trigger.sql");
    let source = br#"
CREATE TABLE "app"."users" (id BIGINT PRIMARY KEY);
CREATE TABLE "audit"."log" (id BIGINT PRIMARY KEY);
CREATE TRIGGER "audit"."capture user"
AFTER UPDATE ON "app"."users"
FOR EACH ROW BEGIN
  INSERT INTO "audit"."log"(id) SELECT id FROM "app"."users";
END;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-semantics")?;
    let graph = normalize_v1(extraction, evidence)?;

    assert!(graph.nodes.iter().all(|node| node.qualified_name != "ON"));
    let trigger = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTrigger)
        .ok_or("missing trigger")?;
    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    assert!(graph.links.iter().any(|edge| {
        edge.source == trigger.id && edge.target == users.id && edge.kind == EdgeKind::Triggers
    }));
    let kinds = graph
        .links
        .iter()
        .map(|edge| edge.kind)
        .collect::<HashSet<_>>();
    assert!(kinds.contains(&EdgeKind::Reads));
    assert!(kinds.contains(&EdgeKind::Writes));

    for node in graph.nodes.iter().filter(|node| {
        !matches!(
            node.kind,
            NodeKind::File | NodeKind::Database | NodeKind::Migration
        )
    }) {
        assert!(
            node.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
                    && item.anchors.len() == 1
            }),
            "node={node:?}"
        );
    }
    for edge in &graph.links {
        if edge.evidence[0].origin == EvidenceOrigin::Convention {
            continue;
        }
        assert!(
            edge.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
                    && item.anchors.len() == 1
            }),
            "edge={edge:?}"
        );
    }
    Ok(())
}
