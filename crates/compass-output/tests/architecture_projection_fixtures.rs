use std::collections::BTreeMap;
use std::error::Error;

use compass_model::GraphDocument;
use compass_output::{
    ARCHITECTURE_VIEWER_SCHEMA, ArchitectureLens, ArchitectureProjectionInput,
    ArchitectureProjectionOptions, ArchitectureQualityStatus, ArchitectureRelationClass,
    ArchitectureScope, ArchitectureSourceScope, project_architecture,
};
use serde_json::json;

#[test]
fn generated_vendor_test_and_documentation_nodes_cannot_shape_production()
-> Result<(), Box<dyn Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {
            "files": [
                {"path":"crates/compass-output/assets/viewer/graph.js","generated":true}
            ]
        },
        "nodes": [
            {"id":"projection","label":"project_architecture","source_file":"crates/compass-output/src/architecture_projection/mod.rs"},
            {"id":"viewer","label":"VisualizationWorkbench","source_file":"packages/compass-viewer/src/workbench/VisualizationWorkbench.tsx"},
            {"id":"bundle","label":"a","source_file":"crates/compass-output/assets/viewer/graph.js"},
            {"id":"vendor","label":"dependency","source_file":"vendor/dependency/lib.rs"},
            {"id":"test","label":"projection_test","source_file":"tests/projection_test.rs"},
            {"id":"docs","label":"Architecture design","source_file":"docs/design/architecture.md"}
        ],
        "links": [
            {"source":"projection","target":"viewer","relation":"calls","confidence":"EXTRACTED"},
            {"source":"projection","target":"viewer","relation":"contains","confidence":"EXTRACTED"},
            {"source":"bundle","target":"projection","relation":"references","confidence":"EXTRACTED"},
            {"source":"vendor","target":"projection","relation":"imports","confidence":"EXTRACTED"},
            {"source":"test","target":"projection","relation":"tests","confidence":"EXTRACTED"},
            {"source":"docs","target":"projection","relation":"references","confidence":"EXTRACTED"}
        ]
    }))?;
    let communities = BTreeMap::from([
        (0, vec!["projection".to_owned(), "bundle".to_owned()]),
        (1, vec!["viewer".to_owned()]),
        (2, vec!["vendor".to_owned()]),
        (3, vec!["test".to_owned()]),
        (4, vec!["docs".to_owned()]),
    ]);
    let labels = BTreeMap::from([
        (0, "Crates Compass Output".to_owned()),
        (1, "Crates Compass Output".to_owned()),
        (2, "Crates Compass Output".to_owned()),
        (3, "Crates Compass Output".to_owned()),
        (4, "Crates Compass Output".to_owned()),
    ]);
    let model = project_architecture(
        ArchitectureProjectionInput {
            document: &document,
            communities: &communities,
            community_labels: Some(&labels),
            overlay: None,
            project_name: "Compass",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;

    assert_eq!(model.schema, ARCHITECTURE_VIEWER_SCHEMA);
    let scopes = model
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.source_scope))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(scopes["bundle"], ArchitectureSourceScope::Generated);
    assert_eq!(scopes["vendor"], ArchitectureSourceScope::Vendor);
    assert_eq!(scopes["test"], ArchitectureSourceScope::Test);
    assert_eq!(scopes["docs"], ArchitectureSourceScope::Documentation);

    let production = model
        .projections
        .iter()
        .find(|projection| projection.scope == ArchitectureScope::Production)
        .ok_or("missing Production")?;
    assert_eq!(production.memberships.len(), 2);
    assert_eq!(production.quality.metrics.generated_vendor_leakage, 0);
    assert_eq!(production.coverage.admitted, 1);
    assert_eq!(production.coverage.cross_group, 1);
    assert_eq!(production.coverage.relation_classes.execution, 1);
    assert_eq!(production.coverage.relation_classes.structure, 1);
    assert_eq!(production.coverage.relation_classes.contextual, 0);
    assert!(
        production
            .groups
            .iter()
            .all(|group| group.name.value != "Other")
    );
    let unique_names = production
        .groups
        .iter()
        .map(|group| group.name.value.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique_names.len(), production.groups.len());

    let all_code = model
        .projections
        .iter()
        .find(|projection| projection.scope == ArchitectureScope::AllCode)
        .ok_or("missing All-code")?;
    assert_eq!(all_code.memberships.len(), model.nodes.len());
    Ok(())
}

