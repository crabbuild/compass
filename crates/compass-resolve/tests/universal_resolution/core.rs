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

fn source_matches(actual: &str, expected: &Path) -> bool {
    if actual.is_empty() {
        return false;
    }
    if actual == expected.to_string_lossy() {
        return true;
    }
    let actual = Path::new(actual);
    actual.is_relative() && expected.ends_with(actual)
}

fn target_source_matches(
    resolved: &compass_languages::Extraction,
    target: &str,
    expected: &Path,
) -> bool {
    resolved
        .nodes
        .iter()
        .any(|node| node.id == target && source_matches(&node.string("source_file"), expected))
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
    assert!(
        first
            .edges
            .iter()
            .filter(|edge| edge.string("extractor").contains(".universal"))
            .all(|edge| {
                !edge.attributes.contains_key("evidence_candidate_id")
                    && !edge.attributes.contains_key("evidence_occurrence_id")
            })
    );
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
    assert!(
        resolved
            .nodes
            .iter()
            .any(|node| source_matches(&node.string("source_file"), &fallback))
    );
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@example/ordered"
        })
        .ok_or("missing ordered package import")?;
    assert!(target_source_matches(&resolved, &import.target, &fallback));
    assert_eq!(import.string("package_condition"), "default");
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
