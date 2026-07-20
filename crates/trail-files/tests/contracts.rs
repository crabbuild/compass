use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;

use serde_json::json;
use trail_files::{
    BuildGuard, Cache, CacheKind, DetectOptions, Manifest, ManifestKind, WatchPathFilter,
    body_content, read_source_lossy, split_file, write_text_atomic,
};

#[test]
fn markdown_frontmatter_matches_legacy_bytes() {
    let cases: &[(&[u8], &[u8])] = &[
        (
            b"---\ntitle: Test\n---\n\nActual body.",
            b"\n\nActual body.",
        ),
        (b"---\ntitle: Test\n---  \nbody", b"  \nbody"),
        (b"---\r\ntitle: Test\r\n---\r\nbody", b"\r\nbody"),
        (b"---\n---\nbody", b"\nbody"),
        (b"---\ntitle: Test\n---", b""),
        (
            b"----\nIntro that must remain.\n---\nbody",
            b"----\nIntro that must remain.\n---\nbody",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(&body_content(input), expected);
    }
}

#[test]
fn watcher_filter_reuses_ignore_and_output_boundaries() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::write(root.join(".graphifyignore"), "ignored/\n*.generated.rs\n")?;
    fs::create_dir(root.join("ignored"))?;
    let filter = WatchPathFilter::new(root, &DetectOptions::default())?;

    assert!(filter.allows(&root.join("src/main.rs")));
    assert!(!filter.allows(&root.join("ignored/secret.rs")));
    assert!(!filter.allows(&root.join("model.generated.rs")));
    assert!(!filter.allows(&root.join(".hidden/main.rs")));
    assert!(!filter.allows(&root.join("graphify-out/graph.json")));
    assert!(!filter.allows(&root.join("README.unknown")));
    Ok(())
}

#[test]
fn cache_round_trip_is_portable_and_partial_safe() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source_directory = directory.path().join("src");
    fs::create_dir(&source_directory)?;
    let source = source_directory.join("main.py");
    fs::write(&source, "def main(): pass\n")?;
    let absolute = source.to_string_lossy().into_owned();
    let value = json!({
        "nodes": [
            {"id": "main", "source_file": absolute},
            {"id": "external_type", "source_file": "", "origin_file": absolute}
        ],
        "edges": [],
        "partial": false
    });
    let mut cache = Cache::new(directory.path(), None)?;
    cache.save(&source, &value, &CacheKind::Ast, None)?;
    assert_eq!(
        cache.load(&source, &CacheKind::Ast, None, true, false)?,
        Some(value)
    );
    cache.flush()?;

    let entries = cache.cached_files();
    assert_eq!(
        entries.len(),
        2,
        "AST entry plus stat-index are visible recursively"
    );

    let partial = json!({"nodes": [], "edges": [], "partial": true});
    cache.save(&source, &partial, &CacheKind::Semantic, None)?;
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, true, false)?,
        None
    );
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, true, true)?,
        Some(partial)
    );
    Ok(())
}

#[test]
fn manifest_round_trip_preserves_independent_stamps() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    fs::write(&source, "fn main() {}\n")?;
    let source = fs::canonicalize(source)?;
    let manifest_path = directory.path().join("graphify-out/manifest.json");
    let mut files = BTreeMap::new();
    files.insert(
        "code".to_owned(),
        vec![source.to_string_lossy().into_owned()],
    );
    let scan = files
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<String>>();

    let mut manifest = Manifest::default();
    manifest.save(
        &files,
        &manifest_path,
        ManifestKind::Ast,
        Some(directory.path()),
        Some(&scan),
        None,
    )?;
    let loaded = Manifest::load(&manifest_path, Some(directory.path()));
    let entry = loaded
        .entries()
        .get(source.to_string_lossy().as_ref())
        .ok_or("missing manifest entry")?;
    assert!(!entry.ast_hash.is_empty());
    assert!(entry.semantic_hash.is_empty());
    let disk = fs::read_to_string(manifest_path)?;
    assert!(disk.contains("\"main.rs\""));
    assert!(!disk.contains(directory.path().to_string_lossy().as_ref()));
    Ok(())
}

#[test]
fn lossy_source_limit_slicing_and_build_guard() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("notes.md");
    fs::write(&source, b"# One\n\xff\n\n# Two\ncontent\n")?;
    let decoded = read_source_lossy(&source, 1_000)?;
    assert!(decoded.contains('\u{fffd}'));
    assert!(read_source_lossy(&source, 2).is_err());
    let slices = split_file(&source, 12)?;
    assert!(slices.len() >= 2);

    let guard = BuildGuard::begin(directory.path())?;
    assert!(BuildGuard::ensure_complete(directory.path()).is_err());
    guard.commit()?;
    BuildGuard::ensure_complete(directory.path())?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_destination_symlink() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target.txt");
    let link = directory.path().join("link.txt");
    fs::write(&target, "old")?;
    symlink(&target, &link)?;
    write_text_atomic(&link, "new")?;
    assert!(link.is_symlink());
    assert_eq!(fs::read_to_string(target)?, "new");
    Ok(())
}
