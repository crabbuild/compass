use std::collections::HashSet;
use std::error::Error;
use std::fs;

use compass_languages::{Engine, Extraction};

fn kinds(extraction: &Extraction) -> HashSet<String> {
    extraction
        .nodes
        .iter()
        .map(|node| node.string("symbol_kind"))
        .collect()
}

fn relations(extraction: &Extraction) -> HashSet<String> {
    extraction
        .edges
        .iter()
        .map(|edge| edge.string("relation"))
        .collect()
}

#[test]
fn rust_semantics_publish_first_class_declarations_and_local_relationships()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("semantic.rs");
    let source = br#"
pub struct Product { pub value: u64 }
pub type ProductId = Product;
pub enum Mode { Fast, Safe }
pub trait Renderable { fn render(&self, product: Product) -> Product; }
pub const DEFAULT: Product = Product { value: 1 };
macro_rules! product { () => { Product { value: 1 } }; }
impl Renderable for Product {
    fn render(&self, product: Product) -> Product { product }
}
pub fn target(product: Product) -> Product { product }
#[test]
fn target_is_rendered() { target(Product { value: 1 }); }
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let node_kinds = kinds(&extraction);
    let edge_kinds = relations(&extraction);

    for expected in [
        "trait",
        "enum_member",
        "type_alias",
        "field",
        "constant",
        "parameter",
        "macro",
    ] {
        assert!(
            node_kinds.contains(expected),
            "missing {expected}: nodes={:?}",
            extraction.nodes
        );
    }
    for expected in ["type_of", "returns", "overrides", "aliases", "tests"] {
        assert!(
            edge_kinds.contains(expected),
            "missing {expected}: edges={:?}",
            extraction.edges
        );
    }
    let test = extraction
        .nodes
        .iter()
        .find(|node| node.label().contains("target_is_rendered"))
        .ok_or("missing test function")?;
    assert_eq!(test.attributes["roles"], serde_json::json!(["test"]));
    for edge in extraction.edges.iter().filter(|edge| {
        matches!(
            edge.string("relation").as_str(),
            "type_of" | "returns" | "overrides" | "aliases" | "tests"
        )
    }) {
        let start = edge.attributes["start_byte"]
            .as_u64()
            .ok_or("missing start byte")?;
        let end = edge.attributes["end_byte"]
            .as_u64()
            .ok_or("missing end byte")?;
        assert!(start < end && end <= source.len() as u64, "edge={edge:?}");
    }
    Ok(())
}

#[test]
fn typescript_semantics_publish_exports_annotations_properties_and_constructors()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("semantic.ts");
    let source = br#"
function sealed<T extends { new (...args: any[]): object }>(constructor: T) {
  return constructor;
}
interface Contract {}
class Base { run(value: Contract): Contract { return value; } }
@sealed
export class Service extends Base {
  current: Contract;
  constructor(value: Contract) { this.current = value; }
  override run(value: Contract): Contract { return value; }
}
export { Service as DefaultService };
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let node_kinds = kinds(&extraction);
    let edge_kinds = relations(&extraction);

    for expected in [
        "constructor",
        "property",
        "parameter",
        "export",
        "annotation",
    ] {
        assert!(
            node_kinds.contains(expected),
            "missing {expected}: nodes={:?}",
            extraction.nodes
        );
    }
    for expected in [
        "type_of",
        "returns",
        "overrides",
        "decorates",
        "aliases",
        "exports",
    ] {
        assert!(
            edge_kinds.contains(expected),
            "missing {expected}: edges={:?}",
            extraction.edges
        );
    }
    Ok(())
}

#[test]
fn csharp_and_objective_c_publish_members_and_protocols() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let csharp = directory.path().join("Semantic.cs");
    let csharp_source = br#"
