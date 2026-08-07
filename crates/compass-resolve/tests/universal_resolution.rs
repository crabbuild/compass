#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use compass_graph::build_from_extraction;
use compass_languages::{CandidateRelation, Engine, RawCall};
use compass_resolve::evidence::{UniversalResolutionIndex, UniversalResolutionLimits};
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
fn rust_nested_function_calls_resolve_to_the_lexical_declaration() {
    let source = b"fn join() { fn call() {} call(); call(); }\n";
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let join = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join")
        .expect("outer function");
    let call = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::call")
        .expect("nested function");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == join.id && edge.target == call.id && edge.string("relation") == "contains"
    }));
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == join.id
                    && edge.target == call.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        2
    );
}

#[test]
fn rust_local_wildcard_resolves_lowercase_calls_without_inventing_external_ones() {
    let provider_source = b"pub fn join() {}\nmod test;\n";
    let caller_source = br#"use super::*;
fn helper() {}
fn partition<T>(_value: &mut [T]) -> usize { 0 }
fn quick_sort<T>(value: &mut [T]) { let _mid = partition(value); join(); }
fn invokes() { helper(); join(); }
"#;
    let external_source =
        b"use external::prelude::*;\nfn caller() { lowercase(); std::iter::once(1); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/join/mod.rs", provider_source),
            extract("src/join/test.rs", caller_source),
            extract("src/external.rs", external_source),
        ],
        &HashMap::from([
            (
                "src/join/mod.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/join/test.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
            (
                "src/external.rs".to_owned(),
                String::from_utf8(external_source.to_vec()).expect("external source"),
            ),
        ]),
    );
    let join = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::join")
        .expect("join declaration");
    let invokes = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::test::invokes")
        .expect("invokes declaration");
    let helper = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::test::helper")
        .expect("same-module helper declaration");
    let partition = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::test::partition")
        .expect("same-module generic declaration");
    let quick_sort = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::join::test::quick_sort")
        .expect("same-module generic caller");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == invokes.id
            && edge.target == join.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == invokes.id
            && edge.target == helper.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") != "wildcard-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == quick_sort.id
            && edge.target == partition.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") != "wildcard-binding"
    }));
    assert!(resolved.nodes.iter().all(|node| {
        !matches!(
            node.string("qualified_name").as_str(),
            "external::prelude::lowercase" | "lowercase"
        )
    }));
    assert!(
        resolved
            .nodes
            .iter()
            .any(|node| node.string("qualified_name") == "std::iter::once")
    );
}

#[test]
fn rust_multiple_wildcards_resolve_one_unique_call_and_fail_closed_on_collision() {
    let first_source = b"pub fn bridge() {}\npub trait Work {}\npub struct Item;\n";
    let second_source = b"pub fn helper() {}\n";
    let caller_source = b"use crate::first::*;\nuse crate::second::*;\nstruct Local { item: Item }\nimpl Work for Local {}\nfn drive() { bridge(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/first.rs", first_source),
            extract("src/second.rs", second_source),
            extract("src/lib.rs", caller_source),
        ],
        &HashMap::from([
            (
                "src/first.rs".to_owned(),
                String::from_utf8(first_source.to_vec()).expect("first source"),
            ),
            (
                "src/second.rs".to_owned(),
                String::from_utf8(second_source.to_vec()).expect("second source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let bridge = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::first::bridge")
        .expect("bridge declaration");
    let work = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::first::Work")
        .expect("Work declaration");
    let item = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::first::Item")
        .expect("Item declaration");
    let local = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Local")
        .expect("Local declaration");
    let field = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Local::item")
        .expect("Local.item declaration");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == bridge.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L5"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == local.id
            && edge.target == work.id
            && edge.string("relation") == "implements"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == field.id
            && edge.target == item.id
            && edge.string("relation") == "type_of"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));

    let competing_source = b"pub fn bridge() {}\npub trait Work {}\npub struct Item;\n";
    let ambiguous = compass_resolve::resolve(
        &[
            extract("src/first.rs", first_source),
            extract("src/second.rs", competing_source),
            extract("src/lib.rs", caller_source),
        ],
        &HashMap::from([
            (
                "src/first.rs".to_owned(),
                String::from_utf8(first_source.to_vec()).expect("first source"),
            ),
            (
                "src/second.rs".to_owned(),
                String::from_utf8(competing_source.to_vec()).expect("competing source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    assert!(ambiguous.edges.iter().all(|edge| {
        !matches!(
            edge.string("relation").as_str(),
            "calls" | "implements" | "type_of"
        ) || !matches!(edge.string("source_location").as_str(), "L3" | "L4" | "L5")
    }));

    let incomplete_source = b"use crate::first::*;\nuse external::prelude::*;\nstruct Local { item: Item }\nimpl Work for Local {}\nfn drive() { bridge(); }\n";
    let incomplete = compass_resolve::resolve(
        &[
            extract("src/first.rs", first_source),
            extract("src/lib.rs", incomplete_source),
        ],
        &HashMap::from([
            (
                "src/first.rs".to_owned(),
                String::from_utf8(first_source.to_vec()).expect("first source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(incomplete_source.to_vec()).expect("incomplete source"),
            ),
        ]),
    );
    assert!(incomplete.edges.iter().all(|edge| {
        !matches!(
            edge.string("relation").as_str(),
            "calls" | "implements" | "type_of"
        ) || !matches!(edge.string("source_location").as_str(), "L3" | "L4" | "L5")
    }));
}

#[test]
fn rust_type_parameters_are_distinct_scoped_nodes_with_exact_type_relationships() {
    let source = br#"struct Wrapper<T: Clone> { value: T }
fn identity<T: Send>(value: T) -> T { value }
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let wrapper = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Wrapper")
        .expect("Wrapper");
    let field = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Wrapper::value")
        .expect("value field");
    let identity = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::identity")
        .expect("identity");
    let parameters = resolved
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "parameter" && node.string("label") == "T")
        .collect::<Vec<_>>();
    assert_eq!(parameters.len(), 2);
    let wrapper_parameter = parameters
        .iter()
        .copied()
        .find(|node| node.string("qualified_name").starts_with("crate::Wrapper"))
        .expect("Wrapper type parameter");
    let function_parameter = parameters
        .iter()
        .copied()
        .find(|node| node.string("qualified_name").starts_with("crate::identity"))
        .expect("identity type parameter");
    assert_ne!(wrapper_parameter.id, function_parameter.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == wrapper.id
            && edge.target == wrapper_parameter.id
            && edge.string("relation") == "contains"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == field.id
            && edge.target == wrapper_parameter.id
            && edge.string("relation") == "type_of"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == identity.id
            && edge.target == function_parameter.id
            && edge.string("relation") == "returns"
    }));
}

#[test]
fn rust_impl_and_method_type_parameters_shadow_without_escaping_their_scopes() {
    let source = br#"trait Marker {}
struct Wrapper<T> { value: T }
impl<T: Marker> Wrapper<T> {
    fn convert<U: Marker>(&self, value: U) -> T { self.value }
}
fn outside(value: T) {}
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let convert = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Wrapper::convert")
        .expect("convert");
    let outside = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::outside")
        .expect("outside");
    let implementation_parameter = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "parameter"
                && node.string("label") == "T"
                && node.string("qualified_name").contains("<impl<T: Marker>")
        })
        .expect("implementation T");
    let method_parameter = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "parameter"
                && node.string("label") == "U"
                && node
                    .string("qualified_name")
                    .starts_with("crate::Wrapper::convert")
        })
        .expect("method U");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == convert.id
            && edge.target == implementation_parameter.id
            && edge.string("relation") == "returns"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == convert.id
            && edge.target == method_parameter.id
            && edge.string("relation") == "references"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != outside.id
            || !matches!(
                edge.target.as_str(),
                target if target == implementation_parameter.id || target == method_parameter.id
            )
    }));
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
fn rust_local_module_reexports_resolve_without_external_placeholders() {
    let provider_source = b"pub fn work() {}\n";
    let root_source = b"mod api;\npub use api::work;\nfn caller() { work(); }\n";
    let provider = extract("src/api.rs", provider_source);
    let root = extract("src/lib.rs", root_source);
    let resolved = compass_resolve::resolve(
        &[provider, root],
        &HashMap::from([
            (
                "src/api.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(root_source.to_vec()).expect("root source"),
            ),
        ]),
    );
    let work = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::work")
        .expect("re-exported function");
    let caller = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::caller")
        .expect("caller function");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == work.id
            && edge.string("relation") == "re_exports"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == caller.id
            && edge.target == work.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "api::work"
            || node
                .attributes
                .get("placeholder")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
    }));
}

#[test]
fn rust_mutually_exclusive_platform_reexports_preserve_every_possible_call_target() {
    let source = br#"mod unix {
    pub fn get_cpu_time() {}
}
mod win {
    pub fn get_cpu_time() {}
}
#[cfg(windows)]
pub use self::win::get_cpu_time;
#[cfg(unix)]
pub use self::unix::get_cpu_time;
#[cfg(not(any(unix, windows)))]
pub fn get_cpu_time() {}
fn measure_cpu() { get_cpu_time(); }
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let measure_cpu = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::measure_cpu")
        .expect("measure_cpu declaration");
    let expected_targets = BTreeSet::from([
        "crate::get_cpu_time".to_owned(),
        "crate::unix::get_cpu_time".to_owned(),
        "crate::win::get_cpu_time".to_owned(),
    ]);
    let actual_targets = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == measure_cpu.id && edge.string("relation") == "calls")
        .filter_map(|edge| {
            resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map(|node| node.string("qualified_name"))
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(actual_targets, expected_targets);

    let ambiguous_source = br#"mod first { pub fn work() {} }
mod second { pub fn work() {} }
pub use self::first::work;
pub use self::second::work;
fn caller() { work(); }
"#;
    let ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", ambiguous_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(ambiguous_source.to_vec()).expect("ambiguous source"),
        )]),
    );
    let caller = ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::caller")
        .expect("caller declaration");
    assert!(
        ambiguous
            .edges
            .iter()
            .all(|edge| { edge.source != caller.id || edge.string("relation") != "calls" })
    );

    let feature_source = br#"mod first { pub fn work() {} }
mod second { pub fn work() {} }
#[cfg(feature = "first")]
pub use self::first::work;
#[cfg(feature = "second")]
pub use self::second::work;
fn caller() { work(); }
"#;
    let feature_ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", feature_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(feature_source.to_vec()).expect("feature source"),
        )]),
    );
    let feature_caller = feature_ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::caller")
        .expect("feature caller declaration");
    assert!(
        feature_ambiguous
            .edges
            .iter()
            .all(|edge| { edge.source != feature_caller.id || edge.string("relation") != "calls" })
    );

    let malformed_fallback_source = br#"mod unix { pub fn work() {} }
mod win { pub fn work() {} }
#[cfg(unix)]
pub use self::unix::work;
#[cfg(windows)]
pub use self::win::work;
pub fn work() {}
fn caller() { work(); }
"#;
    let malformed_fallback = compass_resolve::resolve(
        &[extract("src/lib.rs", malformed_fallback_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(malformed_fallback_source.to_vec()).expect("malformed source"),
        )]),
    );
    let malformed_caller = malformed_fallback
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::caller")
        .expect("malformed caller declaration");
    assert!(
        malformed_fallback.edges.iter().all(|edge| {
            edge.source != malformed_caller.id || edge.string("relation") != "calls"
        })
    );
}

#[test]
fn rust_child_glob_resolves_parent_reexports_with_source_present_sibling_crate() {
    let provider_source = b"pub struct Empty;\npub fn empty() -> Empty { Empty }\n";
    let parent_source = b"mod empty;\npub use self::empty::{Empty, empty};\nmod test;\n";
    let sibling_source = b"pub fn sibling_api() {}\n";
    let child_source =
        b"use rayon_core::*;\nuse super::*;\nfn check_empty() { let _ = empty(); }\n";
    let provider = extract("src/iter/empty.rs", provider_source);
    let parent = extract("src/iter/mod.rs", parent_source);
    let sibling = extract("rayon-core/src/lib.rs", sibling_source);
    let child = extract("src/iter/test.rs", child_source);
    let resolved = compass_resolve::resolve(
        &[provider, parent, sibling, child],
        &HashMap::from([
            (
                "src/iter/empty.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/iter/mod.rs".to_owned(),
                String::from_utf8(parent_source.to_vec()).expect("parent source"),
            ),
            (
                "rayon-core/src/lib.rs".to_owned(),
                String::from_utf8(sibling_source.to_vec()).expect("sibling source"),
            ),
            (
                "src/iter/test.rs".to_owned(),
                String::from_utf8(child_source.to_vec()).expect("child source"),
            ),
        ]),
    );
    let empty = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::iter::empty::empty")
        .expect("re-exported empty function");
    let caller = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::iter::test::check_empty")
        .expect("child caller");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == caller.id
            && edge.target == empty.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));

    let unknown_child_source =
        b"use missing_core::*;\nuse super::*;\nfn check_empty() { let _ = empty(); }\n";
    let unresolved = compass_resolve::resolve(
        &[
            extract("src/iter/empty.rs", provider_source),
            extract("src/iter/mod.rs", parent_source),
            extract("src/iter/test.rs", unknown_child_source),
        ],
        &HashMap::from([
            (
                "src/iter/empty.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/iter/mod.rs".to_owned(),
                String::from_utf8(parent_source.to_vec()).expect("parent source"),
            ),
            (
                "src/iter/test.rs".to_owned(),
                String::from_utf8(unknown_child_source.to_vec()).expect("unknown child source"),
            ),
        ]),
    );
    let unresolved_empty = unresolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::iter::empty::empty")
        .expect("re-exported empty function with unknown sibling");
    let unresolved_caller = unresolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::iter::test::check_empty")
        .expect("child caller with unknown sibling");
    assert!(unresolved.edges.iter().all(|edge| {
        edge.source != unresolved_caller.id
            || edge.target != unresolved_empty.id
            || edge.string("relation") != "calls"
    }));
}

