use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, FileTimes};
use std::time::{Duration, UNIX_EPOCH};

use compass_files::{
    BuildGuard, Cache, CacheKind, DetectOptions, FileSlice, Manifest, ManifestKind, StatHashIndex,
    WatchPathFilter, bisect_slice, body_content, classify_file, file_hash, md5_file,
    prompt_fingerprint, read_slice_text, read_source_lossy, slice_boundaries, split_file,
    write_bytes_atomic, write_json_atomic, write_text_atomic,
};
use compass_files::{FileType, IgnorePolicy};
use serde_json::json;

#[test]
fn database_only_detection_does_not_read_local_files() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("local.rs"), "fn local() {}\n")?;
    fs::write(directory.path().join(".graphifyignore"), "[invalid\n")?;
    let detection = compass_files::detect(
        directory.path(),
        &DetectOptions {
            scan_filesystem: false,
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(detection.total_files, 0);
    assert!(detection.files.values().all(Vec::is_empty));
    assert!(detection.ignored.is_empty());
    Ok(())
}

#[test]
fn google_workspace_shortcuts_are_opt_in_and_sidecars_are_explicit() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let shortcut = directory.path().join("notes.gdoc");
    std::fs::write(&shortcut, r#"{"doc_id":"doc-1"}"#)?;

    let default = compass_files::detect(directory.path(), &DetectOptions::default())?;
    assert!(default.files["document"].is_empty());
    assert_eq!(
        default.google_workspace_shortcuts,
        [std::fs::canonicalize(&shortcut)?]
    );
    assert!(
        default
            .skipped_sensitive
            .iter()
            .any(|message| message.contains("Google Workspace shortcut skipped"))
    );

    let converted_dir = directory.path().join("converted");
    std::fs::create_dir_all(&converted_dir)?;
    let sidecar = converted_dir.join("notes.md");
    std::fs::write(&sidecar, "# Notes\n\nConverted content.\n")?;
    let enabled = compass_files::detect(
        directory.path(),
        &DetectOptions {
            google_workspace: true,
            additional_files: vec![sidecar.clone()],
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(
        enabled.files["document"],
        [std::fs::canonicalize(&sidecar)?.to_string_lossy()]
    );
    assert!(
        !enabled
            .skipped_sensitive
            .iter()
            .any(|message| message.contains("Google Workspace shortcut skipped"))
    );
    Ok(())
}

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
    assert!(!filter.allows(&root.join("compass-out/graph.json")));
    assert!(!filter.allows(&root.join("README.unknown")));
    Ok(())
}

#[test]
fn historical_detection_ignores_caller_local_excludes_but_keeps_committed_rules()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join(".git/info"))?;
    fs::write(root.join(".git/info/exclude"), "local.rs\n")?;
    fs::write(root.join(".gitignore"), "committed.rs\n")?;
    fs::write(root.join("local.rs"), "fn local() {}\n")?;
    fs::write(root.join("committed.rs"), "fn committed() {}\n")?;
    fs::write(root.join("explicit.rs"), "fn explicit() {}\n")?;

    let current = compass_files::detect(root, &DetectOptions::default())?;
    assert_eq!(current.files["code"].len(), 1);
    assert!(current.files["code"][0].ends_with("explicit.rs"));
    let historical = compass_files::detect(
        root,
        &DetectOptions {
            ignore_policy: IgnorePolicy::HistoricalCommit,
            extra_excludes: vec!["explicit.rs".to_owned()],
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(historical.files["code"].len(), 1);
    assert!(historical.files["code"][0].ends_with("local.rs"));
    Ok(())
}

#[test]
fn classification_exercises_manifests_shebangs_media_papers_and_asset_exclusions()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let cases = [
        ("pyproject.toml", "[project]\n", Some(FileType::Code)),
        ("view.blade.php", "<div />\n", Some(FileType::Code)),
        ("main.rs", "fn main() {}\n", Some(FileType::Code)),
        ("photo.PNG", "image", Some(FileType::Image)),
        ("clip.MP4", "video", Some(FileType::Video)),
        ("notes.md", "ordinary notes", Some(FileType::Document)),
        (
            "paper.md",
            "Abstract\nWe propose a method. arXiv 1706.03762\n",
            Some(FileType::Paper),
        ),
        ("unknown.bin", "opaque", None),
        (
            "script",
            "#!/usr/bin/env -S python3 -u\nprint(1)\n",
            Some(FileType::Code),
        ),
        ("plain", "not executable source", None),
    ];
    for (name, contents, expected) in cases {
        let path = directory.path().join(name);
        fs::write(&path, contents)?;
        assert_eq!(classify_file(&path), expected, "{name}");
    }

    let excluded = directory.path().join("Icons.xcassets/App.imageset");
    fs::create_dir_all(&excluded)?;
    let pdf = excluded.join("vector.pdf");
    fs::write(&pdf, b"%PDF")?;
    assert_eq!(classify_file(&pdf), None);

    let ordinary_pdf = directory.path().join("paper.pdf");
    fs::write(&ordinary_pdf, b"%PDF")?;
    assert_eq!(classify_file(&ordinary_pdf), Some(FileType::Paper));
    Ok(())
}

#[test]
fn detector_covers_nested_ignores_memory_sensitive_files_and_large_corpus_warning()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join(".git/info"))?;
    fs::write(root.join(".git/info/exclude"), "excluded-by-git.rs\n")?;
    fs::write(
        root.join(".graphifyignore"),
        "ignored/**\n!ignored/keep.rs\n*.generated.rs\n",
    )?;
    fs::create_dir_all(root.join("ignored"))?;
    fs::write(root.join("ignored/drop.rs"), "fn drop_me() {}\n")?;
    fs::write(root.join("ignored/keep.rs"), "fn keep_me() {}\n")?;
    fs::write(root.join("excluded-by-git.rs"), "fn excluded() {}\n")?;
    fs::write(root.join("model.generated.rs"), "fn generated() {}\n")?;
    fs::write(root.join("main.rs"), "fn main() {}\n")?;
    fs::write(root.join("README.odd"), "unclassified\n")?;
    fs::write(root.join("credentials.txt"), "secret\n")?;
    fs::write(root.join(".env.local"), "TOKEN=nope\n")?;
    fs::write(root.join("song.mp3"), b"audio")?;

    let memory = root.join("graphify-out/memory/nested");
    fs::create_dir_all(&memory)?;
    fs::write(memory.join("remember.md"), "# Durable memory\n")?;

    let large = root.join("large.md");
    fs::write(&large, "word ".repeat(500_001))?;

    let detection = compass_files::detect(root, &DetectOptions::default())?;
    assert!(detection.needs_graph);
    assert!(
        detection
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Large corpus"))
    );
    assert!(
        detection.files["code"]
            .iter()
            .any(|path| path.ends_with("main.rs"))
    );
    assert!(
        detection.files["document"]
            .iter()
            .any(|path| path.ends_with("remember.md"))
    );
    assert!(
        detection.files["video"]
            .iter()
            .any(|path| path.ends_with("song.mp3"))
    );
    assert!(
        detection
            .unclassified
            .iter()
            .any(|path| path.ends_with("README.odd"))
    );
    assert!(
        detection
            .skipped_sensitive
            .iter()
            .any(|path| path.ends_with("credentials.txt"))
    );
    assert!(
        detection
            .ignored
            .iter()
            .any(|path| path.contains("ignored"))
    );
    assert!(detection.graphifyignore_patterns >= 4);
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
fn batched_cache_writes_are_portable_and_refresh_changed_sources() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.rs");
    let second = directory.path().join("second.rs");
    fs::write(&first, "fn first() {}\n")?;
    fs::write(&second, "fn second() {}\n")?;
    let first_value =
        json!({"nodes":[{"id":"first","source_file":first.to_string_lossy()}],"edges":[]});
    let second_value =
        json!({"nodes":[{"id":"second","source_file":second.to_string_lossy()}],"edges":[]});
    let mut cache = Cache::new(directory.path(), None)?;

    cache.save_batch(
        &[
            (first.clone(), first_value.clone()),
            (second.clone(), second_value.clone()),
        ],
        &CacheKind::Ast,
        None,
    )?;
    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false, false)?,
        Some(first_value)
    );
    assert_eq!(
        cache.load(&second, &CacheKind::Ast, None, false, false)?,
        Some(second_value)
    );

    fs::write(&first, "fn first_changed() {}\n")?;
    let changed =
        json!({"nodes":[{"id":"first_changed","source_file":first.to_string_lossy()}],"edges":[]});
    cache.save_batch(&[(first.clone(), changed.clone())], &CacheKind::Ast, None)?;
    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false, false)?,
        Some(changed)
    );
    Ok(())
}

