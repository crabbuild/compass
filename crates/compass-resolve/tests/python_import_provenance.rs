use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::EdgeKind;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_resolve::resolve_with_root;

const UNIVERSAL_PYTHON_PRODUCER: &str = "compass.resolve.python.universal";
const RETIRED_PYTHON_PRODUCER: &str = "compass.resolve.python-imports";
type ResolvedFixture = (tempfile::TempDir, Extraction, HashMap<String, String>);

fn extract(
    engine: &mut Engine,
    root: &Path,
    relative: &str,
    source: &str,
) -> Result<Extraction, Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, source)?;
    Ok(engine
        .extract_source_combined(&path, relative, source.as_bytes())?
        .graph)
}

fn resolve_fixture(files: &[(&str, &str)]) -> Result<ResolvedFixture, Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        extractions.push(extract(&mut engine, root, relative, source)?);
        sources.insert((*relative).to_owned(), (*source).to_owned());
    }
    let resolved = resolve_with_root(&extractions, &sources, root);
    Ok((directory, resolved, sources))
}

fn assert_no_retired_python_projection(extraction: &Extraction) {
    assert!(extraction.edges.iter().all(|edge| {
        edge.string("extractor") != RETIRED_PYTHON_PRODUCER
            && !matches!(
                edge.string("rule").as_str(),
                "python-symbol-import-resolution"
                    | "python-submodule-import-resolution"
                    | "python-module-re-export-resolution"
            )
    }));
}

fn edge_span<'a>(edge: &compass_languages::RawEdgeRecord, source: &'a str) -> &'a str {
    let start = edge
        .attributes
        .get("start_byte")
        .and_then(serde_json::Value::as_u64)
        .expect("edge start") as usize;
    let end = edge
        .attributes
        .get("end_byte")
        .and_then(serde_json::Value::as_u64)
        .expect("edge end") as usize;
    source.get(start..end).expect("UTF-8 edge span")
}

#[test]
fn universal_python_imports_publish_exact_item_spans_and_no_legacy_projection()
-> Result<(), Box<dyn Error>> {
    let caller = concat!(
        "from pkg.api import (\r\n",
        "    Widget as LocalWidget,\r\n",
        ")\r\n",
        "from pkg import mod\r\n",
        "def build():\r\n",
        "    return LocalWidget()\r\n",
    );
    let files = [
        ("caller.py", caller),
        ("pkg/__init__.py", "from .api import Widget\n"),
        ("pkg/api.py", "class Widget:\n    pass\n"),
        ("pkg/mod.py", "VALUE = 1\n"),
    ];
    let (directory, mut resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);
    assert_no_retired_python_projection(&resolved);

    let widget = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/api.py")
                && node.string("symbol_kind") == "class"
                && node.label() == "Widget"
        })
        .ok_or("missing Widget declaration")?;
    let widget_id = widget.id.clone();
    let module = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/mod.py")
                && node.string("symbol_kind") == "file"
        })
        .ok_or("missing mod inventory")?;
    let module_id = module.id.clone();
    let caller_imports = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("extractor") == UNIVERSAL_PYTHON_PRODUCER
                && edge.string("source_file") == "caller.py"
                && edge.string("relation") == "imports_from"
        })
        .collect::<Vec<_>>();
    assert_eq!(caller_imports.len(), 2);
    assert!(caller_imports.iter().any(|edge| {
        edge.target == widget_id && edge_span(edge, caller) == "Widget as LocalWidget"
    }));
    assert!(
        caller_imports
            .iter()
            .any(|edge| { edge.target == module_id && edge_span(edge, caller) == "mod" })
    );

    let package = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/__init__.py")
                && node.string("symbol_kind") == "file"
        })
        .ok_or("missing package inventory")?;
    let package_id = package.id.clone();
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == package_id
            && edge.target == widget_id
            && edge.string("relation") == "re_exports"
            && edge.string("extractor") == UNIVERSAL_PYTHON_PRODUCER
    }));

    let node_ids = resolved
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    resolved.edges.retain(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    });
    let evidence = BuildEvidence::from_extraction(
        directory.path(),
        &resolved,
        "sha256:universal-python-import-hard-cut",
    )?;
    let graph = normalize_v1(resolved, evidence)?;
    assert!(graph.links.iter().all(|edge| {
        edge.evidence
            .iter()
            .all(|evidence| evidence.extractor != RETIRED_PYTHON_PRODUCER)
    }));
    let published_widget = graph
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Widget"
                && node
                    .source_file()
                    .is_some_and(|source| source.ends_with("pkg/api.py"))
        })
        .ok_or("missing published Widget")?;
    assert!(graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Exports
            && edge.target == published_widget.id
            && edge.evidence.iter().any(|evidence| {
                evidence.extractor == UNIVERSAL_PYTHON_PRODUCER
                    && evidence.origin == EvidenceOrigin::Ast
                    && evidence.confidence == EvidenceConfidence::Exact
                    && evidence.anchors.len() == 1
            })
    }));
    Ok(())
}

