use std::collections::HashMap;
use std::path::Path;

use compass_languages::{Engine, file_stem, make_id};

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
            && edge.string("resolution_rule") == "explicitbinding"
    }));
}

#[test]
fn python_runtime_declarations_keep_exact_ownership_through_resolution() {
    let provider = extract("helpers.py", b"def execute():\n    return 1\n");
    let consumer_source = b"def outer(flag):\n    if flag:\n        def duplicate():\n            return 1\n    else:\n        def duplicate():\n            return 2\n    def inner():\n        from helpers import execute\n        return execute()\n    class Runtime:\n        def run(self):\n            return inner()\n    return Runtime\n";
    let consumer = extract("pkg/runtime.py", consumer_source);
    let sources = HashMap::from([(
        "pkg/runtime.py".to_owned(),
        String::from_utf8(consumer_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[provider, consumer], &sources);

    let stem = file_stem(Path::new("pkg/runtime.py"));
    let outer_id = make_id(&[&stem, "outer"]);
    let duplicate_id = make_id(&[&outer_id, "duplicate"]);
    let duplicate_overload_id = make_id(&[&duplicate_id, "overload", "6"]);
    let inner_id = make_id(&[&outer_id, "inner"]);
    let runtime_id = make_id(&[&outer_id, "Runtime"]);
    let run_id = make_id(&[&runtime_id, "run"]);
    let execute = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "execute()" && node.string("source_file") == "helpers.py")
        .expect("provider function");

    for id in [
        &outer_id,
        &duplicate_id,
        &duplicate_overload_id,
        &inner_id,
        &runtime_id,
        &run_id,
    ] {
        assert!(resolved.nodes.iter().any(|node| node.id == *id));
    }
    for (source, target, relation) in [
        (&outer_id, &duplicate_id, "contains"),
        (&outer_id, &duplicate_overload_id, "contains"),
        (&outer_id, &inner_id, "contains"),
        (&outer_id, &runtime_id, "contains"),
        (&runtime_id, &run_id, "method"),
    ] {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.source == *source
                    && edge.target == *target
                    && edge.string("relation") == relation
            }),
            "missing {relation} {source} -> {target}; relevant edges={:#?}",
            resolved
                .edges
                .iter()
                .filter(|edge| edge.source == *source || edge.target == *target)
                .collect::<Vec<_>>()
        );
    }

    let imported = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "imports_from" && edge.target == execute.id);
    assert_eq!(imported.clone().count(), 1);
    assert!(imported.into_iter().all(|edge| edge.source == inner_id));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == execute.id);
    assert_eq!(calls.clone().count(), 1);
    assert!(calls.into_iter().all(|edge| edge.source == inner_id));
    assert!(resolved.edges.iter().all(|edge| {
        !matches!(edge.string("relation").as_str(), "imports_from" | "calls")
            || edge.target != execute.id
            || (edge.source != outer_id && edge.source != make_id(&[&stem]))
    }));
}

