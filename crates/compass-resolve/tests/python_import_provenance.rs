use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction, RawEdgeRecord};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance};
use compass_resolve::resolve_with_root;

const PYTHON_IMPORT_PRODUCER: &str = "compass.resolve.python.universal";
const UNIVERSAL_PYTHON_PRODUCER: &str = PYTHON_IMPORT_PRODUCER;
const RETIRED_PYTHON_PRODUCER: &str = "compass.resolve.python-imports";
const PYTHON_SYMBOL_IMPORT_RULE: &str = "universal-import-explicit-binding";
const PYTHON_REEXPORT_RULE: &str = "universal-reexport-explicit-binding";
type ResolvedFixture = (tempfile::TempDir, Extraction, HashMap<String, String>);

fn is_python_import_edge(edge: &RawEdgeRecord) -> bool {
    edge.string("extractor") == PYTHON_IMPORT_PRODUCER
        && matches!(
            edge.string("context").as_str(),
            "import" | "submodule_import" | "export"
        )
}

fn is_python_import_evidence(evidence: &Provenance) -> bool {
    evidence.extractor == PYTHON_IMPORT_PRODUCER
        && evidence.rule.as_deref().is_some_and(|rule| {
            rule.starts_with("universal-import-") || rule.starts_with("universal-reexport-")
        })
}

fn skip_python_whitespace(statement: &str, mut cursor: usize) -> usize {
    while let Some(rest) = statement.get(cursor..) {
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn python_import_item_spans(statement: &str) -> Vec<(usize, usize)> {
    let Some(import_pos) = statement.find("import") else {
        return Vec::new();
    };
    let mut cursor = skip_python_whitespace(statement, import_pos + "import".len());
    if let Some(rest) = statement.get(cursor..) {
        if rest.starts_with('(') {
            cursor += 1;
            cursor = skip_python_whitespace(statement, cursor);
        }
    }

    let mut spans = Vec::new();
    while cursor < statement.len() {
        cursor = skip_python_whitespace(statement, cursor);
        while let Some(rest) = statement.get(cursor..) {
            if rest.starts_with('\\') {
                cursor += 1;
                cursor = skip_python_whitespace(statement, cursor);
                continue;
            }
            break;
        }

        if cursor >= statement.len() || statement[cursor..].starts_with(')') {
            break;
        }

        let start = cursor;
        let mut end = cursor;
        let mut paren_depth = 0usize;

        while end < statement.len() {
            let ch = statement[end..]
                .chars()
                .next()
                .expect("span has at least one char");
            if ch == '(' {
                paren_depth += 1;
                end += ch.len_utf8();
                continue;
            }
            if ch == ')' {
                if paren_depth == 0 {
                    break;
                }
                paren_depth = paren_depth.saturating_sub(1);
                end += ch.len_utf8();
                continue;
            }
            if ch == ',' {
                if paren_depth == 0 {
                    break;
                }
            }
            if ch == '\\' {
                break;
            }
            if (ch == '\n' || ch == '\r') && paren_depth == 0 {
                break;
            }
            end += ch.len_utf8();
        }

        let mut span_end = end;
        while span_end > start {
            let trailing = statement[..span_end]
                .chars()
                .next_back()
                .expect("span has at least one char");
            if trailing.is_whitespace() || trailing == ')' {
                span_end -= trailing.len_utf8();
            } else {
                break;
            }
        }

        if span_end > start {
            spans.push((start, span_end));
        }

        cursor = end;
        if let Some(rest) = statement.get(cursor..) {
            if rest.starts_with(',') {
                cursor += 1;
            } else if rest.starts_with(')') {
                break;
            } else {
                break;
            }
        }
    }

    spans
}

fn python_import_item_token_spans(statement: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    while cursor < statement.len() {
        cursor = skip_python_whitespace(statement, cursor);
        if cursor >= statement.len() {
            break;
        }
        let start = cursor;
        while cursor < statement.len() {
            let ch = statement[cursor..]
                .chars()
                .next()
                .expect("span has at least one char");
            if ch.is_whitespace() {
                break;
            }
            cursor += ch.len_utf8();
        }
        if start < cursor {
            spans.push((start, cursor));
        }
    }

    spans
}

fn python_import_local_occurrence_span(item: &str) -> Option<(usize, usize)> {
    let tokens = python_import_item_token_spans(item);
    let mut saw_as = false;
    for (start, end) in tokens {
        let token = item.get(start..end).unwrap_or("");
        if saw_as {
            return Some((start, end));
        }
        if token == "as" {
            saw_as = true;
        }
    }
    None
}

fn python_import_occurrence_spans(statement: &str) -> Vec<(usize, usize)> {
    python_import_item_spans(statement)
        .into_iter()
        .filter_map(|(start, end)| {
            if start >= end {
                return None;
            }
            let item = statement.get(start..end)?;
            let span = python_import_local_occurrence_span(item).unwrap_or((0, end - start));
            Some((start + span.0, start + span.1))
        })
        .collect()
}

fn import_occurrence_spans_in_source(
    statement: &str,
    source: &str,
) -> Result<Vec<(usize, usize)>, Box<dyn Error>> {
    let statement_start = source
        .find(statement)
        .ok_or("missing expected import statement")?;
    Ok(python_import_occurrence_spans(statement)
        .into_iter()
        .map(|(start, end)| (statement_start + start, statement_start + end))
        .collect())
}

fn write(root: &Path, relative: &str, source: &str) -> Result<String, Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, source)?;
    Ok(path.to_string_lossy().into_owned())
}

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
    assert!(
        caller_imports
            .iter()
            .any(|edge| edge.target == widget_id && edge_span(edge, caller) == "LocalWidget")
    );
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
            && edge.string("resolution_rule") == "explicit-binding"
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
    assert_eq!(calls[0].string("resolution_rule"), "explicit-binding");
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
    assert_eq!(decorators.len(), 2, "edges={:#?}", resolved.edges);
    assert!(
        decorators.iter().all(|edge| edge.target == target.id),
        "target={target:#?} edges={decorators:#?}"
    );
    assert_eq!(
        decorators
            .iter()
            .map(|edge| edge.string("source_location"))
            .collect::<Vec<_>>(),
        ["L3", "L5"]
    );
    assert!(decorators.iter().all(|edge| {
        edge.attributes.contains_key("start_byte")
            && edge.attributes.contains_key("end_byte")
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("target_qualified_name") != "framework.unused"
            && edge.string("rule") != "python-imported-class-use-inference"
    }));
    Ok(())
}