#[test]
fn rust_source_present_sibling_glob_resolves_associated_call_chains() {
    let provider_source = br#"pub struct Builder;
pub struct Pool;
impl Builder {
    pub fn new() -> Self { Builder }
    pub fn tune(self) -> Self { self }
    pub fn build(self) -> Result<Pool, ()> { todo!() }
}
"#;
    let prelude_source = b"pub fn unrelated() {}\n";
    let consumer_source = b"use crate::prelude::*;\nuse rayon_core::*;\nfn run() { Builder::new().tune().build().unwrap(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("rayon-core/src/lib.rs", provider_source),
            extract("src/prelude.rs", prelude_source),
            extract("src/test.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "rayon-core/src/lib.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/prelude.rs".to_owned(),
                String::from_utf8(prelude_source.to_vec()).expect("prelude source"),
            ),
            (
                "src/test.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    for target in [
        "rayon_core::Builder::new",
        "rayon_core::Builder::tune",
        "rayon_core::Builder::build",
    ] {
        let declaration = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target)
            .expect("provider declaration");
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == declaration.id
                    && edge.string("source_location") == "L3"
                    && edge.string("confidence") == "EXTRACTED"
            }),
            "associated call did not resolve through the source-present sibling glob: {target}"
        );
    }
    let unwrap = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "std::result::Result::unwrap")
        .expect("canonical Result.unwrap target");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == unwrap.id
                && edge.string("source_location") == "L3"
                && edge.string("confidence") == "INFERRED"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_glob_resolves_associated_chain_through_named_reexport() {
    let provider_source = br#"pub struct Builder;
pub struct Pool;
impl Builder {
    pub fn new() -> Self { Builder }
    pub fn tune(self) -> Self { self }
    pub fn build(self) -> Result<Pool, ()> { todo!() }
}
"#;
    let facade_source = b"pub use rayon_core::Builder;\n";
    let consumer_source = b"use crate::*;\nfn run() { Builder::new().tune().build().unwrap(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("rayon-core/src/lib.rs", provider_source),
            extract("src/lib.rs", facade_source),
            extract("tests/named.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "rayon-core/src/lib.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(facade_source.to_vec()).expect("facade source"),
            ),
            (
                "tests/named.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    for target in [
        "rayon_core::Builder::new",
        "rayon_core::Builder::tune",
        "rayon_core::Builder::build",
    ] {
        let declaration = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target)
            .expect("provider declaration");
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == declaration.id
                    && edge.string("source_file") == "tests/named.rs"
                    && edge.string("confidence") == "EXTRACTED"
            }),
            "named reexport did not resolve associated call: {target}"
        );
    }
}

#[test]
fn rust_glob_does_not_guess_through_colliding_named_reexports() {
    let provider_source = br#"pub struct Builder;
impl Builder { pub fn new() -> Self { Builder } }
"#;
    let facade_source = b"pub use first::Builder;\npub use second::Builder;\n";
    let consumer_source = b"use crate::*;\nfn run() { Builder::new(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("first/src/lib.rs", provider_source),
            extract("second/src/lib.rs", provider_source),
            extract("src/lib.rs", facade_source),
            extract("tests/named.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "first/src/lib.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("first source"),
            ),
            (
                "second/src/lib.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("second source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(facade_source.to_vec()).expect("facade source"),
            ),
            (
                "tests/named.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    let declarations = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "first::Builder::new" | "second::Builder::new"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(declarations.len(), 2);
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || !declarations.contains(edge.target.as_str())
    }));
}

#[test]
fn rust_glob_does_not_follow_a_named_reexport_cycle() {
    let root_source = b"mod first;\nmod second;\n";
    let first_source = b"pub use crate::second::Builder;\n";
    let second_source = b"pub use crate::first::Builder;\n";
    let consumer_source = b"use crate::first::*;\nfn run() { Builder::new(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/lib.rs", root_source),
            extract("src/first.rs", first_source),
            extract("src/second.rs", second_source),
            extract("tests/named.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(root_source.to_vec()).expect("root source"),
            ),
            (
                "src/first.rs".to_owned(),
                String::from_utf8(first_source.to_vec()).expect("first source"),
            ),
            (
                "src/second.rs".to_owned(),
                String::from_utf8(second_source.to_vec()).expect("second source"),
            ),
            (
                "tests/named.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("source_file") != "tests/named.rs" || edge.string("relation") != "calls"
    }));
}

#[test]
fn rust_named_reexport_does_not_capture_a_lowercase_receiver() {
    let provider_source = b"pub fn rng() {}\n";
    let facade_source = b"pub use provider::rng;\n";
    let consumer_source =
        b"use crate::*;\nfn run() { let rng = missing(); rng.sample_iter().take(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("provider/src/lib.rs", provider_source),
            extract("src/lib.rs", facade_source),
            extract("tests/receiver.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "provider/src/lib.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(facade_source.to_vec()).expect("facade source"),
            ),
            (
                "tests/receiver.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    let reexported_rng = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "provider::rng")
        .expect("reexported rng declaration");
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("source_file") != "tests/receiver.rs"
            || edge.string("relation") != "calls"
            || edge.target != reexported_rng.id
    }));
    assert!(resolved.nodes.iter().all(|node| {
        let qualified_name = node.string("qualified_name");
        !qualified_name.starts_with("rng()::") && !qualified_name.starts_with("rng().")
    }));
}

#[test]
fn rust_colliding_globs_do_not_guess_an_associated_call_chain() {
    let first_source = br#"pub struct Builder;
impl Builder {
    pub fn new() -> Self { Builder }
    pub fn tune(self) -> Self { self }
}
"#;
    let second_source = first_source;
    let consumer_source =
        b"use crate::first::*;\nuse crate::second::*;\nfn run() { Builder::new().tune(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/first.rs", first_source),
            extract("src/second.rs", second_source),
            extract("src/lib.rs", consumer_source),
        ],
        &HashMap::from([
            (
                "src/first.rs".to_owned(),
                String::from_utf8(first_source.to_vec()).expect("first source"),
            ),
            (
                "src/second.rs".to_owned(),
                String::from_utf8(second_source.to_vec()).expect("second source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(consumer_source.to_vec()).expect("consumer source"),
            ),
        ]),
    );
    let declarations = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "crate::first::Builder::new"
                    | "crate::first::Builder::tune"
                    | "crate::second::Builder::new"
                    | "crate::second::Builder::tune"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(declarations.len(), 4);
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || !declarations.contains(edge.target.as_str())
    }));
}

#[test]
fn rust_importing_a_local_module_targets_its_module_declaration() {
    let root_source = b"mod unwind;\nmod job;\n";
    let unwind_source = b"pub fn halt() {}\n";
    let job_source = b"use crate::unwind;\nfn run() { unwind::halt(); }\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/lib.rs", root_source),
            extract("src/unwind.rs", unwind_source),
            extract("src/job.rs", job_source),
        ],
        &HashMap::from([
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(root_source.to_vec()).expect("root source"),
            ),
            (
                "src/unwind.rs".to_owned(),
                String::from_utf8(unwind_source.to_vec()).expect("unwind source"),
            ),
            (
                "src/job.rs".to_owned(),
                String::from_utf8(job_source.to_vec()).expect("job source"),
            ),
        ]),
    );
    let job_file = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file" && node.string("source_file") == "src/job.rs"
        })
        .expect("job file");
    let unwind_module = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "module"
                && node.string("qualified_name") == "crate::unwind"
        })
        .unwrap_or_else(|| panic!("unwind module; nodes={:#?}", resolved.nodes));
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == job_file.id
                && edge.target == unwind_module.id
                && edge.string("relation") == "imports_from"
                && edge.string("source_location") == "L1"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_local_module_imports_fail_closed_with_a_competing_declaration_kind() {
    let root_source = b"mod unwind;\nstruct unwind;\nmod job;\n";
    let unwind_source = b"pub fn halt() {}\n";
    let job_source = b"use crate::unwind;\n";
    let resolved = compass_resolve::resolve(
        &[
            extract("src/lib.rs", root_source),
            extract("src/unwind.rs", unwind_source),
            extract("src/job.rs", job_source),
        ],
        &HashMap::from([
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(root_source.to_vec()).expect("root source"),
            ),
            (
                "src/unwind.rs".to_owned(),
                String::from_utf8(unwind_source.to_vec()).expect("unwind source"),
            ),
            (
                "src/job.rs".to_owned(),
                String::from_utf8(job_source.to_vec()).expect("job source"),
            ),
        ]),
    );
    let job_file = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file" && node.string("source_file") == "src/job.rs"
        })
        .expect("job file");
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != job_file.id
            || edge.string("relation") != "imports_from"
            || edge.string("source_location") != "L1"
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
fn rust_blanket_impl_is_owned_by_its_exact_scoped_type_parameter() {
    let source = br#"trait Render {}
impl<T> Render for T {}
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let parameter = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<impl<T> Render for T>::<T>")
        .expect("blanket implementation parameter");
    let render = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Render")
        .expect("Render declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == parameter.id
            && edge.target == render.id
            && edge.string("relation") == "implements"
            && edge.string("source_location") == "L2"
    }));

    let ambiguous_source = br#"mod left { pub trait Render {} }
mod right { pub trait Render {} }
use left::*;
use right::*;
impl<T> Render for T {}
"#;
    let ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", ambiguous_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(ambiguous_source.to_vec()).expect("ambiguous source"),
        )]),
    );
    let ambiguous_parameter = ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<impl<T> Render for T>::<T>")
        .expect("ambiguous blanket implementation parameter");
    let render_ids = ambiguous
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name").ends_with("::Render"))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(render_ids.len(), 2);
    assert!(ambiguous.edges.iter().all(|edge| {
        edge.source != ambiguous_parameter.id
            || !render_ids.contains(edge.target.as_str())
            || edge.string("relation") != "implements"
    }));
}

#[test]
fn rust_impl_trait_arguments_are_source_anchored_references_from_the_implementer() {
    let source = br#"trait Convert<T> {}
struct Input;
struct Output;
impl Convert<Input> for Output {}
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let input = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Input")
        .expect("Input declaration");
    let output = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Output")
        .expect("Output declaration");
    let convert = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Convert")
        .expect("Convert declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == output.id
            && edge.target == input.id
            && edge.string("relation") == "references"
            && edge.string("source_location") == "L4"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != output.id
            || edge.target != convert.id
            || edge.string("relation") != "references"
    }));

    let ambiguous_source = br#"mod left { pub struct Input; }
mod right { pub struct Input; }
use left::*;
use right::*;
trait Convert<T> {}
struct Output;
impl Convert<Input> for Output {}
"#;
    let ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", ambiguous_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(ambiguous_source.to_vec()).expect("ambiguous source"),
        )]),
    );
    let ambiguous_output = ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Output")
        .expect("ambiguous Output declaration");
    let input_ids = ambiguous
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name").ends_with("::Input"))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(input_ids.len(), 2);
    assert!(ambiguous.edges.iter().all(|edge| {
        edge.source != ambiguous_output.id
            || !input_ids.contains(edge.target.as_str())
            || edge.string("relation") != "references"
    }));
}

#[test]
fn rust_associated_returns_resolve_per_impl_and_ambiguity_fails_closed() {
    let source = br#"trait Produce { type Output; fn produce() -> Self::Output; }
struct Alpha;
struct Beta;
struct AlphaOutput;
struct BetaOutput;
impl Produce for Alpha {
    type Output = AlphaOutput;
    fn produce() -> Self::Output { AlphaOutput }
}
impl Produce for Beta {
    type Output = BetaOutput;
    fn produce() -> Self::Output { BetaOutput }
}
"#;
    let resolved = compass_resolve::resolve(
        &[extract("src/lib.rs", source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    for (owner, concrete) in [("Alpha", "AlphaOutput"), ("Beta", "BetaOutput")] {
        let associated = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("qualified_name") == format!("<impl Produce for {owner}>::Output")
            })
            .expect("impl-scoped associated type");
        let method = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("qualified_name")
                    == format!("<crate::{owner} as crate::Produce>::produce")
            })
            .expect("impl method");
        let concrete = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == format!("crate::{concrete}"))
            .expect("concrete associated type realization");
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == method.id
                && edge.target == associated.id
                && edge.string("relation") == "returns"
        }));
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == associated.id
                && edge.target == concrete.id
                && edge.string("relation") == "references"
        }));
    }

    let ambiguous_source = br#"trait Produce { type Output; fn produce() -> Self::Output; }
struct Alpha;
struct First;
struct Second;
impl Produce for Alpha {
    type Output = First;
    type Output = Second;
    fn produce() -> Self::Output { First }
}
"#;
    let ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", ambiguous_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(ambiguous_source.to_vec()).expect("ambiguous source"),
        )]),
    );
    let method = ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Alpha as crate::Produce>::produce")
        .expect("ambiguous impl method");
    assert!(
        ambiguous
            .edges
            .iter()
            .all(|edge| { edge.source != method.id || edge.string("relation") != "returns" })
    );
}

#[test]
fn rust_inherited_associated_return_resolves_through_the_exact_supertrait_impl() {
    let traits = br#"pub trait Consumer<Item>: Send + Sized {
    type Reducer;
}
pub trait UnindexedConsumer<I>: Consumer<I> {
    fn to_reducer(&self) -> Self::Reducer;
}
"#;
    let reexports = br#"use self::plumbing::*;
"#;
    let implementation = br#"use super::*;
