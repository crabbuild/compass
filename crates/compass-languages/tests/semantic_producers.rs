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

fn assert_exact_containment(
    extraction: &Extraction,
    target_qualified_prefix: &str,
    owner_qualified_prefix: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let target = extraction
        .nodes
        .iter()
        .find(|node| {
            let qualified = node.string("qualified_name");
            if target_qualified_prefix.ends_with('@') {
                qualified.starts_with(target_qualified_prefix)
            } else {
                qualified == target_qualified_prefix
            }
        })
        .ok_or_else(|| {
            format!(
                "missing target {target_qualified_prefix}: {:?}",
                extraction
                    .nodes
                    .iter()
                    .map(|node| node.string("qualified_name"))
                    .collect::<Vec<_>>()
            )
        })?;
    let owner = match owner_qualified_prefix {
        Some(prefix) => extraction
            .nodes
            .iter()
            .find(|node| {
                let qualified = node.string("qualified_name");
                if prefix.ends_with('@') {
                    qualified.starts_with(prefix)
                } else {
                    qualified == prefix
                }
            })
            .ok_or_else(|| format!("missing owner {prefix}"))?,
        None => extraction
            .nodes
            .iter()
            .find(|node| {
                matches!(node.string("symbol_kind").as_str(), "file" | "source_file")
                    || (node.string("qualified_name").is_empty() && node.label().contains('.'))
            })
            .ok_or_else(|| {
                format!(
                    "missing file owner: {:?}",
                    extraction
                        .nodes
                        .iter()
                        .map(|node| (node.label(), node.string("symbol_kind")))
                        .collect::<Vec<_>>()
                )
            })?,
    };
    let start = target.attributes["start_byte"]
        .as_u64()
        .ok_or("missing target start byte")?;
    let end = target.attributes["end_byte"]
        .as_u64()
        .ok_or("missing target end byte")?;
    let occurrences = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "contains"
                && edge
                    .attributes
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64)
                    == Some(start)
                && edge
                    .attributes
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64)
                    == Some(end)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        occurrences.len(),
        1,
        "containment site {start}..{end} for {target_qualified_prefix}: {occurrences:#?}"
    );
    assert_eq!(occurrences[0].source, owner.id);
    assert_eq!(occurrences[0].target, target.id);
    Ok(())
}

fn assert_unique_node_ids(extraction: &Extraction) {
    let ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        ids.len(),
        extraction.nodes.len(),
        "nodes={:#?}",
        extraction.nodes
    );
}