#[test]
fn universal_python_qualified_calls_resolve_through_package_wildcard_reexports()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "app.py",
            "from django.db import models\n\ndef build():\n    return models.CharField(max_length=255)\n",
        ),
        (
            "direct.py",
            "from django.db.models.fields import *\n\ndef build_direct():\n    return CharField(max_length=255)\n",
        ),
        (
            "ambiguous.py",
            "from django.db.models.fields import *\nfrom other.fields import *\n\ndef build_ambiguous():\n    return CharField(max_length=255)\n",
        ),
        ("django/__init__.py", ""),
        ("django/db/__init__.py", ""),
        (
            "django/db/models/__init__.py",
            "from django.db.models.fields import *\n",
        ),
        (
            "django/db/models/fields.py",
            "class CharField:\n    def __init__(self, max_length):\n        self.max_length = max_length\n",
        ),
        ("other/__init__.py", ""),
        (
            "other/fields.py",
            "class CharField:\n    def __init__(self, max_length):\n        self.max_length = max_length\n",
        ),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    assert_no_retired_python_projection(&resolved);

    let target = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "CharField"
                && node
                    .string("source_file")
                    .ends_with("django/db/models/fields.py")
        })
        .ok_or("missing CharField declaration")?;
    let build = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "build()")
        .ok_or("missing build declaration")?;
    let constructions = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.source == build.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("rule").starts_with("universal-call-")
        })
        .collect::<Vec<_>>();
    assert_eq!(constructions.len(), 1, "edges={:#?}", resolved.edges);
    assert_eq!(
        constructions[0].string("resolution_rule"),
        "wildcard-binding"
    );
    let build_direct = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "build_direct()")
        .ok_or("missing build_direct declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == build_direct.id
            && edge.target == target.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "wildcard-binding"
    }));
    let build_ambiguous = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "build_ambiguous()")
        .ok_or("missing build_ambiguous declaration")?;
    assert!(
        resolved.edges.iter().all(|edge| {
            edge.source != build_ambiguous.id || edge.string("relation") != "calls"
        })
    );
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "django.db.models.CharField"
            || !node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
    }));
    Ok(())
}

#[test]
fn python_call_targets_use_declaration_kind_instead_of_name_capitalization()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "app.py",
            "from pkg import Factory, override_settings\n\ndef build():\n    Factory()\n    override_settings(DEBUG=True)\n",
        ),
        (
            "pkg/__init__.py",
            "from .api import Factory, override_settings\n",
        ),
        (
            "pkg/api.py",
            "def Factory():\n    return None\n\nclass override_settings:\n    pass\n",
        ),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    let build = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "build()")
        .ok_or("missing build declaration")?;
    let factory = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.api.Factory")
        .ok_or("missing Factory declaration")?;
    let settings = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.api.override_settings")
        .ok_or("missing override_settings declaration")?;

    for target in [factory, settings] {
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == build.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("rule").starts_with("universal-call-")
        }));
    }
    assert!(resolved.nodes.iter().all(|node| {
        !matches!(
            node.string("qualified_name").as_str(),
            "pkg.Factory" | "pkg.override_settings"
        ) || node
            .attributes
            .get("external")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    }));
    Ok(())
}