struct ConcreteReducer;
struct ItemConsumer;
impl<T: Send> Consumer<T> for ItemConsumer {
    type Reducer = ConcreteReducer;
}
impl<T: Send> UnindexedConsumer<T> for ItemConsumer {
    fn to_reducer(&self) -> Self::Reducer { ConcreteReducer }
}
"#;
    let trait_extraction = extract("src/iter/plumbing/mod.rs", traits);
    assert!(
        trait_extraction.semantic_evidence.is_some(),
        "trait extraction failed: {:?}",
        trait_extraction.error
    );
    let implementation_extraction = extract("src/iter/extend.rs", implementation);
    assert!(
        implementation_extraction.semantic_evidence.is_some(),
        "implementation extraction failed: {:?}",
        implementation_extraction.error
    );
    let resolved = compass_resolve::resolve(
        &[
            trait_extraction,
            extract("src/iter/mod.rs", reexports),
            implementation_extraction,
        ],
        &HashMap::from([
            (
                "src/iter/plumbing/mod.rs".to_owned(),
                String::from_utf8(traits.to_vec()).expect("traits"),
            ),
            (
                "src/iter/mod.rs".to_owned(),
                String::from_utf8(reexports.to_vec()).expect("reexports"),
            ),
            (
                "src/iter/extend.rs".to_owned(),
                String::from_utf8(implementation.to_vec()).expect("implementation"),
            ),
        ]),
    );
    let associated = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "<impl<T: Send> Consumer<T> for ItemConsumer>::Reducer"
        })
        .expect("Consumer implementation associated type");
    let method = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name").contains("ItemConsumer")
                && node.string("qualified_name").ends_with("::to_reducer")
        })
        .expect("UnindexedConsumer implementation method");

    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == method.id
                && edge.target == associated.id
                && edge.string("relation") == "returns"
                && edge.string("resolution_rule") == "rust-associated-type"
        }),
        "inherited associated return was not resolved to the exact supertrait impl"
    );
}

#[test]
fn rust_inherited_associated_reference_resolves_from_a_parent_module_glob() {
    let traits = br#"mod plumbing {}
use self::plumbing::*;
pub trait ParallelIterator: Sized + Send {
    type Item;
}
pub trait IndexedParallelIterator: ParallelIterator {
    fn drive(&self) -> Self::Item;
}
"#;
    let implementation = br#"use super::*;
struct FoldChunks;
impl ParallelIterator for FoldChunks {
    type Item = u32;
}
impl IndexedParallelIterator for FoldChunks {
    fn drive(&self) -> Self::Item { 0 }
}
"#;
    let resolved = compass_resolve::resolve(
        &[
            extract("src/iter/mod.rs", traits),
            extract("src/iter/fold_chunks.rs", implementation),
        ],
        &HashMap::from([
            (
                "src/iter/mod.rs".to_owned(),
                String::from_utf8(traits.to_vec()).expect("traits"),
            ),
            (
                "src/iter/fold_chunks.rs".to_owned(),
                String::from_utf8(implementation.to_vec()).expect("implementation"),
            ),
        ]),
    );
    let associated = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name")
                .contains("ParallelIterator for FoldChunks")
                && node.string("qualified_name").ends_with("::Item")
        })
        .expect("ParallelIterator implementation associated type");
    let method = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name").contains("FoldChunks")
                && node.string("qualified_name").ends_with("::drive")
        })
        .expect("IndexedParallelIterator implementation method");

    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == method.id
                && edge.target == associated.id
                && edge.string("relation") == "returns"
                && edge.string("resolution_rule") == "rust-associated-type"
        }),
        "inherited associated reference was not resolved through the parent module glob"
    );
}

#[test]
fn rust_inherited_associated_return_fails_closed_for_competing_or_unknown_traits() {
    let ambiguous_source = br#"trait First { type Output; }
trait Second { type Output; }
trait Combined: First + Second {
    fn output(&self) -> Self::Output;
}
struct FirstOutput;
struct SecondOutput;
struct Item;
impl First for Item { type Output = FirstOutput; }
impl Second for Item { type Output = SecondOutput; }
impl Combined for Item {
    fn output(&self) -> Self::Output { FirstOutput }
}
"#;
    let ambiguous = compass_resolve::resolve(
        &[extract("src/lib.rs", ambiguous_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(ambiguous_source.to_vec()).expect("ambiguous source"),
        )]),
    );
    let ambiguous_method = ambiguous
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Item as crate::Combined>::output")
        .expect("ambiguous Combined implementation method");
    assert!(ambiguous.edges.iter().all(|edge| {
        edge.source != ambiguous_method.id || edge.string("relation") != "returns"
    }));

    let incomplete_source = br#"trait Combined: external::Base {
    fn output(&self) -> Self::Output;
}
struct Item;
impl Combined for Item {
    fn output(&self) -> Self::Output { loop {} }
}
"#;
    let incomplete = compass_resolve::resolve(
        &[extract("src/lib.rs", incomplete_source)],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(incomplete_source.to_vec()).expect("incomplete source"),
        )]),
    );
    let incomplete_method = incomplete
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Item as crate::Combined>::output")
        .expect("incomplete Combined implementation method");
    assert!(incomplete.edges.iter().all(|edge| {
        edge.source != incomplete_method.id || edge.string("relation") != "returns"
    }));
}

#[test]
fn rust_generic_trait_impl_calls_resolve_to_exact_impl_owner() {
    let source = br#"trait Render<T> {
    fn render(&self, value: T);
}
struct Container<T>(T);
impl<T> Render<T> for Container<T> {
    fn render(&self, _value: T) {}
}
fn invoke(container: Container<u32>) {
    container.render(1);
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
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Container as crate::Render>::render")
        .expect("generic Render implementation method");

    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == render_method.id
                && edge.string("source_location") == "L9"
        }),
        "generic method call was not resolved"
    );
}

#[test]
fn rust_generic_trait_bound_receiver_resolves_to_trait_method() {
    let source = br#"trait Render { fn render(&self); }
fn invoke<T: Render>(container: T) {
    container.render();
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
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Render::render")
        .expect("Render trait method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == render_method.id
            && edge.string("source_location") == "L3"
    }));
}

#[test]
fn rust_where_clause_generic_receiver_resolves_to_trait_method() {
    let source = br#"trait IndexedParallelIterator {
    fn with_producer(&self);
}
fn bridge<I>(par_iter: I)
where
    I: IndexedParallelIterator,
{
    par_iter.with_producer();
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
    let with_producer = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "crate::IndexedParallelIterator::with_producer"
        })
        .expect("IndexedParallelIterator::with_producer trait method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == with_producer.id
            && edge.string("source_location") == "L8"
    }));
}

#[test]
fn rust_cross_module_where_clause_generic_receiver_resolves_to_trait_method() {
    let provider_source = b"pub trait IndexedParallelIterator {\n    fn with_producer(&self);\n}\n";
    let caller_source = b"use super::IndexedParallelIterator;\nfn bridge<I>(par_iter: I)\nwhere\n    I: IndexedParallelIterator,\n{\n    par_iter.with_producer();\n}\n";
    let provider = extract("src/iter/mod.rs", provider_source);
    let caller = extract("src/iter/plumbing/mod.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/iter/mod.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/iter/plumbing/mod.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let with_producer = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "crate::iter::IndexedParallelIterator::with_producer"
        })
        .expect("IndexedParallelIterator::with_producer trait method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == with_producer.id
            && edge.string("source_location") == "L6"
    }));
}

#[test]
fn rust_cross_file_generic_trait_bound_receiver_keeps_imported_trait_owner() {
    let provider_source = b"pub trait Render { fn render(&self); }\n";
    let caller_source = b"use crate::api::Render;\nfn invoke<T: Render>(container: T) {\n    container.render();\n}\n";
    let provider = extract("src/api.rs", provider_source);
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/api.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Render::render")
        .expect("imported Render trait method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == render_method.id
            && edge.string("source_location") == "L3"
    }));
}

#[test]
fn rust_ambiguous_generic_trait_bounds_do_not_choose_an_arbitrary_method() {
    let source = br#"trait First { fn run(&self); }
trait Second { fn run(&self); }
fn invoke<T: First + Second>(value: T) {
    value.run();
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
    let trait_methods = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "crate::First::run" | "crate::Second::run"
            )
        })
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(trait_methods.len(), 2);
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls"
            || edge.string("source_location") != "L4"
            || !trait_methods.contains(&edge.target)
    }));
}

#[test]
fn rust_turbofish_and_explicit_trait_impl_paths_resolve_exactly() {
    let source = br#"trait Render<T> {
    fn render(&self, value: T);
}
struct Container<T>(T);
impl<T> Render<T> for Container<T> {
    fn render(&self, _value: T) {}
}
fn invoke(container: Container<u32>) {
    Container::<u32>::render(&container, 1);
    <Container<u32> as Render<u32>>::render(&container, 1);
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
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "<crate::Container as crate::Render>::render")
        .expect("generic Render implementation method");

    for line in ["L9", "L10"] {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == render_method.id
                    && edge.string("source_location") == line
            }),
            "generic call at {line} was not resolved"
        );
    }
}

#[test]
fn rust_cargo_manifest_resolution_uses_workspace_alias_and_custom_lib_roots()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "Cargo.toml",
            r#"[workspace]
members = ["crates/*"]

[workspace.dependencies]
provider-alias = { package = "provider-package", path = "crates/provider" }
"#,
        ),
        (
            "crates/provider/Cargo.toml",
            r#"[package]
name = "provider-package"
version = "0.1.0"

[lib]
name = "provider_api"
path = "src/api.rs"
"#,
        ),
        ("crates/provider/src/api.rs", "pub fn work() {}\n"),
        (
            "crates/consumer/Cargo.toml",
            r#"[package]
name = "consumer-package"
version = "0.1.0"

[lib]
name = "consumer_api"
path = "src/custom_root.rs"

[dependencies]
provider_alias = { workspace = true }
"#,
        ),
        (
            "crates/consumer/src/custom_root.rs",
            "use provider_alias::work;\npub fn caller() { work(); }\n",
        ),
    ];
    let mut extractions = Vec::with_capacity(fixtures.len());
    let mut sources = HashMap::new();
    for (relative, source) in fixtures {
        let path = directory.path().join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            let source_file = path.to_str().ok_or("non-UTF-8 fixture path")?;
            extractions.push(extract(source_file, source.as_bytes()));
            sources.insert(source_file.to_owned(), source.to_owned());
        }
    }

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, directory.path());
    let provider = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "provider_api::work"
                && node.string("source_file").ends_with("src/api.rs")
        })
        .ok_or("missing provider endpoint")?;
    let caller = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "consumer_api::caller"
                && node.string("source_file").ends_with("src/custom_root.rs")
        })
        .ok_or("missing consumer endpoint")?;
    assert!(
        !resolved
            .nodes
            .iter()
            .any(|node| node.string("qualified_name") == "provider_alias::work")
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == caller.id
            && edge.target == provider.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    Ok(())
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
    assert_eq!(
        calls[0].string("resolution_rule"),
        "direct-receiver-successor-dispatch"
    );
}

#[test]
fn python_super_call_follows_a_complete_single_inheritance_chain() {
    let provider = extract(
        "pkg/base.py",
        b"class Grandparent:\n    def run(self):\n        return None\nclass Parent(Grandparent):\n    pass\n",
    );
    let caller_source =
        b"from pkg.base import Parent\nclass Child(Parent):\n    def run(self):\n        super().run()\n";
    let caller = extract("pkg/child.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/child.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let grandparent_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.base.Grandparent::run")
        .unwrap_or_else(|| panic!("grandparent method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == grandparent_run.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L4");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
}

#[test]
fn python_local_class_call_resolves_an_inherited_class_method() {
    let provider = extract(
        "pkg/base.py",
        b"class Base:\n    @classmethod\n    def check(cls):\n        return []\n",
    );
    let caller_source = b"from pkg.base import Base\ndef verify():\n    class Model(Base):\n        pass\n    return Model.check()\n";
    let caller = extract("pkg/checks.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/checks.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let base_check = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.base.Base::check")
        .unwrap_or_else(|| panic!("base method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == base_check.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L5");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
}

#[test]
fn python_rebound_local_class_receiver_does_not_invent_inherited_dispatch() {
    let source = b"class Base:\n    @classmethod\n    def check(cls):\n        return []\ndef verify(replacement):\n    class Model(Base):\n        pass\n    Model = replacement\n    return Model.check()\n";
    let extracted = extract("pkg/checks.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/checks.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L9"
    }));
}

#[test]
fn python_super_call_uses_complete_c3_order_across_multiple_bases() {
    let source = b"class Left:\n    pass\nclass Right:\n    def run(self):\n        return None\nclass Child(Left, Right):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let right_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Right::run")
        .unwrap_or_else(|| panic!("right method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == right_run.id
            && edge.string("source_location") == "L8"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
    }));
}

#[test]
fn python_super_call_marks_a_later_base_behind_unknown_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Known:\n    def run(self):\n        return None\nclass Child(Unknown, Known):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    let known_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Known::run")
        .unwrap_or_else(|| panic!("known method; nodes={:#?}", resolved.nodes));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == known_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_bound_method_receiver_resolves_exact_class_method() {
    let source = b"class Model:\n    def check(self):\n        return None\n\n    def verify(self):\n        return self.check()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let check = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Model::check")
        .unwrap_or_else(|| panic!("check method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == check.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L6");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
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
fn python_bound_receiver_reaches_a_later_leaf_separated_base() {
    let source = b"from external import Unknown\nclass Leaf:\n    pass\nclass Branch(Unknown):\n    def run(self):\n        return None\nclass Child(Leaf, Branch):\n    def call(self):\n        return self.run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let branch_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Branch::run")
        .unwrap_or_else(|| panic!("branch method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == branch_run.id
            && edge.string("source_location") == "L9"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
    }));
}

#[test]
fn python_bound_receiver_marks_a_later_base_behind_unknown_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Known:\n    def run(self):\n        return None\nclass Child(Unknown, Known):\n    def call(self):\n        return self.run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    let known_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Known::run")
        .unwrap_or_else(|| panic!("known method; nodes={:#?}", resolved.nodes));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == known_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_discovers_closed_world_descendant_dispatch() {
    let source = b"class Provider:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Concrete(Provider, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L6"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_marks_a_first_base_target_with_external_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Provider(Unknown):\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Concrete(Provider, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_preserves_every_proven_descendant_target() {
    let source = b"class First:\n    def run(self):\n        return None\nclass Second:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass UsesFirst(First, Mixin):\n    pass\nclass UsesSecond(Second, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let targets = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_location") == "L9"
                && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
        })
        .map(|edge| edge.target.as_str())
        .collect::<BTreeSet<_>>();
    let expected = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "pkg.models.First::run" | "pkg.models.Second::run"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(targets, expected);
    assert!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                targets.contains(edge.target.as_str())
                    && edge.string("source_location") == "L9"
                    && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
            })
            .all(|edge| edge.string("confidence") == "INFERRED")
    );
}

