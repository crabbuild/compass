use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::{EdgeKind, NodeKind};

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
        br#"{"mcpServers":{"runtime":{"command":"node","env":{"TOKEN":"secret"}}}}"#,
    )?;
    write(root, "guide.md", b"# Guide\n\n[Runtime](runtime.rs)\n")?;

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
    let graph = match normalize_v1(extraction.clone(), evidence) {
        Ok(graph) => graph,
        Err(error) => {
            let message = format!("{error:?}");
            let blocker = "invalid contains endpoints enum -> enum_member";
            assert!(message.contains(blocker), "{message}");
            assert_eq!(
                message.matches(" has invalid ").count(),
                message.matches(blocker).count(),
                "{message}"
            );
            let mut locally_compatible = extraction;
            locally_compatible.edges.retain(|edge| {
                !(edge.string("relation") == "contains"
                    && enum_ids.contains(&edge.source)
                    && member_ids.contains(&edge.target))
            });
            let evidence = BuildEvidence::from_extraction(
                root,
                &locally_compatible,
                "sha256:semantic-producers-local-validator",
            )?;
            normalize_v1(locally_compatible, evidence)?
        }
    };
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
    Ok(())
}