#[test]
fn python_partial_callable_aliases_are_source_backed_through_package_reexports()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "caller.py",
            "from pkg import route\n\ndef build():\n    return route('home/')\n",
        ),
        ("pkg/__init__.py", "from .routes import route\n"),
        (
            "pkg/routes.py",
            "from functools import partial\n\ndef _route(value, *, Pattern):\n    return Pattern(value)\n\nroute = partial(_route, Pattern=str)\n",
        ),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    let build = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "build()")
        .ok_or("missing build declaration")?;
    let route = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.routes.route")
        .ok_or("missing source-backed route callable alias")?;
    let underlying = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.routes._route")
        .ok_or("missing underlying route function")?;

    assert_eq!(route.string("symbol_kind"), "function");
    assert!(route.string("source_file").ends_with("pkg/routes.py"));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == build.id
            && edge.target == route.id
            && edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == route.id
            && edge.target == underlying.id
            && edge.string("relation") == "references"
    }));
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "pkg.route"
            || node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
    }));
    Ok(())
}

#[test]
fn python_module_singletons_are_source_backed_with_exact_initializer_types()
-> Result<(), Box<dyn Error>> {
    let files = [
        (
            "caller.py",
            "from pkg.state import singleton\n\ndef use():\n    return singleton\n",
        ),
        (
            "pkg/state.py",
            "class Service:\n    pass\n\nsingleton = Service()\n",
        ),
    ];
    let (_, resolved, _) = resolve_fixture(&files)?;
    let singleton = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.state.singleton")
        .ok_or("missing source-backed singleton")?;
    let service = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.state.Service")
        .ok_or("missing singleton initializer type")?;
    let use_function = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "caller.use")
        .ok_or("missing singleton consumer")?;

    assert_eq!(singleton.string("symbol_kind"), "variable");
    assert!(singleton.string("source_file").ends_with("pkg/state.py"));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == singleton.id
            && edge.target == service.id
            && edge.string("relation") == "type_of"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == use_function.id
            && edge.target == singleton.id
            && edge.string("relation") == "references"
            && edge.string("resolution_rule") == "explicit-binding"
    }));
    assert!(resolved.nodes.iter().all(|node| {
        node.string("qualified_name") != "pkg.state.singleton"
            || node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
    }));
    Ok(())
}

#[test]
fn python_receiver_methods_never_rebind_to_same_named_imports() -> Result<(), Box<dyn Error>> {
    let files = [(
        "case.py",
        "from django.conf import settings\n\nclass Case:\n    def test(self):\n        return self.settings(DEBUG=True)\n",
    )];
    let (_, resolved, _) = resolve_fixture(&files)?;
    let test_method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "case.Case::test")
        .ok_or("missing test method")?;

    assert!(resolved.edges.iter().all(|edge| {
        edge.source != test_method.id
            || edge.string("relation") != "calls"
            || resolved.nodes.iter().all(|node| {
                node.id != edge.target || node.string("qualified_name") != "django.conf.settings"
            })
    }));
    Ok(())
}

