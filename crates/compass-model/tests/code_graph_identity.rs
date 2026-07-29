use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::identity::{edge_id, file_id, normalize_repository_path, route_id, symbol_id};
use compass_model::provenance::SourceAnchor;

fn anchor(file: &str, start_byte: u64, end_byte: u64) -> SourceAnchor {
    SourceAnchor {
        file: file.to_owned(),
        start_byte,
        end_byte,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 1,
    }
}

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
        route_id("express", "src/routes.ts", "get", "/orders", "router", None,),
        route_id(
            "express",
            "src/routes.ts",
            "post",
            "/orders",
            "router",
            None,
        )
    );
}

#[test]
fn route_identity_includes_the_declaration_site() {
    let first = anchor("src/routes.ts", 10, 20);
    let second = anchor("src/routes.ts", 30, 40);
    assert_ne!(
        route_id(
            "express",
            "src/routes.ts",
            "get",
            "/orders",
            "router",
            Some(&first),
        ),
        route_id(
            "express",
            "src/routes.ts",
            "get",
            "/orders",
            "router",
            Some(&second),
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
