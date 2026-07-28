use compass_model::code_graph::{
    BuildMetadata, CODE_GRAPH_SCHEMA_V1, DiagnosticSeverity, EdgeKind, EdgeRecord, GraphMetadata,
    NodeKind, NodeRecord, NodeRole,
};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use serde::Serialize;
use serde_json::{Value, json};

fn serialized<T: Serialize>(value: T) -> Result<String, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(value)?)
}

#[test]
fn v1_vocabularies_serialize_to_the_closed_contract() -> Result<(), Box<dyn std::error::Error>> {
    let node_kinds = [
        (NodeKind::File, "file"),
        (NodeKind::Module, "module"),
        (NodeKind::Package, "package"),
        (NodeKind::Namespace, "namespace"),
        (NodeKind::Class, "class"),
        (NodeKind::Struct, "struct"),
        (NodeKind::Interface, "interface"),
        (NodeKind::Trait, "trait"),
        (NodeKind::Protocol, "protocol"),
        (NodeKind::Enum, "enum"),
        (NodeKind::EnumMember, "enum_member"),
        (NodeKind::TypeAlias, "type_alias"),
        (NodeKind::Function, "function"),
        (NodeKind::Method, "method"),
        (NodeKind::Constructor, "constructor"),
        (NodeKind::Property, "property"),
        (NodeKind::Field, "field"),
        (NodeKind::Variable, "variable"),
        (NodeKind::Constant, "constant"),
        (NodeKind::Parameter, "parameter"),
        (NodeKind::Import, "import"),
        (NodeKind::Export, "export"),
        (NodeKind::Macro, "macro"),
        (NodeKind::Annotation, "annotation"),
        (NodeKind::Route, "route"),
        (NodeKind::Component, "component"),
        (NodeKind::Event, "event"),
        (NodeKind::Message, "message"),
        (NodeKind::Topic, "topic"),
        (NodeKind::Queue, "queue"),
        (NodeKind::Job, "job"),
        (NodeKind::Resource, "resource"),
        (NodeKind::Schema, "schema"),
        (NodeKind::Query, "query"),
        (NodeKind::Migration, "migration"),
        (NodeKind::ConfigKey, "config_key"),
        (NodeKind::Database, "database"),
        (NodeKind::DatabaseSchema, "database_schema"),
        (NodeKind::DatabaseTable, "database_table"),
        (NodeKind::DatabaseView, "database_view"),
        (NodeKind::DatabaseColumn, "database_column"),
        (NodeKind::DatabaseIndex, "database_index"),
        (NodeKind::DatabaseConstraint, "database_constraint"),
        (NodeKind::DatabaseProcedure, "database_procedure"),
        (NodeKind::DatabaseTrigger, "database_trigger"),
    ];
    for (kind, expected) in node_kinds {
        assert_eq!(serialized(kind)?, expected);
    }

    let edge_kinds = [
        (EdgeKind::Contains, "contains"),
        (EdgeKind::Calls, "calls"),
        (EdgeKind::Imports, "imports"),
        (EdgeKind::Exports, "exports"),
        (EdgeKind::Extends, "extends"),
        (EdgeKind::Implements, "implements"),
        (EdgeKind::References, "references"),
        (EdgeKind::TypeOf, "type_of"),
        (EdgeKind::Returns, "returns"),
        (EdgeKind::Instantiates, "instantiates"),
        (EdgeKind::Overrides, "overrides"),
        (EdgeKind::Decorates, "decorates"),
        (EdgeKind::RoutesTo, "routes_to"),
        (EdgeKind::Reads, "reads"),
        (EdgeKind::Writes, "writes"),
        (EdgeKind::Aliases, "aliases"),
        (EdgeKind::Registers, "registers"),
        (EdgeKind::Handles, "handles"),
        (EdgeKind::Publishes, "publishes"),
        (EdgeKind::Subscribes, "subscribes"),
        (EdgeKind::Produces, "produces"),
        (EdgeKind::Consumes, "consumes"),
        (EdgeKind::Schedules, "schedules"),
        (EdgeKind::Triggers, "triggers"),
        (EdgeKind::Tests, "tests"),
        (EdgeKind::DependsOn, "depends_on"),
        (EdgeKind::Documents, "documents"),
        (EdgeKind::MapsTo, "maps_to"),
    ];
    for (kind, expected) in edge_kinds {
        assert_eq!(serialized(kind)?, expected);
    }

    assert_eq!(serialized(NodeRole::RouteHandler)?, "route_handler");
    assert_eq!(serialized(EvidenceOrigin::Config)?, "config");
    assert_eq!(serialized(EvidenceOrigin::Artifact)?, "artifact");
    assert_eq!(serialized(EvidenceConfidence::Ambiguous)?, "ambiguous");
    assert_eq!(serialized(DiagnosticSeverity::Warning)?, "warning");
    Ok(())
}