#[test]
fn malformed_and_non_object_cache_entries_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.py");
    fs::write(&source, "def main(): pass\n")?;
    let mut cache = Cache::new(directory.path(), None)?;

    cache.save(&source, &json!("scalar"), &CacheKind::Semantic, None)?;
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, false, false)?,
        Some(json!("scalar"))
    );

    let entry = fs::read_dir(cache.directory(&CacheKind::Semantic, None))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .ok_or("missing semantic cache entry")?;
    fs::write(entry, b"not-json")?;
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, false, false)?,
        None
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
    let not_a_directory = directory.path().join("not-a-directory");
    fs::write(&not_a_directory, "file")?;
    assert!(BuildGuard::begin(&not_a_directory.join("output")).is_err());

    let broken_guard = BuildGuard::begin(directory.path())?;
    let marker = directory.path().join(".compass-build-incomplete");
    fs::remove_file(&marker)?;
    fs::create_dir(&marker)?;
    assert!(broken_guard.commit().is_err());
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

#[test]
fn cache_versions_legacy_fingerprints_pruning_and_cleanup_are_total() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("root");
    let cache_root = directory.path().join("cache-root");
    fs::create_dir_all(&root)?;
    fs::create_dir_all(cache_root.join("compass-out/cache/ast/vold"))?;
    fs::write(
        cache_root.join("compass-out/cache/ast/vold/stale.json"),
        "{}",
    )?;
    fs::write(cache_root.join("compass-out/cache/ast/legacy.json"), "{}")?;
    fs::create_dir_all(cache_root.join("compass-out/cache/ast/keep"))?;
    fs::write(
        cache_root.join("compass-out/cache/ast/keep/marker"),
        "preserved",
    )?;
    fs::write(
        cache_root.join("compass-out/cache/ast/preserved.txt"),
        "preserved",
    )?;
    let source = root.join("main.md");
    fs::write(&source, "---\ntitle: ignored\n---\nbody\n")?;

    let mut cache = Cache::new(&root, Some(&cache_root))?.with_extractor_version("current");
    assert!(
        cache
            .directory(&CacheKind::Ast, None)
            .ends_with("ast/vcurrent")
    );
    assert!(
        cache
            .directory(&CacheKind::SemanticMode("deep".to_owned()), Some("abc"))
            .ends_with("semantic-deep/pabc")
    );
    cache.save(
        &source,
        &json!({
            "nodes":[{"source_file":source},{"source_file":"relative.md"},"bad"],
            "edges":[{"source_file":""}],
            "hyperedges":[{"source_file":"outside.md"}],
            "raw_calls":[{"source_file":"relative.md"}]
        }),
        &CacheKind::Semantic,
        None,
    )?;
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, Some("new"), false, true)?,
        None
    );
    let legacy = cache
        .load(&source, &CacheKind::Semantic, Some("new"), true, true)?
        .ok_or("legacy cache entry")?;
    assert!(
        legacy["nodes"][0]["source_file"]
            .as_str()
            .is_some_and(|path| path.ends_with("main.md"))
    );
    assert!(
        legacy["nodes"][1]["source_file"]
            .as_str()
            .is_some_and(|path| path.ends_with("relative.md"))
    );

    cache.save(
        &source,
        &json!({"nodes":[],"edges":[]}),
        &CacheKind::SemanticMode("deep".to_owned()),
        Some("old"),
    )?;
    let before = cache.cached_files();
    assert_eq!(before.len(), 1, "identical hashes deduplicate across modes");
    assert!(cache.prune_semantic(&BTreeSet::new()) >= 2);
    cache.clear();
    assert!(cache.cached_files().len() <= 1);
    assert!(!cache_root.join("compass-out/cache/ast/vold").exists());
    assert!(
        cache_root
            .join("compass-out/cache/ast/keep/marker")
            .exists()
    );
    assert!(
        cache_root
            .join("compass-out/cache/ast/preserved.txt")
            .exists()
    );

    let missing = root.join("missing.md");
    cache.save(&missing, &json!({}), &CacheKind::Ast, None)?;
    assert!(Cache::new(root.join("missing-root"), None).is_err());
    Ok(())
}