#[test]
fn python_import_resolution_publishes_truthful_spanned_provenance() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let multiline_import = "from pkg.api import (\n    Widget,\n)";
    let caller_source = format!(
        "# from pkg import commented\n\
         TEXT = \"\"\"\n\
         from pkg import stringy\n\
         \"\"\"\n\
         {multiline_import}\n\
         from pkg import mod\n\
         class Consumer:\n\
             item: Widget\n"
    );
    let files = [
        ("caller.py", caller_source.as_str()),
        ("pkg/__init__.py", "from .api import Widget\n"),
        ("pkg/api.py", "class Widget:\n    pass\n"),
        ("pkg/mod.py", "VALUE = 1\n"),
        ("pkg/commented.py", "COMMENTED = True\n"),
        ("pkg/stringy.py", "STRINGY = True\n"),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = write(root, relative, source)?;
        extractions.push(engine.extract(Path::new(&path))?);
        sources.insert(path, source.to_owned());
    }

    let mut extraction = resolve_with_root(&extractions, &sources, root);
    let resolver_edges = extraction
        .edges
        .iter()
        .filter(|edge| is_python_import_edge(edge))
        .collect::<Vec<_>>();
    let rules = resolver_edges
        .iter()
        .map(|edge| edge.string("rule"))
        .collect::<HashSet<_>>();
    for rule in [PYTHON_SYMBOL_IMPORT_RULE, PYTHON_REEXPORT_RULE] {
        assert!(rules.contains(rule), "missing raw resolver rule {rule}");
    }
    assert!(
        !rules.contains("python-imported-class-use-inference"),
        "unused imports must not create relationships to every class"
    );
    assert!(resolver_edges.iter().all(|edge| {
        edge.string("language") == "python" && edge.string("extractor") == PYTHON_IMPORT_PRODUCER
    }));
    assert!(resolver_edges.iter().all(|edge| {
        edge.string("_origin") == "ast" && edge.string("confidence") == "EXTRACTED"
    }));

    let multiline_import_spans =
        import_occurrence_spans_in_source(multiline_import, &caller_source)?;
    let (expected_start, expected_end) = multiline_import_spans
        .first()
        .copied()
        .ok_or("missing multiline import item span")?;
    let multiline_edge = resolver_edges
        .iter()
        .find(|edge| {
            edge.string("rule") == PYTHON_SYMBOL_IMPORT_RULE
                && edge.string("source_file").ends_with("caller.py")
        })
        .ok_or("missing multiline symbol import")?;
    assert_eq!(
        multiline_edge
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64),
        Some(expected_start as u64)
    );
    assert_eq!(
        multiline_edge
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64),
        Some(expected_end as u64)
    );
    assert_eq!(
        multiline_edge
            .attributes
            .get("line_end")
            .and_then(serde_json::Value::as_u64)
            .zip(
                multiline_edge
                    .attributes
                    .get("line_start")
                    .and_then(serde_json::Value::as_u64)
            )
            .map(|(end, start)| end - start),
        Some(0)
    );

    let ignored_targets = extraction
        .nodes
        .iter()
        .filter(|node| {
            let source_file = node.string("source_file");
            source_file.ends_with("pkg/commented.py") || source_file.ends_with("pkg/stringy.py")
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(!ignored_targets.is_empty());
    assert!(
        resolver_edges
            .iter()
            .all(|edge| !ignored_targets.contains(edge.target.as_str())),
        "commented or string-contained imports produced resolver edges"
    );

    let node_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    extraction.edges.retain(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    });
    let evidence =
        BuildEvidence::from_extraction(root, &extraction, "sha256:python-import-provenance")?;
    let graph = normalize_v1(extraction, evidence)?;
    let published = graph
        .links
        .iter()
        .flat_map(|edge| {
            edge.evidence
                .iter()
                .filter(|evidence| is_python_import_evidence(evidence))
                .map(move |evidence| (edge, evidence))
        })
        .collect::<Vec<_>>();
    let published_rules = published
        .iter()
        .filter_map(|(_, evidence)| evidence.rule.as_deref())
        .collect::<HashSet<_>>();
    for rule in [PYTHON_SYMBOL_IMPORT_RULE, PYTHON_REEXPORT_RULE] {
        assert!(
            published_rules.contains(rule),
            "missing published resolver rule {rule}"
        );
    }
    for (edge, evidence) in &published {
        let _ = edge;
        assert_eq!(evidence.origin, EvidenceOrigin::Ast);
        assert_eq!(evidence.confidence, EvidenceConfidence::Exact);
        assert_eq!(evidence.anchors.len(), 1);
        assert!(evidence.wiring_site.is_none());
    }
    let multiline_evidence = published
        .iter()
        .map(|(_, evidence)| *evidence)
        .find(|evidence| {
            evidence.rule.as_deref() == Some(PYTHON_SYMBOL_IMPORT_RULE)
                && evidence.anchors.first().is_some_and(|anchor| {
                    anchor.file == "caller.py" && anchor.start_byte == expected_start as u64
                })
        })
        .ok_or("missing published multiline import evidence")?;
    let anchor = multiline_evidence
        .anchors
        .first()
        .ok_or("missing multiline anchor")?;
    assert_eq!(anchor.start_byte, expected_start as u64);
    assert_eq!(anchor.end_byte, expected_end as u64);
    assert_eq!(anchor.end_line - anchor.start_line, 0);
    Ok(())
}