#[test]
fn python_possible_dispatch_fails_closed_when_descendants_exceed_the_lookup_bound() {
    let source = b"class First:\n    def run(self):\n        return None\nclass Second:\n    def run(self):\n        return None\nclass Third:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass UsesFirst(First, Mixin):\n    pass\nclass UsesSecond(Second, Mixin):\n    pass\nclass UsesThird(Third, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let limits = UniversalResolutionLimits {
        candidates_per_lookup: 2,
        ..UniversalResolutionLimits::default()
    };
    let index = UniversalResolutionIndex::new(&evidence, limits).expect("bounded index");
    let mut nodes = extracted.nodes.clone();
    let mut edges = extracted.edges.clone();
    index.materialize(&mut nodes, &mut edges);

    assert!(edges.iter().all(|edge| {
        !matches!(
            edge.string("resolution_rule").as_str(),
            "closed-world-receiver-dispatch" | "incomplete-hierarchy-receiver-dispatch"
        )
    }));
}

#[test]
fn python_bound_receiver_keeps_exact_target_and_marks_subclass_override_as_possible() {
    let source = b"class Base:\n    def run(self):\n        return None\n    def call(self):\n        return self.run()\nclass Child(Base):\n    def run(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L5")
        .map(|edge| {
            (
                resolved
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .map(|node| node.string("qualified_name"))
                    .expect("call target"),
                edge.string("confidence"),
                edge.string("resolution_rule"),
            )
        })
        .collect::<BTreeSet<_>>();

    assert!(calls.contains(&(
        "pkg.models.Base::run".to_owned(),
        "EXTRACTED".to_owned(),
        "linearized-receiver-dispatch".to_owned(),
    )));
    assert!(calls.contains(&(
        "pkg.models.Child::run".to_owned(),
        "INFERRED".to_owned(),
        "closed-world-receiver-dispatch".to_owned(),
    )));
}

#[test]
fn python_mixin_receiver_does_not_match_an_unrelated_same_name_member() {
    let source = b"class Mixin:\n    def call(self):\n        return self.run()\nclass Unrelated:\n    def run(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_location") == "L3"
    }));
}

#[test]
fn python_mixin_receiver_rejects_an_inconsistent_descendant_c3() {
    let source = b"class X:\n    pass\nclass Y:\n    pass\nclass A(X, Y):\n    def run(self):\n        return None\nclass B(Y, X):\n    pass\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Broken(A, B, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_location") == "L12"
    }));
}

#[test]
fn python_mixin_super_discovers_the_member_after_it_in_descendant_c3() {
    let source = b"class Mixin:\n    def call(self):\n        return super().run()\nclass Provider:\n    def run(self):\n        return None\nclass Concrete(Mixin, Provider):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L3"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
    }));
}

#[test]
fn python_bound_receiver_resolves_a_source_proven_class_callable_alias() {
    let source = b"def helper():\n    return None\n\nclass UsesHelper:\n    helper_alias = helper\n\n    def run(self):\n        return self.helper_alias()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let helper = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.helper")
        .unwrap_or_else(|| panic!("helper function; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == helper.id
            && edge.string("source_location") == "L8"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
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
            && edge.string("relation") == "references"
            && edge.string("context") == "return"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == callback.id
            && edge.string("relation") == "indirect_call"
            && edge.string("context") == "return")
    }));
}

#[test]
fn python_callable_values_are_references_without_invocation_evidence() {
    let provider = extract(
        "pkg/provider.py",
        br#"class ValidationError(Exception):
    pass

def callback():
    return None
"#,
    );
    let consumer = extract(
        "app.py",
        br#"from pkg.provider import ValidationError, callback
from outside import unknown

def register(consume):
    consume(ValidationError)
    consume(callback)
    consume(unknown)
"#,
    );
    let resolved = compass_resolve::resolve(&[provider, consumer], &HashMap::new());
    let validation_error = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.provider.ValidationError")
        .expect("ValidationError class");
    let callback = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.provider.callback")
        .expect("callback function");
    let unknown = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "outside.unknown")
        .expect("external unknown reference");

    let validation_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == validation_error.id)
        .collect::<Vec<_>>();
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == validation_error.id
                && edge.string("relation") == "references"
                && edge.string("source_file") == "app.py"
                && edge.string("source_location") == "L5"
        }),
        "validation edges: {validation_edges:#?}"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.string("relation") == "indirect_call"
            && resolved
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.string("label") == "ValidationError"))
    }));
    let callback_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == callback.id)
        .collect::<Vec<_>>();
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == callback.id
                && edge.string("relation") == "references"
                && edge.string("context") == "argument"
        }),
        "callback edges: {callback_edges:#?}"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == callback.id && edge.string("relation") == "indirect_call")
    }));
    assert_eq!(unknown.string("symbol_kind"), "variable");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == unknown.id
            && edge.string("relation") == "references"
            && edge.string("context") == "argument"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == unknown.id && edge.string("relation") == "indirect_call")
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
fn python_recursive_calls_preserve_exact_self_loops_and_occurrences() {
    let source = b"def recurse(value):\n    if value:\n        return recurse(value - 1)\n    return recurse(0)\n\nclass Walker:\n    def walk(self, value):\n        if value:\n            return self.walk(value - 1)\n        return None\n";
    let extracted = extract("pkg/recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let recurse = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.recursive.recurse")
        .unwrap_or_else(|| panic!("recursive function; nodes={:#?}", resolved.nodes));
    let walk = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.recursive.Walker::walk")
        .unwrap_or_else(|| panic!("recursive method; nodes={:#?}", resolved.nodes));

    let recursive_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.source == recurse.id
                && edge.target == recurse.id
                && edge.string("relation") == "calls"
        })
        .collect::<Vec<_>>();
    assert_eq!(recursive_calls.len(), 2);
    assert_eq!(
        recursive_calls
            .iter()
            .map(|edge| edge.string("source_location"))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["L3".to_owned(), "L4".to_owned()])
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == walk.id
            && edge.target == walk.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L9"
    }));

    let published = build_from_extraction(&resolved, true, None);
    assert_eq!(
        published
            .links
            .iter()
            .filter(|edge| {
                edge.source == recurse.id
                    && edge.target == recurse.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        2,
        "recursive occurrences must survive graph endpoint normalization"
    );
}

#[test]
fn python_shadowed_or_unknown_receivers_never_invent_recursive_calls() {
    let source = b"def parameter_shadow(parameter_shadow):\n    return parameter_shadow()\n\ndef local_shadow():\n    local_shadow = callback\n    return local_shadow()\n\ndef closure_shadow(closure_shadow):\n    def inner():\n        return closure_shadow()\n    return inner()\n\ndef annotated_shadow():\n    annotated_shadow: object\n    return annotated_shadow()\n\ndef augmented_shadow():\n    augmented_shadow += callback\n    return augmented_shadow()\n\nclass Walker:\n    def walk_other(self, other):\n        return other.walk_other()\n";
    let extracted = extract("pkg/not_recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/not_recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let declarations = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "pkg.not_recursive.parameter_shadow"
                    | "pkg.not_recursive.local_shadow"
                    | "pkg.not_recursive.closure_shadow"
                    | "pkg.not_recursive.annotated_shadow"
                    | "pkg.not_recursive.augmented_shadow"
                    | "pkg.not_recursive.Walker::walk_other"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();

    let false_shadow_edges = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && declarations.contains(edge.target.as_str())
        })
        .map(|edge| edge.string("source_location"))
        .collect::<Vec<_>>();
    assert!(
        false_shadow_edges.is_empty(),
        "false calls through statically shadowed names: {false_shadow_edges:?}"
    );
}

#[test]
fn python_global_directive_preserves_module_recursive_call() {
    let source = b"def global_recurse():\n    global global_recurse\n    return global_recurse()\n";
    let extracted = extract("pkg/global_recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/global_recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let recurse = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.global_recursive.global_recurse")
        .unwrap_or_else(|| panic!("global recursive function; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == recurse.id
            && edge.target == recurse.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
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
fn go_variadic_ranges_and_nested_closures_resolve_element_receivers() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
type Command struct{}
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(workers ...*Worker) {
    for _, worker := range workers {
        worker.Run()
    }
    visit := func(command *Command) {
        for _, subCommand := range command.Commands() {
            if subCommand.IsAvailableCommand() {
                _ = subCommand.Name()
            }
        }
    }
    _ = visit
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
    let command_available = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Command::IsAvailableCommand")
        .expect("Command.IsAvailableCommand declaration");
    let command_name = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Command::Name")
        .expect("Command.Name declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L10"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == command_available.id
            && edge.string("source_location") == "L14"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == command_name.id
            && edge.string("source_location") == "L15"
    }));
}

#[test]
fn go_top_level_range_variable_preserves_method_attribution() {
    let go_source = br#"package pkg
type Command struct{}
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(command *Command) {
    for _, c := range command.Commands() {
        if !c.IsAvailableCommand() {
            continue
        }
        _ = c.Name()
    }
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods.into_iter().zip(["L7", "L8", "L11"]) {
        assert!(resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == target
                && edge.string("source_location") == location
        }));
    }
}

#[test]
fn go_multi_return_range_inside_closure_preserves_element_method_attribution() {
    let go_source = br#"package pkg
type Command struct{}
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(command *Command) {
    visit := func() {
        cmd, _, _ := command.Find(nil)
        for _, subCommand := range cmd.Commands() {
            if subCommand.IsAvailableCommand() {
                _ = subCommand.Name()
            }
        }
    }
    _ = visit
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Find", "Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods.into_iter().zip(["L9", "L10", "L11", "L12"]) {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target
                    && edge.string("source_location") == location
            }),
            "missing Go call at {location}"
        );
    }
}

#[test]
fn go_cobra_shape_preserves_nested_closure_and_multi_return_method_attribution() {
    let go_source = br#"package pkg
type ShellCompDirective int
type Command struct { helpCommand *Command }
func (*Command) Root() *Command { return nil }
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func (c *Command) initDefaultHelpCmd() {
    c.helpCommand = &Command{}
    c.helpCommand.ValidArgsFunction = func(cmd *Command, args []string, toComplete string) ([]string, ShellCompDirective) {
        cmd, _, _ := c.Root().Find(args)
        for _, subCmd := range cmd.Commands() {
            if subCmd.IsAvailableCommand() || subCmd == cmd.helpCommand {
                _ = subCmd.Name()
            }
        }
        return nil, 0
    }
}
"#;
    let extracted = extract("pkg/command.go", go_source);
    let sources = HashMap::from([(
        "pkg/command.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Root", "Find", "Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods
        .into_iter()
        .zip(["L12", "L12", "L13", "L14", "L15"])
    {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target
                    && edge.string("source_location") == location
            }),
            "missing Cobra-shaped Go call at {location}"
        );
    }
}

#[test]
fn go_cobra_exact_find_guard_preserves_range_element_method_attribution() {
    let go_source = br#"package pkg
import "strings"
type ShellCompDirective int
type Completion struct{}
type Command struct {
    helpCommand *Command
    ValidArgsFunction func(*Command, []string, string) ([]Completion, ShellCompDirective)
    Short string
}
func (*Command) Root() *Command { return nil }
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func (*Command) HasSubCommands() bool { return true }
func CompletionWithDesc(choice string, description string) Completion { return Completion{} }
func (c *Command) initDefaultHelpCmd() {
    if !c.HasSubCommands() { return }
    if c.helpCommand == nil {
        c.helpCommand = &Command{
            ValidArgsFunction: func(c *Command, args []string, toComplete string) ([]Completion, ShellCompDirective) {
                var completions []Completion
                cmd, _, e := c.Root().Find(args)
                if e != nil { return nil, 0 }
                if cmd == nil { cmd = c.Root() }
                for _, subCmd := range cmd.Commands() {
                    if subCmd.IsAvailableCommand() || subCmd == cmd.helpCommand {
                        if strings.HasPrefix(subCmd.Name(), toComplete) {
                            completions = append(completions, CompletionWithDesc(subCmd.Name(), subCmd.Short))
                        }
                    }
                }
                return completions, 0
            },
        }
    }
}
"#;
    let extracted = extract("pkg/command.go", go_source);
    let sources = HashMap::from([(
        "pkg/command.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    for method in ["Root", "Find", "Commands", "IsAvailableCommand", "Name"] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
            .unwrap_or_else(|| panic!("Command.{method} declaration"))
            .id
            .clone();
        assert!(
            resolved
                .edges
                .iter()
                .any(|edge| { edge.string("relation") == "calls" && edge.target == target }),
            "missing Cobra exact-shape Go call to {method}"
        );
    }
}

#[test]
fn go_for_clause_initializers_and_empty_closures_keep_exact_attribution() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
func (worker *Worker) Parent() *Worker { return nil }
func caller(worker *Worker) {
    for current := worker; current != nil; current = current.Parent() {
        current.Run()
    }
    visit := func() {
        worker.Run()
    }
    _ = visit
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .expect("Go universal evidence");
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
    let worker_parent = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Parent")
        .expect("Worker.Parent declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_parent.id
            && edge.string("source_location") == "L6"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L7"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L10"
    }));

    let closure_scope_ids = evidence
        .scopes
        .iter()
        .filter(|scope| scope.kind == "closure")
        .map(|scope| scope.id.as_str())
        .collect::<HashSet<_>>();
    let empty_closure_call = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.range.start_line == 10 && occurrence.spelling == "Run")
        .expect("empty closure call occurrence");
    assert!(
        empty_closure_call
            .scope_id
            .as_deref()
            .is_some_and(|scope_id| closure_scope_ids.contains(scope_id)),
        "empty closures must own their call occurrences"
    );
}

