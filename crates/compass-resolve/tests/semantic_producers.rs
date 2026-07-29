use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};

fn append(target: &mut Extraction, mut source: Extraction) {
    target.nodes.append(&mut source.nodes);
    target.edges.append(&mut source.edges);
    target.framework_facts.append(&mut source.framework_facts);
    if let Some(raw_calls) = source.raw_calls {
        target
            .raw_calls
            .get_or_insert_with(Vec::new)
            .extend(raw_calls);
    }
}

fn write(root: &Path, relative: &str, source: &[u8]) -> Result<(), Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, source)?;
    Ok(())
}

fn assert_public_containment(
    graph: &GraphDocument,
    target_file: &str,
    target_prefix: &str,
    owner_prefix: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let expected_name = target_prefix
        .rsplit("::")
        .next()
        .unwrap_or(target_prefix)
        .split('@')
        .next()
        .unwrap_or(target_prefix);
    let target = graph
        .nodes
        .iter()
        .find(|node| {
            node.qualified_name.starts_with(target_prefix)
                && node.name == expected_name
                && node.source_file() == Some(target_file)
        })
        .ok_or_else(|| format!("missing public target {target_prefix}"))?;
    let target_site = target.source.as_ref().ok_or("missing target source")?;
    let occurrences = graph
        .links
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Contains && edge.relationship_site.as_ref() == Some(target_site)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        occurrences.len(),
        1,
        "public containment for {target_prefix}, source={target_site:?}, target_edges={:#?}",
        graph
            .links
            .iter()
            .filter(|edge| edge.target == target.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(occurrences[0].target, target.id);
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.id == occurrences[0].source)
        .ok_or("missing public containment owner")?;
    if let Some(prefix) = owner_prefix {
        assert!(
            if prefix.ends_with('@') {
                owner.qualified_name.starts_with(prefix)
            } else {
                owner.qualified_name == prefix
            },
            "owner={} expected={prefix}",
            owner.qualified_name
        );
    } else {
        assert_eq!(
            owner.kind,
            NodeKind::File,
            "target={target:#?} occurrence={:#?} owner={owner:#?}",
            occurrences[0]
        );
        assert_eq!(owner.source_file(), target.source_file());
    }
    Ok(())
}

fn assert_every_public_containment_site_matches_its_target(
    graph: &GraphDocument,
    files: &[&str],
) -> Result<(), Box<dyn Error>> {
    for edge in graph.links.iter().filter(|edge| {
        edge.kind == EdgeKind::Contains
            && edge
                .relationship_site
                .as_ref()
                .is_some_and(|site| files.contains(&site.file.as_str()))
    }) {
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing managed containment site")?;
        let target = graph
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing managed containment target")?;
        assert_eq!(
            target.source.as_ref(),
            Some(site),
            "stale public containment endpoint: target={target:#?} edge={edge:#?}"
        );
    }
    Ok(())
}