#[test]
fn repeated_python_import_occurrences_survive_resolution_and_publication()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "caller.py",
            "from pkg.api import Widget, Widget as WidgetAlias\n\
             from pkg import mod, mod as mod_alias\n",
        ),
        (
            "pkg/__init__.py",
            "from .api import Widget, Widget as AliasWidget\n",
        ),
        ("pkg/api.py", "class Widget:\n    pass\n"),
        ("pkg/mod.py", "VALUE = 1\n"),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = write(root, relative, source)?;
        extractions.push(engine.extract(Path::new(&path))?);
        sources.insert(path, source.to_owned());
    }
    let mut forward = resolve_with_root(&extractions, &sources, root);
    let resolver_edges = forward
        .edges
        .iter()
        .filter(|edge| is_python_import_edge(edge))
        .collect::<Vec<_>>();
    let rule_counts = resolver_edges
        .iter()
        .fold(HashMap::new(), |mut counts, edge| {
            *counts.entry(edge.string("rule")).or_insert(0_usize) += 1;
            counts
        });
    assert_eq!(
        rule_counts.get(PYTHON_SYMBOL_IMPORT_RULE),
        Some(&4),
        "rules={rule_counts:?}"
    );
    assert_eq!(rule_counts.get(PYTHON_REEXPORT_RULE), Some(&2));

    let occurrence_identities = resolver_edges
        .iter()
        .filter_map(|edge| {
            let rule = edge
                .attributes
                .get("_occurrence_rule")
                .and_then(serde_json::Value::as_str)?;
            Some((
                rule,
                edge.string("source_file"),
                edge.attributes
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64),
                edge.attributes
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64),
            ))
        })
        .collect::<HashSet<_>>();
    assert_eq!(occurrence_identities.len(), resolver_edges.len());
    assert!(occurrence_identities.iter().all(|(rule, ..)| {
        rule.starts_with("universal-import-") || rule.starts_with("universal-reexport-")
    }));
    let repeated_connectivity = resolver_edges
        .iter()
        .fold(HashMap::new(), |mut counts, edge| {
            *counts
                .entry((&edge.source, &edge.target, edge.string("relation")))
                .or_insert(0_usize) += 1;
            counts
        });
    assert_eq!(
        repeated_connectivity
            .values()
            .filter(|count| **count == 2)
            .count(),
        3,
        "expected repeated symbol, submodule, and re-export connectivity"
    );

    let forward_nodes = forward
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    forward.edges.retain(|edge| {
        forward_nodes.contains(edge.source.as_str()) && forward_nodes.contains(edge.target.as_str())
    });
    let forward_evidence =
        BuildEvidence::from_extraction(root, &forward, "sha256:python-import-occurrences")?;
    let forward_graph = normalize_v1(forward, forward_evidence)?;
    let forward_ids = forward_graph
        .links
        .iter()
        .filter(|edge| edge.evidence.iter().any(is_python_import_evidence))
        .map(|edge| edge.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(forward_ids.len(), 6);

    extractions.reverse();
    let mut reversed = resolve_with_root(&extractions, &sources, root);
    let reversed_nodes = reversed
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    reversed.edges.retain(|edge| {
        reversed_nodes.contains(edge.source.as_str())
            && reversed_nodes.contains(edge.target.as_str())
    });
    let reversed_evidence =
        BuildEvidence::from_extraction(root, &reversed, "sha256:python-import-occurrences")?;
    let reversed_graph = normalize_v1(reversed, reversed_evidence)?;
    let reversed_ids = reversed_graph
        .links
        .iter()
        .filter(|edge| edge.evidence.iter().any(is_python_import_evidence))
        .map(|edge| edge.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(reversed_ids, forward_ids);
    Ok(())
}

#[test]
fn backslash_continued_python_imports_have_complete_crlf_spans_and_fail_closed()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let continued_symbols = concat!(
        "from pkg.api import Widget, \\\r\n",
        "    Widget as AliasWidget, \\\r\n",
        "    helper",
    );
    let continued_submodules = concat!("from pkg import mod, \\\r\n", "    mod as mod_alias");
    let caller_source = [
        "# from pkg.api import Commented\r\n",
        "TEXT = \"from pkg.api import Stringy\"\r\n",
        "BLOCK = \"\"\"\r\nfrom pkg.api import TripleString\r\n\"\"\"\r\n",
        continued_symbols,
        "  # trailing comment\r\n",
        continued_submodules,
        "\r\n",
        "from pkg.api import Broken, \\  \r\n",
        "    Broken as ignored\r\n",
        "from pkg.api import (\r\n",
        "    Missing,\r\n",
        "from pkg.api import helper as recovered\r\n",
    ]
    .concat();
    let files = [
        ("caller.py", caller_source.as_str()),
        ("pkg/__init__.py", "# package\n"),
        (
            "pkg/api.py",
            "class Widget:\n    pass\n\
             def helper():\n    return 1\n\
             class Commented:\n    pass\n\
             class Stringy:\n    pass\n\
             class TripleString:\n    pass\n\
             class Broken:\n    pass\n\
             class Missing:\n    pass\n",
        ),
        ("pkg/mod.py", "VALUE = 1\n"),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = write(root, relative, source)?;
        extractions.push(engine.extract(Path::new(&path))?);
        sources.insert(path, source.to_owned());
    }

    let mut extraction = resolve_with_root(&extractions, &sources, root);
    let resolver_edges = extraction
        .edges
        .iter()
        .filter(|edge| {
            is_python_import_edge(edge) && edge.string("source_file").ends_with("caller.py")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolver_edges.len(),
        5,
        "comments, strings, malformed continuations, and parser-error recovery regions must not emit imports"
    );

    let symbol_spans = import_occurrence_spans_in_source(continued_symbols, &caller_source)?;
    assert_eq!(symbol_spans.len(), 3);
    let symbol_edges = resolver_edges
        .iter()
        .filter(|edge| {
            edge.attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|start| symbol_spans.iter().any(|(s, _)| *s == start as usize))
        })
        .collect::<Vec<_>>();
    assert_eq!(symbol_edges.len(), 3);
    assert!(symbol_edges.iter().all(|edge| {
        let Some(start) = edge
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
        else {
            return false;
        };
        let Some((_, end)) = symbol_spans
            .iter()
            .find(|(expected_start, _)| *expected_start == start)
            .copied()
        else {
            return false;
        };

        edge.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            == Some(end as u64)
            && edge
                .attributes
                .get("line_end")
                .and_then(serde_json::Value::as_u64)
                .zip(
                    edge.attributes
                        .get("line_start")
                        .and_then(serde_json::Value::as_u64),
                )
                .is_some_and(|(end, start)| end == start)
    }));

    let submodule_spans = import_occurrence_spans_in_source(continued_submodules, &caller_source)?;
    assert_eq!(submodule_spans.len(), 2);
    let submodule_edges = resolver_edges
        .iter()
        .filter(|edge| {
            edge.attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|start| submodule_spans.iter().any(|(s, _)| *s == start as usize))
        })
        .collect::<Vec<_>>();
    assert_eq!(submodule_edges.len(), 2);
    assert!(submodule_edges.iter().all(|edge| {
        let Some(start) = edge
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
        else {
            return false;
        };
        let Some((_, end)) = submodule_spans
            .iter()
            .find(|(expected_start, _)| *expected_start == start)
            .copied()
        else {
            return false;
        };

        edge.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            == Some(end as u64)
    }));

    let malformed_starts = ["from pkg.api import Broken", "from pkg.api import ("]
        .into_iter()
        .map(|statement| {
            caller_source
                .find(statement)
                .ok_or("missing malformed import")
        })
        .collect::<Result<HashSet<_>, _>>()?;
    assert!(resolver_edges.iter().all(|edge| {
        edge.attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|start| !malformed_starts.contains(&(start as usize)))
    }));

    let node_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    extraction.edges.retain(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    });
    let evidence =
        BuildEvidence::from_extraction(root, &extraction, "sha256:python-import-continuations")?;
    let graph = normalize_v1(extraction, evidence)?;
    let published_symbol_anchors = graph
        .links
        .iter()
        .flat_map(|edge| &edge.evidence)
        .filter(|evidence| {
            evidence.extractor == PYTHON_IMPORT_PRODUCER
                && evidence.rule.as_deref() == Some(PYTHON_SYMBOL_IMPORT_RULE)
                && evidence.anchors.first().is_some_and(|anchor| {
                    symbol_spans
                        .iter()
                        .any(|(start, _)| anchor.start_byte == *start as u64)
                })
        })
        .filter_map(|evidence| evidence.anchors.first())
        .collect::<Vec<_>>();
    assert_eq!(published_symbol_anchors.len(), 3);
    assert!(published_symbol_anchors.iter().all(|anchor| {
        symbol_spans.iter().any(|(start, end)| {
            anchor.start_byte == *start as u64 && anchor.end_byte == *end as u64
        })
    }));
    Ok(())
}

