use std::collections::HashSet;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, extract_sql_content};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::EvidenceOrigin;

#[test]
fn sql_domain_facts_publish_as_a_valid_closed_v1_graph() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V20260728__accounts.sql");
    let source = br#"
CREATE SCHEMA app;
CREATE TABLE app.accounts (
    id BIGINT PRIMARY KEY,
    owner_id BIGINT,
    CONSTRAINT accounts_owner_fk FOREIGN KEY (owner_id) REFERENCES app.users(id)
);
CREATE TABLE app.users (id BIGINT PRIMARY KEY);
CREATE INDEX accounts_owner_idx ON app.accounts(owner_id);
CREATE VIEW app.account_owners AS
    SELECT account.id FROM app.accounts account JOIN app.users owner ON owner.id = account.owner_id;
UPDATE app.accounts SET owner_id = 1 WHERE id = 1;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;

    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    let absolute_extraction = extract_sql_content(&root.join(relative), source);
    let absolute_evidence =
        BuildEvidence::from_extraction(root, &absolute_extraction, "sha256:test-config")?;
    let absolute_graph = normalize_v1(absolute_extraction, absolute_evidence)?;
    assert_eq!(absolute_graph.nodes, graph.nodes);
    assert_eq!(absolute_graph.links, graph.links);
    let kinds = graph
        .nodes
        .iter()
        .map(|node| node.kind)
        .collect::<HashSet<_>>();
    for expected in [
        NodeKind::Database,
        NodeKind::DatabaseSchema,
        NodeKind::DatabaseTable,
        NodeKind::DatabaseColumn,
        NodeKind::DatabaseIndex,
        NodeKind::DatabaseConstraint,
        NodeKind::DatabaseView,
        NodeKind::Migration,
        NodeKind::Query,
    ] {
        assert!(kinds.contains(&expected), "missing {expected:?}");
    }
    let edge_kinds = graph
        .links
        .iter()
        .map(|edge| edge.kind)
        .collect::<HashSet<_>>();
    for expected in [
        EdgeKind::Contains,
        EdgeKind::References,
        EdgeKind::Reads,
        EdgeKind::Writes,
    ] {
        assert!(edge_kinds.contains(&expected), "missing {expected:?}");
    }
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.capability == "node:database_table" && coverage.producer == "compass.languages.sql"
    }));
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.capability == "edge:writes" && coverage.producer == "compass.languages.sql"
    }));

    let migration = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Migration)
        .ok_or("missing migration")?;
    assert_eq!(migration.evidence[0].origin, EvidenceOrigin::Convention);
    let query = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Query)
        .ok_or("missing query")?;
    assert!(query.source.is_some());
    assert!(query.evidence[0].anchors.len() == 1);
    Ok(())
}

#[test]
fn json_config_keys_publish_with_config_provenance_and_stable_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("tsconfig.json");
    let source = br#"{
  "$schema": "https://json.schemastore.org/tsconfig",
  "extends": "./base.json",
  "compilerOptions": {
    "strict": true,
    "paths": {
      "@app/*": ["src/*"]
    }
  },
  "dependencies": {
    "react": "19.0.0"
  }
}"#;
    fs::write(root.join(relative), source)?;

    let extraction = Engine::default().extract_source(relative, source)?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    let config_keys = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::ConfigKey)
        .collect::<Vec<_>>();
    assert!(config_keys.iter().any(|node| {
        node.qualified_name == "compilerOptions.paths.@app/*"
            && node.evidence[0].origin == EvidenceOrigin::Config
    }));
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Resource)
    );
    assert!(graph.nodes.iter().any(|node| node.kind == NodeKind::Schema));
    assert!(
        graph
            .links
            .iter()
            .any(|edge| edge.kind == EdgeKind::Imports)
    );
    assert!(
        graph
            .links
            .iter()
            .all(|edge| edge.evidence[0].origin == EvidenceOrigin::Config)
    );

    let compiler_options = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "compilerOptions")
        .ok_or("missing compilerOptions config key")?;
    let paths = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "compilerOptions.paths")
        .ok_or("missing compilerOptions.paths config key")?;
    let nested_path = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "compilerOptions.paths.@app/*")
        .ok_or("missing nested config key")?;
    assert!(graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Contains
            && edge.source == compiler_options.id
            && edge.target == paths.id
    }));
    assert!(graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Contains && edge.source == paths.id && edge.target == nested_path.id
    }));
    Ok(())
}

#[test]
fn markdown_frontmatter_publishes_nested_config_graph_without_changing_graph_v1()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("docs/guide.md");
    let source = br#"---
title: Graph Guide
tags: [markdown, compass]
api_token: do-not-publish
authors:
  - name: Ada
    role: editor
site:
  navigation:
    label: Guide