#[test]
fn go_variadic_declarations_use_source_arity_for_calls() {
    let go_source = br#"package pkg
type Worker struct{}
func fanout(workers ...*Worker) {}
func caller(worker *Worker) {
    fanout(worker, worker)
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .expect("Go universal evidence");
    let fanout = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "fanout")
        .expect("variadic declaration");
    assert_eq!(fanout.parameter_count, Some(1));
    assert!(fanout.variadic);

    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let fanout_node = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.fanout")
        .expect("fanout node");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == fanout_node.id
            && edge.string("source_location") == "L5"
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
        assert_eq!(external.string("symbol_kind"), "type_alias");
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
fn rust_external_generic_bounds_materialize_as_interfaces() {
    let source = b"fn execute<T: Send + external::Ready>(value: T) {}\n";
    let extracted = extract("src/lib.rs", source);
    let sources = HashMap::from([(
        "src/lib.rs".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let parameter = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "parameter"
                && node.string("qualified_name") == "crate::execute::<T>"
        })
        .unwrap_or_else(|| panic!("generic parameter; nodes={:#?}", resolved.nodes));

    let bound = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "external::Ready"
                && node
                    .attributes
                    .get("external")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("external bound; nodes={:#?}", resolved.nodes));
    assert_eq!(bound.string("symbol_kind"), "interface");
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == parameter.id
            && edge.target == bound.id
            && edge.string("relation") == "references"
    }));
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
fn rust_ambiguous_local_trait_self_call_does_not_invent_an_external_method() {
    let source = br#"trait Fill<T> { fn fill<I>(&mut self, values: I); }
impl Fill<char> for String {
    fn fill<I>(&mut self, _values: I) { self.push_str(""); }
}
impl<'a> Fill<&'a char> for String {
    fn fill<I>(&mut self, values: I) {
        self.fill(values.convert());
    }
}
impl Fill<()> for () {
    fn fill<I>(&mut self, _values: I) {}
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
    let local_fill_methods = resolved
        .nodes
        .iter()
        .filter(|node| {
            node.string("symbol_kind") == "method"
                && node.string("qualified_name").ends_with("::fill")
        })
        .count();
    assert_eq!(local_fill_methods, 4, "fixture must retain every overload");
    let external_push = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "String::push_str")
        .expect("external inherent String.push_str method");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == external_push.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
            && edge.string("resolution_rule") == "qualified-external"
    }));
    assert!(
        resolved
            .nodes
            .iter()
            .all(|node| node.string("qualified_name") != "String::fill"),
        "an ambiguous local method must not become any placeholder"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.string("relation") == "calls"
            && edge.string("source_location") == "L7"
            && edge.string("resolution_rule") == "qualified-external")
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
fn rust_module_path_uses_import_namespace_over_same_named_value() {
    let source = br#"use std::io;
use std::thread;
struct ThreadBuilder;
impl ThreadBuilder {
    fn name(&self) -> Option<&str> { None }
    fn run(self) {}
}
trait ThreadSpawn { fn spawn(&mut self, thread: ThreadBuilder) -> io::Result<()>; }
struct DefaultSpawn;
impl ThreadSpawn for DefaultSpawn {
    fn spawn(&mut self, thread: ThreadBuilder) -> io::Result<()> {
        let builder = thread::Builder::new();
        if let Some(name) = thread.name() { let _ = name; }
        builder.spawn(|| thread.run())?;
        Ok(())
    }
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
    let constructor = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "std::thread::Builder::new")
        .unwrap_or_else(|| panic!("external Builder.new target; nodes={:#?}", resolved.nodes));
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == constructor.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "qualified-external"
            && edge.string("source_location") == "L12"
    }));
}

#[test]
fn rust_cross_file_generic_receiver_parameters_resolve_through_imported_impls() {
    let provider = extract(
        "src/api.rs",
        b"pub trait Render<T> { fn render(&self, value: T); }\npub struct Container<T>(T);\nimpl<T> Render<T> for Container<T> { fn render(&self, _value: T) {} }\n",
    );
    let caller_source =
        b"use crate::api::Container;\nfn invoke(container: Container<u32>) {\n    container.render(1);\n}\n";
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let render_method = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "<crate::api::Container as crate::api::Render>::render"
        })
        .expect("cross-file generic Render implementation method");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == render_method.id
            && edge.string("source_location") == "L3"
    }));
}

#[test]
fn rust_chained_associated_function_result_resolves_trait_method() {
    let provider_source = br#"pub trait Drain { fn par_drain(self); }
pub struct Guard;
impl Guard { pub fn new() -> Self { Self } }
impl Drain for &mut Guard { fn par_drain(self) {} }
"#;
    let caller_source = br#"use crate::api::{Drain, Guard};
fn run() { Guard::new().par_drain(); }
"#;
    let provider = extract("src/api.rs", provider_source);
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/api.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let par_drain = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "<crate::api::Guard as crate::api::Drain>::par_drain"
        })
        .expect("Guard Drain.par_drain implementation");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == par_drain.id
                && edge.string("source_location") == "L2"
                && edge.string("confidence") == "EXTRACTED"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_chained_method_result_resolves_source_proven_member() {
    let source = br#"struct Input;
struct Output;
impl Input { fn transform(self) -> Output { Output } }
impl Output { fn finish(self) {} }
fn run(input: Input) { input.transform().finish(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let finish = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Output::finish")
        .expect("Output.finish method");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == finish.id
                && edge.string("source_location") == "L5"
                && edge.string("confidence") == "EXTRACTED"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_unresolved_prelude_return_does_not_invent_a_local_result_receiver() {
    let source = br#"struct Builder;
struct Pool;
struct Error;
impl Builder { fn build(self) -> Result<Pool, Error> { todo!() } }
fn run(builder: Builder) { builder.build().unwrap(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "crate::Result::unwrap"
            && node.string("qualified_name") != "crate::Result"
    }));
    assert!(
        resolved
            .nodes
            .iter()
            .any(|node| node.string("qualified_name") == "std::result::Result::unwrap")
    );
}

#[test]
fn rust_source_local_result_return_still_resolves_its_exact_member() {
    let source = br#"struct ResultValue;
impl ResultValue { fn unwrap(self) {} }
struct Builder;
impl Builder { fn build(self) -> ResultValue { ResultValue } }
fn run(builder: Builder) { builder.build().unwrap(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let unwrap = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::ResultValue::unwrap")
        .expect("ResultValue.unwrap method");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == unwrap.id
            && edge.string("source_location") == "L5"
            && edge.string("confidence") == "EXTRACTED"
    }));
}

#[test]
fn rust_imported_external_return_preserves_its_qualified_member() {
    let source = br#"use external::Handle;
struct Builder;
impl Builder { fn build(self) -> Handle { todo!() } }
fn run(builder: Builder) { builder.build().close(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let close = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "external::Handle::close")
        .expect("qualified external Handle.close placeholder");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == close.id
            && edge.string("source_location") == "L4"
            && edge.string("confidence") == "INFERRED"
    }));
}

#[test]
fn rust_unresolved_prelude_field_does_not_invent_a_local_option_receiver() {
    let source = br#"struct Holder { value: Option<u32> }
impl Holder { fn run(&mut self) { self.value.take(); } }
"#;
    let extracted = extract("src/lib.rs", source);
    let take_candidate = extracted
        .semantic_evidence
        .as_ref()
        .and_then(|evidence| {
            evidence
                .candidates
                .iter()
                .find(|candidate| candidate.target_spelling == "take")
        })
        .expect("self.value.take candidate");
    assert_eq!(
        take_candidate.constraints.qualified_name.as_deref(),
        Some("std::option::Option::take")
    );
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "crate::Option::take"
            && node.string("qualified_name") != "crate::Option"
    }));
    let take_targets = resolved
        .nodes
        .iter()
        .map(|node| node.string("qualified_name"))
        .filter(|qualified| qualified.ends_with("::take"))
        .collect::<Vec<_>>();
    assert!(
        take_targets
            .iter()
            .any(|target| target == "std::option::Option::take"),
        "unexpected take targets: {take_targets:?}"
    );
}

#[test]
fn rust_source_local_field_type_still_resolves_its_exact_member() {
    let source = br#"struct Value;
impl Value { fn take(&mut self) {} }
struct Holder { value: Value }
impl Holder { fn run(&mut self) { self.value.take(); } }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let take = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Value::take")
        .expect("Value.take method");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == take.id
            && edge.string("source_location") == "L4"
            && edge.string("confidence") == "EXTRACTED"
    }));
}

#[test]
fn rust_chained_generic_method_result_uses_the_outer_nominal_type() {
    let source = br#"struct DefaultSpawn;
struct CustomSpawn<F>(F);
struct Builder<S = DefaultSpawn>(S);
impl<S> Builder<S> { fn build(self) {} }
impl Builder<DefaultSpawn> {
    fn spawn_handler<F>(self, spawn: F) -> Builder<CustomSpawn<F>> {
        Builder(CustomSpawn(spawn))
    }
}
fn run(builder: Builder) { builder.spawn_handler(|| {}).build(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let build = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Builder::build")
        .expect("Builder.build method");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == build.id
                && edge.string("source_location") == "L10"
                && edge.string("confidence") == "EXTRACTED"
        }),
        "edges={:#?}",
        resolved.edges
    );
    assert!(resolved.nodes.iter().all(|node| {
        !node
            .string("qualified_name")
            .contains("spawn_handler(|| {})::build")
    }));
}

#[test]
fn rust_nested_cross_file_method_results_resolve_each_source_proven_stage() {
    let provider_source = br#"pub struct Start;
pub struct Split;
pub struct Filtered;
impl Start { pub fn split(self) -> Split { Split } }
impl Split { pub fn filter(self) -> Filtered { Filtered } }
impl Filtered { pub fn finish(self) {} }
"#;
    let caller_source = br#"use crate::pipeline::Start;
fn run(start: Start) { start.split().filter().finish(); }
"#;
    let provider = extract("src/pipeline.rs", provider_source);
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/pipeline.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    for qualified_name in [
        "crate::pipeline::Start::split",
        "crate::pipeline::Split::filter",
        "crate::pipeline::Filtered::finish",
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .unwrap_or_else(|| panic!("missing {qualified_name}"));
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target.id
                    && edge.string("source_location") == "L2"
                    && edge.string("confidence") == "EXTRACTED"
            }),
            "missing exact call to {qualified_name}: {:#?}",
            resolved.edges
        );
    }
    assert!(resolved.nodes.iter().all(|node| {
        let qualified = node.string("qualified_name");
        !qualified.contains("split()::filter") && !qualified.contains("filter()::finish")
    }));
}

#[test]
fn rust_nested_trait_method_results_resolve_tuple_field_chains() {
    let source = br#"trait ParallelIterator {
    fn filter(self) -> Filter { Filter }
    fn drive_unindexed(self) {}
}
trait ParallelString { fn par_split(&self) -> Split { Split } }
impl ParallelString for str {}
struct Split;
impl ParallelIterator for Split {}
struct Filter;
impl ParallelIterator for Filter { fn drive_unindexed(self) {} }
struct SplitWhitespace<'a>(&'a str);
impl<'a> SplitWhitespace<'a> {
    fn drive(self) { self.0.par_split().filter().drive_unindexed(); }
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
    for qualified_name in [
        "crate::ParallelString::par_split",
        "crate::ParallelIterator::filter",
        "<crate::Filter as crate::ParallelIterator>::drive_unindexed",
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .unwrap_or_else(|| panic!("missing {qualified_name}: {:#?}", resolved.nodes));
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target.id
                    && edge.string("source_location") == "L13"
                    && edge.string("confidence") == "EXTRACTED"
            }),
            "missing exact call to {qualified_name}: {:#?}",
            resolved.edges
        );
    }
}

#[test]
fn rust_nested_call_result_preserves_a_typed_receiver_fallback() {
    let source = br#"struct Container;
impl Container { fn into_return_value(self) {} }
fn run(value: Container) { value.into_inner().into_return_value(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let into_return_value = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Container::into_return_value")
        .expect("Container.into_return_value method");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == into_return_value.id
                && edge.string("source_location") == "L3"
                && edge.string("confidence") == "EXTRACTED"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_chained_call_result_follows_a_unique_aliased_owner_prefix() {
    let provider_source = br#"mod guard {
    pub trait Drain { fn par_drain(self); }
    pub struct Guard;
    impl Guard { pub fn new() -> Self { Self } }
    impl Drain for Guard { fn par_drain(self) {} }
}
use self::guard::Guard;
"#;
    let caller_source = b"fn run() { super::Guard::new().par_drain(); }\n";
    let provider = extract("src/api/mod.rs", provider_source);
    let caller = extract("src/api/caller.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/api/mod.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/api/caller.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let par_drain = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name")
                == "<crate::api::guard::Guard as crate::api::guard::Drain>::par_drain"
        })
        .expect("aliased Guard Drain.par_drain implementation");
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == par_drain.id
                && edge.string("source_location") == "L1"
        }),
        "edges={:#?}",
        resolved.edges
    );
}

#[test]
fn rust_chained_call_result_with_ambiguous_members_fails_closed() {
    let source = br#"trait First { fn run(self); }
trait Second { fn run(self); }
struct Guard;
impl Guard { fn new() -> Self { Self } }
impl First for Guard { fn run(self) {} }
impl Second for Guard { fn run(self) {} }
fn invoke() { Guard::new().run(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let ambiguous_runs = resolved
        .nodes
        .iter()
        .filter(|node| {
            node.string("symbol_kind") == "method"
                && node.string("qualified_name").ends_with("::run")
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(ambiguous_runs.len(), 4);
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.string("relation") == "calls"
            && edge.string("source_location") == "L7"
            && ambiguous_runs.contains(edge.target.as_str()))
    }));
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "crate::Guard::run"
            && node.string("qualified_name") != "crate::Guard::new::run"
    }));
}

#[test]
fn rust_chained_call_result_with_ambiguous_trait_defaults_fails_closed() {
    let source = br#"trait First { fn finish(self) {} }
trait Second { fn finish(self) {} }
struct ResultValue;
struct Start;
impl Start { fn make(self) -> ResultValue { ResultValue } }
impl First for ResultValue {}
impl Second for ResultValue {}
fn invoke(start: Start) { start.make().finish(); }
"#;
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let ambiguous_finishes = resolved
        .nodes
        .iter()
        .filter(|node| {
            node.string("symbol_kind") == "method"
                && node.string("qualified_name").ends_with("::finish")
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(ambiguous_finishes.len(), 2);
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.string("relation") == "calls"
            && edge.string("source_location") == "L8"
            && ambiguous_finishes.contains(edge.target.as_str()))
    }));
    assert!(
        resolved
            .nodes
            .iter()
            .all(|node| node.string("qualified_name") != "crate::ResultValue::finish")
    );
}