#[test]
fn python_super_call_resolves_an_exact_direct_base_method_through_shared_c3_dispatch() {
    let provider = extract(
        "pkg/base.py",
        b"class Base:\n    def run(self):\n        return None\n",
    );
    let caller_source = b"from pkg.base import Base\nclass Child(Base):\n    def run(self):\n        super().run()\n";
    let caller = extract("pkg/child.py", caller_source);
    let sources = HashMap::from([(
        "pkg/child.py".to_owned(),
        String::from_utf8(caller_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[provider, caller], &sources);
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/base.py"
                && node.label().trim_start_matches('.').trim_end_matches("()") == "run"
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
        "directreceiversuccessordispatch"
    );
}

#[test]
fn python_self_and_cls_calls_resolve_only_through_the_proven_receiver_hierarchy() {
    let source = b"class Base:\n    def inherited(self):\n        return None\nclass Owner(Base):\n    def own(self):\n        return None\n    @classmethod\n    def build(cls):\n        cls.own()\n        cls.inherited()\n    def run(self):\n        self.own()\n        self.inherited()\n        self.missing()\nclass Unrelated:\n    def missing(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    let calls_at = |line: &str| {
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "calls" && edge.string("source_location") == line
            })
            .collect::<Vec<_>>()
    };
    let target_location = |target: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.id == target)
            .map(|node| node.string("source_location"))
            .unwrap_or_default()
    };
    for line in ["L9", "L12"] {
        let calls = calls_at(line);
        assert_eq!(calls.len(), 1, "own receiver call at {line}: {calls:#?}");
        assert_eq!(target_location(&calls[0].target), "L5");
        assert_eq!(
            calls[0].string("resolution_rule"),
            "linearizedreceiverdispatch"
        );
    }
    for line in ["L10", "L13"] {
        let calls = calls_at(line);
        assert_eq!(
            calls.len(),
            1,
            "inherited receiver call at {line}: {calls:#?}"
        );
        assert_eq!(target_location(&calls[0].target), "L2");
        assert_eq!(
            calls[0].string("resolution_rule"),
            "linearizedreceiverdispatch"
        );
    }
    assert!(
        calls_at("L14").is_empty(),
        "unrelated method must fail closed"
    );
}

#[test]
fn python_self_call_with_an_external_base_cannot_rebind_to_a_local_class() {
    let source = b"import logging\nclass Handler(logging.Handler):\n    def emit(self, record):\n        self.format(record)\nclass Formatter:\n    def format(self, record):\n        return record\n";
    let extracted = extract("pkg/log.py", source);
    let sources = HashMap::from([(
        "pkg/log.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L4"
    }));
}

#[test]
fn python_self_call_does_not_guess_a_runtime_subclass_override() {
    let source = b"class Base:\n    def run(self):\n        self.hook()\nclass Child(Base):\n    def hook(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L3"
    }));
}

#[test]
fn python_receiver_construction_resolves_an_exact_nested_type() {
    let source = b"class Owner:\n    class Product:\n        pass\n    def build(self):\n        return self.Product()\n";
    let extracted = extract("pkg/models.py", source);
    assert_eq!(extracted.error, None);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let product = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "class"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("nested product class");
    let constructions = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L5")
        .collect::<Vec<_>>();
    assert_eq!(constructions.len(), 1, "{constructions:#?}");
    assert_eq!(constructions[0].target, product.id);
    assert_eq!(
        constructions[0].string("resolution_rule"),
        "linearizedreceiverdispatch"
    );
}

#[test]
fn python_self_call_uses_a_source_proven_first_base_before_an_external_boundary() {
    let source = b"class Known:\n    def prepare(self):\n        return None\nclass Owner(Known, External):\n    def run(self):\n        self.prepare()\n";
    let extracted = extract("pkg/models.py", source);
    assert_eq!(extracted.error, None);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let prepare = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("known first-base member");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L6")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert_eq!(calls[0].target, prepare.id);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearizedreceiverdispatch"
    );
}

#[test]
fn python_self_call_follows_single_inheritance_to_a_proven_first_base_member() {
    let source = b"class Known:\n    def prepare(self):\n        return None\nclass Middle(Known, External):\n    pass\nclass Owner(Middle):\n    def run(self):\n        self.prepare()\n";
    let extracted = extract("pkg/models.py", source);
    assert_eq!(extracted.error, None);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let prepare = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("known first-base member");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L8")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert_eq!(calls[0].target, prepare.id);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearizedreceiverdispatch"
    );
}

