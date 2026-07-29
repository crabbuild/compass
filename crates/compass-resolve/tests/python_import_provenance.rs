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