#[test]
fn manifest_change_detection_distinguishes_corpus_hash_kind_and_legacy_time()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    fs::write(&source, "fn main() {}\n")?;
    let source = fs::canonicalize(source)?;
    let source_key = source.to_string_lossy().into_owned();
    let mut files = BTreeMap::from([("code".to_owned(), vec![source_key.clone()])]);

    assert!(!Manifest::default().is_unchanged(&files, ManifestKind::Ast));

    let manifest_path = directory.path().join("manifest.json");
    let current_hash = md5_file(&source)?;
    fs::write(
        &manifest_path,
        serde_json::to_vec(&json!({
            (source_key.clone()): {
                "mtime": 0.0,
                "ast_hash": current_hash,
                "semantic_hash": "different"
            }
        }))?,
    )?;
    let current = Manifest::load(&manifest_path, None);
    assert!(current.is_unchanged(&files, ManifestKind::Ast));
    assert!(!current.is_unchanged(&files, ManifestKind::Semantic));

    files.insert("code".to_owned(), Vec::new());
    assert!(!current.is_unchanged(&files, ManifestKind::Ast));

    let legacy_time = UNIX_EPOCH + Duration::from_secs(1);
    fs::OpenOptions::new()
        .write(true)
        .open(&source)?
        .set_times(FileTimes::new().set_modified(legacy_time))?;
    fs::write(
        &manifest_path,
        serde_json::to_vec(&json!({(source_key.clone()): 1.0}))?,
    )?;
    let legacy = Manifest::load(&manifest_path, None);
    let legacy_files = BTreeMap::from([("code".to_owned(), vec![source_key])]);
    assert!(legacy.is_unchanged(&legacy_files, ManifestKind::Ast));
    Ok(())
}