#[test]
fn python_direct_base_publication_cannot_fall_through_to_a_same_named_local_class() {
    let provider = extract("pkg/provider.py", b"class Transform:\n    pass\n");
    let caller_source = b"from pkg.provider import Transform\nclass Child(Transform):\n    pass\nclass Transform:\n    pass\n";
    let caller = extract("pkg/models.py", caller_source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(caller_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[provider, caller], &sources);
    let imported = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "class" && node.string("source_file") == "pkg/provider.py"
        })
        .expect("imported base");
    let local = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "class"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L4"
        })
        .expect("same-named local class");
    let bases = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "inherits")
        .collect::<Vec<_>>();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0].target, imported.id);
    assert_ne!(bases[0].target, local.id);
    assert_eq!(bases[0].string("resolution_rule"), "exacthierarchybase");
}

#[test]
fn python_nested_sibling_base_uses_its_lexical_class_identity() {
    let source = b"class Outer:\n    class Base:\n        def run(self):\n            return None\n    class Child(Base):\n        def run(self):\n            super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let base = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "class"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("nested base");
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L3"
        })
        .expect("nested base method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "inherits"
            && edge.target == base.id
            && edge.string("source_location") == "L5"
            && edge.string("resolution_rule") == "exacthierarchybase"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == base_run.id
            && edge.string("source_location") == "L7"
            && edge.string("resolution_rule") == "directreceiversuccessordispatch"
    }));
}

#[test]
fn python_super_call_resolves_a_direct_successor_before_its_external_ancestor() {
    let source = b"from external import Unknown\nclass Base(Unknown):\n    def run(self):\n        return None\nclass Child(Base):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L3"
        })
        .expect("direct successor method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_location") == "L7"
            && edge.target == base_run.id
            && edge.string("resolution_rule") == "directreceiversuccessordispatch"
    }));
}

#[test]
fn python_super_call_uses_declared_c3_order_for_multiple_bases() {
    let source = b"class Left:\n    def run(self):\n        return None\nclass Right:\n    def run(self):\n        return None\nclass Child(Left, Right):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let left_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("left method");

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L9")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].target, left_run.id);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "directreceiversuccessordispatch"
    );
}

#[test]
fn python_super_call_resolves_a_method_inherited_beyond_the_direct_base() {
    let source = b"class Root:\n    def run(self):\n        return None\nclass Middle(Root):\n    pass\nclass Child(Middle):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let root_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L2"
        })
        .expect("root method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_location") == "L8"
            && edge.target == root_run.id
            && edge.string("resolution_rule") == "linearizedreceiverdispatch"
    }));
}

#[test]
fn python_super_call_fails_closed_for_a_dynamic_base_expression() {
    let source = b"class Base:\n    def run(self):\n        return None\ndef make_base():\n    return Base\nclass Child(make_base()):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L8"
    }));
}

#[test]
fn python_super_dispatch_cannot_fall_through_to_a_same_named_import() {
    let source = b"import copy\nclass Base:\n    def copy(self):\n        return None\nclass Child(Base):\n    def copy(self):\n        return super().copy()\n";
    let extracted = extract("pkg/models.py", source);
    assert_eq!(extracted.error, None);
    let sources = HashMap::from([(
        "pkg/models.py".to_owned(),
        String::from_utf8(source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let base_copy = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/models.py"
                && node.string("source_location") == "L3"
        })
        .expect("base copy method");

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L7")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].target, base_copy.id);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "directreceiversuccessordispatch"
    );
}

#[test]
fn repeated_external_type_uses_share_the_exact_import_binding() {
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
                && node.string("symbol_kind") == "type_alias"
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].string("source_location"), "L1");
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
fn qualified_calls_require_a_binding_or_an_exact_language_receiver() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
func caller(untyped any, worker *Worker) {
    untyped.Run()
    worker.Run()
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.label().trim_start_matches('.').trim_end_matches("()") == "Run"
        })
        .unwrap_or_else(|| panic!("declared method; nodes={:#?}", resolved.nodes));
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == run.id)
        .collect::<Vec<_>>();

    assert_eq!(calls.len(), 1, "untyped receivers must fail closed");
    assert_eq!(calls[0].string("source_location"), "L6");
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