#[test]
fn python_import_token_grammar_is_atomic_and_span_stable() -> Result<(), Box<dyn Error>> {
    let statements = [
        "from pkg import(Widget)",
        "from pkg\timport Widget as TabWidget",
        "from pkg import *",
        "from pkg import *, Widget",
        "from pkg import Widget, *",
        "from pkg import (*)",
        "from pkg import (*, Widget)",
        "from pkg import (Widget, *)",
        "fromx pkg import Widget",
        "from pkg importWidget",
        "from pkg import helper as recovered",
    ];
    let valid_statements = [statements[0], statements[1], statements[2], statements[10]];
    let mut newline_snapshots = Vec::new();

    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let caller_source = statements.join(newline) + newline;
        let files = [
            ("caller.py", caller_source.as_str()),
            (
                "pkg/__init__.py",
                "class Widget:\n    pass\n\ndef helper():\n    return 1\n",
            ),
        ];
        let mut engine = Engine::default();
        let mut extractions = Vec::new();
        let mut sources = HashMap::new();
        for (relative, source) in files {
            let path = write(root, relative, source)?;
            extractions.push(engine.extract(Path::new(&path))?);
            sources.insert(path, source.to_owned());
        }

        let mut extraction = resolve_with_root(&extractions, &sources, root);
        let resolver_edges = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.string("extractor") == PYTHON_IMPORT_PRODUCER
                    && edge.string("rule") == PYTHON_SYMBOL_IMPORT_RULE
                    && edge.string("source_file").ends_with("caller.py")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resolver_edges.len(),
            valid_statements.len(),
            "valid wildcards must be exact while keyword-prefix near matches and malformed statements emit no partial facts: {resolver_edges:#?}"
        );

        let mut expected_spans = valid_statements
            .iter()
            .map(|statement| import_occurrence_spans_in_source(statement, &caller_source))
            .collect::<Result<Vec<Vec<_>>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        expected_spans.sort_unstable();
        let mut raw_spans = resolver_edges
            .iter()
            .map(|edge| {
                let start = edge
                    .attributes
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("missing raw import start")? as usize;
                let end = edge
                    .attributes
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("missing raw import end")? as usize;
                Ok((start, end))
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        raw_spans.sort_unstable();
        assert_eq!(raw_spans, expected_spans);
        for (start, end) in &raw_spans {
            let span = caller_source
                .get(*start..*end)
                .ok_or("raw import span is not a UTF-8 boundary")?;
            assert!(
                valid_statements
                    .iter()
                    .any(|statement| statement.contains(span)),
                "raw import span must come from a valid statement"
            );
        }

        let node_ids = extraction
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        extraction.edges.retain(|edge| {
            node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
        });
        let evidence =
            BuildEvidence::from_extraction(root, &extraction, "sha256:python-import-grammar")?;
        let graph = normalize_v1(extraction, evidence)?;
        let published_anchors = graph
            .links
            .iter()
            .flat_map(|edge| &edge.evidence)
            .filter(|evidence| {
                evidence.extractor == PYTHON_IMPORT_PRODUCER
                    && evidence.rule.as_deref() == Some(PYTHON_SYMBOL_IMPORT_RULE)
                    && evidence
                        .anchors
                        .first()
                        .is_some_and(|anchor| anchor.file == "caller.py")
            })
            .filter_map(|evidence| evidence.anchors.first())
            .collect::<Vec<_>>();
        assert_eq!(published_anchors.len(), valid_statements.len());

        let mut published_spans = published_anchors
            .iter()
            .map(|anchor| (anchor.start_byte as usize, anchor.end_byte as usize))
            .collect::<Vec<_>>();
        published_spans.sort_unstable();
        assert_eq!(published_spans, expected_spans);
        let mut snapshot = Vec::new();
        for anchor in published_anchors {
            snapshot.push((
                caller_source
                    .get(anchor.start_byte as usize..anchor.end_byte as usize)
                    .ok_or("published import span is not a UTF-8 boundary")?
                    .to_owned(),
                anchor.start_line,
                anchor.start_column,
                anchor.end_line,
                anchor.end_column,
            ));
        }
        snapshot.sort();
        newline_snapshots.push(snapshot);
    }

    assert_eq!(newline_snapshots[0], newline_snapshots[1]);
    Ok(())
}