#[test]
fn manifest_incremental_tracks_changes_deletions_exclusions_and_legacy_entries()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    fs::write(&first, "fn first() {}\n")?;
    fs::write(&second, "fn second() {}\n")?;
    let first = fs::canonicalize(first)?;
    let second = fs::canonicalize(second)?;
    let manifest_path = root.join("compass-out/manifest.json");

    let fresh = Manifest::incremental(
        root,
        &manifest_path,
        &DetectOptions::default(),
        ManifestKind::Both,
    )?;
    assert_eq!(fresh.new_total, 2);
    let mut manifest = Manifest::default();
    manifest.save(
        &fresh.detection.files,
        &manifest_path,
        ManifestKind::Both,
        Some(root),
        None,
        None,
    )?;
    assert!(manifest.is_unchanged(&fresh.detection.files, ManifestKind::Ast));
    assert!(manifest.is_unchanged(&fresh.detection.files, ManifestKind::Semantic));

    let warm = Manifest::incremental(
        root,
        &manifest_path,
        &DetectOptions::default(),
        ManifestKind::Both,
    )?;
    assert_eq!(warm.new_total, 0);
    let excluded = root.join("excluded.rs");
    fs::write(&excluded, "fn excluded() {}\n")?;
    let mut with_excluded = warm.detection.files.clone();
    with_excluded
        .entry("code".to_owned())
        .or_default()
        .push(fs::canonicalize(&excluded)?.to_string_lossy().into_owned());
    manifest.save(
        &with_excluded,
        &manifest_path,
        ManifestKind::Both,
        Some(root),
        None,
        None,
    )?;
    fs::write(&first, "fn first_changed() {}\n")?;
    fs::remove_file(&second)?;
    fs::write(root.join(".graphifyignore"), "excluded.rs\n")?;
    let delta = Manifest::incremental(
        root,
        &manifest_path,
        &DetectOptions::default(),
        ManifestKind::Both,
    )?;
    assert!(
        delta.new_files["code"]
            .iter()
            .any(|path| path == first.to_string_lossy().as_ref())
    );
    assert!(
        delta
            .deleted_files
            .iter()
            .any(|path| path == second.to_string_lossy().as_ref())
    );
    assert!(
        delta
            .excluded_files
            .iter()
            .any(|path| path.ends_with("excluded.rs"))
    );

    let clear = BTreeSet::from([first.to_string_lossy().into_owned()]);
    manifest.save(
        &delta.detection.files,
        &manifest_path,
        ManifestKind::Ast,
        Some(root),
        None,
        Some(&clear),
    )?;
    let loaded = Manifest::load(&manifest_path, Some(root));
    assert!(
        loaded.entries()[first.to_string_lossy().as_ref()]
            .semantic_hash
            .is_empty()
    );

    fs::write(
        &manifest_path,
        format!(
            "{{\"{}\":1.0,\"object.rs\":{{\"mtime\":2.0,\"hash\":\"legacy\"}},\"bad\":null}}",
            first.to_string_lossy().replace('\\', "\\\\")
        ),
    )?;
    assert_eq!(Manifest::load(&manifest_path, None).entries().len(), 2);
    fs::write(&manifest_path, "[]")?;
    assert!(Manifest::load(&manifest_path, None).entries().is_empty());
    fs::write(&manifest_path, "not json")?;
    assert!(Manifest::load(&manifest_path, None).entries().is_empty());
    Ok(())
}