#[test]
fn rust_typed_receivers_resolve_parameters_and_local_values_exactly() {
    let provider_source = b"pub struct Client;\nimpl Client { pub fn send(&self) {} }\n";
    let caller_source = b"use crate::api::Client;\nfn run(client: &Client) { let local: Client = Client; client.send(); local.send(); }\n";
    let provider = extract("src/api.rs", provider_source);
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/api.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::run")
        .expect("run declaration");
    let send = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Client::send")
        .expect("Client.send declaration");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.source == run.id && edge.target == send.id && edge.string("relation") == "calls"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls.len(),
        2,
        "typed receivers should share one exact endpoint"
    );
    assert!(calls.iter().all(|edge| {
        edge.string("resolution_rule") != "deferred-receiver"
            && edge.string("confidence") == "EXTRACTED"
    }));
}

#[test]
fn rust_typed_field_receivers_resolve_without_flattening_to_the_outer_type() {
    let source = b"pub struct Transport;
impl Transport { pub fn send(&self) {} }
pub struct Client { transport: Transport }
pub struct Holder { client: Client }
impl Holder { pub fn run(&self) { self.client.transport.send(); } }
";
    let extracted = extract("src/lib.rs", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "src/lib.rs".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Holder::run")
        .expect("Holder.run declaration");
    let send = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Transport::send")
        .expect("Transport.send declaration");
    let call = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.source == run.id && edge.target == send.id && edge.string("relation") == "calls"
        })
        .expect("self.client.send call");
    assert_ne!(call.string("resolution_rule"), "deferred-receiver");
    assert_eq!(call.string("confidence"), "EXTRACTED");
}

#[test]
fn rust_typed_receivers_respect_nested_shadowing() {
    let provider_source = b"pub struct Client;
impl Client { pub fn send(&self) {} }
";
    let caller_source = b"use crate::api::Client;
fn run(client: &Client) {
    { let client: Unknown = Unknown; client.send(); }
    client.send();
}
";
    let provider = extract("src/api.rs", provider_source);
    let caller = extract("src/lib.rs", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([
            (
                "src/api.rs".to_owned(),
                String::from_utf8(provider_source.to_vec()).expect("provider source"),
            ),
            (
                "src/lib.rs".to_owned(),
                String::from_utf8(caller_source.to_vec()).expect("caller source"),
            ),
        ]),
    );
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::run")
        .expect("run declaration");
    let send = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Client::send")
        .expect("Client.send declaration");
    let exact_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.source == run.id
                && edge.target == send.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") != "deferred-receiver"
        })
        .collect::<Vec<_>>();
    assert_eq!(exact_calls.len(), 1, "only the outer binding is exact");
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == run.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "deferred-receiver"
    }));
}

#[test]
fn rust_unresolved_lexical_receiver_is_deferred_without_becoming_external() {
    let source = b"fn run(world: Unknown) {\n    world.spawn();\n}\n";
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
        .find(|node| node.string("qualified_name") == "crate::Unknown::spawn")
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
fn javascript_package_exports_choose_import_and_require_conditions()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let package = directory.path().join("packages/conditional/package.json");
    let import_target = directory.path().join("packages/conditional/src/import.ts");
    let require_target = directory
        .path()
        .join("packages/conditional/src/require.cjs");
    let wildcard_target = directory
        .path()
        .join("packages/conditional/src/features/button.ts");
    let typescript_consumer = directory.path().join("app/consumer.ts");
    let javascript_consumer = directory.path().join("app/consumer.cjs");
    let package_source = br##"{
        "name": "@example/conditional",
        "exports": {
            ".": {
                "import": "./src/import.ts",
                "require": "./src/require.cjs",
                "default": "./src/fallback.js"
            },
            "./features/*": {
                "import": "./src/features/*.ts"
            },
            "./fallback": ["./src/features/missing.ts", "./src/features/button.ts"]
        }
    }"##;
    let import_source = br#"export const imported = true;"#;
    let require_source = br#"module.exports = { required: true };"#;
    let wildcard_source = br#"export const button = true;"#;
    let typescript_source = br#"import { imported } from "@example/conditional";
import { button } from "@example/conditional/features/button";
import { button as fallback } from "@example/conditional/fallback";
export const value = imported && button && fallback;
"#;
    let javascript_source = br#"const { required } = require("@example/conditional");
module.exports = required;
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&import_target, import_source.as_slice()),
        (&require_target, require_source.as_slice()),
        (&wildcard_target, wildcard_source.as_slice()),
        (&typescript_consumer, typescript_source.as_slice()),
        (&javascript_consumer, javascript_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            import_target.to_str().ok_or("non-UTF-8 fixture path")?,
            import_source,
        ),
        extract(
            require_target.to_str().ok_or("non-UTF-8 fixture path")?,
            require_source,
        ),
        extract(
            wildcard_target.to_str().ok_or("non-UTF-8 fixture path")?,
            wildcard_source,
        ),
        extract(
            typescript_consumer
                .to_str()
                .ok_or("non-UTF-8 fixture path")?,
            typescript_source,
        ),
        extract(
            javascript_consumer
                .to_str()
                .ok_or("non-UTF-8 fixture path")?,
            javascript_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&import_target, import_source.as_slice()),
        (&require_target, require_source.as_slice()),
        (&wildcard_target, wildcard_source.as_slice()),
        (&typescript_consumer, typescript_source.as_slice()),
        (&javascript_consumer, javascript_source.as_slice()),
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
    let import_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == import_target.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing import condition target")?;
    let require_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == require_target.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing require condition target")?;
    let wildcard_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == wildcard_target.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing wildcard target")?;
    let module_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "imports_from")
        .collect::<Vec<_>>();
    assert!(module_edges.iter().any(|edge| {
        edge.string("source_file") == typescript_consumer.to_string_lossy()
            && edge.target == import_id
            && edge.string("package_condition") == "import"
            && edge.string("resolution_rule") == "package-exports"
    }));
    assert!(module_edges.iter().any(|edge| {
        edge.string("source_file") == typescript_consumer.to_string_lossy()
            && edge.target == wildcard_id
            && edge.string("package_condition") == "import"
    }));
    assert!(module_edges.iter().any(|edge| {
        edge.string("source_file") == typescript_consumer.to_string_lossy()
            && edge.string("module") == "@example/conditional/fallback"
            && edge.target == wildcard_id
            && edge.string("package_condition") == "default"
    }));
    assert!(
        module_edges.iter().any(|edge| {
            edge.string("source_file") == javascript_consumer.to_string_lossy()
                && edge.target == require_id
                && edge.string("context") == "require"
                && edge.string("package_condition") == "require"
        }),
        "module_edges={module_edges:#?}"
    );
    Ok(())
}

#[test]
fn typescript_paths_aliases_resolve_extension_substitution_and_named_symbols()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let config_source = br#"{
        // JSONC is accepted by TypeScript project configuration.
        "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@/*": ["./src/*",], },
        },
    }"#;
    let implementation_source = br#"export class Widget { run() {} }"#;
    let consumer_source = br#"import { Widget } from "@/api.js";
export function make() { return new Widget(); }
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let implementation_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == implementation.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing implementation file")?;
    let declaration_id = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Widget"
                && node.string("source_file") == implementation.to_string_lossy()
        })
        .map(|node| node.id.clone())
        .ok_or("missing Widget declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "@/api.js"
            && edge.target == implementation_id
            && edge.string("resolution_rule") == "typescript-paths"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports"
            && edge.target == declaration_id
            && edge.string("local_name") == "Widget"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == declaration_id
            && edge.string("source_file") == consumer.to_string_lossy()
    }));
    Ok(())
}

#[test]
fn javascript_jsconfig_base_url_resolves_bare_module() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("jsconfig.json");
    let implementation = root.join("src/api.js");
    let consumer = root.join("app/consumer.js");
    for path in [&config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let config_source = br#"{"compilerOptions":{"baseUrl":"."}}"#;
    let implementation_source = br#"export function api() { return true; }"#;
    let consumer_source = br#"import { api } from "src/api";
export const value = api();
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let implementation_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == implementation.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing JavaScript implementation file")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "src/api"
            && edge.target == implementation_id
            && edge.string("resolution_rule") == "typescript-base-url"
    }));
    Ok(())
}

#[test]
fn typescript_paths_choose_nearest_config_and_ordered_fallbacks()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let root_config = root.join("tsconfig.json");
    let nested_config = root.join("app/tsconfig.json");
    let first = root.join("app/src/first.ts");
    let second = root.join("app/src/second.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&root_config, &nested_config, &first, &second, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let root_config_source = br#"{
        "compilerOptions": { "paths": { "@/*": ["./wrong/*"] } }
    }"#;
    let nested_config_source = br#"{
        "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@/*": ["./missing/*", "./src/*"] }
        }
    }"#;
    let first_source = br#"export const first = true;"#;
    let second_source = br#"export const second = true;"#;
    let consumer_source = br#"import { second } from "@/second.js";
export const value = second;
"#;
    for (path, source) in [
        (&root_config, root_config_source.as_slice()),
        (&nested_config, nested_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            first.to_str().ok_or("non-UTF-8 fixture path")?,
            first_source,
        ),
        extract(
            second.to_str().ok_or("non-UTF-8 fixture path")?,
            second_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&root_config, root_config_source.as_slice()),
        (&nested_config, nested_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let second_file_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == second.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing second file")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "@/second.js"
            && edge.target == second_file_id
            && edge.string("resolution_config") == "app/tsconfig.json"
    }));
    Ok(())
}

#[test]
fn typescript_paths_leave_same_depth_config_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first_config = root.join("app/tsconfig.json");
    let second_config = root.join("app/tsconfig.alt.json");
    let first = root.join("app/one.ts");
    let second = root.join("app/two.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&first_config, &second_config, &first, &second, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let first_config_source = br#"{"compilerOptions":{"paths":{"@/shared":["./one.ts"]}}}"#;
    let second_config_source = br#"{"compilerOptions":{"paths":{"@/shared":["./two.ts"]}}}"#;
    let first_source = br#"export const one = true;"#;
    let second_source = br#"export const two = true;"#;
    let consumer_source = br#"import { one } from "@/shared";
export const value = one;
"#;
    for (path, source) in [
        (&first_config, first_config_source.as_slice()),
        (&second_config, second_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            first.to_str().ok_or("non-UTF-8 fixture path")?,
            first_source,
        ),
        extract(
            second.to_str().ok_or("non-UTF-8 fixture path")?,
            second_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&first_config, first_config_source.as_slice()),
        (&second_config, second_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let file_ids = resolved
        .nodes
        .iter()
        .filter(|node| {
            [first.to_string_lossy(), second.to_string_lossy()]
                .iter()
                .any(|source| node.string("source_file") == *source)
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@/shared"
        })
        .ok_or("missing ambiguous import")?;
    assert!(!file_ids.contains(import.target.as_str()));
    assert!(import.string("resolution_rule").is_empty());
    Ok(())
}

#[test]
fn typescript_config_extends_inherits_paths_and_project_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let base = root.join("tsconfig.base.json");
    let config = root.join("tsconfig.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&base, &config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let base_source = br#"{
        "compilerOptions": {
            "baseUrl": "..",
            "paths": { "@/*": ["src/*"] },
            "module": "NodeNext",
            "moduleResolution": "NodeNext"
        }
    }"#;
    let config_source = br#"{
        "extends": "./tsconfig.base.json",
        "compilerOptions": { "allowJs": true },
        "references": [{ "path": "./src" }]
    }"#;
    let implementation_source = br#"export class Widget {}"#;
    let consumer_source = br#"import { Widget } from "@/api.js";
export const value = new Widget();
"#;
    for (path, source) in [
        (&base, base_source.as_slice()),
        (&config, config_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&base, base_source.as_slice()),
        (&config, config_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let implementation_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == implementation.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing inherited implementation")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == consumer.to_string_lossy()
        })
        .ok_or("missing inherited alias import")?;
    assert_eq!(import.target, implementation_id);
    assert_eq!(import.string("resolution_rule"), "typescript-paths");
    assert_eq!(import.string("resolution_config"), "tsconfig.json");
    assert_eq!(import.string("module_resolution"), "nodenext");
    assert_eq!(import.string("module_kind"), "nodenext");
    assert_eq!(
        import.attributes.get("resolution_project_references"),
        Some(&serde_json::json!(["src"]))
    );
    Ok(())
}

#[test]
fn typescript_relative_imports_use_module_suffixes_and_root_dirs()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let consumer = root.join("src/app/consumer.ts");
    let suffixed = root.join("src/app/feature.ios.ts");
    let generated = root.join("generated/app/runtime.ts");
    let config_source = br#"{
        "compilerOptions": {
            "module": "ESNext",
            "moduleResolution": "Bundler",
            "moduleSuffixes": [".ios", ""],
            "rootDirs": ["src", "generated"]
        }
    }"#;
    let consumer_source = br#"import { feature } from "./feature.js";
import { runtime } from "./runtime.js";
export const value = feature + runtime;
"#;
    let suffixed_source = br#"export const feature = 1;"#;
    let generated_source = br#"export const runtime = 2;"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
        (&suffixed, suffixed_source.as_slice()),
        (&generated, generated_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            suffixed.to_str().ok_or("non-UTF-8 fixture path")?,
            suffixed_source,
        ),
        extract(
            generated.to_str().ok_or("non-UTF-8 fixture path")?,
            generated_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
        (&suffixed, suffixed_source.as_slice()),
        (&generated, generated_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let suffixed_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == suffixed.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing suffixed target")?;
    let generated_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == generated.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing rootDirs target")?;
    let imports = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == consumer.to_string_lossy()
        })
        .collect::<Vec<_>>();
    assert!(imports.iter().any(|edge| {
        edge.target == suffixed_id && edge.string("resolution_rule") == "typescript-relative"
    }));
    assert!(imports.iter().any(|edge| {
        edge.target == generated_id && edge.string("resolution_rule") == "typescript-root-dirs"
    }));
    assert!(imports.iter().all(|edge| {
        edge.string("module_resolution") == "bundler"
            && edge.string("module_kind") == "esnext"
            && edge.string("resolution_config") == "tsconfig.json"
    }));
    Ok(())
}

