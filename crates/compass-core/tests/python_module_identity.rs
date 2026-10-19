use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn sources(root: &Path, package: bool) -> TestResult {
    fs::create_dir_all(root)?;
    let relative = if package { "." } else { "" };
    let mut factory =
        format!("from pathlib import Path\nfrom {relative}validation import intact\n\n");
    factory.push_str(concat!(
        "def open_snapshot(path: Path) -> str:\n",
        "    if not intact(path):\n",
        "        return create_snapshot(path)\n",
        "    return path.read_text()\n\n",
        "def create_snapshot(path: Path) -> str:\n",
        "    path.write_text(\"{}\")\n",
        "    return \"{}\"\n",
    ));
    fs::write(root.join("factory.py"), factory)?;
    fs::write(
        root.join("validation.py"),
        concat!(
            "import json\nfrom pathlib import Path\n\n",
            "def intact(path: Path) -> bool:\n",
            "    try:\n",
            "        json.loads(path.read_text())\n",
            "    except (OSError, ValueError):\n",
            "        return False\n",
            "    return True\n",
        ),
    )?;
    fs::write(
        root.join("disconnected.py"),
        "def intact(path):\n    return False\n",
    )?;
    let mut recovery = format!("from {relative}factory import open_snapshot\n\n");
    recovery.push_str(concat!(
        "def test_damaged_local_indexes_are_checked_and_restored(tmp_path):\n",
        "    path = tmp_path / \"state.json\"\n",
        "    path.write_text(\"{\")\n",
        "    assert open_snapshot(path) == \"{}\"\n",
        "    assert open_snapshot(path) == \"{}\"\n",
    ));
    fs::write(root.join("test_recovery.py"), recovery)?;
    if package {
        fs::write(
            root.join("__init__.py"),
            "from .factory import open_snapshot\n",
        )?;
    }
    Ok(())
}

fn build(root: &Path, output: &Path) -> TestResult<GraphDocument> {
    build_with_force(root, output, false)
}

fn build_with_force(root: &Path, output: &Path, force: bool) -> TestResult<GraphDocument> {
    let mut options = BuildOptions::new(root);
    options.output_root = Some(output.to_path_buf());
    options.graph_storage = GraphStorage::Json;
    options.no_cluster = true;
    options.no_viz = true;
    options.force = force;
    options.max_workers = Some(1);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let built = build_local_graph(&options)?;
    Ok(GraphDocument::load(&built.output_dir.join("graph.json"))?)
}

fn assert_calls(root: &Path, graph: &GraphDocument, prefix: &str) -> TestResult<BTreeSet<String>> {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut observed = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for edge in graph.links.iter().filter(|e| e.kind == EdgeKind::Calls) {
        let source = nodes
            .get(edge.source.as_str())
            .ok_or("missing call source")?;
        let target = nodes
            .get(edge.target.as_str())
            .ok_or("missing call target")?;
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing call occurrence")?;
        let owner = source.source.as_ref().ok_or("missing callable source")?;
        assert_eq!(owner.file, site.file);
        assert!(
            owner.start_byte <= site.start_byte
                && site.start_byte < site.end_byte
                && site.end_byte <= owner.end_byte
        );
        assert!(!edge.evidence.is_empty());
        assert!(
            edge.evidence
                .iter()
                .all(|e| e.origin != EvidenceOrigin::Heuristic
                    && e.confidence == EvidenceConfidence::Exact)
        );
        assert!(
            edge.evidence
                .iter()
                .any(|e| e.origin == EvidenceOrigin::Ast && e.anchors.contains(site))
        );
        let bytes = fs::read(root.join(&site.file))?;
        let spelling = bytes
            .get(site.start_byte as usize..site.end_byte as usize)
            .ok_or("call outside source")?;
        assert!(matches!(
            spelling,
            b"intact" | b"create_snapshot" | b"open_snapshot"
        ));
        observed.insert((
            source.qualified_name.clone(),
            target.qualified_name.clone(),
            site.start_line,
        ));
        assert!(
            identities.insert(edge.id.clone()),
            "duplicate call identity"
        );
    }
    let expected = BTreeSet::from([
        (
            format!("{prefix}factory.open_snapshot"),
            format!("{prefix}validation.intact"),
            5,
        ),
        (
            format!("{prefix}factory.open_snapshot"),
            format!("{prefix}factory.create_snapshot"),
            6,
        ),
        (
            format!("{prefix}test_recovery.test_damaged_local_indexes_are_checked_and_restored"),
            format!("{prefix}factory.open_snapshot"),
            6,
        ),
        (
            format!("{prefix}test_recovery.test_damaged_local_indexes_are_checked_and_restored"),
            format!("{prefix}factory.open_snapshot"),
            7,
        ),
    ]);
    assert_eq!(observed, expected);
    assert_eq!(
        identities.len(),
        4,
        "parallel calls must keep separate source occurrences"
    );
    let intact: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function && n.qualified_name.ends_with(".intact"))
        .collect();
    assert_eq!(
        intact.len(),
        2,
        "unrelated same-name definition must remain distinct"
    );
    for edge in graph.links.iter().filter(|e| e.kind == EdgeKind::Calls) {
        let target = nodes.get(edge.target.as_str()).ok_or("missing target")?;
        assert_ne!(target.source_file(), Some("disconnected.py"));
    }
    Ok(identities)
}

