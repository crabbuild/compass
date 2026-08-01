#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use compass_languages::{Engine, RawCall};
use serde_json::Map;

fn extract(path: &str, source: &[u8]) -> compass_languages::Extraction {
    Engine::default()
        .extract_source(Path::new(path), source)
        .expect("extract fixture")
}

fn universal_edges(
    extraction: &compass_languages::Extraction,
) -> Vec<(String, String, String, u64)> {
    let mut edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("extractor").contains(".universal"))
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge.string("relation"),
                edge.attributes
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    edges.sort();
    edges
}

#[test]
fn universal_resolution_is_language_isolated_and_input_order_deterministic() {
    let go_definition = extract("pkg/release.go", b"package pkg\nfunc release() {}\n");
    let go_caller = extract(
        "pkg/caller.go",
        b"package pkg\nfunc caller() { release() }\n",
    );
    let python_collision = extract("pkg/collision.py", b"def release():\n    return None\n");
    let sources = HashMap::from([
        (
            "pkg/release.go".to_owned(),
            "package pkg\nfunc release() {}\n".to_owned(),
        ),
        (
            "pkg/caller.go".to_owned(),
            "package pkg\nfunc caller() { release() }\n".to_owned(),
        ),
        (
            "pkg/collision.py".to_owned(),
            "def release():\n    return None\n".to_owned(),
        ),
    ]);

    let first = compass_resolve::resolve(
        &[
            go_definition.clone(),
            go_caller.clone(),
            python_collision.clone(),
        ],
        &sources,
    );
    let second = compass_resolve::resolve(&[python_collision, go_caller, go_definition], &sources);
    assert_eq!(universal_edges(&first), universal_edges(&second));

    let release = first
        .nodes
        .iter()
        .find(|node| node.label() == "release()" && node.string("language") == "go")
        .expect("Go release definition");
    assert!(first.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == release.id
            && edge.string("language") == "go"
    }));
    assert!(first.edges.iter().all(|edge| {
        !(edge.string("language") == "go"
            && first
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.string("language") == "python"))
    }));
}

#[test]
fn universal_overloads_receive_stable_publication_discriminators() {
    let source = br#"class Widget:
    @property
    def value(self):
        return self._value

    @value.setter
    def value(self, value):
        self._value = value
"#;
    let extracted = extract("pkg/widget.py", source);
    let sources = HashMap::from([(
        "pkg/widget.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let methods = resolved
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "pkg.widget.Widget::value")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2);
    assert_ne!(methods[0].id, methods[1].id);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.string("overload_discriminator"))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["overload:0".to_owned(), "overload:1".to_owned()])
    );
    let widget = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.widget.Widget")
        .expect("Widget declaration");
    assert!(methods.iter().all(|method| {
        resolved.edges.iter().any(|edge| {
            edge.source == widget.id
                && edge.target == method.id
                && edge.string("relation") == "contains"
        })
    }));
}

#[test]
fn empty_hard_cut_source_materializes_its_file_node() {
    let extracted = extract("pkg/__init__.py", b"");
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([("pkg/__init__.py".to_owned(), String::new())]),
    );

    let file = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file" && node.string("source_file") == "pkg/__init__.py"
        })
        .expect("empty Python source inventory node");
    assert_eq!(file.label(), "__init__.py");
    assert_eq!(file.string("language"), "python");
    assert_eq!(
        file.attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        file.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
}

