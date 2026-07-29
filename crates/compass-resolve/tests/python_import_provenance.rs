use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::EdgeKind;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};
use compass_resolve::resolve_with_root;

const PYTHON_IMPORT_PRODUCER: &str = "compass.resolve.python-imports";

fn write(root: &Path, relative: &str, source: &str) -> Result<String, Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, source)?;
    Ok(path.to_string_lossy().into_owned())
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
        .filter(|edge| edge.string("extractor") == PYTHON_IMPORT_PRODUCER)
        .collect::<Vec<_>>();
    let rules = resolver_edges
        .iter()
        .map(|edge| edge.string("rule"))
        .collect::<HashSet<_>>();
    for rule in [
        "python-symbol-import-resolution",
        "python-submodule-import-resolution",
        "python-module-re-export-resolution",
        "python-imported-class-use-inference",
    ] {
        assert!(rules.contains(rule), "missing raw resolver rule {rule}");
    }
    assert!(resolver_edges.iter().all(|edge| {
        edge.string("language") == "python" && edge.string("extractor") == PYTHON_IMPORT_PRODUCER
    }));
    assert!(resolver_edges.iter().all(|edge| {
        if edge.string("rule") == "python-imported-class-use-inference" {
            edge.string("_origin") == "heuristic"
                && edge.string("confidence") == "INFERRED"
                && edge
                    .attributes
                    .get("confidence_score")
                    .and_then(serde_json::Value::as_f64)
                    == Some(0.8)
        } else {
            edge.string("_origin") == "convention" && edge.string("confidence") == "EXTRACTED"
        }
    }));

    let caller_path = root.join("caller.py").to_string_lossy().into_owned();
    let expected_start = caller_source
        .find(multiline_import)
        .ok_or("missing multiline import")?;
    let expected_end = expected_start + multiline_import.len();
    let multiline_edge = resolver_edges
        .iter()
        .find(|edge| {
            edge.string("rule") == "python-symbol-import-resolution"
                && edge.string("source_file") == caller_path
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
        Some(2)
    );

    let ignored_targets = extraction
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("source_file")
                    .strip_prefix(root.to_str().unwrap_or_default()),
                Some("/pkg/commented.py" | "/pkg/stringy.py")
            )
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
                .filter(|evidence| evidence.extractor == PYTHON_IMPORT_PRODUCER)
                .map(move |evidence| (edge, evidence))
        })
        .collect::<Vec<_>>();
    let published_rules = published
        .iter()
        .filter_map(|(_, evidence)| evidence.rule.as_deref())
        .collect::<HashSet<_>>();
    for rule in [
        "python-symbol-import-resolution",
        "python-submodule-import-resolution",
        "python-module-re-export-resolution",
        "python-imported-class-use-inference",
    ] {
        assert!(
            published_rules.contains(rule),
            "missing published resolver rule {rule}"
        );
    }
    for (edge, evidence) in &published {
        if evidence.rule.as_deref() == Some("python-imported-class-use-inference") {
            assert_eq!(edge.kind, EdgeKind::References);
            assert_eq!(evidence.origin, EvidenceOrigin::Heuristic);
            assert_eq!(evidence.confidence, EvidenceConfidence::Inferred);
            assert!(evidence.anchors.is_empty());
            assert_eq!(evidence.score, Some(0.8));
            assert!(evidence.wiring_site.is_some());
        } else {
            assert_eq!(evidence.origin, EvidenceOrigin::Convention);
            assert_eq!(evidence.confidence, EvidenceConfidence::Exact);
            assert_eq!(evidence.anchors.len(), 1);
            assert!(evidence.wiring_site.is_none());
        }
    }
    let multiline_evidence = published
        .iter()
        .map(|(_, evidence)| *evidence)
        .find(|evidence| {
            evidence.rule.as_deref() == Some("python-symbol-import-resolution")
                && evidence
                    .anchors
                    .first()
                    .is_some_and(|anchor| anchor.file == "caller.py")
        })
        .ok_or("missing published multiline import evidence")?;
    let anchor = multiline_evidence
        .anchors
        .first()
        .ok_or("missing multiline anchor")?;
    assert_eq!(anchor.start_byte, expected_start as u64);
    assert_eq!(anchor.end_byte, expected_end as u64);
    assert_eq!(anchor.end_line - anchor.start_line, 2);
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
        .filter(|edge| edge.string("extractor") == PYTHON_IMPORT_PRODUCER)
        .collect::<Vec<_>>();
    let rule_counts = resolver_edges
        .iter()
        .fold(HashMap::new(), |mut counts, edge| {
            *counts.entry(edge.string("rule")).or_insert(0_usize) += 1;
            counts
        });
    assert_eq!(rule_counts.get("python-symbol-import-resolution"), Some(&4));
    assert_eq!(
        rule_counts.get("python-submodule-import-resolution"),
        Some(&2)
    );
    assert_eq!(
        rule_counts.get("python-module-re-export-resolution"),
        Some(&2)
    );

    let occurrence_rules = resolver_edges
        .iter()
        .filter_map(|edge| {
            edge.attributes
                .get("_occurrence_rule")
                .and_then(serde_json::Value::as_str)
        })
        .collect::<HashSet<_>>();
    assert_eq!(occurrence_rules.len(), resolver_edges.len());
    for alias in ["WidgetAlias", "mod_alias", "AliasWidget"] {
        assert!(
            occurrence_rules.iter().any(|rule| rule.ends_with(alias)),
            "missing alias in occurrence identity: {alias}"
        );
    }
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
        4,
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
        .filter(|edge| {
            edge.evidence
                .iter()
                .any(|evidence| evidence.extractor == PYTHON_IMPORT_PRODUCER)
        })
        .map(|edge| edge.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(forward_ids.len(), 8);

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
        .filter(|edge| {
            edge.evidence
                .iter()
                .any(|evidence| evidence.extractor == PYTHON_IMPORT_PRODUCER)
        })
        .map(|edge| edge.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(reversed_ids, forward_ids);
    Ok(())
}

#[test]
fn backslash_continued_python_imports_have_complete_crlf_spans_and_recover()
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
    let caller_path = root.join("caller.py").to_string_lossy().into_owned();
    let resolver_edges = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.string("extractor") == PYTHON_IMPORT_PRODUCER
                && edge.string("source_file") == caller_path
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolver_edges.len(),
        6,
        "comments, strings, and malformed continuations must not emit imports"
    );

    let symbol_start = caller_source
        .find(continued_symbols)
        .ok_or("missing continued symbol statement")?;
    let symbol_end = symbol_start + continued_symbols.len();
    let symbol_edges = resolver_edges
        .iter()
        .filter(|edge| {
            edge.attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                == Some(symbol_start as u64)
        })
        .collect::<Vec<_>>();
    assert_eq!(symbol_edges.len(), 3);
    assert!(symbol_edges.iter().all(|edge| {
        edge.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            == Some(symbol_end as u64)
            && edge
                .attributes
                .get("line_end")
                .and_then(serde_json::Value::as_u64)
                .zip(
                    edge.attributes
                        .get("line_start")
                        .and_then(serde_json::Value::as_u64),
                )
                .is_some_and(|(end, start)| end - start == 2)
    }));

    let submodule_start = caller_source
        .find(continued_submodules)
        .ok_or("missing continued submodule statement")?;
    let submodule_end = submodule_start + continued_submodules.len();
    let submodule_edges = resolver_edges
        .iter()
        .filter(|edge| {
            edge.attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                == Some(submodule_start as u64)
        })
        .collect::<Vec<_>>();
    assert_eq!(submodule_edges.len(), 2);
    assert!(submodule_edges.iter().all(|edge| {
        edge.attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            == Some(submodule_end as u64)
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
                && evidence.rule.as_deref() == Some("python-symbol-import-resolution")
                && evidence
                    .anchors
                    .first()
                    .is_some_and(|anchor| anchor.start_byte == symbol_start as u64)
        })
        .filter_map(|evidence| evidence.anchors.first())
        .collect::<Vec<_>>();
    assert_eq!(published_symbol_anchors.len(), 3);
    assert!(published_symbol_anchors.iter().all(|anchor| {
        anchor.end_byte == symbol_end as u64 && anchor.end_line - anchor.start_line == 2
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
    let valid_statements = [statements[0], statements[1], statements[10]];
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
        let caller_path = root.join("caller.py").to_string_lossy().into_owned();
        let resolver_edges = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.string("extractor") == PYTHON_IMPORT_PRODUCER
                    && edge.string("rule") == "python-symbol-import-resolution"
                    && edge.string("source_file") == caller_path
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resolver_edges.len(),
            valid_statements.len(),
            "wildcards, keyword-prefix near matches, and malformed statements must not emit exact partial facts"
        );

        let mut expected_spans = valid_statements
            .iter()
            .map(|statement| {
                caller_source
                    .find(statement)
                    .map(|start| (start, start + statement.len()))
                    .ok_or("missing valid import statement")
            })
            .collect::<Result<Vec<_>, _>>()?;
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
            assert!(
                valid_statements.contains(
                    &caller_source
                        .get(*start..*end)
                        .ok_or("raw import span is not a UTF-8 boundary")?
                ),
                "raw import span must cover its complete valid statement"
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
                    && evidence.rule.as_deref() == Some("python-symbol-import-resolution")
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
                edge.string("extractor") == PYTHON_IMPORT_PRODUCER
                    && edge.string("source_file") == caller_path
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resolver_edges.len(),
            valid_statements.len(),
            "hard keywords and non-Python whitespace must not emit exact resolver facts"
        );

        let mut expected_spans = valid_statements
            .iter()
            .map(|statement| {
                caller_source
                    .find(statement)
                    .map(|start| (start, start + statement.len()))
                    .ok_or("missing valid keyword-boundary import")
            })
            .collect::<Result<Vec<_>, _>>()?;
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
                evidence.extractor == PYTHON_IMPORT_PRODUCER
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
            let statement = caller_source
                .get(anchor.start_byte as usize..anchor.end_byte as usize)
                .ok_or("published keyword-boundary span is not a UTF-8 boundary")?;
            assert!(valid_statements.iter().any(|valid| valid == statement));
            snapshot.push((
                statement.to_owned(),
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