#[test]
fn universal_imports_keep_exact_targets_across_colliding_module_leaf_names()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "alpha/consumer.py",
            "from .models import Widget\ndef build():\n    return Widget()\n",
        ),
        ("alpha/models.py", "class Widget:\n    pass\n"),
        ("beta/models.py", "class Widget:\n    pass\n"),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);
    assert_no_retired_python_projection(&resolved);

    let alpha_widget = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("alpha/models.py") && node.label() == "Widget"
        })
        .ok_or("missing alpha Widget")?;
    let beta_widget = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("beta/models.py") && node.label() == "Widget"
        })
        .ok_or("missing beta Widget")?;
    assert_ne!(alpha_widget.id, beta_widget.id);

    let consumer = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("alpha/consumer.py")
                && node.string("symbol_kind") == "file"
        })
        .ok_or("missing consumer module")?;
    let build = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("alpha/consumer.py") && node.label() == "build()"
        })
        .ok_or("missing build")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == consumer.id
            && edge.target == alpha_widget.id
            && edge.string("relation") == "imports_from"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == build.id
            && edge.target == alpha_widget.id
            && edge.string("relation") == "calls"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.target != beta_widget.id
            || !matches!(edge.string("relation").as_str(), "imports_from" | "calls")
    }));
    Ok(())
}

#[test]
fn explicit_import_binding_shadows_a_same_named_outer_declaration() -> Result<(), Box<dyn Error>> {
    let files = [
        (
            "facade.py",
            "def render():\n    from pkg.template import render\n    return render()\n",
        ),
        ("pkg/template.py", "def render():\n    return 'template'\n"),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);

    let wrapper = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("facade.py") && node.label() == "render()"
        })
        .ok_or("missing wrapper")?;
    let imported = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/template.py") && node.label() == "render()"
        })
        .ok_or("missing imported render")?;
    assert_ne!(wrapper.id, imported.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == wrapper.id
            && edge.target == imported.id
            && edge.string("relation") == "imports_from"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == wrapper.id
            && edge.target == imported.id
            && edge.string("relation") == "calls"
    }));
    Ok(())
}

#[test]
fn identity_module_alias_resolves_to_the_exact_source_inventory() -> Result<(), Box<dyn Error>> {
    let files = [
        ("caller.py", "from pkg import signals\n"),
        ("pkg/__init__.py", "from . import signals\n"),
        ("pkg/signals.py", "VALUE = 1\n"),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);

    let caller = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("caller.py")
                && node.string("symbol_kind") == "file"
        })
        .ok_or("missing caller")?;
    let signals = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/signals.py")
                && node.string("symbol_kind") == "file"
        })
        .ok_or("missing signals module")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == caller.id
            && edge.target == signals.id
            && edge.string("relation") == "imports_from"
            && edge.string("resolution_rule") == "explicitbinding"
    }));
    Ok(())
}

#[test]
fn function_local_python_imports_are_owned_by_the_function_and_do_not_leak()
-> Result<(), Box<dyn Error>> {
    let caller = concat!(
        "def with_import():\n",
        "    from pkg.api import run\n",
        "    return run()\n",
        "\n",
        "def sibling():\n",
        "    return run()\n",
    );
    let files = [
        ("caller.py", caller),
        ("pkg/__init__.py", ""),
        ("pkg/api.py", "def run():\n    return 1\n"),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);
    assert_no_retired_python_projection(&resolved);

    let declaration = |name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("source_file").ends_with("caller.py") && node.label() == name)
            .map(|node| node.id.clone())
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let with_import = declaration("with_import()");
    let sibling = declaration("sibling()");
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file").ends_with("pkg/api.py") && node.label() == "run()")
        .ok_or("missing run definition")?;

    let import_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "imports_from" && edge.target == run.id)
        .collect::<Vec<_>>();
    assert_eq!(import_edges.len(), 1);
    assert_eq!(import_edges[0].source, with_import);
    assert_eq!(edge_span(import_edges[0], caller), "run");

    let call_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == run.id)
        .collect::<Vec<_>>();
    assert_eq!(call_edges.len(), 1);
    assert_eq!(call_edges[0].source, with_import);
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != sibling || edge.target != run.id || edge.string("relation") != "calls"
    }));
    Ok(())
}