#[test]
fn qualified_external_python_calls_are_canonical_internally_and_source_scoped_on_publication()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "first.py",
            "from unittest import mock\n\
             def first():\n    mock.patch('service.first')\n",
        ),
        (
            "second.py",
            "from unittest import mock\n\
             def second():\n    mock.patch('service.second')\n",
        ),
        (
            "other.py",
            "from vendor import mock\n\
             def other():\n    mock.patch('service.other')\n",
        ),
        (
            "ambiguous.py",
            "from unittest import mock\n\
             from vendor import mock\n\
             def ambiguous():\n    mock.patch('service.ambiguous')\n",
        ),
        ("pkg/__init__.py", ""),
        ("pkg/mock.py", "def patch(value):\n    return value\n"),
        (
            "internal.py",
            "from pkg import mock\n\
             def internal():\n    mock.patch('service.internal')\n",
        ),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = write(root, relative, source)?;
        extractions.push(engine.extract(Path::new(&path))?);
        sources.insert(path, source.to_owned());
    }

    let mut extraction = resolve_with_root(&extractions, &sources, root);
    let placeholders = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.string("extractor") == "compass.resolve.python.universal"
                && node.string("source_file").is_empty()
                && node.string("external_role") == "calls"
        })
        .collect::<Vec<_>>();
    assert_eq!(placeholders.len(), 2);
    assert!(placeholders.iter().all(|node| {
        node.attributes
            .get("external")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }));
    assert_eq!(
        placeholders
            .iter()
            .filter(|node| node.string("qualified_name") == "unittest.mock.patch")
            .count(),
        1,
        "the same qualified external symbol must have one canonical node"
    );
    assert_eq!(
        placeholders
            .iter()
            .filter(|node| node.string("qualified_name") == "vendor.mock.patch")
            .count(),
        1,
        "the later import is the active binding in ambiguous.py"
    );
    assert!(
        placeholders
            .iter()
            .all(|node| node.string("qualified_name") != "pkg.mock.patch")
    );

    let placeholder_ids = placeholders
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let external_edges = extraction
        .edges
        .iter()
        .filter(|edge| placeholder_ids.contains(edge.target.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(external_edges.len(), 4);
    assert!(external_edges.iter().all(|edge| {
        edge.string("relation") == "calls"
            && edge.string("context") == "external_call"
            && edge.string("confidence") == "INFERRED"
            && edge
                .attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .zip(
                    edge.attributes
                        .get("end_byte")
                        .and_then(serde_json::Value::as_u64),
                )
                .is_some_and(|(start, end)| start < end)
    }));

    let node_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    extraction.edges.retain(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    });
    let evidence =
        BuildEvidence::from_extraction(root, &extraction, "sha256:python-external-calls")?;
    let graph = normalize_v1(extraction, evidence)?;
    let published_placeholders = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == NodeKind::Function
                && node.source.is_none()
                && matches!(
                    node.qualified_name.as_str(),
                    "unittest.mock.patch" | "vendor.mock.patch"
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        published_placeholders.len(),
        4,
        "published wiring-scoped placeholders: {published_placeholders:#?}"
    );
    for qualified_name in ["unittest.mock.patch", "vendor.mock.patch"] {
        assert_eq!(
            published_placeholders
                .iter()
                .filter(|node| node.qualified_name == qualified_name)
                .count(),
            2,
            "each external call site must retain a separate unresolved identity"
        );
    }
    assert!(published_placeholders.iter().all(|node| {
        node.evidence.iter().any(|evidence| {
            evidence.origin == EvidenceOrigin::Heuristic
                && evidence.confidence == EvidenceConfidence::Inferred
                && evidence.rule.as_deref() == Some("external-symbol-placeholder")
                && evidence.anchors.is_empty()
                && evidence.wiring_site.is_some()
        })
    }));
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                edge.kind == EdgeKind::Calls
                    && published_placeholders
                        .iter()
                        .any(|node| node.id == edge.target)
            })
            .count(),
        4
    );
    Ok(())
}