#[test]
fn normalized_source_id_collisions_preserve_every_file_node() {
    let paths = ["pkg/.util.py", "pkg/_util.py", "pkg/~util.py"];
    let extracted = paths
        .iter()
        .map(|path| extract(path, b""))
        .collect::<Vec<_>>();
    let sources = paths
        .iter()
        .map(|path| ((*path).to_owned(), String::new()))
        .collect::<HashMap<_, _>>();
    let resolved = compass_resolve::resolve(&extracted, &sources);

    let files = resolved
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "file")
        .map(|node| (node.id.clone(), node.string("source_file")))
        .collect::<Vec<_>>();
    assert_eq!(files.len(), paths.len());
    assert_eq!(
        files
            .iter()
            .map(|(_, source)| source.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        paths.into_iter().collect::<std::collections::BTreeSet<_>>()
    );
    assert_eq!(
        files
            .iter()
            .map(|(id, _)| id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        paths.len()
    );
}

#[test]
fn universal_batches_discard_stale_untyped_raw_calls_in_both_merge_paths() {
    let provider = extract(
        "pkg/provider.py",
        b"def actual():\n    return None\ndef legacy_only():\n    return None\n",
    );
    let mut caller = extract("app.py", b"def caller():\n    actual()\n");
    let caller_id = caller
        .semantic_evidence
        .as_ref()
        .and_then(|evidence| {
            evidence
                .declarations
                .iter()
                .find(|declaration| declaration.name == "caller")
        })
        .expect("caller declaration evidence")
        .graph_node_id
        .clone();
    caller.raw_calls = Some(vec![RawCall {
        caller_nid: caller_id,
        callee: "legacy_only".to_owned(),
        is_member_call: Some(false),
        source_file: "app.py".to_owned(),
        source_location: "L2".to_owned(),
        receiver: None,
        receiver_type: None,
        lang: None,
        extensions: Map::new(),
    }]);

    for resolved in [
        compass_resolve::resolve(&[provider.clone(), caller.clone()], &HashMap::new()),
        compass_resolve::resolve_owned_with_root(
            vec![provider.clone(), caller.clone()],
            &HashMap::new(),
            Path::new("."),
        ),
    ] {
        let legacy = resolved
            .nodes
            .iter()
            .find(|node| node.label() == "legacy_only()")
            .expect("legacy-only target");
        assert!(
            resolved
                .edges
                .iter()
                .all(|edge| { edge.target != legacy.id || edge.string("relation") != "calls" })
        );
    }
}

#[test]
fn rust_trait_impl_methods_and_enum_payloads_keep_exact_owners() {
    let source = br#"
trait Execute { fn execute(&self); }
struct Local;
struct Remote;
impl Execute for Local { fn execute(&self) {} }
impl Execute for Remote { fn execute(&self) {} }
enum Event { Local(Local), Remote { value: Remote } }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let nodes = resolved
        .nodes
        .iter()
        .map(|node| (node.string("qualified_name"), node.id.as_str()))
        .collect::<HashMap<_, _>>();

    for owner in ["Local", "Remote"] {
        let owner_id = nodes[&format!("crate::{owner}")];
        let method_id = nodes
            .iter()
            .find_map(|(qualified, id)| {
                qualified
                    .starts_with(&format!("<crate::{owner} as crate::Execute>::execute"))
                    .then_some(*id)
            })
            .expect("trait impl method");
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == owner_id
                && edge.target == method_id
                && edge.string("relation") == "contains"
        }));
    }

    let event_id = nodes["crate::Event"];
    for payload in ["crate::Local", "crate::Remote"] {
        let payload_id = nodes[payload];
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == event_id
                && edge.target == payload_id
                && edge.string("relation") == "references"
        }));
    }
}

#[test]
fn rust_nonstandard_crate_root_preserves_nested_module_identity() {
    let flags = extract(
        "crates/core/flags/mod.rs",
        b"trait Flag {}\nenum Category { Output }\nmod defs;\n",
    );
    let definitions_source = br#"use crate::flags::{Category, Flag};
struct AfterContext;
impl Flag for AfterContext {
    fn category(&self) -> Category { Category::Output }
}
"#;
    let definitions = extract("crates/core/flags/defs.rs", definitions_source);
    let resolved = compass_resolve::resolve(
        &[flags, definitions],
        &HashMap::from([(
            "crates/core/flags/defs.rs".to_owned(),
            String::from_utf8(definitions_source.to_vec()).expect("source"),
        )]),
    );
    let after_context = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "core::flags::defs::AfterContext")
        .expect("AfterContext declaration");
    let flag = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "core::flags::Flag")
        .expect("Flag declaration");
    let category = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "core::flags::Category")
        .expect("Category declaration");
    let category_method = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name")
                == "<core::flags::defs::AfterContext as core::flags::Flag>::category"
        })
        .expect("Category method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == after_context.id
            && edge.target == flag.id
            && edge.string("relation") == "implements"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == category_method.id
            && edge.target == category.id
            && edge.string("relation") == "returns"
    }));
}