---
# Body
"#;
    fs::create_dir_all(root.join("docs"))?;
    fs::write(root.join(relative), source)?;

    let extraction = Engine::default().extract_source(relative, source)?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    assert_eq!(graph.graph.schema, "compass.graph/1");
    assert!(graph.nodes.iter().all(|node| {
        !matches!(
            node.details,
            Some(compass_model::code_graph::NodeDetails::Document(_))
        )
    }));

    let document = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "docs/guide.md")
        .ok_or("missing Markdown document")?;
    assert_eq!(document.name, "Graph Guide");
    let keys = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::ConfigKey)
        .collect::<Vec<_>>();
    for expected in [
        "frontmatter/title",
        "frontmatter/tags",
        "frontmatter/api_token",
        "frontmatter/authors",
        "frontmatter/authors/0",
        "frontmatter/authors/0/name",
        "frontmatter/authors/0/role",
        "frontmatter/site",
        "frontmatter/site/navigation",
        "frontmatter/site/navigation/label",
    ] {
        assert!(
            keys.iter().any(|node| node.qualified_name == expected),
            "missing {expected}: {keys:#?}"
        );
    }
    assert!(keys.iter().all(|node| {
        node.source.is_some()
            && node.evidence[0].origin == EvidenceOrigin::Config
            && matches!(
                node.details,
                Some(compass_model::code_graph::NodeDetails::Config(
                    ref details
                )) if details.format == "yaml_frontmatter"
                    && details.key_path.starts_with('/')
            )
    }));
    let token = keys
        .iter()
        .find(|node| node.qualified_name == "frontmatter/api_token")
        .ok_or("missing token key")?;
    assert_eq!(token.name, "api_token");
    assert!(!serde_json::to_string(&graph)?.contains("do-not-publish"));
    let author = keys
        .iter()
        .find(|node| node.qualified_name == "frontmatter/authors/0")
        .ok_or("missing author item")?;
    let author_name = keys
        .iter()
        .find(|node| node.qualified_name == "frontmatter/authors/0/name")
        .ok_or("missing author name")?;
    assert!(graph.links.iter().any(|edge| {
        edge.source == author.id
            && edge.target == author_name.id
            && edge.kind == EdgeKind::Contains
            && edge.evidence[0].origin == EvidenceOrigin::Config
    }));

    let stable_ids = keys
        .iter()
        .map(|node| (node.qualified_name.clone(), node.id.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let changed = String::from_utf8(source.to_vec())?.replace("Graph Guide", "Compass Graph Guide");
    fs::write(root.join(relative), &changed)?;
    let extraction = Engine::default().extract_source(relative, changed.as_bytes())?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let changed_graph = normalize_v1(extraction, evidence)?;
    let changed_ids = changed_graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::ConfigKey)
        .map(|node| (node.qualified_name.clone(), node.id.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(changed_ids, stable_ids);
    Ok(())
}

#[test]
fn package_manifests_publish_dependency_endpoints_instead_of_dangling_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("go.mod");
    fs::write(
        &path,
        "module example.test/app\nrequire example.test/dependency v1.2.3\n",
    )?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Package)
            .count(),
        2
    );
    let dependency = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::DependsOn)
        .ok_or("missing dependency edge")?;
    assert_eq!(dependency.evidence[0].origin, EvidenceOrigin::Config);
    assert!(graph.nodes.iter().any(|node| node.id == dependency.target));
    Ok(())
}

#[test]
fn package_manifest_self_dependency_is_diagnostic_not_topology()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("go.mod");
    fs::write(
        &path,
        "module example.test/app\nrequire example.test/app v1.2.3\n",
    )?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    assert!(graph.links.iter().all(|edge| edge.source != edge.target));
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "suppressed_dependency_self_loop" })
    );
    Ok(())
}

#[test]
fn terraform_publishes_resources_packages_config_keys_and_forward_references()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("infra/main.tf");
    let source = br#"
variable "region" { type = string }
resource "aws_instance" "app" {
  subnet_id = aws_subnet.main.id
  depends_on = [module.network]
}
module "network" {
  source = "./network"
}
resource "aws_subnet" "main" {
  cidr_block = "10.0.0.0/24"
}
"#;
    fs::create_dir_all(root.join("infra"))?;
    fs::write(root.join(relative), source)?;

    let extraction = Engine::default().extract_source(relative, source)?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    for expected in [NodeKind::Resource, NodeKind::Package, NodeKind::ConfigKey] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == expected),
            "missing {expected:?}"
        );
    }
    for expected in [
        EdgeKind::Contains,
        EdgeKind::References,
        EdgeKind::DependsOn,
    ] {
        assert!(
            graph.links.iter().any(|edge| edge.kind == expected),
            "missing {expected:?}"
        );
    }
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| node.evidence[0].origin == EvidenceOrigin::Config)
    );
    Ok(())
}