#[test]
fn python_import_keywords_and_whitespace_are_lexically_exact() -> Result<(), Box<dyn Error>> {
    let hard_keywords = [
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ];
    let mut valid_statements = vec![
        "from pkg import Widget".to_owned(),
        "from\tpkg\u{000C}import\tWidget\u{000C}as\u{000C}TabFormWidget".to_owned(),
        "from pkg import _".to_owned(),
        "from pkg import case".to_owned(),
        "from pkg import match".to_owned(),
        "from pkg import type".to_owned(),
        "from pkg import classifier as subclass".to_owned(),
        "from pkg import fromage as reimport".to_owned(),
    ];
    let mut statements = valid_statements.clone();
    for keyword in hard_keywords {
        statements.push(format!("from pkg import {keyword}"));
        statements.push(format!("from pkg import Widget as {keyword}"));
    }
    for whitespace in ['\u{00A0}', '\u{2003}', '\u{202F}'] {
        statements.extend([
            format!("{whitespace}from pkg import Widget"),
            format!("from{whitespace}pkg import Widget"),
            format!("from pkg{whitespace}import Widget"),
            format!("from pkg import{whitespace}Widget"),
            format!("from pkg import Widget{whitespace}as Alias"),
            format!("from pkg import Widget as{whitespace}Alias"),
            format!("from pkg import Widget,{whitespace}helper"),
            format!("from pkg import Widget{whitespace}"),
        ]);
    }
    let recovered = "from pkg import helper as recovered".to_owned();
    statements.push(recovered.clone());
    valid_statements.push(recovered);
    let mut newline_snapshots = Vec::new();

    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let caller_source = statements.join(newline) + newline;
        let mut engine = Engine::default();
        let mut extractions = Vec::new();
        let mut sources = HashMap::new();

        let caller_path = write(root, "caller.py", &caller_source)?;
        extractions.push(engine.extract(Path::new(&caller_path))?);
        sources.insert(caller_path.clone(), caller_source.clone());
        let package_source = concat!(
            "class Widget:\n    pass\n",
            "def _():\n    pass\n",
            "def case():\n    pass\n",
            "def match():\n    pass\n",
            "def type():\n    pass\n",
            "def classifier():\n    pass\n",
            "def fromage():\n    pass\n",
            "def helper():\n    pass\n",
        );
        let package_path = write(root, "pkg/__init__.py", package_source)?;
        extractions.push(engine.extract(Path::new(&package_path))?);
        sources.insert(package_path, package_source.to_owned());
        for keyword in hard_keywords {
            let relative = format!("pkg/{keyword}.py");
            let module_path = write(root, &relative, "VALUE = 1\n")?;
            extractions.push(engine.extract(Path::new(&module_path))?);
            sources.insert(module_path, "VALUE = 1\n".to_owned());
        }

        let mut extraction = resolve_with_root(&extractions, &sources, root);
        let resolver_edges = extraction
            .edges
            .iter()
            .filter(|edge| {
                is_python_import_edge(edge) && edge.string("source_file").ends_with("caller.py")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resolver_edges.len(),
            valid_statements.len(),
            "hard keywords and non-Python whitespace must not emit exact resolver facts"
        );

        let mut expected_spans = valid_statements
            .iter()
            .map(|statement| import_occurrence_spans_in_source(statement, &caller_source))
            .collect::<Result<Vec<Vec<_>>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        expected_spans.sort_unstable();
        let mut raw_spans = resolver_edges
            .iter()
            .map(|edge| {
                let start =
                    edge.attributes
                        .get("start_byte")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("missing raw keyword-boundary start")? as usize;
                let end = edge
                    .attributes
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("missing raw keyword-boundary end")? as usize;
                Ok((start, end))
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        raw_spans.sort_unstable();
        assert_eq!(raw_spans, expected_spans);

        let node_ids = extraction
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        extraction.edges.retain(|edge| {
            node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
        });
        let evidence =
            BuildEvidence::from_extraction(root, &extraction, "sha256:python-import-lexing")?;
        let graph = normalize_v1(extraction, evidence)?;
        let published_anchors = graph
            .links
            .iter()
            .flat_map(|edge| &edge.evidence)
            .filter(|evidence| {
                is_python_import_evidence(evidence)
                    && evidence
                        .anchors
                        .first()
                        .is_some_and(|anchor| anchor.file == "caller.py")
            })
            .filter_map(|evidence| evidence.anchors.first())
            .collect::<Vec<_>>();
        assert_eq!(published_anchors.len(), valid_statements.len());

        let mut published_spans = published_anchors
            .iter()
            .map(|anchor| (anchor.start_byte as usize, anchor.end_byte as usize))
            .collect::<Vec<_>>();
        published_spans.sort_unstable();
        assert_eq!(published_spans, expected_spans);
        let mut snapshot = Vec::new();
        for anchor in published_anchors {
            let span = caller_source
                .get(anchor.start_byte as usize..anchor.end_byte as usize)
                .ok_or("published keyword-boundary span is not a UTF-8 boundary")?;
            assert!(valid_statements.iter().any(|valid| valid.contains(span)));
            snapshot.push((
                span.to_owned(),
                anchor.start_line,
                anchor.start_column,
                anchor.end_line,
                anchor.end_column,
            ));
        }
        snapshot.sort();
        newline_snapshots.push(snapshot);
    }

    assert_eq!(newline_snapshots[0], newline_snapshots[1]);
    Ok(())
}