#[test]
fn typed_records_use_camel_case_fields_and_networkx_edge_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let node: NodeRecord = serde_json::from_value(json!({
        "id": "node:orders-controller",
        "kind": "method",
        "roles": ["controller", "route_handler"],
        "name": "show",
        "qualifiedName": "OrdersController.show",
        "language": "typescript",
        "framework": "nestjs",
        "source": {
            "file": "src/orders/controller.ts",
            "startByte": 1842,
            "endByte": 1881,
            "startLine": 61,
            "startColumn": 8,
            "endLine": 61,
            "endColumn": 47
        },
        "details": {
            "type": "symbol",
            "data": {
                "signature": "show(id: string): Promise<Order>",
                "modifiers": ["public"]
            }
        },
        "evidence": [{
            "origin": "ast",
            "extractor": "compass.languages.typescript",
            "confidence": "exact",
            "anchors": [{
                "file": "src/orders/controller.ts",
                "startByte": 1842,
                "endByte": 1881,
                "startLine": 61,
                "startColumn": 8,
                "endLine": 61,
                "endColumn": 47
            }]
        }]
    }))?;
    assert_eq!(node.kind, NodeKind::Method);
    assert_eq!(node.roles, [NodeRole::Controller, NodeRole::RouteHandler]);

    let edge: EdgeRecord = serde_json::from_value(json!({
        "id": "edge:orders-route",
        "key": "edge:orders-route",
        "source": "route:get-orders",
        "target": "node:orders-controller",
        "kind": "routes_to",
        "details": {
            "type": "route",
            "data": {
                "stage": "handler",
                "operation": "GET"
            }
        },
        "evidence": [{
            "origin": "ast",
            "extractor": "compass.languages.nestjs",
            "confidence": "exact",
            "anchors": []
        }]
    }))?;
    assert!(edge.has_networkx_identity());

    let value = serde_json::to_value(edge)?;
    assert_eq!(value["kind"], "routes_to");
    assert_eq!(value["key"], value["id"]);
    assert!(value.get("relation").is_none());
    Ok(())
}

#[test]
fn strict_records_reject_unknown_values_and_fields() {
    let base_node = json!({
        "id": "node:x",
        "kind": "function",
        "name": "x",
        "qualifiedName": "x",
        "evidence": [{
            "origin": "ast",
            "extractor": "compass.languages.rust",
            "confidence": "exact",
            "anchors": []
        }]
    });

    let mut unknown_kind = base_node.clone();
    unknown_kind["kind"] = json!("callable");
    assert!(serde_json::from_value::<NodeRecord>(unknown_kind).is_err());

    let mut unknown_role = base_node.clone();
    unknown_role["roles"] = json!(["endpoint"]);
    assert!(serde_json::from_value::<NodeRecord>(unknown_role).is_err());

    let mut unknown_origin = base_node.clone();
    unknown_origin["evidence"][0]["origin"] = json!("program_ir");
    assert!(serde_json::from_value::<NodeRecord>(unknown_origin).is_err());

    let mut unknown_confidence = base_node.clone();
    unknown_confidence["evidence"][0]["confidence"] = json!("probable");
    assert!(serde_json::from_value::<NodeRecord>(unknown_confidence).is_err());

    let mut unknown_severity = base_node.clone();
    unknown_severity["diagnostics"] = json!([{
        "severity": "fatal",
        "code": "invalid",
        "message": "invalid"
    }]);
    assert!(serde_json::from_value::<NodeRecord>(unknown_severity).is_err());

    let mut unknown_field = base_node;
    unknown_field["relation"] = json!("calls");
    assert!(serde_json::from_value::<NodeRecord>(unknown_field).is_err());

    let unknown_edge_kind = json!({
        "id": "edge:x",
        "key": "edge:x",
        "source": "node:a",
        "target": "node:b",
        "kind": "invokes",
        "evidence": []
    });
    assert!(serde_json::from_value::<EdgeRecord>(unknown_edge_kind).is_err());

    let unknown_detail_field = json!({
        "id": "node:details",
        "kind": "function",
        "name": "details",
        "qualifiedName": "details",
        "details": {
            "type": "symbol",
            "data": {
                "signature": "details()",
                "dynamic": true
            }
        },
        "evidence": []
    });
    assert!(serde_json::from_value::<NodeRecord>(unknown_detail_field).is_err());
}

#[test]
fn graph_metadata_constructor_pins_the_first_supported_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let metadata = GraphMetadata::v1(BuildMetadata {
        builder_version: "compass/1.0.0".to_owned(),
        schema_fingerprint: "sha256:schema".to_owned(),
        source_tree_digest: "sha256:source".to_owned(),
        configuration_digest: "sha256:config".to_owned(),
        generation_id: "sha256:generation".to_owned(),
    });

    let value = serde_json::to_value(metadata)?;
    assert_eq!(value["schema"], CODE_GRAPH_SCHEMA_V1);
    assert_eq!(value["build"]["builderVersion"], "compass/1.0.0");
    assert!(value["build"].get("timestamp").is_none());

    let mut unknown = value;
    unknown["build"]["hostname"] = Value::String("builder.local".to_owned());
    assert!(serde_json::from_value::<GraphMetadata>(unknown).is_err());
    Ok(())
}