#[test]
fn rust_generic_impl_preserves_exact_type_ownership() {
    let source = br#"trait Render {}
struct Container<T>(T);
impl<T> Render for Container<T> {
    fn render(&self) {}
}
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let container = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Container")
        .expect("Container declaration");
    let render_trait = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Render")
        .expect("Render trait");
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Container as crate::Render>::render")
        .expect("Container.render method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == container.id
            && edge.target == render_trait.id
            && edge.string("relation") == "implements"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == container.id
            && edge.target == render_method.id
            && edge.string("relation") == "contains"
    }));
}

#[test]
fn python_super_call_resolves_only_the_exact_direct_base_method() {
    let provider = extract(
        "pkg/base.py",
        b"class Base:\n    def run(self):\n        return None\n",
    );
    let caller_source =
        b"from pkg.base import Base\nclass Child(Base):\n    def run(self):\n        super().run()\n";
    let caller = extract("pkg/child.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/child.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/base.py"
                && node.string("qualified_name") == "pkg.base.Base::run"
        })
        .unwrap_or_else(|| panic!("base method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == base_run.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L4");
    assert_eq!(calls[0].string("resolution_rule"), "explicit-binding");
}

#[test]
fn python_super_call_with_multiple_bases_cannot_terminal_match_an_unrelated_method() {
    let source = b"class Unrelated:\n    def run(self):\n        return None\nclass Left:\n    pass\nclass Right:\n    pass\nclass Child(Left, Right):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L9"
    }));
}

#[test]
fn python_shadowed_super_call_does_not_bind_the_builtin_hierarchy() {
    let source = b"class Base:\n    def run(self):\n        return None\nclass Child(Base):\n    def run(self, super):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L6"
    }));
}

#[test]
fn ambiguous_same_package_targets_fail_closed() {
    let first = extract("pkg/first.go", b"package pkg\nfunc duplicate() {}\n");
    let second = extract("pkg/second.go", b"package pkg\nfunc duplicate() {}\n");
    let caller = extract(
        "pkg/caller.go",
        b"package pkg\nfunc caller() { duplicate() }\n",
    );
    let resolved = compass_resolve::resolve(&[first, second, caller], &HashMap::new());
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls"
            || resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .is_none_or(|node| node.label() != "duplicate()")
    }));
}

#[test]
fn explicit_python_alias_resolves_before_external_fallback() {
    let provider = extract(
        "tools/runner.py",
        b"def execute(value):\n    return value\n",
    );
    let caller = extract(
        "app.py",
        b"from tools.runner import execute as run\ndef main():\n    run(1)\n",
    );
    let resolved = compass_resolve::resolve(&[provider, caller], &HashMap::new());
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "execute()")
        .expect("provider function");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == target.id
            && edge.string("resolution_rule") == "explicit-binding"
    }));
}