class Value {}
class Base {
    public virtual Value Run(Value value) => value;
}
class Service : Base {
    public const int Limit = 4;
    private Value current;
    public Value Current { get; set; }
    public Service(Value current) { this.current = current; }
    public override Value Run(Value value) => value;
}
"#;
    let csharp_extraction = Engine::default().extract_source(&csharp, csharp_source)?;
    let node_kinds = kinds(&csharp_extraction);
    let edge_kinds = relations(&csharp_extraction);
    for expected in ["constructor", "property", "field", "constant", "parameter"] {
        assert!(
            node_kinds.contains(expected),
            "missing {expected}: nodes={:?}",
            csharp_extraction.nodes
        );
    }
    for expected in ["type_of", "returns", "overrides"] {
        assert!(
            edge_kinds.contains(expected),
            "missing {expected}: edges={:?}",
            csharp_extraction.edges
        );
    }

    let objc = directory.path().join("Protocol.mm");
    let objc_source =
        b"@protocol BaseProtocol\n@end\n@protocol Renderable <BaseProtocol>\n- (void)render;\n@end\n";
    let objc_extraction = Engine::default().extract_source(&objc, objc_source)?;
    assert!(
        kinds(&objc_extraction).contains("protocol"),
        "nodes={:?}",
        objc_extraction.nodes
    );
    let protocol_ids = objc_extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "protocol")
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let protocol_edges = objc_extraction
        .edges
        .iter()
        .filter(|edge| {
            (edge.string("relation") == "contains" && protocol_ids.contains(&edge.target))
                || (edge.string("relation") == "implements" && protocol_ids.contains(&edge.source))
        })
        .collect::<Vec<_>>();
    assert!(
        protocol_edges.len() >= 3,
        "edges={:?}",
        objc_extraction.edges
    );
    for edge in protocol_edges {
        assert!(
            edge.attributes["start_byte"].as_u64() < edge.attributes["end_byte"].as_u64(),
            "edge={edge:?}"
        );
    }
    Ok(())
}

#[test]
fn config_and_document_artifacts_publish_valid_dependency_schema_and_documentation_facts()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("runtime.rs");
    fs::write(&source_path, b"pub fn runtime() {}\n")?;
    let guide_path = directory.path().join("guide.md");
    fs::write(&guide_path, b"# Guide\n\n[Runtime](runtime.rs)\n")?;
    let schema_path = directory.path().join("runtime.schema.json");
    fs::write(
        &schema_path,
        br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}"#,
    )?;
    let mcp_path = directory.path().join("mcp.json");
    fs::write(
        &mcp_path,
        br#"{"mcpServers":{"runtime":{"command":"node","env":{"TOKEN":"secret"}}}}"#,
    )?;

    let guide = Engine::default().extract(&guide_path)?;
    assert!(
        relations(&guide).contains("documents"),
        "edges={:?}",
        guide.edges
    );
    assert!(
        guide.edges.iter().any(|edge| {
            edge.string("relation") == "documents" && edge.string("_origin") == "artifact"
        }),
        "edges={:?}",
        guide.edges
    );
    let schema = Engine::default().extract(&schema_path)?;
    assert!(
        kinds(&schema).contains("schema"),
        "nodes={:?}",
        schema.nodes
    );
    let mcp = Engine::default().extract(&mcp_path)?;
    let config = mcp
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "config_key")
        .ok_or("missing MCP environment config key")?;
    assert_eq!(config.string("format"), "mcp");
    assert_eq!(config.string("key_path"), "mcpServers.runtime.env.TOKEN");
    assert!(
        mcp.edges
            .iter()
            .any(|edge| { edge.target == config.id && edge.string("relation") == "depends_on" })
    );
    assert!(
        !mcp.nodes
            .iter()
            .any(|node| node.string("symbol_kind") == "variable"),
        "nodes={:?}",
        mcp.nodes
    );
    Ok(())
}

#[test]
fn unresolved_annotations_do_not_create_cross_file_type_relationships() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("negative.ts");
    let source = b"let value: MissingType; function load(): MissingType { return value; }\n";
    let extraction = Engine::default().extract_source(&path, source)?;
    assert!(
        extraction.edges.iter().all(|edge| {
            !matches!(
                edge.string("relation").as_str(),
                "type_of" | "returns" | "overrides" | "aliases"
            )
        }),
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}