#[test]
fn javascript_relative_named_imports_repoint_to_the_unique_export()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let target = root.join("src/target.ts");
    let consumer = root.join("src/consumer.ts");
    let target_source = br#"export function greet() { return "hello"; }"#;
    let consumer_source = br#"import { greet } from "./target.js";
export function run() { return greet(); }
"#;
    for (path, source) in [
        (&target, target_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            target.to_str().ok_or("non-UTF-8 fixture path")?,
            target_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&target, target_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let greet = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "greet()" && node.string("source_file") == target.to_string_lossy()
        })
        .ok_or("missing greet declaration")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports"
                && edge.string("module") == "./target.js"
                && edge.string("local_name") == "greet"
        })
        .ok_or("missing relative named import")?;
    assert_eq!(import.target, greet.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == greet.id
            && edge.string("source_file") == consumer.to_string_lossy()
    }));
    Ok(())
}

#[test]
fn javascript_package_imports_use_the_nearest_package_and_type_condition()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/toolkit/package.json");
    let implementation = root.join("packages/toolkit/src/internal/tool.ts");
    let consumer = root.join("packages/toolkit/src/consumer.ts");
    let package_source = br##"{
  "name": "@example/toolkit",
  "imports": {
    "#internal/*": {
      "types": "./src/internal/*.ts",
      "default": "./src/internal/*.js"
    }
  }
}"##;
    let implementation_source = br#"export function tool() { return 1; }"#;
    let consumer_source = br##"import { tool } from "#internal/tool";
export const value = tool();
"##;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
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
    let sources = [
        (&package, package_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let implementation_id = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "tool()"
                && node.string("source_file") == implementation.to_string_lossy()
        })
        .map(|node| node.id.clone())
        .ok_or("missing package-import target")?;
    let module_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "#internal/tool"
        })
        .ok_or("missing package imports edge")?;
    assert_eq!(module_import.string("resolution_rule"), "package-imports");
    assert_eq!(module_import.string("package_condition"), "types");
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports"
                && edge.string("module") == "#internal/tool"
                && edge.string("local_name") == "tool"
        })
        .ok_or("missing named package import")?;
    assert_eq!(import.target, implementation_id);
    Ok(())
}

#[test]
fn typescript_package_types_versions_selects_the_admitted_declaration_target()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/typed/package.json");
    let declaration = root.join("packages/typed/types/index.ts");
    let consumer = root.join("app/consumer.ts");
    let package_source = br#"{
  "name": "@example/typed",
  "types": "./src/index.d.ts",
  "typesVersions": { "*": { "*": ["types/*"] } }
}"#;
    let declaration_source = br#"export function helper(): string { return "ok"; }"#;
    let consumer_source = br#"import { helper } from "@example/typed";
export const value = helper();
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            declaration.to_str().ok_or("non-UTF-8 fixture path")?,
            declaration_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let module_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@example/typed"
        })
        .ok_or("missing package import")?;
    assert_eq!(module_import.string("resolution_rule"), "typesVersions");
    let helper = resolved
        .nodes
        .iter()
        .find(|node| {
            matches!(node.label(), "helper()" | "helper")
                && node.string("source_file") == declaration.to_string_lossy()
        })
        .ok_or("missing typesVersions declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports"
            && edge.target == helper.id
            && edge.string("local_name") == "helper"
    }));
    Ok(())
}

#[test]
fn typescript_include_exclude_ownership_blocks_out_of_project_alias_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let allowed = root.join("src/allowed.ts");
    let excluded = root.join("src/excluded.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } },
  "include": ["src/**/*.ts"],
  "exclude": ["src/excluded.ts"]
}"#;
    let allowed_source = br#"export const allowed = true;"#;
    let excluded_source = br#"export const excluded = true;"#;
    let consumer_source = br#"import { allowed } from "@/allowed";
import { excluded } from "@/excluded";
export const value = allowed && excluded;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&allowed, allowed_source.as_slice()),
        (&excluded, excluded_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            allowed.to_str().ok_or("non-UTF-8 fixture path")?,
            allowed_source,
        ),
        extract(
            excluded.to_str().ok_or("non-UTF-8 fixture path")?,
            excluded_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&allowed, allowed_source.as_slice()),
        (&excluded, excluded_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let allowed_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == allowed.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing allowed target")?;
    let excluded_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == excluded.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing excluded target")?;
    let imports = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == consumer.to_string_lossy()
        })
        .collect::<Vec<_>>();
    assert!(imports.iter().any(|edge| {
        edge.string("module") == "@/allowed"
            && edge.target == allowed_id
            && edge.string("resolution_rule") == "typescript-paths"
    }));
    let excluded_import = imports
        .iter()
        .find(|edge| edge.string("module") == "@/excluded")
        .ok_or("missing excluded import")?;
    assert_ne!(excluded_import.target, excluded_id);
    assert!(excluded_import.string("resolution_rule").is_empty());
    Ok(())
}

#[test]
fn typescript_type_roots_resolve_admitted_declaration_packages()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let declaration = root.join("types/ambient/index.d.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "typeRoots": ["types"] }
}"#;
    let declaration_source = br#"export const ambient = true;"#;
    let consumer_source = br#"import { ambient } from "ambient";
export const value = ambient;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            declaration.to_str().ok_or("non-UTF-8 fixture path")?,
            declaration_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let declaration_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == declaration.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing typeRoots declaration")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "ambient"
        })
        .ok_or("missing typeRoots import")?;
    assert_eq!(import.target, declaration_id);
    assert_eq!(import.string("resolution_rule"), "typescript-type-roots");
    Ok(())
}

#[test]
fn typescript_custom_conditions_are_selected_before_default_package_exports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let package = root.join("packages/conditional/package.json");
    let browser = root.join("packages/conditional/browser.ts");
    let fallback = root.join("packages/conditional/fallback.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "customConditions": ["browser"] }
}"#;
    let package_source = br#"{
  "name": "@example/conditional",
  "exports": { ".": { "browser": "./browser.ts", "default": "./fallback.ts" } }
}"#;
    let browser_source = br#"export const selected = "browser";"#;
    let fallback_source = br#"export const selected = "fallback";"#;
    let consumer_source = br#"import { selected } from "@example/conditional";
export const value = selected;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            browser.to_str().ok_or("non-UTF-8 fixture path")?,
            browser_source,
        ),
        extract(
            fallback.to_str().ok_or("non-UTF-8 fixture path")?,
            fallback_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let browser_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == browser.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing custom-condition target")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("module") == "@example/conditional"
        })
        .ok_or("missing custom-condition import")?;
    assert_eq!(import.target, browser_id);
    assert_eq!(import.string("package_condition"), "browser");
    Ok(())
}

#[test]
fn package_export_condition_selection_preserves_manifest_key_order()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let package = root.join("packages/ordered/package.json");
    let browser = root.join("packages/ordered/browser.ts");
    let fallback = root.join("packages/ordered/fallback.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "customConditions": ["browser"] }
}"#;
    // `default` is intentionally before `browser`: conditional exports are
    // ordered by the manifest, so the active default branch wins and Compass
    // must not reorder keys according to its condition preference.
    let package_source = br#"{
  "name": "@example/ordered",
  "exports": { ".": { "default": "./fallback.ts", "browser": "./browser.ts" } }
}"#;
    let browser_source = br#"export const selected = "browser";"#;
    let fallback_source = br#"export const selected = "fallback";"#;
    let consumer_source = br#"import { selected } from "@example/ordered";
export const value = selected;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            browser.to_str().ok_or("non-UTF-8 fixture path")?,
            browser_source,
        ),
        extract(
            fallback.to_str().ok_or("non-UTF-8 fixture path")?,
            fallback_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let fallback_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == fallback.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing ordered fallback target")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@example/ordered"
        })
        .ok_or("missing ordered package import")?;
    assert_eq!(import.target, fallback_id);
    assert_eq!(import.string("package_condition"), "default");
    Ok(())
}

#[test]
fn javascript_package_resolution_mode_respects_node10_and_classic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/mode/package.json");
    let conditional = root.join("packages/mode/conditional.ts");
    let legacy = root.join("packages/mode/legacy.ts");
    let node10_config = root.join("node10/tsconfig.json");
    let node10_consumer = root.join("node10/consumer.ts");
    let classic_config = root.join("classic/tsconfig.json");
    let classic_consumer = root.join("classic/consumer.ts");
    let package_source = br#"{
  "name": "@example/mode",
  "exports": { ".": "./conditional.ts" },
  "main": "./legacy.ts"
}"#;
    let conditional_source = br#"export const selected = "conditional";"#;
    let legacy_source = br#"export const selected = "legacy";"#;
    let node10_config_source = br#"{
  "compilerOptions": { "moduleResolution": "node10" }
}"#;
    let node10_consumer_source = br#"import { selected } from "@example/mode";
export const value = selected;
"#;
    let classic_config_source = br#"{
  "compilerOptions": { "moduleResolution": "classic" }
}"#;
    let classic_consumer_source = br#"import { selected } from "@example/mode";
export const value = selected;
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&conditional, conditional_source.as_slice()),
        (&legacy, legacy_source.as_slice()),
        (&node10_config, node10_config_source.as_slice()),
        (&node10_consumer, node10_consumer_source.as_slice()),
        (&classic_config, classic_config_source.as_slice()),
        (&classic_consumer, classic_consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            conditional.to_str().ok_or("non-UTF-8 fixture path")?,
            conditional_source,
        ),
        extract(
            legacy.to_str().ok_or("non-UTF-8 fixture path")?,
            legacy_source,
        ),
        extract(
            node10_consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            node10_consumer_source,
        ),
        extract(
            classic_consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            classic_consumer_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&conditional, conditional_source.as_slice()),
        (&legacy, legacy_source.as_slice()),
        (&node10_config, node10_config_source.as_slice()),
        (&node10_consumer, node10_consumer_source.as_slice()),
        (&classic_config, classic_config_source.as_slice()),
        (&classic_consumer, classic_consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let legacy_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == legacy.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing legacy package target")?;
    let conditional_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == conditional.to_string_lossy())
        .map(|node| node.id.clone())
        .ok_or("missing conditional package target")?;
    let node10_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == node10_consumer.to_string_lossy()
        })
        .ok_or("missing Node10 package import")?;
    assert_eq!(node10_import.target, legacy_id);
    assert_ne!(node10_import.target, conditional_id);
    assert_eq!(node10_import.string("resolution_rule"), "package-legacy");
    assert_eq!(node10_import.string("package_condition"), "main");
    assert_eq!(node10_import.string("module_resolution"), "node10");

    let classic_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("source_file") == classic_consumer.to_string_lossy()
        })
        .ok_or("missing Classic package import")?;
    assert_ne!(classic_import.target, legacy_id);
    assert_ne!(classic_import.target, conditional_id);
    assert!(classic_import.attributes.get("target_file").is_none());
    assert_eq!(
        classic_import.string("resolution_rule"),
        "package-classic-unresolved"
    );
    assert_eq!(classic_import.string("module_resolution"), "classic");
    Ok(())
}

#[test]
fn typescript_config_extends_cycles_fail_closed_with_diagnostic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first = root.join("tsconfig.json");
    let second = root.join("tsconfig.other.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("src/consumer.ts");
    let first_source = br#"{
        "extends": "./tsconfig.other.json",
        "compilerOptions": { "paths": { "@/*": ["src/*"] } }
    }"#;
    let second_source = br#"{
        "extends": "./tsconfig.json",
        "compilerOptions": { "baseUrl": "." }
    }"#;
    let implementation_source = br#"export const api = true;"#;
    let consumer_source = br#"import { api } from "@/api";
export const value = api;
"#;
    for (path, source) in [
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    let error = resolved.error.as_deref().unwrap_or_default();
    assert!(error.contains("extends cycle"), "error={error:?}");
    let import = resolved
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "imports_from" && edge.string("module") == "@/api")
        .ok_or("missing cyclic import")?;
    assert!(import.string("resolution_rule").is_empty());
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

#[test]
fn typescript_candidate_preserves_dynamic_member_as_unresolved() {
    let source = br#"class Known { run() {} }
const value = getValue();
value.run();
new Known().run();
function exact(value: string) {}
exact();
"#;
    let batch = Engine::default()
        .extract_source_universal_candidate_evidence(
            Path::new("src/dynamic.ts"),
            "src/dynamic.ts",
            source,
        )
        .expect("candidate evidence");
    let dynamic_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate.constraints.exact_target_declaration_id.is_none()
                && candidate.constraints.qualified_name.is_none()
        })
        .map(|candidate| candidate.id.clone())
        .expect("dynamic call candidate");
    let known_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate.constraints.exact_target_declaration_id.is_some()
        })
        .map(|candidate| candidate.id.clone())
        .expect("known nominal call candidate");
    let arity_mismatch_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "exact"
                && candidate.constraints.argument_count == Some(0)
                && candidate.constraints.exact_target_declaration_id.is_none()
        })
        .map(|candidate| candidate.id.clone())
        .expect("arity-mismatch candidate");
    let index = UniversalResolutionIndex::new(&[batch], UniversalResolutionLimits::default())
        .expect("candidate resolution index");
    assert_eq!(
        index.resolve(&dynamic_id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    );
    assert!(matches!(
        index.resolve(&known_id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    assert_eq!(
        index.resolve(&arity_mismatch_id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    );
}

#[test]
fn typescript_candidate_resolves_relative_and_default_imports_across_files()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "lib/default.ts",
            br#"class DefaultWidget { run() {} }
export default DefaultWidget;
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "../lib/api.js";
import DefaultWidget from "../lib/default";
new Widget();
new Widget().run();
new DefaultWidget();
new DefaultWidget().run();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget")
        .ok_or("missing Widget declaration")?;
    let default_widget = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "DefaultWidget")
        .ok_or("missing default declaration")?;
    let widget_construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Constructs
                && candidate.target_spelling == "Widget"
        })
        .ok_or("missing Widget construction candidate")?;
    let default_construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Constructs
                && candidate.target_spelling == "DefaultWidget"
        })
        .ok_or("missing default construction candidate")?;
    let widget_member_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .ok_or("missing imported member call candidate")?;
    let default_member_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("default"))
        })
        .ok_or("missing default imported member call candidate")?;
    let widget_run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing Widget.run declaration")?;
    let default_run = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing DefaultWidget.run declaration")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&widget_construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget.id
    ));
    assert!(matches!(
        index.resolve(&default_construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &default_widget.id
    ));
    let widget_member_decision = index.resolve(&widget_member_call.id);
    assert!(matches!(
        widget_member_decision,
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget_run.id
    ));
    assert!(matches!(
        index.resolve(&default_member_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &default_run.id
    ));
    Ok(())
}

#[test]
fn typescript_candidate_does_not_use_terminal_name_for_relative_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "app/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "./api";
new Widget();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let app_widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget")
        .ok_or("missing app Widget")?;
    let lib_widget = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget")
        .ok_or("missing lib Widget")?;
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing construction candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &app_widget.id
    ));
    assert_ne!(app_widget.id, lib_widget.id);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_exact_javascript_interop() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/runtime.js",
            br#"export class Runtime { run() {} }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Runtime } from "../lib/runtime.js";