fn assert_containment_sites_belong_to_targets(extraction: &Extraction) {
    for edge in extraction.edges.iter().filter(|edge| {
        matches!(
            edge.string("relation").as_str(),
            "contains" | "defines" | "method"
        )
    }) {
        let Some(target) = extraction.nodes.iter().find(|node| node.id == edge.target) else {
            continue;
        };
        if target.string("qualified_name").is_empty() {
            continue;
        }
        assert!(
            target.attributes.contains_key("start_byte")
                && target.attributes.contains_key("end_byte"),
            "legacy semantic alias retained containment: target={target:#?} edge={edge:#?}"
        );
        assert_eq!(
            (
                edge.attributes["start_byte"].as_u64(),
                edge.attributes["end_byte"].as_u64()
            ),
            (
                target.attributes["start_byte"].as_u64(),
                target.attributes["end_byte"].as_u64()
            ),
            "public containment alias must use its target declaration site: target={target:#?} edge={edge:#?}"
        );
    }
    for target in extraction.nodes.iter().filter(|node| {
        !node.string("qualified_name").is_empty()
            && node.attributes.contains_key("start_byte")
            && node.attributes.contains_key("end_byte")
            && matches!(
                node.string("symbol_kind").as_str(),
                "module"
                    | "namespace"
                    | "trait"
                    | "struct"
                    | "enum"
                    | "type_alias"
                    | "class"
                    | "interface"
                    | "record"
                    | "field"
                    | "property"
                    | "constant"
                    | "enum_member"
                    | "function"
                    | "method"
                    | "constructor"
                    | "parameter"
                    | "macro"
                    | "export"
                    | "annotation"
            )
    }) {
        let occurrences = extraction
            .edges
            .iter()
            .filter(|edge| {
                matches!(edge.string("relation").as_str(), "contains" | "method")
                    && edge.target == target.id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            occurrences.len(),
            1,
            "managed target must have exactly one containment: target={target:#?} edges={occurrences:#?}"
        );
        let edge = occurrences[0];
        let Some(site_start) = edge
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let Some(site_end) = edge
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let target_start = target.attributes["start_byte"].as_u64().unwrap_or_default();
        let target_end = target.attributes["end_byte"].as_u64().unwrap_or_default();
        assert_eq!(
            (site_start, site_end),
            (target_start, target_end),
            "managed containment site must equal target declaration: target={target:#?} edge={edge:#?}"
        );
    }
}

#[test]
fn javascript_prototype_and_fn_assignments_publish_bounded_methods() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("prototype.js");
    let source = br#"
function Widget() {}
function Other() {}
function Plugin() {}
function helper() {}
Widget.prototype.render = function () { helper(); };
Other.prototype.render = () => helper();
Plugin.fn.install = function () { helper(); };
const config = {};
config.render = function () { helper(); };
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let prototype_methods = extraction
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "Widget.prototype::render" | "Other.prototype::render" | "Plugin.fn::install"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(prototype_methods.len(), 3);
    assert!(prototype_methods.iter().all(|node| {
        node.string("symbol_kind") == "method"
            && extraction.edges.iter().any(|edge| {
                edge.string("relation") == "contains"
                    && edge.target == node.id
                    && extraction.nodes.iter().any(|owner| {
                        owner.id == edge.source
                            && matches!(owner.label(), "Widget()" | "Other()" | "Plugin()")
                    })
            })
            && extraction.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.source == node.id
                    && extraction
                        .nodes
                        .iter()
                        .any(|target| target.id == edge.target && target.label() == "helper()")
            })
    }));
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.label() == ".render()")
            .count(),
        2,
        "ordinary object-property assignments must not become prototype methods"
    );
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("qualified_name") != "config::render")
    );
    assert_unique_node_ids(&extraction);
    Ok(())
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
    let macro_node = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "macro")
        .ok_or("missing macro")?;
    assert!(
        macro_node.string("qualified_name").starts_with("product@"),
        "top-level macro identity must be lexical, not checkout-dependent: {macro_node:?}"
    );
    assert!(
        !macro_node
            .string("qualified_name")
            .contains(&directory.path().to_string_lossy().replace(['/', '\\'], "_")),
        "top-level macro identity embedded the checkout path: {macro_node:?}"
    );
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
fn rust_universal_occurrences_preserve_qualified_call_sites() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("qualified.rs");
    let source = br#"
struct Graph {}
impl Graph {
    fn new() -> Self { Self {} }
    fn add_edge(&mut self) {}
}
fn build(mut graph: Graph) {
    Graph::new();
    HashMap::new();
    graph.add_edge();
}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .universal_evidence
        .first()
        .ok_or("missing Rust universal evidence")?;
    assert_eq!(evidence.schema, "compass.languages.evidence/1");
    assert_eq!(evidence.adapter_id, "compass.rust");
    assert_eq!(evidence.adapter_version, 1);

    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == compass_languages::OccurrenceRole::Call)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3, "occurrences={calls:#?}");
    for (spelling, qualifier) in [("new", "Graph"), ("new", "HashMap"), ("add_edge", "graph")] {
        let occurrence = calls
            .iter()
            .find(|occurrence| {
                occurrence.spelling == spelling
                    && occurrence.qualifier.as_deref() == Some(qualifier)
            })
            .ok_or_else(|| format!("missing {qualifier}::{spelling}: {calls:#?}"))?;
        let start = usize::try_from(occurrence.anchor.start_byte)?;
        let end = usize::try_from(occurrence.anchor.end_byte)?;
        assert!(start < end && end <= source.len());
        assert_eq!(
            std::str::from_utf8(&source[start..end])?.replace('.', "::"),
            format!("{qualifier}::{spelling}()")
        );
    }
    assert!(
        calls
            .windows(2)
            .all(|pair| pair[0].anchor != pair[1].anchor),
        "each source use must keep a distinct occurrence: {calls:#?}"
    );
    Ok(())
}