#[test]
fn scoped_duplicates_and_overloads_preserve_every_occurrence() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let rust_path = directory.path().join("scoped.rs");
    let rust_source = br#"
mod alpha { pub struct Item { pub value: u8 } }
mod beta { pub struct Item { pub value: u16 } }
"#;
    let rust = Engine::default().extract_source(&rust_path, rust_source)?;
    let rust_items = rust
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "struct" && node.label() == "Item")
        .collect::<Vec<_>>();
    assert_eq!(rust_items.len(), 2, "nodes={:?}", rust.nodes);
    assert!(
        rust_items
            .iter()
            .any(|node| node.string("qualified_name").contains("alpha::Item"))
    );
    assert!(
        rust_items
            .iter()
            .any(|node| node.string("qualified_name").contains("beta::Item"))
    );

    let ts_path = directory.path().join("scoped.ts");
    let ts_source = br#"
namespace Alpha { export class Item { value: string; } }
namespace Beta { export class Item { value: number; } }
class Service {
  constructor(value: string);
  constructor(value: number);
  constructor(value: string | number) {}
  run(value: string): void;
  run(value: number): void;
  run(value: string | number): void {}
}
"#;
    let ts = Engine::default().extract_source(&ts_path, ts_source)?;
    let ts_items = ts
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "class" && node.label() == "Item")
        .collect::<Vec<_>>();
    assert_eq!(ts_items.len(), 2, "nodes={:?}", ts.nodes);
    assert_eq!(
        ts.nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "constructor")
            .count(),
        3,
        "nodes={:?}",
        ts.nodes
    );
    assert!(
        ts.nodes
            .iter()
            .filter(|node| {
                node.string("symbol_kind") == "constructor"
                    || (node.string("symbol_kind") == "method" && node.label().contains("run"))
            })
            .all(|node| !node.string("overload_discriminator").is_empty())
    );

    let csharp_path = directory.path().join("Scoped.cs");
    let csharp_source = br#"
namespace Alpha { class Item { int value; } }
namespace Beta { class Item { string value; } }
class Service {
  public Service(string value) {}
  public Service(int value) {}
  public void Run(string value) {}
  public void Run(int value) {}
}
"#;
    let csharp = Engine::default().extract_source(&csharp_path, csharp_source)?;
    let csharp_items = csharp
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "class" && node.label() == "Item")
        .collect::<Vec<_>>();
    assert_eq!(csharp_items.len(), 2, "nodes={:?}", csharp.nodes);
    assert_eq!(
        csharp
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "constructor")
            .count(),
        2,
        "nodes={:?}",
        csharp.nodes
    );
    assert_eq!(
        csharp
            .nodes
            .iter()
            .filter(|node| {
                node.string("symbol_kind") == "method" && node.label().contains("Run")
            })
            .count(),
        2,
        "nodes={:?}",
        csharp.nodes
    );
    Ok(())
}

#[test]
fn semantic_modifiers_exports_decorators_and_test_attributes_are_ast_only()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let csharp = directory.path().join("Modifiers.cs");
    let csharp_source = br#"
class Base { public virtual void Run() {} }
class Service : Base {
  public string Marker = " const ";
  public void Run() { string marker = " override "; }
}
"#;
    let extraction = Engine::default().extract_source(&csharp, csharp_source)?;
    assert!(
        !extraction
            .nodes
            .iter()
            .any(|node| node.string("symbol_kind") == "constant"),
        "nodes={:?}",
        extraction.nodes
    );
    assert!(
        !relations(&extraction).contains("overrides"),
        "edges={:?}",
        extraction.edges
    );

    let ts = directory.path().join("Modifiers.ts");
    let ts_source = br#"