#[test]
fn bare_python_modules_do_not_depend_on_checkout_directory_names() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut previous = None;
    for name in ["repo", "checkout", "alternate-checkout"] {
        let root = directory.path().join(name);
        sources(&root, false)?;
        let graph = build(&root, &directory.path().join(format!("out-{name}")))?;
        let identities = assert_calls(&root, &graph, "")?;
        if let Some(previous) = &previous {
            assert_eq!(
                previous, &identities,
                "equivalent bare modules must preserve call IDs"
            );
        } else {
            previous = Some(identities);
        }
    }
    Ok(())
}

#[test]
fn package_python_modules_keep_their_source_package_namespace() -> TestResult {
    let directory = tempfile::tempdir()?;
    for name in ["repo", "package_one"] {
        let root = directory.path().join(name);
        sources(&root, true)?;
        let graph = build(&root, &directory.path().join(format!("out-{name}")))?;
        assert_calls(&root, &graph, &format!("{name}."))?;
    }
    Ok(())
}

#[test]
fn nested_python_package_imports_keep_exact_target_and_occurrence() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("package_one");
    for child in ["alpha", "beta"] {
        fs::create_dir_all(root.join(child))?;
        fs::write(
            root.join(child).join("__init__.py"),
            "from .worker import work\n",
        )?;
        fs::write(root.join(child).join("worker.py"), "def work(): return 1\n")?;
    }
    fs::write(root.join("__init__.py"), "")?;
    let text = "from .alpha import work\n\ndef run():\n    work()\n    work()\n";
    fs::write(root.join("client.py"), text)?;
    let graph = build(&root, &directory.path().join("out"))?;
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let calls: Vec<_> = graph
        .links
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .collect();
    assert_eq!(calls.len(), 2);
    let mut occurrences = BTreeSet::new();
    for edge in calls {
        let source = nodes
            .get(edge.source.as_str())
            .ok_or("missing nested call source")?;
        let target = nodes
            .get(edge.target.as_str())
            .ok_or("missing nested call target")?;
        assert_eq!(source.qualified_name, "package_one.client.run");
        assert_eq!(target.qualified_name, "package_one.alpha.worker.work");
        assert_eq!(target.source_file(), Some("alpha/worker.py"));
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing nested call occurrence")?;
        assert_eq!(site.file, "client.py");
        assert_eq!(
            &text.as_bytes()[site.start_byte as usize..site.end_byte as usize],
            b"work"
        );
        assert!(occurrences.insert((site.start_line, edge.id.clone())));
        assert!(
            edge.evidence
                .iter()
                .all(|e| e.origin != EvidenceOrigin::Heuristic
                    && e.confidence == EvidenceConfidence::Exact)
        );
        assert!(
            edge.evidence
                .iter()
                .any(|e| e.origin == EvidenceOrigin::Ast && e.anchors.contains(site))
        );
    }
    assert_eq!(
        occurrences
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [4, 5]
    );
    Ok(())
}

