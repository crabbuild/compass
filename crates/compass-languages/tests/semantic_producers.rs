use std::collections::HashSet;
use std::error::Error;
use std::fs;

use compass_languages::{BindingKind, CandidateRelation, Engine, Extraction, SemanticRole};

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
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing JavaScript universal evidence")?;
    let prototype_methods = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "property"
                && ["Widget.prototype", "Other.prototype", "Plugin.fn"]
                    .iter()
                    .any(|prefix| declaration.qualified_name.contains(prefix))
        })
        .collect::<Vec<_>>();
    assert_eq!(prototype_methods.len(), 3);
    let helper = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "helper")
        .ok_or("missing helper declaration")?;
    let helper_calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "helper"
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(helper.id.as_str())
        })
        .count();
    assert!(helper_calls >= 3, "candidates={:?}", evidence.candidates);
    assert!(
        evidence
            .declarations
            .iter()
            .any(|declaration| declaration.qualified_name.contains("config.render"))
    );
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
fn macro_is_invoked() { product!(); }
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let declaration_kinds = evidence
        .declarations
        .iter()
        .map(|declaration| declaration.kind.as_str())
        .collect::<HashSet<_>>();

    for expected in [
        "trait",
        "enum_member",
        "type_alias",
        "field",
        "constant",
        "macro",
        "method",
        "function",
    ] {
        assert!(
            declaration_kinds.contains(expected),
            "missing {expected}: declarations={:?}",
            evidence.declarations
        );
    }
    let macro_declaration = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "macro")
        .ok_or("missing macro")?;
    assert!(
        macro_declaration.qualified_name.ends_with("::product"),
        "top-level macro identity must be lexical: {macro_declaration:?}"
    );
    assert!(
        !macro_declaration
            .qualified_name
            .contains(&directory.path().to_string_lossy().replace(['/', '\\'], "_")),
        "top-level macro identity embedded the checkout path: {macro_declaration:?}"
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Implements
            && candidate.constraints.qualified_name.as_deref()
                == Some("crate::semantic::Renderable")
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::InvokesMacro
            && candidate.target_spelling == "product"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::References
            && candidate.target_spelling == "Product"
    }));
    let test = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "target_is_rendered")
        .ok_or("missing test function")?;
    assert_eq!(test.kind, "function");
    for occurrence in &evidence.occurrences {
        let start = occurrence.range.start_byte;
        let end = occurrence.range.end_byte;
        assert!(
            start < end && end <= source.len() as u64,
            "occurrence={occurrence:?}"
        );
    }
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());
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
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust universal evidence")?;
    assert_eq!(
        evidence.adapter.evidence_schema,
        "compass.languages.evidence/1"
    );
    assert_eq!(evidence.adapter.id, "compass.rust");
    assert_eq!(evidence.adapter.version, 15);

    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == compass_languages::SemanticRole::Call)
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
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert!(start < end && end <= source.len());
        assert_eq!(
            std::str::from_utf8(&source[start..end])?.replace('.', "::"),
            format!("{qualifier}::{spelling}")
        );
    }
    assert!(
        calls.windows(2).all(|pair| pair[0].range != pair[1].range),
        "each source use must keep a distinct occurrence: {calls:#?}"
    );
    Ok(())
}

#[test]
fn rust_qualified_calls_emit_exact_owner_constraints_and_fail_closed() -> Result<(), Box<dyn Error>>
{
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
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing universal evidence")?;
    let local = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "new"
                && candidate.constraints.qualified_name.as_deref()
                    == Some("crate::qualified::Graph::new")
        })
        .ok_or("missing exact Graph::new candidate")?;
    assert!(!local.constraints.allow_external);
    let hash_map = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "new"
                && candidate.constraints.qualified_name.as_deref() == Some("HashMap::new")
        })
        .ok_or("missing HashMap::new candidate")?;
    assert!(!hash_map.constraints.allow_external);
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());
    Ok(())
}