#[test]
fn public_normalization_accepts_every_remediated_semantic_producer() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    write(
        root,
        "runtime.rs",
        br#"
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
"#,
    )?;
    write(
        root,
        "runtime.ts",
        br#"
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
"#,
    )?;
    write(
        root,
        "Runtime.cs",
        br#"
class Value {}
class BaseService {
    public virtual Value Run(Value value) => value;
}
class RuntimeService : BaseService {
    public const int Limit = 4;
    private Value current;
    public Value Current { get; set; }
    public RuntimeService(Value current) { this.current = current; }
    public override Value Run(Value value) => value;
}
"#,
    )?;
    write(
        root,
        "Protocol.mm",
        b"@protocol RuntimeProtocol\n- (void)run;\n@end\n",
    )?;
    write(
        root,
        "runtime.schema.json",
        br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}"#,
    )?;
    write(
        root,
        "mcp.json",
        br#"{
  "mcpServers": {
    "runtime": {
      "command": "node",
      "env": {
        "TOKEN": "secret"
      }
    }
  }
}"#,
    )?;
    write(
        root,
        "guide.md",
        b"# Guide\n\n[One](runtime.rs)\n\n  [Two](runtime.rs)\n",
    )?;
    write(
        root,
        "rust_ownership.rs",
        br#"struct Shared {}
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
"#,
    )?;
    write(
        root,
        "ts_ownership.ts",
        br#"class Shared {}
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
"#,
    )?;
    write(
        root,
        "CsharpOwnership.cs",
        b"class Shared {} namespace One { class Item { void Run(int value) {} } class Outer { class Leaf {} } } namespace Two { class Item { void Run(int value) {} } class Shared {} }\n",
    )?;

    let mut extraction = Extraction::default();
    let mut engine = Engine::default();
    for path in [
        "runtime.rs",
        "runtime.ts",
        "Runtime.cs",
        "Protocol.mm",
        "runtime.schema.json",
        "mcp.json",
        "guide.md",
        "rust_ownership.rs",
        "ts_ownership.ts",
        "CsharpOwnership.cs",
    ] {
        append(&mut extraction, engine.extract(&root.join(path))?);
    }
    let enum_ids = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "enum")
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let member_ids = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "enum_member")
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "contains"
            && enum_ids.contains(&edge.source)
            && member_ids.contains(&edge.target)
    }));
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:semantic-producers")?;
    let graph = normalize_v1(extraction, evidence)?;
    let node_kinds = graph
        .nodes
        .iter()
        .map(|node| node.kind)
        .collect::<HashSet<_>>();
    let edge_kinds = graph
        .links
        .iter()
        .map(|edge| edge.kind)
        .collect::<HashSet<_>>();

    for expected in [
        NodeKind::Trait,
        NodeKind::Protocol,
        NodeKind::EnumMember,
        NodeKind::TypeAlias,
        NodeKind::Constructor,
        NodeKind::Property,
        NodeKind::Field,
        NodeKind::Constant,
        NodeKind::Parameter,
        NodeKind::Export,
        NodeKind::Macro,
        NodeKind::Annotation,
        NodeKind::Schema,
    ] {
        assert!(node_kinds.contains(&expected), "missing {expected:?}");
    }
    for expected in [
        EdgeKind::TypeOf,
        EdgeKind::Returns,
        EdgeKind::Overrides,
        EdgeKind::Decorates,
        EdgeKind::Aliases,
        EdgeKind::Tests,
        EdgeKind::DependsOn,
        EdgeKind::Documents,
    ] {
        assert!(edge_kinds.contains(&expected), "missing {expected:?}");
    }

    let dependency = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::DependsOn)
        .ok_or("missing dependency")?;
    let source = graph
        .nodes
        .iter()
        .find(|node| node.id == dependency.source)
        .ok_or("missing dependency source")?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.id == dependency.target)
        .ok_or("missing dependency target")?;
    assert_eq!(source.kind, NodeKind::Component);
    assert_eq!(target.kind, NodeKind::ConfigKey);

    for (file, target, owner) in [
        ("rust_ownership.rs", "Shared@", None),
        ("rust_ownership.rs", "one::Contract@", Some("one")),
        ("rust_ownership.rs", "one::Item@", Some("one")),
        ("rust_ownership.rs", "one::Mode@", Some("one")),
        ("rust_ownership.rs", "two::Contract@", Some("two")),
        ("rust_ownership.rs", "two::Item@", Some("two")),
        ("rust_ownership.rs", "two::Mode@", Some("two")),
        ("rust_ownership.rs", "two::Shared@", Some("two")),
        ("ts_ownership.ts", "Shared@", None),
        ("ts_ownership.ts", "One::Item@", Some("One")),
        ("ts_ownership.ts", "One::Nested::Leaf@", Some("One::Nested")),
        ("ts_ownership.ts", "Two::Item@", Some("Two")),
        ("ts_ownership.ts", "Two::Shared@", Some("Two")),
        ("CsharpOwnership.cs", "Shared@", None),
        ("CsharpOwnership.cs", "One::Item@", Some("One")),
        ("CsharpOwnership.cs", "One::Outer@", Some("One")),
        (
            "CsharpOwnership.cs",
            "One::Outer::Leaf@",
            Some("One::Outer@"),
        ),
        ("CsharpOwnership.cs", "Two::Item@", Some("Two")),
        ("CsharpOwnership.cs", "Two::Shared@", Some("Two")),
    ] {
        assert_public_containment(&graph, file, target, owner)?;
    }
    assert_every_public_containment_site_matches_its_target(
        &graph,
        &["rust_ownership.rs", "ts_ownership.ts", "CsharpOwnership.cs"],
    )?;

    let mut document_sites = graph
        .links
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Documents)
        .filter_map(|edge| edge.relationship_site.as_ref())
        .filter(|site| site.file == "guide.md")
        .collect::<Vec<_>>();
    document_sites.sort_by_key(|site| site.start_byte);
    assert_eq!(document_sites.len(), 2);
    assert_eq!(
        (
            document_sites[0].start_byte,
            document_sites[0].end_byte,
            document_sites[0].start_line,
            document_sites[0].start_column,
            document_sites[0].end_line,
            document_sites[0].end_column,
        ),
        (9, 26, 3, 0, 3, 17)
    );
    assert_eq!(
        (
            document_sites[1].start_byte,
            document_sites[1].end_byte,
            document_sites[1].start_line,
            document_sites[1].start_column,
            document_sites[1].end_line,
            document_sites[1].end_column,
        ),
        (30, 47, 5, 2, 5, 19)
    );

    let config = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::ConfigKey)
        .and_then(|node| node.source.as_ref())
        .ok_or("missing public config key source")?;
    assert_eq!(
        (
            config.start_line,
            config.start_column,
            config.end_line,
            config.end_column,
        ),
        (6, 8, 6, 15)
    );
    let dependency_site = dependency
        .relationship_site
        .as_ref()
        .ok_or("missing public dependency site")?;
    assert_eq!(
        (
            dependency_site.start_byte,
            dependency_site.end_byte,
            dependency_site.start_line,
            dependency_site.start_column,
            dependency_site.end_line,
            dependency_site.end_column,
        ),
        (config.start_byte, config.end_byte, 6, 8, 6, 15)
    );
    Ok(())
}