#[test]
fn slicing_hashing_atomic_writes_and_stat_index_cover_hostile_boundaries()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("unicode.md");
    fs::write(&source, "αβ\n# Heading\n\nbody\n")?;
    assert_eq!(slice_boundaries("", 0), vec![(0, 0)]);
    let zero_limit = slice_boundaries("abc", 0);
    assert_eq!(zero_limit, vec![(0, 1), (1, 2), (2, 3)]);
    let ranges = slice_boundaries("one\n\n# two\nthree", 9);
    assert_eq!(ranges.first().map(|range| range.0), Some(0));
    assert_eq!(ranges.last().map(|range| range.1), Some(16));

    let slices = split_file(&source, 8)?;
    assert!(slices.len() > 1);
    assert!(!read_slice_text(&slices[0])?.is_empty());
    let whole = FileSlice {
        path: source.clone(),
        start: 0,
        end: usize::MAX,
        index: 0,
        total: 1,
    };
    let (left, right) = bisect_slice(&whole)?.ok_or("bisected slice")?;
    assert_eq!(left.end, right.start);
    assert!(right.end < usize::MAX);
    let tiny = FileSlice {
        end: 1,
        ..whole.clone()
    };
    assert!(bisect_slice(&tiny)?.is_none());
    let missing = FileSlice {
        path: directory.path().join("missing.md"),
        ..tiny
    };
    assert!(read_slice_text(&missing).is_err());
    assert!(bisect_slice(&missing).is_err());
    let binary = directory.path().join("data.bin");
    fs::write(&binary, "long binary payload")?;
    assert_eq!(split_file(&binary, 1)?.len(), 1);

    assert_eq!(
        prompt_fingerprint(" prompt  \r\n"),
        prompt_fingerprint("prompt\n")
    );
    assert_eq!(md5_file(&source)?.len(), 32);
    assert_eq!(file_hash(&source, directory.path())?.len(), 64);
    assert!(file_hash(directory.path(), directory.path()).is_err());
    let mut index = StatHashIndex::load(directory.path(), "graphify-out");
    let first_hash = index.hash(&source, directory.path())?;
    assert_eq!(index.hash(&source, directory.path())?, first_hash);
    assert_eq!(index.word_count(&source, |_| 4), 4);
    assert_eq!(index.word_count(&source, |_| 99), 4);
    assert_eq!(index.word_count(&missing.path, |_| 7), 7);
    index.flush()?;
    index.flush()?;

    let nested = directory.path().join("nested/out.bin");
    write_bytes_atomic(&nested, b"bytes")?;
    assert_eq!(fs::read(&nested)?, b"bytes");
    let json_path = directory.path().join("nested/value.json");
    write_json_atomic(&json_path, &json!({"x":1}), true)?;
    assert!(fs::read_to_string(json_path)?.contains("\n"));
    fs::write(directory.path().join("not-a-directory"), "file")?;
    assert!(write_text_atomic(directory.path().join("not-a-directory/child"), "x").is_err());

    let guard = BuildGuard::begin(directory.path())?;
    fs::remove_file(directory.path().join(".compass-build-incomplete"))?;
    guard.commit()?;
    Ok(())
}