#[test]
fn rust_module_identity_follows_cargo_package_and_lib_names() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname = \"codex-thread-store\"\nversion = \"0.1.0\"\n",
    )?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    let path = directory.path().join("src/lib.rs");
    let source = b"pub struct StoredThreadHistory {}\n";
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust universal evidence")?;
    assert!(
        evidence
            .declarations
            .iter()
            .any(|declaration| declaration.qualified_name
                == "codex_thread_store::StoredThreadHistory"),
        "declarations={:?}",
        evidence.declarations
    );

    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname = \"package-name\"\nversion = \"0.1.0\"\n[lib]\nname = \"explicit_lib_name\"\n",
    )?;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust universal evidence after manifest update")?;
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.qualified_name == "explicit_lib_name::StoredThreadHistory"
    }));

    std::fs::create_dir_all(directory.path().join("db/src"))?;
    std::fs::write(
        directory.path().join("db/Cargo.toml"),
        "[package]\nname = \"internal-storage\"\nversion = \"0.1.0\"\n",
    )?;
    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\ndb = { path = \"db\", package = \"internal-storage\" }\n",
    )?;
    let path = directory.path().join("src/main.rs");
    let source = b"use db::StoredThreadHistory;\nfn build() { let _ = StoredThreadHistory; }\n";
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust universal evidence after dependency alias")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Imports
            && candidate.constraints.qualified_name.as_deref()
                == Some("internal_storage::StoredThreadHistory")
    }));
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
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing TypeScript universal evidence")?;

    for expected in [
        "constructor",
        "property",
        "parameter",
        "export",
        "annotation",
    ] {
        assert!(
            (expected == "export"
                && evidence
                    .bindings
                    .iter()
                    .any(|binding| binding.kind == BindingKind::Reexport))
                || evidence
                    .declarations
                    .iter()
                    .any(|declaration| declaration.kind == expected)
                || evidence.occurrences.iter().any(|occurrence| {
                    matches!(
                        occurrence.role,
                        SemanticRole::Annotation | SemanticRole::TypeReference
                    ) && expected == "annotation"
                }),
            "missing {expected}: declarations={:?}, bindings={:?}, occurrences={:?}",
            evidence.declarations,
            evidence.bindings,
            evidence.occurrences
        );
    }
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.target_spelling == "Contract"
            && candidate.constraints.qualified_name.as_deref() == Some("semantic.Contract")
    }));
    assert!(
        evidence
            .candidates
            .iter()
            .any(|candidate| candidate.relation == compass_languages::CandidateRelation::Decorates)
    );
    assert!(
        evidence
            .candidates
            .iter()
            .any(|candidate| candidate.relation == compass_languages::CandidateRelation::Reexports)
    );
    assert!(
        evidence
            .candidates
            .iter()
            .any(|candidate| candidate.relation == CandidateRelation::Reexports)
    );
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
    let csharp_evidence = csharp_extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing C# universal evidence")?;
    for expected in ["constructor", "property", "field", "constant", "parameter"] {
        assert!(
            csharp_evidence
                .declarations
                .iter()
                .any(|declaration| declaration.kind == expected),
            "missing {expected}: declarations={:?}",
            csharp_evidence.declarations
        );
    }
    for expected in [
        CandidateRelation::TypeOf,
        CandidateRelation::Returns,
        CandidateRelation::Overrides,
    ] {
        assert!(
            csharp_evidence
                .candidates
                .iter()
                .any(|candidate| candidate.relation == expected),
            "missing {expected:?}: candidates={:?}",
            csharp_evidence.candidates
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
fn vscode_mcp_servers_publish_exact_command_package_and_environment_facts()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let vscode = directory.path().join(".vscode");
    fs::create_dir(&vscode)?;
    let mcp_path = vscode.join("mcp.json");
    fs::write(
        &mcp_path,
        br#"{
  "servers": {
    "date-fns": {
      "type": "stdio",
      "command": "pnpm",
      "args": ["date-fns-mcp"],
      "env": {"DATE_FNS_TOKEN": "do-not-publish"}
    }
  }
}"#,
    )?;

    let extraction = Engine::default().extract(&mcp_path)?;
    assert!(extraction.error.is_none(), "error={:?}", extraction.error);
    let labels = extraction
        .nodes
        .iter()
        .map(|node| (node.label().to_owned(), node.string("symbol_kind")))
        .collect::<HashSet<_>>();
    for expected in [
        ("date-fns".to_owned(), "component".to_owned()),
        ("pnpm".to_owned(), "function".to_owned()),
        ("date-fns-mcp".to_owned(), "package".to_owned()),
        ("DATE_FNS_TOKEN".to_owned(), "config_key".to_owned()),
    ] {
        assert!(labels.contains(&expected), "nodes={:?}", extraction.nodes);
    }
    let environment = extraction
        .nodes
        .iter()
        .find(|node| node.label() == "DATE_FNS_TOKEN")
        .ok_or("missing VS Code MCP environment key")?;
    assert_eq!(
        environment.string("key_path"),
        "servers.date-fns.env.DATE_FNS_TOKEN"
    );
    assert!(!format!("{extraction:?}").contains("do-not-publish"));
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "references")
            .count(),
        2
    );
    assert!(
        extraction.edges.iter().any(|edge| {
            edge.target == environment.id && edge.string("relation") == "depends_on"
        })
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
    let rust_evidence = rust
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let rust_items = rust_evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "struct" && declaration.name == "Item")
        .collect::<Vec<_>>();
    assert_eq!(
        rust_items.len(),
        2,
        "declarations={:?}",
        rust_evidence.declarations
    );
    assert!(
        rust_items
            .iter()
            .any(|declaration| declaration.qualified_name.ends_with("::alpha::Item"))
    );
    assert!(
        rust_items
            .iter()
            .any(|declaration| declaration.qualified_name.ends_with("::beta::Item"))
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
    let ts_evidence = ts
        .semantic_evidence
        .as_ref()
        .ok_or("missing TypeScript universal evidence")?;
    let ts_items = ts_evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "class" && declaration.name == "Item")
        .collect::<Vec<_>>();
    assert_eq!(
        ts_items.len(),
        2,
        "declarations={:?}",
        ts_evidence.declarations
    );
    assert_eq!(
        ts_evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "constructor")
            .count(),
        3,
        "declarations={:?}",
        ts_evidence.declarations
    );
    let overloaded = ts_evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "constructor"
                || (declaration.kind == "method" && declaration.name == "run")
        })
        .collect::<Vec<_>>();
    assert!(
        overloaded.len() >= 6,
        "declarations={:?}",
        ts_evidence.declarations
    );
    assert_eq!(
        overloaded
            .iter()
            .map(|declaration| declaration.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        overloaded.len(),
        "overload declarations must retain distinct identities"
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
    let csharp_evidence = csharp
        .semantic_evidence
        .as_ref()
        .ok_or("missing C# universal evidence")?;
    let csharp_items = csharp_evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "class" && declaration.name == "Item")
        .collect::<Vec<_>>();
    assert_eq!(
        csharp_items.len(),
        2,
        "declarations={:?}",
        csharp_evidence.declarations
    );
    assert!(
        csharp_items
            .iter()
            .any(|declaration| declaration.qualified_name == "Alpha.Item")
    );
    assert!(
        csharp_items
            .iter()
            .any(|declaration| declaration.qualified_name == "Beta.Item")
    );
    assert_eq!(
        csharp_evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "constructor")
            .count(),
        2,
        "declarations={:?}",
        csharp_evidence.declarations
    );
    assert_eq!(
        csharp_evidence
            .declarations
            .iter()
            .filter(|declaration| { declaration.kind == "method" && declaration.name == "Run" })
            .count(),
        2,
        "declarations={:?}",
        csharp_evidence.declarations
    );
    let overloaded = csharp_evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "constructor"
                || (declaration.kind == "method" && declaration.name == "Run")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        overloaded
            .iter()
            .map(|declaration| declaration.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        overloaded.len(),
        "C# overload declarations must retain distinct evidence identities"
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
    let rust_evidence = rust
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    for target in [
        "crate::ownership::Shared",
        "crate::ownership::one::Contract",
        "crate::ownership::one::Item",
        "crate::ownership::one::Mode",
        "crate::ownership::two::Contract",
        "crate::ownership::two::Item",
        "crate::ownership::two::Mode",
        "crate::ownership::two::Shared",
    ] {
        assert!(
            rust_evidence.declarations.iter().any(|declaration| {
                declaration.qualified_name == target
                    && rust_evidence.candidates.iter().any(|candidate| {
                        candidate.relation == compass_languages::CandidateRelation::Contains
                            && candidate.constraints.qualified_name.as_deref() == Some(target)
                    })
            }),
            "missing declaration or containment for {target}: {rust_evidence:#?}"
        );
    }
    let graph_ids = rust_evidence
        .declarations
        .iter()
        .map(|declaration| declaration.graph_node_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(graph_ids.len(), rust_evidence.declarations.len());

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
    let ts_evidence = ts
        .semantic_evidence
        .as_ref()
        .ok_or("missing TypeScript universal evidence")?;
    for (target_suffix, owner_suffix) in [
        (".Shared", None),
        (".One.Item", Some(".One")),
        (".One.Nested", Some(".One")),
        (".One.Nested.Leaf", Some(".One.Nested")),
        (".Two.Item", Some(".Two")),
        (".Two.Shared", Some(".Two")),
    ] {
        let target = ts_evidence
            .declarations
            .iter()
            .find(|declaration| {
                (target_suffix == ".Shared" && declaration.qualified_name == "ownership.Shared")
                    || (target_suffix != ".Shared"
                        && declaration.qualified_name.ends_with(target_suffix))
                        && declaration.name == target_suffix.rsplit('.').next().unwrap_or_default()
            })
            .ok_or_else(|| {
                format!(
                    "missing target {target_suffix}: {:?}",
                    ts_evidence
                        .declarations
                        .iter()
                        .map(|declaration| declaration.qualified_name.as_str())
                        .collect::<Vec<_>>()
                )
            })?;
        let scope = ts_evidence
            .scopes
            .iter()
            .find(|scope| scope.id == target.scope_id.clone().unwrap_or_default())
            .ok_or_else(|| format!("missing scope for {target_suffix}"))?;
        let owner = ts_evidence
            .declarations
            .iter()
            .find(|declaration| {
                scope
                    .owner_declaration_id
                    .as_deref()
                    .is_some_and(|owner_id| declaration.id == owner_id)
            })
            .ok_or_else(|| format!("missing scope owner for {target_suffix}"))?;
        match owner_suffix {
            Some(expected) => assert!(
                owner.qualified_name.ends_with(expected),
                "target {target_suffix} owner={}",
                owner.qualified_name
            ),
            None => assert_eq!(
                owner.kind, "module",
                "target {target_suffix} owner={owner:#?}"
            ),
        }
    }
    let declaration_ids = ts_evidence
        .declarations
        .iter()
        .map(|declaration| declaration.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(declaration_ids.len(), ts_evidence.declarations.len());

    let csharp_path = directory.path().join("Ownership.cs");
    let csharp_source = b"class Shared {} namespace One { class Item { void Run(int value) {} } class Outer { class Leaf {} } } namespace Two { class Item { void Run(int value) {} } class Shared {} }\n";
    let csharp = Engine::default().extract_source(&csharp_path, csharp_source)?;
    let csharp_evidence = csharp
        .semantic_evidence
        .as_ref()
        .ok_or("missing C# universal evidence")?;
    for (target, owner) in [
        ("Shared", None),
        ("One", None),
        ("One.Item", Some("One")),
        ("One.Outer", Some("One")),
        ("One.Outer.Leaf", Some("One.Outer")),
        ("Two", None),
        ("Two.Item", Some("Two")),
        ("Two.Shared", Some("Two")),
    ] {
        let declaration = csharp_evidence
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name == target)
            .ok_or_else(|| format!("missing C# declaration {target}: {csharp_evidence:#?}"))?;
        let owner_declaration = match owner {
            Some(owner) => csharp_evidence
                .declarations
                .iter()
                .find(|declaration| declaration.qualified_name == owner)
                .ok_or_else(|| format!("missing C# owner {owner}"))?,
            None => csharp_evidence
                .declarations
                .iter()
                .find(|candidate| candidate.kind == "file")
                .ok_or("missing C# file declaration")?,
        };
        assert!(
            csharp_evidence.candidates.iter().any(|candidate| {
                candidate.relation == CandidateRelation::Owns
                    && candidate.source_declaration_id == owner_declaration.id
                    && candidate.constraints.exact_target_declaration_id.as_deref()
                        == Some(declaration.id.as_str())
            }),
            "missing exact C# ownership {owner:?} -> {target}: {csharp_evidence:#?}"
        );
    }
    let declaration_ids = csharp_evidence
        .declarations
        .iter()
        .map(|declaration| declaration.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        declaration_ids.len(),
        csharp_evidence.declarations.len(),
        "C# declarations must retain unique evidence identities"
    );
    assert_eq!(
        csharp_evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "method" && declaration.name == "Run")
            .count(),
        2,
        "C# universal evidence must retain both scoped method occurrences: {csharp_evidence:#?}"
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
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing TypeScript universal evidence")?;
    for exported in ["First", "Second", "Service"] {
        assert!(
            evidence.bindings.iter().any(
                |binding| binding.kind == BindingKind::Reexport && binding.spelling == exported
            ),
            "missing re-export {exported}: bindings={:?}",
            evidence.bindings
        );
    }
    assert!(
        evidence.occurrences.iter().any(|occurrence| {
            occurrence.role == SemanticRole::Decorator
                && occurrence.spelling == "decorate"
                && occurrence.qualifier.as_deref() == Some("ns.decorate")
        }),
        "missing decorator occurrence: occurrences={:?}",
        evidence.occurrences
    );
    assert!(
        evidence
            .candidates
            .iter()
            .any(|candidate| candidate.relation == CandidateRelation::Decorates),
        "missing decorator candidate: candidates={:?}",
        evidence.candidates
    );
    let exports = evidence
        .bindings
        .iter()
        .filter(|binding| binding.kind == BindingKind::Reexport)
        .map(|binding| binding.spelling.as_str())
        .collect::<HashSet<_>>();
    assert!(
        exports.contains("First"),
        "bindings={:?}",
        evidence.bindings
    );
    assert!(
        exports.contains("Second"),
        "bindings={:?}",
        evidence.bindings
    );
    assert!(
        exports.contains("Service"),
        "bindings={:?}",
        evidence.bindings
    );

    let rust = directory.path().join("attributes.rs");
    let rust_source = br#"
fn target() {}
#[test]
#[should_panic]
fn checks_target() { target(); panic!("expected"); }
"#;
    let extraction = Engine::default().extract_source(&rust, rust_source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let test = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "checks_target")
        .ok_or("missing multi-attribute test")?;
    assert_eq!(test.kind, "function");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == test.id
            && candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "target"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.source_declaration_id == test.id
            && candidate.relation == compass_languages::CandidateRelation::InvokesMacro
            && candidate.target_spelling == "panic"
    }));
    assert!(!extraction.extensions.contains_key("_semantic_work"));
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
fn rust_universal_evidence_is_bounded_and_deterministic() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("many_tests.rs");
    let mut source = String::new();
    source.push_str("fn target() {}\n");
    for index in 0..64 {
        source.push_str(&format!("#[test]\nfn checks_{index}() {{ target(); }}\n"));
    }
    let first = Engine::default().extract_source(&path, source.as_bytes())?;
    let second = Engine::default().extract_source(&path, source.as_bytes())?;
    assert_eq!(first.semantic_evidence, second.semantic_evidence);
    let evidence = first
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    assert_eq!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == compass_languages::CandidateRelation::Calls
                    && candidate.target_spelling == "target"
            })
            .count(),
        64
    );
    assert!(
        evidence.declarations.len() <= compass_languages::EvidenceLimits::default().declarations
    );
    assert!(evidence.occurrences.len() <= compass_languages::EvidenceLimits::default().occurrences);
    assert!(!first.extensions.contains_key("_semantic_work"));
    assert!(first.nodes.is_empty() && first.edges.is_empty() && first.raw_calls.is_none());

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
    let deep_evidence = deep
        .semantic_evidence
        .as_ref()
        .ok_or("missing deep Rust semantic evidence")?;
    assert_eq!(
        deep_evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "module")
            .count(),
        64,
        "evidence={deep_evidence:?}"
    );
    assert!(deep_evidence.scopes.len() <= compass_languages::EvidenceLimits::default().scopes);
    Ok(())
}