#[test]
fn package_entry_changes_invalidate_unchanged_python_modules() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("package_one");
    let output = directory.path().join("warm");
    fs::create_dir(&root)?;
    fs::write(
        root.join("worker.py"),
        "def inner(): return 1\n\ndef outer(): return inner()\n",
    )?;
    let mut initial = None;
    for (step, package) in [true, false, true].into_iter().enumerate() {
        if package {
            fs::write(root.join("__init__.py"), "")?;
        } else {
            fs::remove_file(root.join("__init__.py"))?;
        }
        // The callable source does not change; only the source-root package entry does.
        let expected = if package { "package_one." } else { "" };
        let inspect =
            |graph: &GraphDocument| -> TestResult<BTreeSet<String>> {
                let calls: Vec<_> = graph
                    .links
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Calls)
                    .collect();
                assert_eq!(calls.len(), 1);
                let edge = calls[0];
                let source = graph
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.source)
                    .ok_or("missing source")?;
                let target = graph
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.target)
                    .ok_or("missing target")?;
                assert_eq!(source.qualified_name, format!("{expected}worker.outer"));
                assert_eq!(target.qualified_name, format!("{expected}worker.inner"));
                assert!(edge.relationship_site.is_some());
                assert!(edge.evidence.iter().any(|e| e.origin == EvidenceOrigin::Ast
                    && e.confidence == EvidenceConfidence::Exact));
                Ok(BTreeSet::from([edge.id.clone()]))
            };
        let updated = build(&root, &output)?;
        let identities = inspect(&updated)?;
        let warm = build(&root, &output)?;
        assert_eq!(identities, inspect(&warm)?);
        let forced = build_with_force(
            &root,
            &directory.path().join(format!("forced-{step}")),
            true,
        )?;
        assert_eq!(identities, inspect(&forced)?);
        assert_eq!(updated.nodes, forced.nodes);
        assert_eq!(updated.links, forced.links);
        if step == 0 {
            initial = Some(identities);
        } else if step == 2 {
            assert_eq!(initial, Some(identities));
        }
    }
    Ok(())
}

#[test]
fn non_identifier_source_paths_keep_local_calls_and_cannot_impersonate_import_paths() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let mut previous = None;
    for name in ["repo", "checkout", "alternate-checkout"] {
        let root = directory.path().join(name);
        fs::create_dir_all(root.join("apps/billing-service"))?;
        fs::create_dir_all(root.join("foo.bar"))?;
        fs::write(
            root.join("apps/billing-service/test-helper.py"),
            "def inner(): return 1\n\ndef outer():\n    inner()\n    inner()\n",
        )?;
        fs::write(root.join("foo.bar/worker.py"), "def work(): return 1\n")?;
        fs::write(root.join("foo%2Ebar.py"), "def work(): return 2\n")?;
        fs::write(
            root.join("client.py"),
            "from foo.bar.worker import work\n\ndef run():\n    work()\n",
        )?;
        let graph = build(&root, &directory.path().join(format!("out-{name}")))?;
        let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.qualified_name == "foo%2Ebar.worker.work")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.qualified_name == "foo%252Ebar.work")
        );
        let mut identities = BTreeSet::new();
        for edge in graph.links.iter().filter(|e| e.kind == EdgeKind::Calls) {
            let source = nodes
                .get(edge.source.as_str())
                .ok_or("missing script call source")?;
            let target = nodes
                .get(edge.target.as_str())
                .ok_or("missing script call target")?;
            assert_eq!(
                source.qualified_name,
                "apps.billing-service.test-helper.outer"
            );
            assert_eq!(
                target.qualified_name,
                "apps.billing-service.test-helper.inner"
            );
            let site = edge
                .relationship_site
                .as_ref()
                .ok_or("missing script call site")?;
            assert_eq!(site.file, "apps/billing-service/test-helper.py");
            assert!(
                edge.evidence
                    .iter()
                    .all(|e| e.origin != EvidenceOrigin::Heuristic
                        && e.confidence == EvidenceConfidence::Exact)
            );
            assert!(
                edge.evidence
                    .iter()
                    .any(|e| e.origin == EvidenceOrigin::Ast && e.anchors.contains(site))
            );
            assert!(identities.insert(edge.id.clone()));
        }
        assert_eq!(
            identities.len(),
            2,
            "local occurrences survive; an absent import path must not bind to a dotted directory spelling"
        );
        if let Some(previous) = &previous {
            assert_eq!(previous, &identities);
        } else {
            previous = Some(identities);
        }
    }
    Ok(())
}