#[test]
fn explicit_python_type_alias_resolves_before_same_scope_declaration() {
    let provider = extract("framework/serializer.py", b"class Serializer:\n    pass\n");
    let consumer = extract(
        "app/serializer.py",
        b"from framework.serializer import Serializer as PythonSerializer\nclass Serializer(PythonSerializer):\n    pass\n",
    );
    let resolved = compass_resolve::resolve(&[provider, consumer], &HashMap::new());
    let imported = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "framework.serializer.Serializer")
        .expect("imported serializer");
    let local = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.serializer.Serializer")
        .expect("local serializer");

    let inheritance = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == local.id || edge.string("relation") == "inherits")
        .collect::<Vec<_>>();
    assert!(
        inheritance.iter().any(|edge| {
            edge.source == local.id
                && edge.target == imported.id
                && edge.string("relation") == "inherits"
                && edge.string("resolution_rule") == "explicit-binding"
        }),
        "imported={imported:#?} local={local:#?} inheritance={inheritance:#?}"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.source == local.id
            && edge.target == local.id
            && edge.string("relation") == "inherits")
    }));
}

#[test]
fn function_local_import_binding_shadows_same_named_outer_declaration() {
    let provider = extract("pkg/template.py", b"def render():\n    return 'template'\n");
    let consumer = extract(
        "facade.py",
        b"def render():\n    from pkg.template import render\n    return render()\n",
    );
    let resolved = compass_resolve::resolve(&[provider, consumer], &HashMap::new());
    let wrapper = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "facade.render")
        .expect("wrapper render");
    let imported = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.template.render")
        .expect("imported render");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == wrapper.id
            && edge.target == imported.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.source == wrapper.id
            && edge.target == wrapper.id
            && edge.string("relation") == "calls")
    }));
}

#[test]
fn later_python_imports_and_dotted_bindings_resolve_exact_final_endpoints() {
    let package_provider = extract("pkg/api.py", b"def execute():\n    return None\n");
    let alias_provider = extract("other/tools.py", b"def execute():\n    return None\n");
    let callback_provider = extract("tools/runner.py", b"def callback():\n    return None\n");
    let caller = extract(
        "app.py",
        br#"def before():
    callback()
    return callback

from tools.runner import callback
import pkg.api
import other.tools as alias

def dotted():
    pkg.api.execute()
    alias.execute()
"#,
    );
    let resolved = compass_resolve::resolve(
        &[package_provider, alias_provider, callback_provider, caller],
        &HashMap::new(),
    );
    for qualified in [
        "pkg.api.execute",
        "other.tools.execute",
        "tools.runner.callback",
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified)
            .unwrap_or_else(|| panic!("missing target {qualified}"));
        assert!(resolved.edges.iter().any(|edge| {
            edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "explicit-binding"
        }));
    }
    let callback = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "tools.runner.callback")
        .expect("callback target");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == callback.id
            && edge.string("relation") == "indirect_call"
            && edge.string("context") == "return"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
}

#[test]
fn sequential_python_import_rebindings_resolve_by_occurrence_time() {
    let first = extract("first.py", b"def selected():\n    return 1\n");
    let second = extract("second.py", b"def selected():\n    return 2\n");
    let caller = extract(
        "app.py",
        b"from first import selected\nselected()\nfrom second import selected\nselected()\n",
    );
    let resolved = compass_resolve::resolve(&[first, second, caller], &HashMap::new());

    for (qualified, location) in [("first.selected", "L2"), ("second.selected", "L4")] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified)
            .unwrap_or_else(|| panic!("missing {qualified}"));
        let edges = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.target == target.id
                    && edge.string("relation") == "calls"
                    && edge.string("resolution_rule") == "explicit-binding"
            })
            .collect::<Vec<_>>();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].string("source_location"), location);
    }
}

