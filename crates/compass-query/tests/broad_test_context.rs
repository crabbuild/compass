mod support;

use std::error::Error;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{GraphDocument, NodeKind, NodeRole};
use compass_model::identity::file_id;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_query::{open, open_with_store};
use compass_store::SqliteStore;

fn request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}

#[test]
fn implementation_questions_demote_test_context_but_keep_explicit_test_intent()
-> Result<(), Box<dyn Error>> {
    for (production_path, test_path, test_role) in [
        ("src/codec.live.ts", "src/codec.test.ts", None),
        ("src/codec.live.ts", "src/codec.spec.ts", None),
        ("src/codec_live.go", "src/codec_test.go", None),
        ("src/codec_live.py", "src/test_codec.py", None),
        ("src/contest.ts", "src/codec.test.ts", None),
        ("src/latest.ts", "src/codec.spec.ts", None),
        (
            "src/codec.live.ts",
            "src/codec.flagged.ts",
            Some(NodeRole::Test),
        ),
        (
            "src/codec.live.ts",
            "src/codec.fixture.ts",
            Some(NodeRole::Fixture),
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        support::write_graph(&path)?;
        let mut graph = GraphDocument::load(&path)?;
        let template = graph.graph.files.first().cloned().ok_or("missing file")?;
        graph.nodes.clear();
        graph.links.clear();
        for (id, file, role) in [
            ("z:production", production_path, None),
            ("a:test", test_path, test_role),
        ] {
            let source = directory.path().join(file);
            std::fs::create_dir_all(source.parent().ok_or("missing parent")?)?;
            std::fs::write(source, b"code")?;
            let mut record = template.clone();
            record.id = file_id(file);
            record.path = file.to_owned();
            graph.graph.files.push(record);
            let mut node =
                support::node(id, NodeKind::Function, "format_payload", "format_payload");
            node.source.as_mut().ok_or("missing source")?.file = file.to_owned();
            for proof in &mut node.evidence {
                for anchor in &mut proof.anchors {
                    anchor.file = file.to_owned();
                }
            }
            if let Some(role) = role {
                node.roles.push(role);
            }
            graph.nodes.push(node);
        }
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let json = open(&path, None, &directory.path().join("json-cache"))?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = open_with_store(&store, &path, None, &directory.path().join("store-cache"))?;
        let explicit = if test_role == Some(NodeRole::Fixture) {
            "where are payload formatting fixtures"
        } else {
            "where are payload formatting tests"
        };
        for (question, expected) in [
            ("where are payloads formatted", "z:production"),
            (explicit, "a:test"),
        ] {
            let response = json.discover_ranked(request(question))?;
            assert_eq!(
                response.candidates.first().map(|row| row.node.id.as_str()),
                Some(expected),
                "{test_path}: {question}"
            );
            assert_eq!(
                response.candidates,
                stored.discover_ranked(request(question))?.candidates,
                "{test_path}: backend parity"
            );
            assert!(
                response
                    .candidates
                    .iter()
                    .any(|row| row.node.id == "a:test")
            );
            assert!(
                response
                    .candidates
                    .iter()
                    .any(|row| row.node.id == "z:production")
            );
        }
    }
    Ok(())
}