#[test]
fn overview_omission_is_metadata_not_a_connected_group() -> Result<(), Box<dyn Error>> {
    let nodes = (0..40)
        .map(|index| {
            json!({
                "id": format!("n{index}"),
                "label": format!("Subsystem{index}"),
                "source_file": format!("packages/package-{index}/src/lib.rs")
            })
        })
        .collect::<Vec<_>>();
    let links = (0..39)
        .map(|index| {
            json!({
                "source": format!("n{index}"),
                "target": format!("n{}", index + 1),
                "relation":"imports",
                "confidence":"EXTRACTED"
            })
        })
        .collect::<Vec<_>>();
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": nodes,
        "links": links
    }))?;
    let communities = (0..40)
        .map(|index| (index, vec![format!("n{index}")]))
        .collect::<BTreeMap<_, _>>();
    let defaults = ArchitectureProjectionOptions::default();
    let options = ArchitectureProjectionOptions {
        limits: compass_output::ArchitectureProjectionLimits {
            max_overview_groups: 8,
            max_overview_routes: 8,
            ..defaults.limits
        },
        ..defaults
    };
    let model = project_architecture(
        ArchitectureProjectionInput {
            document: &document,
            communities: &communities,
            community_labels: None,
            overlay: None,
            project_name: "Many packages",
            built_at_commit: None,
            generated_at: None,
        },
        &options,
    )?;
    let production = model
        .projections
        .iter()
        .find(|projection| projection.scope == ArchitectureScope::Production)
        .ok_or("missing Production")?;
    assert_eq!(production.omissions.total_groups, 40);
    assert_eq!(production.omissions.shown_groups, 8);
    assert_eq!(production.omissions.omitted_groups, 32);
    assert!(production.omissions.omitted_nodes > 0);
    assert!(production.groups.iter().all(|group| group.id != "other"));
    assert!(
        production
            .routes
            .iter()
            .all(|route| { route.source_group != "other" && route.target_group != "other" })
    );
    assert_eq!(
        production.quality.status,
        ArchitectureQualityStatus::Degraded
    );
    Ok(())
}

