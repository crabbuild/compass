use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::identity::{edge_id, file_id, normalize_repository_path, route_id, symbol_id};
use compass_model::provenance::SourceAnchor;

#[test]
fn identities_are_schema_versioned_length_prefixed_and_portable() {
    assert_eq!(
        normalize_repository_path(r".\src\orders\..\orders\handler.ts"),
        "src/orders/handler.ts"
    );
    assert_eq!(
        file_id("src/orders/handler.ts"),
        file_id(r".\src\orders\handler.ts")
    );
    assert_ne!(file_id("ab/c"), file_id("a/bc"));
    assert_ne!(
        symbol_id(
            "typescript",
            "src/orders/handler.ts",
            NodeKind::Function,
            "orders.handle",
            ""
        ),
        symbol_id(
            "typescript",
            "src/orders/handler.ts",
            NodeKind::Method,
            "orders.handle",
            ""
        )
    );
    assert_ne!(
        route_id(
            "express",
            "src/routes.ts",
            "get",
            "/orders",
            "router",
            "orders.list",
        ),
        route_id(
            "express",
            "src/routes.ts",
            "post",
            "/orders",
            "router",
            "orders.list",
        )
    );
}

#[test]
fn route_identity_uses_semantic_target_namespace() {
    assert_ne!(
        route_id(
            "express",
            "src/routes.ts",
            "get",
            "/orders",
            "router",
            "orders.list",
        ),
        route_id(
            "express",
            "src/routes.ts",
            "get",
            "/orders",
            "router",
            "orders.show",
        )
    );
}

#[test]
fn relationship_site_participates_in_edge_identity() {
    let first = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 10,
        end_byte: 14,
        start_line: 2,
        start_column: 0,
        end_line: 2,
        end_column: 4,
    };
    let second = SourceAnchor {
        start_byte: 20,
        end_byte: 24,
        start_line: 3,
        end_line: 3,
        ..first.clone()
    };
    assert_ne!(
        edge_id("a", EdgeKind::Calls, "b", Some(&first), None),
        edge_id("a", EdgeKind::Calls, "b", Some(&second), None)
    );
    assert_ne!(
        edge_id("a", EdgeKind::Calls, "b", Some(&first), None),
        edge_id(
            "a",
            EdgeKind::Calls,
            "b",
            Some(&first),
            Some("dynamic-dispatch")
        )
    );
}
