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