#[test]
fn rust_qualified_calls_bind_only_to_the_exact_local_owner() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("qualified.rs");
    let source = br#"
struct Graph {}
impl Graph {
    fn new() -> Self { Self {} }
}
fn build() {
    Graph::new();
    HashMap::new();
}
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let local_new = extraction
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name")
                .starts_with("impl Graph::new(")
        })
        .ok_or("missing Graph::new declaration")?;
    let calls = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == local_new.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "edges={:#?}", extraction.edges);
    let call = calls[0];
    let start = usize::try_from(
        call.attributes["start_byte"]
            .as_u64()
            .ok_or("missing start")?,
    )?;
    let end = usize::try_from(call.attributes["end_byte"].as_u64().ok_or("missing end")?)?;
    assert_eq!(&source[start..end], b"Graph::new()");

    let hash_map = extraction
        .universal_evidence
        .first()
        .ok_or("missing universal evidence")?
        .relationship_candidates
        .iter()
        .find(|candidate| {
            candidate.spelling == "new" && candidate.qualifier.as_deref() == Some("HashMap")
        })
        .ok_or("missing HashMap::new candidate")?;
    assert!(hash_map.external_identity);
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
fn scoped_declaration_sites_rebind_every_legacy_containment_occurrence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;

    let rust_path = directory.path().join("ownership.rs");
    let rust_source = br#"struct Shared {}
fn over(value: i32) {}
fn over(value: &str) {}
mod one {
    trait Contract { fn run(&self); }
    struct Item {}
    enum Mode { A, B }
    struct Shared {}
    impl Contract for Item { fn run(&self) {} }
}
mod two {
    trait Contract { fn run(&self); }
    struct Item {}
    enum Mode { A, B }
    struct Shared {}
    impl Contract for Item { fn run(&self) {} }
}
"#;
    let rust = Engine::default().extract_source(&rust_path, rust_source)?;
    for (target, owner) in [
        ("Shared@", None),
        ("one::Contract@", Some("one")),
        ("one::Item@", Some("one")),
        ("one::Mode@", Some("one")),
        ("two::Contract@", Some("two")),
        ("two::Item@", Some("two")),
        ("two::Mode@", Some("two")),
        ("two::Shared@", Some("two")),
    ] {
        assert_exact_containment(&rust, target, owner)?;
    }
    assert_unique_node_ids(&rust);
    assert_containment_sites_belong_to_targets(&rust);

    let ts_path = directory.path().join("ownership.ts");
    let ts_source = br#"class Shared {}
