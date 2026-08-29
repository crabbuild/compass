use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_core::{BuildOptions, BuildPurpose, GraphStorage, build_local_graph};
use compass_files::BuildScope;
use compass_model::code_graph::GraphDocument;

const VENDOR_SOURCES: [(&str, &str); 2] = [
    ("vendor/parser-pack/src/lib.rs", "rust"),
    ("vendor/go-dependency/tool.go", "go"),
];

fn build_document(
    root: &Path,
    output_root: PathBuf,
    configure: impl FnOnce(&mut BuildOptions),
) -> Result<GraphDocument, Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.output_root = Some(output_root);
    options.no_cluster = true;
    options.no_viz = true;
    options.graph_storage = GraphStorage::Json;
    options.purpose = BuildPurpose::Extract;
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    configure(&mut options);
    let result = build_local_graph(&options)?;
    Ok(GraphDocument::load(&result.output_dir.join("graph.json"))?)
}

fn assert_vendor_publication(document: &GraphDocument, expected: bool) {
    let inventory = document
        .graph
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.language.as_deref()))
        .collect::<BTreeMap<_, _>>();
    for (source, language) in VENDOR_SOURCES {
        assert_eq!(
            inventory.get(source).copied(),
            expected.then_some(Some(language)),
            "unexpected inventory state for {source}"
        );
        assert_eq!(
            document.nodes.iter().any(|node| node
                .source
                .as_ref()
                .is_some_and(|anchor| anchor.file == source)),
            expected,
            "unexpected published node state for {source}"
        );
    }
}

#[test]
fn vendor_sources_are_published_by_default_and_respect_explicit_exclusions()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("checkout");
    fs::create_dir_all(root.join("src"))?;
    fs::create_dir_all(root.join("vendor/parser-pack/src"))?;
    fs::create_dir_all(root.join("vendor/go-dependency"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"vendor/parser-pack\"]\nresolver = \"3\"\n",
    )?;
    fs::write(
        root.join("vendor/parser-pack/Cargo.toml"),
        "[package]\nname = \"parser-pack\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::write(
        root.join("vendor/parser-pack/src/lib.rs"),
        "pub fn parse_vendor_source() -> usize { 1 }\n",
    )?;
    fs::write(
        root.join("src/lib.rs"),
        "pub fn keep_workspace_source() -> usize { 1 }\n",
    )?;
    fs::write(
        root.join("vendor/go-dependency/tool.go"),
        "package dependency\n\nfunc VendorTool() int { return 1 }\n",
    )?;

    let default = build_document(&root, directory.path().join("default-output"), |_| {})?;
    assert_vendor_publication(&default, true);

    fs::write(root.join(".compassignore"), "vendor/**\n")?;
    let ignored = build_document(&root, directory.path().join("compassignore-output"), |_| {})?;
    assert_vendor_publication(&ignored, false);
    fs::remove_file(root.join(".compassignore"))?;

    let excluded = build_document(&root, directory.path().join("exclude-output"), |options| {
        options.extra_excludes = vec!["vendor/**".to_owned()]
    })?;
    assert_vendor_publication(&excluded, false);

    let scope = BuildScope {
        include: Vec::new(),
        exclude: vec!["vendor/**".to_owned()],
    }
    .normalize(&root)?;
    let scoped = build_document(&root, directory.path().join("scope-output"), |options| {
        options.scope = scope
    })?;
    assert_vendor_publication(&scoped, false);
    Ok(())
}