#[test]
fn universal_declarations_project_without_prebuilt_graph_nodes() {
    let mut provider = extract(
        "tools/runner.py",
        b"def execute(value):\n    return value\n",
    );
    let mut caller = extract(
        "app.py",
        b"from tools.runner import execute as run\ndef main():\n    run(1)\n",
    );
    provider.nodes.clear();
    provider.edges.clear();
    caller.nodes.clear();
    caller.edges.clear();

    let resolved = compass_resolve::resolve(&[provider, caller], &HashMap::new());
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "execute()")
        .expect("projected provider function");
    let source = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "main()")
        .expect("projected caller function");

    assert_eq!(
        target.string("extractor"),
        "compass.languages.python.universal"
    );
    assert_eq!(source.string("_origin"), "ast");
    assert!(resolved.nodes.iter().any(|node| {
        node.string("symbol_kind") == "file" && node.string("source_file") == "app.py"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == source.id
            && edge.target == target.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
}

#[test]
fn repeated_external_base_uses_share_one_canonical_class_and_exact_edges() {
    let source = b"from ctypes import Structure\nclass First(Structure):\n    pass\nclass Second(Structure):\n    pass\n";
    let extracted = extract("models.py", source);
    let sources = HashMap::from([(
        "models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let targets = resolved
        .nodes
        .iter()
        .filter(|node| {
            node.string("qualified_name") == "ctypes.Structure"
                && node.string("symbol_kind") == "class"
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].string("source_location"), "");
    assert_eq!(
        targets[0]
            .attributes
            .get("placeholder")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let mut sites = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "inherits" && edge.target == targets[0].id)
        .map(|edge| edge.string("source_location"))
        .collect::<Vec<_>>();
    sites.sort();
    assert_eq!(sites, ["L2", "L4"]);
}

#[test]
fn qualified_calls_use_exact_receiver_bindings_before_terminal_names() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
type Other struct{}
func (other *Other) Run() {}
func caller(untyped any, worker *Worker, other *Other) {
    untyped.Run()
    worker.Run()
    other.Run()
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let worker_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("qualified_name") == "pkg.Worker::Run"
        })
        .unwrap_or_else(|| panic!("Worker.Run declaration; nodes={:#?}", resolved.nodes));
    let other_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("qualified_name") == "pkg.Other::Run"
        })
        .unwrap_or_else(|| panic!("Other.Run declaration; nodes={:#?}", resolved.nodes));
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .collect::<Vec<_>>();

    assert_eq!(calls.len(), 2, "untyped receivers must fail closed");
    assert!(
        calls
            .iter()
            .any(|edge| { edge.target == worker_run.id && edge.string("source_location") == "L8" })
    );
    assert!(
        calls
            .iter()
            .any(|edge| { edge.target == other_run.id && edge.string("source_location") == "L9" })
    );
}

#[test]
fn go_closures_resolve_typed_parameters_and_captured_receivers() {
    let go_source = br#"package pkg
import "io/fs"
type Worker struct{}
func (worker *Worker) Run() {}
func caller(worker *Worker) {
    visit := func(entry fs.DirEntry) {
        entry.Name()
        worker.Run()
    }
    shadow := func(worker any) {
        worker.Run()
    }
    variadic := func(worker ...*Worker) {
        worker.Run()
    }
    capture := func() {
        worker.Run()
    }
    _ = visit
    _ = shadow
    _ = variadic
    _ = capture
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let worker_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Run")
        .expect("Worker.Run declaration");
    let entry_name = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "io/fs.DirEntry::Name")
        .expect("external fs.DirEntry.Name method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L8"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == entry_name.id
            && edge.string("source_location") == "L7"
            && edge.string("resolution_rule") == "qualified-external"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L17"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        let location = edge.string("source_location");
        edge.string("relation") == "calls" && (location == "L11" || location == "L14")
    }));
}

#[test]
fn qualified_external_types_are_not_rebound_to_local_terminal_names() {
    let go_source = br#"package auth
import deviceflow "example.com/deviceflow"
import authcode "example.com/authcode"
type Client struct {
    inner *deviceflow.Client
    browser *authcode.Client
}
"#;
    let extracted = extract("auth/client.go", go_source);
    let sources = HashMap::from([(
        "auth/client.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let local = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "auth.Client")
        .expect("local Client declaration");

    for (qualified_name, line) in [
        ("example.com/deviceflow.Client", "L5"),
        ("example.com/authcode.Client", "L6"),
    ] {
        let external = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .unwrap_or_else(|| panic!("external {qualified_name}; nodes={:#?}", resolved.nodes));
        assert_ne!(external.id, local.id);
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == local.id
                && edge.target == external.id
                && edge.string("relation") == "references"
                && edge.string("source_location") == line
        }));
    }
    assert!(resolved.edges.iter().all(|edge| edge.source != edge.target));
}

