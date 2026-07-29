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
    let objc_source = b"@protocol Renderable\n- (void)render;\n@end\n";
    let objc_extraction = Engine::default().extract_source(&objc, objc_source)?;
    assert!(
        kinds(&objc_extraction).contains("protocol"),
        "nodes={:?}",
        objc_extraction.nodes
    );
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