namespace ns { export function decorate(value: object) { return value; } }
class A {}
class B {}
@ns.decorate()
class Service { run() { const marker = " override "; } }
export { A as First, B as Second, Service };
"#;
    let extraction = Engine::default().extract_source(&ts, ts_source)?;
    assert!(
        !relations(&extraction).contains("overrides"),
        "edges={:?}",
        extraction.edges
    );
    let exports = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "export")
        .map(|node| node.label())
        .collect::<HashSet<_>>();
    assert!(exports.contains("First"), "nodes={:?}", extraction.nodes);
    assert!(exports.contains("Second"), "nodes={:?}", extraction.nodes);
    assert!(exports.contains("Service"), "nodes={:?}", extraction.nodes);
    assert!(
        extraction.nodes.iter().any(|node| {
            node.string("symbol_kind") == "annotation" && node.label() == "ns.decorate"
        }),
        "nodes={:?}",
        extraction.nodes
    );

    let rust = directory.path().join("attributes.rs");
    let rust_source = br#"
fn target() {}
#[test]
#[should_panic]
fn checks_target() { target(); panic!("expected"); }
"#;
    let extraction = Engine::default().extract_source(&rust, rust_source)?;
    let test = extraction
        .nodes
        .iter()
        .find(|node| node.label().contains("checks_target"))
        .ok_or("missing multi-attribute test")?;
    assert_eq!(test.attributes["roles"], serde_json::json!(["test"]));
    assert!(relations(&extraction).contains("tests"));
    Ok(())
}

#[test]
fn markdown_occurrences_fences_and_config_relationships_have_exact_ranges()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = directory.path().join("runtime.rs");
    fs::write(&runtime, b"pub fn runtime() {}\n")?;
    let guide = directory.path().join("guide.md");
    fs::write(
        &guide,
        b"# Guide\n\n[One](runtime.rs)\n\n[Two](runtime.rs)\n\n~~~rust\n[Example](runtime.rs)\n~~~\n\n````\n[Backtick](runtime.rs)\n````\n",
    )?;
    let extraction = Engine::default().extract(&guide)?;
    let documents = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "documents")
        .collect::<Vec<_>>();
    assert_eq!(documents.len(), 2, "edges={:?}", extraction.edges);
    assert_ne!(
        documents[0].attributes["start_byte"],
        documents[1].attributes["start_byte"]
    );
    assert!(
        documents.iter().all(|edge| {
            edge.string("_origin") == "artifact"
                && edge.attributes["start_byte"].as_u64() < edge.attributes["end_byte"].as_u64()
        }),
        "edges={:?}",
        extraction.edges
    );

    let mcp = directory.path().join("mcp.json");
    fs::write(
        &mcp,
        br#"{
  "mcpServers": {
    "runtime": {
      "command": "node",
      "env": {
        "TOKEN": "do-not-publish"
      }
    }
  }
}"#,
    )?;
    let extraction = Engine::default().extract(&mcp)?;
    let config = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "config_key")
        .ok_or("missing config key")?;
    assert_eq!(config.attributes["start_line"], serde_json::json!(6));
    assert!(config.attributes["start_byte"].as_u64() < config.attributes["end_byte"].as_u64());
    assert!(!format!("{extraction:?}").contains("do-not-publish"));
    let dependency = extraction
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "depends_on")
        .ok_or("missing config dependency")?;
    assert_eq!(dependency.attributes["start_line"], serde_json::json!(6));
    Ok(())
}

#[test]
fn semantic_inventory_reports_bounded_deterministic_work() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("large.rs");
    let mut source = String::new();
    for index in 0..500 {
        source.push_str(&format!(
            "mod m{index} {{ pub struct Item{index} {{ pub value: u64 }} }}\n"
        ));
    }
    let extraction = Engine::default().extract_source(&path, source.as_bytes())?;
    let work = extraction
        .extensions
        .get("_semantic_work")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing semantic work counter")?;
    let visits = work["ast_visits"].as_u64().ok_or("missing visits")?;
    let lookups = work["index_lookups"].as_u64().ok_or("missing lookups")?;
    assert!(visits < 20_000, "work={work:?}");
    assert!(lookups < 10_000, "work={work:?}");
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| {
                node.string("symbol_kind") == "struct"
                    && node.string("qualified_name").contains("Item")
            })
            .count(),
        500
    );
    Ok(())
}
