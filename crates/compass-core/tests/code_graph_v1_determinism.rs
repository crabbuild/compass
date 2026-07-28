use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, build_local_graph};
use compass_model::code_graph::GraphDocument;

const SOURCE: &str = r#"
pub struct Store;

impl Store {
    pub fn load(&self) -> usize { 1 }
}

pub fn handler(store: &Store) -> usize {
    store.load()
}
"#;

fn build(root: &Path) -> Result<(Vec<u8>, bool), Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let result = build_local_graph(&options)?;
    let path = result.output_dir.join("graph.json");
    let bytes = fs::read(&path)?;
    GraphDocument::load(&path)?;
    Ok((bytes, result.outputs_changed))
}

#[test]
fn clean_warm_restored_and_checkout_root_builds_are_byte_identical() -> Result<(), Box<dyn Error>> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    for root in [first.path(), second.path()] {
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/lib.rs"), SOURCE)?;
    }

    let (cold, cold_changed) = build(first.path())?;
    assert!(cold_changed);
    let (warm, warm_changed) = build(first.path())?;
    assert!(!warm_changed);
    assert_eq!(warm, cold);

    let (other_root, _) = build(second.path())?;
    assert_eq!(other_root, cold);

    fs::write(
        first.path().join("src/lib.rs"),
        format!("{SOURCE}\npub fn changed() {{}}\n"),
    )?;
    let (changed, changed_output) = build(first.path())?;
    assert!(changed_output);
    assert_ne!(changed, cold);

    fs::write(first.path().join("src/lib.rs"), SOURCE)?;
    let (restored, restored_output) = build(first.path())?;
    assert!(restored_output);
    assert_eq!(restored, cold);
    Ok(())
}