#[test]
fn go_module_imports_resolve_exported_functions_by_exact_source_directory() {
    let provider = extract(
        "cmd/entire/cli/trailers/trailers.go",
        b"package trailers\nfunc ParseMetadata(value string) {}\n",
    );
    let caller_source = br#"package checkpoint
import "github.com/entireio/cli/cmd/entire/cli/trailers"
func Load(value string) {
    visit := func() {
        trailers.ParseMetadata(value)
    }
    visit()
}
"#;
    let caller = extract("cmd/entire/cli/checkpoint/load.go", caller_source);
    let sources = HashMap::from([(
        "cmd/entire/cli/checkpoint/load.go".to_owned(),
        String::from_utf8(caller_source.to_vec()).expect("source"),
    )]);
    let resolved =
        compass_resolve::resolve_with_root(&[provider, caller], &sources, Path::new("."));
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "ParseMetadata()")
        .expect("provider function");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == target.id
            && edge.string("source_location") == "L5"
    }));
}

#[test]
fn go_packages_with_the_same_terminal_directory_keep_distinct_alias_owners() {
    let provider = extract(
        "api/checkpoint/metadata.go",
        b"package checkpoint\ntype Summary struct{}\n",
    );
    let alias = extract(
        "cmd/entire/cli/checkpoint/aliases.go",
        br#"package checkpoint
import api "github.com/example/project/api/checkpoint"
type Summary = api.Summary
"#,
    );
    let consumer_source = b"package checkpoint\nfunc Read() *Summary { return nil }\n";
    let consumer = extract("cmd/entire/cli/checkpoint/reader.go", consumer_source);
    let sources = HashMap::from([(
        "cmd/entire/cli/checkpoint/reader.go".to_owned(),
        String::from_utf8(consumer_source.to_vec()).expect("source"),
    )]);
    let resolved =
        compass_resolve::resolve_with_root(&[provider, alias, consumer], &sources, Path::new("."));
    let provider = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "api/checkpoint.Summary")
        .expect("provider Summary");
    let alias = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "cmd/entire/cli/checkpoint.Summary")
        .expect("local Summary alias");

    assert_ne!(provider.id, alias.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "returns"
            && edge.target == alias.id
            && edge.string("source_file") == "cmd/entire/cli/checkpoint/reader.go"
            && edge.string("source_location") == "L2"
    }));
}

#[test]
fn go_selector_chains_use_declared_direct_field_types() {
    let types = br#"package generated
type Schema struct{}
type Body struct {
    Value Schema
    Pointer *Schema
    Many []Schema
}
"#;
    let methods = br#"package generated
func (schema *Schema) Encode() {}
func (body *Body) Encode() {
    body.Value.Encode()
    body.Pointer.Encode()
    body.Many.Encode()
}
"#;
    let types = extract("generated/types.go", types);
    let methods_extraction = extract("generated/methods.go", methods);
    let sources = HashMap::from([(
        "generated/methods.go".to_owned(),
        String::from_utf8(methods.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[types, methods_extraction], &sources);
    let schema_encode = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "generated.Schema::Encode")
        .expect("Schema.Encode declaration");
    let call_sites = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == schema_encode.id)
        .map(|edge| edge.string("source_location"))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        call_sites,
        std::collections::BTreeSet::from(["L4".to_owned(), "L5".to_owned()])
    );
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L6"
    }));
}

#[test]
fn rust_impl_self_calls_and_tuple_struct_constructors_resolve_exactly() {
    let source = br#"struct Widget(u64);
impl Widget {
    fn first(&self) { self.second(); (*self).second(); }
    fn second(&self) {}
}
fn build() -> Widget { Widget(1) }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let widget = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Widget")
        .expect("Widget declaration");
    let second = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Widget::second")
        .expect("Widget.second declaration");

    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == second.id
                    && edge.string("source_location") == "L3"
            })
            .count(),
        2
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == widget.id
            && edge.string("source_location") == "L6"
    }));
}