new Runtime().run();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let runtime = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Runtime")
        .ok_or("missing JavaScript Runtime declaration")?;
    assert_eq!(runtime.language, "javascript");
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing JavaScript Runtime.run declaration")?;
    let construct = batches[1]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing interop construction candidate")?;
    let call = batches[1]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .ok_or("missing interop member call candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &runtime.id
    ));
    assert!(matches!(
        index.resolve(&call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}

#[test]
fn typescript_candidate_follows_cross_file_reexport_aliases()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "lib/barrel.ts",
            br#"export { Widget as PublicWidget } from "./api";
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { PublicWidget } from "../lib/barrel";
new PublicWidget().run();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget")
        .ok_or("missing re-exported Widget declaration")?;
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing re-exported Widget.run declaration")?;
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing re-exported construction candidate")?;
    let call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .ok_or("missing re-exported member call candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget.id
    ));
    assert!(matches!(
        index.resolve(&call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_duplicate_module_realizations_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "app/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/api.js",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "./api";
new Widget();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing construction candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Ambiguous { candidate_count } if candidate_count == 2
    ));
    Ok(())
}

#[test]
fn typescript_candidate_consumes_project_path_targets_in_shared_resolution()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "tsconfig.json",
            br#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": { "@/*": ["src/*"] }
  }
}
"#
            .as_slice(),
        ),
        (
            "src/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "@/api";
new Widget().run();
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        if relative.ends_with(".ts") {
            extraction.semantic_evidence = Some(
                Engine::default().extract_source_universal_candidate_evidence(
                    Path::new(relative),
                    relative,
                    source,
                )?,
            );
        }
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    let owned = compass_resolve::resolve_owned_with_root(extractions, &sources, root);
    for resolved in [&resolved, &owned] {
        assert!(
            resolved.error.is_none(),
            "resolver error: {:?}",
            resolved.error
        );
        let calls = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("source_file") == "app/consumer.ts"
                    && edge.string("relation") == "calls"
            })
            .collect::<Vec<_>>();
        assert!(
            calls.iter().any(|edge| {
                edge.string("resolution_rule") == "project-module-binding"
                    && resolved.nodes.iter().any(|node| {
                        node.id == edge.target
                            && node.string("source_file") == "src/api.ts"
                            && node.label() == "Widget"
                    })
            }),
            "project construction target missing: {calls:#?}"
        );
        assert!(
            calls.iter().any(|edge| {
                edge.string("resolution_rule") == "member-binding"
                    && resolved.nodes.iter().any(|node| {
                        node.id == edge.target
                            && node.string("source_file") == "src/api.ts"
                            && node.label() == ".run()"
                    })
            }),
            "project member target missing: {calls:#?}"
        );
    }
    Ok(())
}

#[test]
fn typescript_candidate_merges_imported_interface_members_across_declarations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Config { run(): void }
export interface Config { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Config } from "../lib/types";
export function use(config: Config) { config.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    let owned = compass_resolve::resolve_owned_with_root(extractions, &sources, root);
    for resolved in [&resolved, &owned] {
        assert!(
            resolved.error.is_none(),
            "resolver error: {:?}",
            resolved.error
        );
        let inspect = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("source_file") == "lib/types.ts" && node.label() == ".inspect()"
            })
            .ok_or("missing merged interface member")?;
        assert!(resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
                && edge.string("resolution_rule") == "member-binding"
        }));
    }
    Ok(())
}

#[test]
fn typescript_candidate_leaves_duplicate_merged_interface_members_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Config { inspect(): void }
export interface Config { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Config } from "../lib/types";
export function use(config: Config) { config.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Item>) { box.item.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported generic member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_nested_imported_generic_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Wrapper<U> { value: U }
export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box, Wrapper } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Wrapper<Item>>) { box.item.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing nested imported generic member")?;
    let consumer_evidence = extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.as_ref())
        .find(|evidence| {
            evidence
                .declarations
                .iter()
                .any(|declaration| declaration.range.source_file == "app/consumer.ts")
        })
        .ok_or("missing nested consumer evidence")?;
    let nested_call = consumer_evidence
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Calls)
        .ok_or("missing nested generic call candidate")?;
    assert_eq!(
        nested_call.constraints.qualified_name.as_deref(),
        Some("../lib/types::Box<../lib/types::Wrapper<../lib/item::Item>>.item.value.inspect")
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_nested_generic_member_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Wrapper<U> { value: U }
export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box, Wrapper } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Wrapper<Item>>) { box.item.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_does_not_invent_nested_generic_primitive_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box } from "../lib/types";
export function use(box: Box<string>) { box.item.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_object_type_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type Boxed<T> = { value: T };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Boxed } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Boxed<Item>) { box.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported alias member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_nominal_generic_type_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Box } from "./box";
export type Alias<T> = Box<T>;
"#
            .as_slice(),
        ),
        (
            "lib/box.ts",
            br#"export interface Box<T> { value: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Alias } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Alias<Item>) { box.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing nominal alias member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_homomorphic_mapped_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type Copy<T> = { [K in keyof T]: T[K] };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Copy } from "../lib/types";
import type { Item } from "../lib/item";
export function use(value: Copy<Item>) { value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported mapped member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_publishes_local_conditional_branch_members()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"class Item { inspect(): void {} }
class Other { other(): void {} }
type Choose<T> = T extends Item ? Item : Other;
type ChooseObject<T> = T extends object ? T : never;
function selected(value: Choose<Item>) { value.inspect(); }
function rejected(value: Choose<Other>) { value.inspect(); }
function union(value: Choose<Item | Other>) { value.inspect(); }
function direct(value: Item extends Item ? Item : Other) { value.inspect(); }
function object(value: ChooseObject<Item>) { value.inspect(); }
"#;
    let mut extraction = extract("src/conditional.ts", source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new("src/conditional.ts"),
            "src/conditional.ts",
            source,
        )?,
    );
    let resolved = compass_resolve::resolve(
        &[extraction],
        &HashMap::from([(
            "src/conditional.ts".to_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
    );
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == "src/conditional.ts" && node.label() == ".inspect()"
        })
        .ok_or("missing conditional Item.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == inspect.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|edge| { edge.string("source_file") == "src/conditional.ts" })
    );
    Ok(())
}

#[test]
fn typescript_candidate_publishes_literal_indexed_alias_member_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"interface Nested { inspect(): void }
interface Item { nested: Nested }
type NestedAlias = Item["nested"];
export function use(value: NestedAlias) { value.inspect(); }
"#;
    let relative = "src/indexed-alias.ts";
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve(
        &[extraction],
        &HashMap::from([(relative.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == relative && node.label() == ".inspect()")
        .ok_or("missing indexed alias Nested.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == relative
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "exact-source-declaration"
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_indexed_alias_member_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Nested { inspect(): void }
export interface Item { nested: Nested }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type NestedOf<T> = T["nested"];
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { NestedOf } from "../lib/types";
import type { Item } from "../lib/item";
export function use(value: NestedOf<Item>) { value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported indexed alias Nested.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_array_and_tuple_member_chains()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Box<T> {
    values: T[];
    pair: [T, string];
    nullable: NonNullable<T | undefined>;
    awaited: Awaited<Promise<T>>;
    readonlyValue: Readonly<T>;
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Item } from "../lib/item";
import type { Box } from "../lib/types";
export function use(values: Item[], box: Box<Item>) {
    values[0].inspect();
    box.values[0].inspect();
    box.pair[0].inspect();
    box.nullable.inspect();
    box.awaited.inspect();
    box.readonlyValue.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported array element member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls
            .iter()
            .filter(|edge| edge.target == inspect.id)
            .count(),
        6
    );
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_generic_callable_return_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/generic-return.ts";
    let source = br#"class Item { inspect(): void {} }
function identity<T>(value: T): T { return value; }
export function use() { identity(new Item()).inspect(); }
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == relative && node.label() == ".inspect()")
        .ok_or("missing generic return member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_file") == relative)
        .collect::<Vec<_>>();
    let inspect_calls = calls
        .iter()
        .filter(|edge| edge.target == inspect.id)
        .collect::<Vec<_>>();
    assert_eq!(inspect_calls.len(), 1);
    assert_eq!(
        inspect_calls[0].string("resolution_rule"),
        "exact-source-declaration"
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_return_member_chains()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export function make(value: Item): Item { return value; }
export const makeArrow = (value: Item): Item => value;
export function identity<T>(value: T): T { return value; }
export interface Box<T> { value: T }
export function box<T>(value: T): Box<T> { return { value }; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make, makeArrow, identity, box } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    makeArrow(value).inspect();
    identity(value).inspect();
    box(value).value.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable return member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_callable_return_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export function make(value: Item): Item { return value; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) { make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_member_returns_and_explicit_generics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export class Factory {
    static make(value: Item): Item { return value; }
    static identity<T>(value: T): T { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    Factory.make(value).inspect();
    Factory.identity(value).inspect();
    Factory.identity<Item>(value).inspect();
    Factory.identity<Item>(unknownValue).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable member return")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_ambiguous_imported_callable_member_returns_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export class Factory {
    static make(value: Item): Item { return value; }
}
export class Factory {
    static make(value: Item): Item { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) { Factory.make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_properties_and_typed_objects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"import type { Item } from "./item";
interface TypedApi { make: (value: Item) => Item }
export declare const typed: TypedApi;
export const api = {
    make: (value: Item): Item => value,
    identity: <T>(value: T): T => value,
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { api, typed } from "../lib/api";
import type { Item } from "../lib/item";
export function use(value: Item) {
    api.make(value).inspect();
    api.identity<Item>(value).inspect();
    typed.make(value).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable property return")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    let consumer_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
        })
        .collect::<Vec<_>>();
    assert_eq!(consumer_calls.len(), 6);
    assert!(
        consumer_calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_duplicate_imported_callable_properties_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"import type { Item } from "./item";
export const api = {
    make: (value: Item): Item => value,
    make: (value: Item): Item => value,
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { api } from "../lib/api";
import type { Item } from "../lib/item";
export function use(value: Item) { api.make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_selects_unique_imported_overload_by_argument_type()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/other.ts",
            br#"export interface Other { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
import type { Other } from "./other";
export function make(value: Item): Item { return value; }
export function make(value: Other): Other { return value; }
export class Factory {
    static create(value: Item): Item { return value; }
    static create(value: Other): Other { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory, make } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    Factory.create(value).inspect();
    const current = make(value);
    const alias = current;
    alias.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing selected overload return member")?;
    let other_inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/other.ts" && node.label() == ".inspect()")
        .ok_or("missing alternate overload return member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "calls"
                    && edge.string("source_file") == "app/consumer.ts"
                    && edge.target == inspect.id
                    && edge.string("resolution_rule") == "member-binding"
            })
            .count(),
        3
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == other_inspect.id
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "deferred-receiver"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_overload_ambiguity_and_mismatch_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/other.ts",
            br#"export interface Other { inspect(): void }
export interface Third { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
import type { Other, Third } from "./other";
export function make(value: Item): Item { return value; }
export function make(value: Item): Item { return value; }
export function other(value: Other): Other { return value; }
export function other(value: Third): Third { return value; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make, other } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    other(value).inspect();
    make(unknownValue).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_index_signature_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export interface Shape { [key: string]: Item }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported index member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_index_signature_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Shape<T> { [key: string]: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
import type { Item } from "../lib/item";
export function use(shape: Shape<Item>, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported generic index member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_index_signature_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export interface Shape { [key: string]: Item }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_does_not_invent_imported_index_signature_primitive_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Shape { [key: string]: string }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_straight_line_reassignment_to_latest_member()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/reassignment.ts";
    let source = br#"class First { run() {} }
class Second { run() {} }
export function use() {
    let current = new First();
    current = new Second();
    current.run();
}
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let first_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.label() == ".run()"
                && node.string("qualified_name").ends_with("First.run")
        })
        .ok_or("missing First.run member")?;
    let second_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.label() == ".run()"
                && node.string("qualified_name").ends_with("Second.run")
        })
        .ok_or("missing Second.run member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == relative
            && edge.target == second_run.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == relative
            && edge.target == first_run.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    Ok(())
}

#[test]
fn typescript_callable_values_materialize_as_references_not_indirect_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/callback-values.ts";
    let source = br#"function onValue(value: string) {}
const alias = onValue;
const alias2 = alias;
const handlers = [onValue, alias2];
consume(onValue);
consume(alias2);
consume(handlers[0]);
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let on_value = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.string("qualified_name").ends_with(".onValue")
                && node.string("symbol_kind") == "function"
        })
        .ok_or("missing callable declaration")?;
    let alias2 = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.string("qualified_name").ends_with(".alias2")
                && node.string("symbol_kind") == "variable"
        })
        .ok_or("missing callable alias declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references"
            && edge.string("source_file") == relative
            && edge.target == on_value.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references"
            && edge.string("source_file") == relative
            && edge.target == alias2.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "indirect_call" && edge.string("source_file") == relative
    }));
    Ok(())
}
