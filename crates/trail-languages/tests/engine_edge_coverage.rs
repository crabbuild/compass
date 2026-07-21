use std::error::Error;
use std::fs;

use trail_languages::{Engine, ExtractError};

#[test]
fn python_indirect_rationale_types_and_binding_shapes_are_extracted() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("advanced.py");
    fs::write(
        &source,
        r#"""A module rationale long enough to become a rationale node."""
from package import imported as alias
from typing import Annotated, Callable, Generic, Optional, TypeVar, Union

T = TypeVar("T")
external_map = {"handler": external_handler}
external_list = [first_handler, second_handler]

class Base(Generic[T]):
    """A class rationale long enough to be indexed safely."""

class Service(Base[ExternalType]):
    def execute(
        self,
        callback: Callable[[InputType], OutputType],
        value: Annotated[Optional[Union[InputType, None]], "meta"],
    ) -> tuple[OutputType, ...]:
        """A function rationale long enough to be indexed safely."""
        # WHY: retain this adapter for compatibility with old callers
        local = callback
        assigned = (external_factory, local)
        consume(external_argument, named=external_keyword)
        mapping = {"one": dictionary_handler, "bound": local}
        handlers = {set_handler, local}
        alias()
        with open_resource() as resource:
            resource.use()
        for item in iterator_factory():
            item.run()
        try:
            risky()
        except ErrorType as error:
            error.handle()
        return external_result

def top_level(arg: InputType) -> OutputType:
    alias()
    return external_top
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.string("file_type") == "rationale")
    );
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "rationale_for")
    );
    let raw = extraction.raw_calls.as_deref().unwrap_or_default();
    for callee in ["set_handler", "external_result", "external_top"] {
        assert!(
            raw.iter().any(|call| call.callee == callee),
            "missing {callee}; calls={raw:?}"
        );
    }
    assert!(extraction.nodes.len() >= 2);
    Ok(())
}

#[test]
fn generated_python_javascript_exports_and_static_type_families_cover_rare_ast_shapes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "migration.py",
            r#"""This generated module rationale must be suppressed by migration detection."""
revision = "abc"
down_revision = "def"
def upgrade():
    """Nested rationale remains discoverable for the upgrade function."""
    pass
"#,
        ),
        (
            "module.ts",
            r#"export const handler = (value: Input): Output => factory(value);
export const mapping = { first: handler };
class Box<T extends Base> extends Parent implements Contract {
  field: Array<Item>;
  run(arg: Promise<Input>): Result<Output> { return helper(arg); }
}
"#,
        ),
        (
            "Types.kt",
            r#"enum class Mode { FAST, SAFE }
class Box<T : Base>(val item: Item) : Parent(), Contract {
    val values: List<External> = listOf()
    fun <R : Result> run(input: Input): Map<String, R> = helper(input)
}
"#,
        ),
        (
            "Types.scala",
            r#"trait Contract
class Box[T <: Base](value: Input) extends Parent with Contract {
  val field: Either[Failure, T] = ???
  def run[R](input: Option[Input]): (R, Output) = helper(input)
}
"#,
        ),
        (
            "Types.java",
            r#"enum Mode { FAST(1), SAFE(2); Mode(int n) {} }
class Box<T extends Base> extends Parent implements Contract {
  java.util.List<Item> field;
  <R extends Result> R run(Input input) { return helper(input); }
}
"#,
        ),
        (
            "types.c",
            r#"typedef struct Payload Payload;
struct Box { Payload *payload; };
Result run(const Input *input, Output **output) { return helper(input); }
"#,
        ),
    ];
    let mut engine = Engine::default();
    for (name, text) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, text)?;
        let extraction = engine.extract(&path)?;
        assert!(!extraction.nodes.is_empty(), "{name}");
    }
    let migration = engine.extract(&directory.path().join("migration.py"))?;
    assert!(
        !migration
            .nodes
            .iter()
            .any(|node| node.label().contains("generated module"))
    );
    assert!(!migration.nodes.is_empty());

    let missing = directory.path().join("missing.py");
    assert!(matches!(
        engine.extract(&missing),
        Err(ExtractError::File(_))
    ));
    let unsupported = directory.path().join("unsupported.unknown");
    fs::write(&unsupported, "data")?;
    assert!(matches!(
        engine.extract(&unsupported),
        Err(ExtractError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn typescript_import_resolution_checks_extensions_and_directory_indexes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("pkg"))?;
    fs::write(
        directory.path().join("target.ts"),
        "export function target() {}\n",
    )?;
    fs::write(
        directory.path().join("view.tsx"),
        "export const View = () => null;\n",
    )?;
    fs::write(
        directory.path().join("pkg/index.tsx"),
        "export const item = 1;\n",
    )?;
    let source = directory.path().join("main.js");
    fs::write(
        &source,
        r#"import { target } from "./target.js";
import { View } from "./view.jsx";
import { item } from "./pkg";
export function run() { target(); View(); return item; }
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| matches!(edge.string("relation").as_str(), "imports" | "imports_from")),
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}