#[test]
fn function_local_python_import_shadows_ambiguous_file_bindings() -> Result<(), Box<dyn Error>> {
    let files = [
        (
            "caller.py",
            concat!(
                "from pkg.first import run\n",
                "from pkg.second import run\n",
                "def local():\n",
                "    from pkg.exact import run\n",
                "    return run()\n",
                "def sibling():\n",
                "    return run()\n",
            ),
        ),
        ("pkg/__init__.py", ""),
        ("pkg/first.py", "def run():\n    return 1\n"),
        ("pkg/second.py", "def run():\n    return 2\n"),
        ("pkg/exact.py", "def run():\n    return 3\n"),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_eq!(resolved.error, None);
    assert_no_retired_python_projection(&resolved);

    let node_id = |source: &str, label: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("source_file").ends_with(source) && node.label() == label)
            .map(|node| node.id.clone())
            .unwrap_or_else(|| panic!("missing {source}:{label}"))
    };
    let local = node_id("caller.py", "local()");
    let sibling = node_id("caller.py", "sibling()");
    let exact_run = node_id("pkg/exact.py", "run()");

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == exact_run)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].source, local);
    assert!(
        resolved
            .edges
            .iter()
            .all(|edge| { edge.source != sibling || edge.string("relation") != "calls" })
    );
    Ok(())
}

#[test]
fn universal_python_reexports_follow_a_bounded_multi_hop_alias_chain_deterministically()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "caller.py",
            "from pkg import run\ndef main():\n    return run()\n",
        ),
        ("pkg/__init__.py", "from .facade import execute as run\n"),
        ("pkg/facade.py", "from .impl import execute\n"),
        ("pkg/impl.py", "def execute():\n    return 1\n"),
    ];
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        extractions.push(extract(&mut engine, root, relative, source)?);
        sources.insert(relative.to_owned(), source.to_owned());
    }

    let forward = resolve_with_root(&extractions, &sources, root);
    extractions.reverse();
    let reverse = resolve_with_root(&extractions, &sources, root);
    assert_eq!(forward.error, None);
    assert_eq!(reverse.error, None);
    assert_no_retired_python_projection(&forward);
    assert_no_retired_python_projection(&reverse);

    let canonical_edges = |extraction: &Extraction| {
        let mut edges = extraction
            .edges
            .iter()
            .filter(|edge| edge.string("extractor") == UNIVERSAL_PYTHON_PRODUCER)
            .map(|edge| {
                (
                    edge.source.clone(),
                    edge.target.clone(),
                    edge.string("relation"),
                    edge.string("source_file"),
                    edge.attributes
                        .get("start_byte")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        edges.sort();
        edges
    };
    assert_eq!(canonical_edges(&forward), canonical_edges(&reverse));

    let target = forward
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("pkg/impl.py") && node.label() == "execute()"
        })
        .ok_or("missing implementation")?;
    let main = forward
        .nodes
        .iter()
        .find(|node| node.string("source_file").ends_with("caller.py") && node.label() == "main()")
        .ok_or("missing main")?;
    let calls = forward
        .edges
        .iter()
        .filter(|edge| {
            edge.source == main.id && edge.target == target.id && edge.string("relation") == "calls"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("resolution_rule"), "explicitbinding");
    Ok(())
}

#[test]
fn universal_python_decorators_resolve_through_package_reexports() -> Result<(), Box<dyn Error>> {
    let files = [
        (
            "app.py",
            "from framework import used\n\n@used('class')\nclass Consumer:\n    @used('method')\n    def run(self):\n        pass\n",
        ),
        ("framework/__init__.py", "from .decorators import used\n"),
        (
            "framework/decorators.py",
            "def used(value):\n    return value\n",
        ),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_no_retired_python_projection(&resolved);
    let target = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "used()"
                && node
                    .string("source_file")
                    .ends_with("framework/decorators.py")
        })
        .ok_or("missing decorator definition")?;
    let decorators = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("context") == "decorator")
        .collect::<Vec<_>>();
    assert_eq!(decorators.len(), 2);
    assert!(decorators.iter().all(|edge| {
        edge.target == target.id
            && edge.string("extractor") == UNIVERSAL_PYTHON_PRODUCER
            && edge.string("resolution_rule") == "explicitbinding"
    }));
    Ok(())
}