#[test]
fn rust_local_qualified_call_resolves_before_a_wildcard_candidate() {
    let source = br#"pub use crate::support::*;
struct App;
impl App { fn new() -> Self { Self } }
fn builds() { App::new(); }
"#;
    let extracted = extract("crates/bevy_app/src/app.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "crates/bevy_app/src/app.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "bevy_app::app::App::new")
        .expect("App.new declaration");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("source_location") == "L4"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_import_binding_resolves_the_qualified_associated_function() {
    let provider = extract(
        "src/api.rs",
        b"pub struct Widget;\npub trait Build { fn build() -> Self; }\nimpl Widget { pub fn new() -> Self { Self } }\nimpl Build for Widget { fn build() -> Self { Self } }\n",
    );
    let caller_source =
        b"use crate::api::Widget;\nfn build() { Widget::new(); Widget::build(); }\n";
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let widget = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Widget")
        .expect("Widget declaration");
    let constructor = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Widget::new")
        .expect("Widget.new declaration");
    let trait_constructor = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "<crate::api::Widget as crate::api::Build>::build"
        })
        .expect("Widget trait build declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.target == constructor.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L2"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == widget.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L2")
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == trait_constructor.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "member-binding"
            && edge.string("source_location") == "L2"
    }));
}

#[test]
fn rust_unresolved_lexical_receiver_is_deferred_without_becoming_external() {
    let source = b"struct World;\nfn run(world: World) { world.spawn(); }\n";
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let deferred = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "world::spawn")
        .expect("deferred world.spawn target");

    assert_eq!(
        deferred
            .attributes
            .get("placeholder")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        deferred
            .attributes
            .get("external")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        deferred
            .attributes
            .get("deferred_receiver")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == deferred.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "deferred-receiver"
            && edge.string("source_location") == "L2"
    }));
}

#[test]
fn rust_wildcard_bindings_resolve_local_exports_and_preserve_external_candidates() {
    let provider = extract(
        "src/api.rs",
        b"pub struct Widget(pub u64);\npub fn new() {}\n",
    );
    let local_source = b"use crate::api::*;\nfn build(value: u64) -> Widget { Widget(value) }\nfn boxed() { Box::new(1); }\n";
    let local = extract("src/lib.rs", local_source);
    let external_source =
        b"use framework::prelude::*;\nfn load(value: External) -> External { External(value) }\n";
    let external = extract("src/external.rs", external_source);
    let resolved = compass_resolve::resolve(
        &[provider, local, external],
        &HashMap::from([
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(local_source.to_vec()).expect("local source"),
            ),
            (
                "src/external.rs".to_owned(),
                String::from_utf8(external_source.to_vec()).expect("external source"),
            ),
        ]),
    );
    let widget = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Widget")
        .expect("local wildcard target");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == widget.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));
    let box_new = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Box::new")
        .expect("qualified Box constructor placeholder");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == box_new.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
    }));
    let api_new = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::new")
        .expect("api new declaration");
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == api_new.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3")
    }));

    let external = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "framework::prelude::External")
        .expect("external wildcard placeholder");
    assert_eq!(
        external
            .attributes
            .get("placeholder")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == external.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "qualified-external"
    }));
}