namespace One {
    class Item {
        run(value: string): void;
        run(value: number): void {}
    }
    namespace Nested { class Leaf {} }
}
namespace Two {
    class Item {
        run(value: string): void;
        run(value: number): void {}
    }
    class Shared {}
}
"#;
    let ts = Engine::default().extract_source(&ts_path, ts_source)?;
    for (target, owner) in [
        ("Shared@", None),
        ("One::Item@", Some("One")),
        ("One::Nested", Some("One")),
        ("One::Nested::Leaf@", Some("One::Nested")),
        ("Two::Item@", Some("Two")),
        ("Two::Shared@", Some("Two")),
    ] {
        assert_exact_containment(&ts, target, owner)?;
    }
    assert_unique_node_ids(&ts);
    assert_containment_sites_belong_to_targets(&ts);

    let csharp_path = directory.path().join("Ownership.cs");
    let csharp_source = b"class Shared {} namespace One { class Item { void Run(int value) {} } class Outer { class Leaf {} } } namespace Two { class Item { void Run(int value) {} } class Shared {} }\n";
    let csharp = Engine::default().extract_source(&csharp_path, csharp_source)?;
    for (target, owner) in [
        ("Shared@", None),
        ("One::Item@", Some("One")),
        ("One::Outer@", Some("One")),
        ("One::Outer::Leaf@", Some("One::Outer@")),
        ("Two::Item@", Some("Two")),
        ("Two::Shared@", Some("Two")),
    ] {
        assert_exact_containment(&csharp, target, owner)?;
    }
    assert_unique_node_ids(&csharp);
    assert_containment_sites_belong_to_targets(&csharp);
    assert_eq!(
        csharp
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "method"
                    && csharp.nodes.iter().any(|node| {
                        node.id == edge.target && node.string("symbol_kind") == "method"
                    })
            })
            .count(),
        2,
        "C# semantic methods must retain exact raw method ownership for XAML consumers: edges={:#?}",
        csharp.edges
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
    assert_eq!(
        documents[0].attributes["column_start"],
        serde_json::json!(0)
    );
    assert_eq!(documents[0].attributes["column_end"], serde_json::json!(17));
    assert_eq!(
        documents[1].attributes["column_start"],
        serde_json::json!(0)
    );
    assert_eq!(documents[1].attributes["column_end"], serde_json::json!(17));

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
    assert_eq!(config.attributes["column_start"], serde_json::json!(8));
    assert_eq!(config.attributes["column_end"], serde_json::json!(15));
    assert!(config.attributes["start_byte"].as_u64() < config.attributes["end_byte"].as_u64());
    assert!(!format!("{extraction:?}").contains("do-not-publish"));
    let dependency = extraction
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "depends_on")
        .ok_or("missing config dependency")?;
    assert_eq!(dependency.attributes["start_line"], serde_json::json!(6));
    assert_eq!(dependency.attributes["column_start"], serde_json::json!(8));
    assert_eq!(dependency.attributes["column_end"], serde_json::json!(15));
    Ok(())
}

#[test]
fn semantic_inventory_reports_bounded_deterministic_work() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("many_tests.rs");
    let mut source = String::new();
    source.push_str("fn target() {}\n");
    for index in 0..64 {
        source.push_str(&format!("#[test]\nfn checks_{index}() {{ target(); }}\n"));
    }
    let extraction = Engine::default().extract_source(&path, source.as_bytes())?;
    let work = extraction
        .extensions
        .get("_semantic_work")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing semantic work counter")?;
    let visits = work["ast_visits"].as_u64().ok_or("missing visits")?;
    let scans = work["inventory_scan_visits"]
        .as_u64()
        .ok_or("missing inventory scan visits")?;
    assert!(scans < visits * 24, "work={work:?}");
    assert_eq!(work["call_index_writes"], serde_json::json!(64));
    assert_eq!(work["call_index_reads"], serde_json::json!(64));
    assert_eq!(work["test_call_visits"], serde_json::json!(64));
    assert_eq!(work["callable_scan_visits"], serde_json::json!(130));
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "tests")
            .count(),
        64
    );

    let deep_path = directory.path().join("deep.rs");
    let mut deep_source = String::new();
    for index in 0..64 {
        deep_source.push_str(&format!("mod level_{index} {{ "));
    }
    deep_source.push_str("struct Leaf {}");
    for _ in 0..64 {
        deep_source.push_str(" }");
    }
    let deep = Engine::default().extract_source(&deep_path, deep_source.as_bytes())?;
    let deep_work = deep
        .extensions
        .get("_semantic_work")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing deep semantic work counter")?;
    assert_eq!(
        deep_work["scope_frame_extensions"],
        serde_json::json!(65),
        "work={deep_work:?}"
    );
    Ok(())
}