#[test]
fn relation_classes_preserve_original_relationships_but_control_lenses()
-> Result<(), Box<dyn Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": [
            {"id":"a","source_file":"src/a.rs"},
            {"id":"b","source_file":"src/b.rs"}
        ],
        "links": [
            {"source":"a","target":"b","relation":"calls"},
            {"source":"a","target":"b","relation":"imports"},
            {"source":"a","target":"b","relation":"contains"},
            {"source":"a","target":"b","relation":"references"},
            {"source":"a","target":"b","relation":"future_relation"}
        ]
    }))?;
    let communities = BTreeMap::from([(0, vec!["a".to_owned()]), (1, vec!["b".to_owned()])]);
    let model = project_architecture(
        ArchitectureProjectionInput {
            document: &document,
            communities: &communities,
            community_labels: None,
            overlay: None,
            project_name: "Relations",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;
    assert_eq!(model.relationships.len(), 5);
    let classes = model
        .relationships
        .iter()
        .map(|relationship| (relationship.relation.as_str(), relationship.relation_class))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(classes["calls"], ArchitectureRelationClass::Execution);
    assert_eq!(classes["imports"], ArchitectureRelationClass::Dependency);
    assert_eq!(classes["contains"], ArchitectureRelationClass::Structure);
    assert_eq!(classes["references"], ArchitectureRelationClass::Contextual);
    assert_eq!(
        classes["future_relation"],
        ArchitectureRelationClass::Unknown
    );
    assert!(ArchitectureLens::Architecture.admits(classes["calls"]));
    assert!(ArchitectureLens::Architecture.admits(classes["imports"]));
    assert!(!ArchitectureLens::Architecture.admits(classes["contains"]));
    assert!(!ArchitectureLens::Architecture.admits(classes["references"]));
    Ok(())
}

#[test]
fn names_do_not_control_identity_and_unaffected_groups_survive_edits() -> Result<(), Box<dyn Error>>
{
    let base: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": [
            {"id":"a","label":"Alpha","source_file":"crates/runtime/src/a.rs"},
            {"id":"b","label":"Beta","source_file":"crates/storage/src/b.rs"}
        ],
        "links": [{"source":"a","target":"b","relation":"calls"}]
    }))?;
    let communities = BTreeMap::from([(7, vec!["a".to_owned()]), (42, vec!["b".to_owned()])]);
    let first = project_architecture(
        ArchitectureProjectionInput {
            document: &base,
            communities: &communities,
            community_labels: Some(&BTreeMap::from([
                (7, "Runtime".to_owned()),
                (42, "Storage".to_owned()),
            ])),
            overlay: None,
            project_name: "Fixture",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;
    let renamed = project_architecture(
        ArchitectureProjectionInput {
            document: &base,
            communities: &communities,
            community_labels: Some(&BTreeMap::from([
                (7, "Request Runtime".to_owned()),
                (42, "Ledger Storage".to_owned()),
            ])),
            overlay: None,
            project_name: "Fixture",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;
    let ids = |model: &compass_output::ArchitectureViewModel| {
        model.projections[0]
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(ids(&first), ids(&renamed));
    assert_eq!(first.relationships[0].id, renamed.relationships[0].id);

    let edited: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": [
            {"id":"a","label":"Alpha","source_file":"crates/runtime/src/a.rs"},
            {"id":"a2","label":"AlphaHelper","source_file":"crates/runtime/src/a2.rs"},
            {"id":"b","label":"Beta","source_file":"crates/storage/src/b.rs"}
        ],
        "links": [{"source":"a","target":"b","relation":"calls"}]
    }))?;
    let edited_communities = BTreeMap::from([
        (7, vec!["a".to_owned(), "a2".to_owned()]),
        (42, vec!["b".to_owned()]),
    ]);
    let after = project_architecture(
        ArchitectureProjectionInput {
            document: &edited,
            communities: &edited_communities,
            community_labels: None,
            overlay: None,
            project_name: "Fixture",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;
    let before_node_index = first
        .nodes
        .iter()
        .position(|node| node.id == "b")
        .ok_or("missing b node")?;
    let after_node_index = after
        .nodes
        .iter()
        .position(|node| node.id == "b")
        .ok_or("missing b node after")?;
    let storage_before = first.projections[0]
        .memberships
        .iter()
        .find(|item| item.node_index == before_node_index)
        .ok_or("missing b")?;
    let storage_after = after.projections[0]
        .memberships
        .iter()
        .find(|item| item.node_index == after_node_index)
        .ok_or("missing b after")?;
    assert_eq!(
        first.projections[0].groups[storage_before.group_index].id,
        after.projections[0].groups[storage_after.group_index].id
    );
    Ok(())
}

#[test]
fn zero_configuration_corpus_covers_workspace_monorepo_mixed_and_sparse_shapes()
-> Result<(), Box<dyn Error>> {
    let paths = [
        ("cargo", "crates/runtime/src/lib.rs"),
        ("npm", "packages/web/src/app.ts"),
        ("mixed", "services/search/src/main.py"),
        ("single", "src/domain.rs"),
        ("generated", "generated/client/api.ts"),
        ("vendor", "vendor/sqlite/sqlite.c"),
        ("test", "tests/runtime_test.rs"),
        ("unknown", ""),
    ];
    let nodes = paths
        .iter()
        .map(|(id, path)| {
            if path.is_empty() {
                json!({"id":id,"label":id,"community":0})
            } else {
                json!({"id":id,"label":id,"source_file":path,"community":0})
            }
        })
        .collect::<Vec<_>>();
    let document: GraphDocument = serde_json::from_value(json!({
        "graph": {},
        "nodes": nodes,
        "links": [
            {"source":"cargo","target":"npm","relation":"imports"},
            {"source":"npm","target":"mixed","relation":"calls"}
        ]
    }))?;
    let communities = BTreeMap::from([(
        0,
        paths
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect::<Vec<_>>(),
    )]);
    let model = project_architecture(
        ArchitectureProjectionInput {
            document: &document,
            communities: &communities,
            community_labels: None,
            overlay: None,
            project_name: "Corpus",
            built_at_commit: None,
            generated_at: None,
        },
        &ArchitectureProjectionOptions::default(),
    )?;
    let production = model
        .projections
        .iter()
        .find(|projection| projection.scope == ArchitectureScope::Production)
        .ok_or("missing Production")?;
    let all_code = model
        .projections
        .iter()
        .find(|projection| projection.scope == ArchitectureScope::AllCode)
        .ok_or("missing All-code")?;
    assert_eq!(production.memberships.len(), 4);
    assert_eq!(all_code.memberships.len(), 8);
    assert_eq!(production.quality.metrics.generated_vendor_leakage, 0);
    assert_eq!(production.quality.metrics.duplicate_names, 0);
    assert!(production.groups.len() >= 4);

    let corpora = [
        (
            "cargo-workspace",
            ["crates/api/src/lib.rs", "crates/store/src/lib.rs"],
            2,
        ),
        (
            "npm-monorepo",
            ["packages/web/src/app.ts", "packages/data/src/store.ts"],
            2,
        ),
        (
            "mixed-language",
            ["services/search/main.py", "packages/ui/src/view.tsx"],
            2,
        ),
        (
            "generated-heavy",
            ["src/runtime.rs", "generated/client/api.ts"],
            1,
        ),
        (
            "vendor-heavy",
            ["src/runtime.c", "third_party/sqlite/sqlite.c"],
            1,
        ),
        ("test-heavy", ["src/runtime.go", "tests/runtime_test.go"], 1),
        ("single-package", ["src/domain.rs", "src/storage.rs"], 2),
        (
            "sparse-relationships",
            ["app/domain.rb", "app/storage.rb"],
            2,
        ),
    ];
    for (name, source_paths, expected_production) in corpora {
        let corpus_document: GraphDocument = serde_json::from_value(json!({
            "graph": {},
            "nodes": source_paths.iter().enumerate().map(|(index, path)| json!({
                "id":format!("{name}-{index}"),
                "label":format!("Entry{index}"),
                "source_file":path
            })).collect::<Vec<_>>(),
            "links": []
        }))?;
        let corpus_communities = BTreeMap::from([(
            0,
            (0..source_paths.len())
                .map(|index| format!("{name}-{index}"))
                .collect(),
        )]);
        let corpus_model = project_architecture(
            ArchitectureProjectionInput {
                document: &corpus_document,
                communities: &corpus_communities,
                community_labels: None,
                overlay: None,
                project_name: name,
                built_at_commit: None,
                generated_at: None,
            },
            &ArchitectureProjectionOptions::default(),
        )?;
        let corpus_production = corpus_model
            .projections
            .iter()
            .find(|projection| projection.scope == ArchitectureScope::Production)
            .ok_or("missing corpus Production")?;
        assert_eq!(
            corpus_production.memberships.len(),
            expected_production,
            "{name}"
        );
        assert_ne!(
            corpus_production.quality.status,
            ArchitectureQualityStatus::Insufficient,
            "{name}"
        );
        assert_eq!(
            corpus_production.quality.metrics.generated_vendor_leakage, 0,
            "{name}"
        );
    }
    Ok(())
}