#[test]
fn typescript_workspace_package_exports_follow_nodenext_reexports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let package = directory.path().join("packages/timezone/package.json");
    let barrel = directory.path().join("packages/timezone/src/index.ts");
    let implementation = directory.path().join("packages/timezone/src/date/index.ts");
    let consumer = directory.path().join("packages/app/src/consumer.ts");
    for path in [&package, &barrel, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let package_source = br#"{"name":"@example/timezone","exports":{".":"./src/index.ts"}}"#;
    let barrel_source = br#"export * from "./date/index.js";"#;
    let implementation_source = br#"export class ZonedDate {}"#;
    let consumer_source = br#"import { ZonedDate } from "@example/timezone";
export function makeDate() { return new ZonedDate(); }
function consume(value: unknown) { return value; }
export const wrappedDate = consume(ZonedDate);
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&barrel, barrel_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            barrel.to_str().ok_or("non-UTF-8 fixture path")?,
            barrel_source,
        ),
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    assert!(extractions[0].nodes.iter().any(|node| {
        node.string("symbol_kind") == "file"
            && node.string("source_file") == package.to_string_lossy()
    }));
    assert!(extractions[3].edges.iter().any(|edge| {
        edge.string("relation") == "imports_from" && edge.string("module") == "@example/timezone"
    }));
    let sources = [
        (&package, package_source.as_slice()),
        (&barrel, barrel_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, directory.path());
    assert_eq!(resolved.error, None);
    let declaration = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "ZonedDate"
                && node.string("source_file") == implementation.to_string_lossy()
        })
        .ok_or("missing ZonedDate declaration")?;
    let barrel_node = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file"
                && node.string("source_file") == barrel.to_string_lossy()
        })
        .ok_or("missing barrel file")?;
    let implementation_node = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file"
                && node.string("source_file") == implementation.to_string_lossy()
        })
        .ok_or("missing implementation file")?;
    let consumer_modules = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == consumer.to_string_lossy()
        })
        .map(|edge| {
            (
                edge.target.clone(),
                edge.string("module"),
                edge.string("target_file"),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        consumer_modules
            .iter()
            .any(|(target, _, _)| { target == &barrel_node.id }),
        "consumer modules: {consumer_modules:#?}"
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "re_exports"
            && edge.source == barrel_node.id
            && edge.target == implementation_node.id
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports"
            && edge.target == declaration.id
            && edge.string("source_file") == consumer.to_string_lossy()
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == declaration.id
            && edge.string("source_file") == consumer.to_string_lossy()
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references"
            && edge.target == declaration.id
            && edge.string("source_file") == consumer.to_string_lossy()
            && edge.string("context") == "argument"
    }));
    Ok(())
}

#[test]
fn duplicate_typescript_workspace_package_names_remain_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let consumer = directory.path().join("app/consumer.ts");
    let consumer_source = br#"import { Widget } from "@example/duplicate";
new Widget();
"#;
    let package_source = br#"{"name":"@example/duplicate","exports":"./index.ts"}"#;
    let implementation_source = br#"export class Widget {}"#;
    let mut fixtures = vec![(consumer.clone(), consumer_source.as_slice())];
    for package in ["first", "second"] {
        fixtures.extend([
            (
                directory.path().join(format!("{package}/package.json")),
                package_source.as_slice(),
            ),
            (
                directory.path().join(format!("{package}/index.ts")),
                implementation_source.as_slice(),
            ),
        ]);
    }
    for (path, source) in &fixtures {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = fixtures
        .iter()
        .map(|(path, source)| {
            Ok(extract(
                path.to_str().ok_or("non-UTF-8 fixture path")?,
                source,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let sources = fixtures
        .iter()
        .map(|(path, source)| {
            Ok((
                path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
                String::from_utf8(source.to_vec())?,
            ))
        })
        .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, directory.path());
    let widgets = resolved
        .nodes
        .iter()
        .filter(|node| {
            node.label() == "Widget"
                && ["first/index.ts", "second/index.ts"]
                    .iter()
                    .any(|suffix| node.string("source_file").ends_with(suffix))
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(widgets.len() >= 2);
    assert!(resolved.edges.iter().all(|edge| {
        !(widgets.contains(edge.target.as_str())
            && matches!(edge.string("relation").as_str(), "imports" | "calls")
            && edge.string("source_file") == consumer.to_string_lossy())
    }));
    Ok(())
}
