use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, FileTimes};
use std::time::{Duration, UNIX_EPOCH};

use compass_files::{
    BuildGuard, CACHE_ENCODING_VERSION, Cache, CacheKind, CacheOptions, DetectOptions, FileSlice,
    Manifest, ManifestKind, StatHashIndex, WatchPathFilter, bisect_slice, body_content,
    classify_file, file_hash, md5_file, prompt_fingerprint, read_slice_text, read_source_lossy,
    slice_boundaries, split_file, write_bytes_atomic, write_json_atomic, write_text_atomic,
};
use compass_files::{FileType, IgnorePolicy};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn database_only_detection_does_not_read_local_files() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("local.rs"), "fn local() {}\n")?;
    fs::write(directory.path().join(".compassignore"), "[invalid\n")?;
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
fn markdown_file_hash_includes_frontmatter_semantics() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("document.md");
    fs::write(&path, "---\ntitle: First\n---\n\nShared body\n")?;
    let first_hash = file_hash(&path, directory.path())?;
    fs::write(&path, "---\ntitle: Second\n---\n\nShared body\n")?;
    let second_hash = file_hash(&path, directory.path())?;
    assert_ne!(first_hash, second_hash);
    Ok(())
}

#[test]
fn watcher_filter_reuses_ignore_and_output_boundaries() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::write(root.join(".compassignore"), "ignored/\n*.generated.rs\n")?;
    fs::create_dir(root.join("ignored"))?;
    let filter = WatchPathFilter::new(root, &DetectOptions::default())?;

    assert!(filter.allows(&root.join("src/main.rs")));
    assert!(!filter.allows(&root.join("ignored/secret.rs")));
    assert!(!filter.allows(&root.join("model.generated.rs")));
    assert!(!filter.allows(&root.join(".hidden/main.rs")));
    assert!(!filter.allows(&root.join("compass-out/graph.json")));
    assert!(!filter.allows(&root.join("compass-out/graph.json")));
    assert!(!filter.allows(&root.join("README.unknown")));
    Ok(())
}

#[test]
fn detection_ignores_compass_generated_output() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::write(root.join("main.rs"), "fn main() {}\n")?;
    fs::create_dir(root.join("compass-out"))?;
    fs::write(root.join("compass-out/graph.json"), "{}\n")?;
    fs::write(root.join("compass-out/.compass_labels.json"), "{}\n")?;
    fs::create_dir(root.join("graphify-out"))?;
    fs::write(
        root.join("graphify-out/generated.rs"),
        "fn generated() {}\n",
    )?;

    let detection = compass_files::detect(root, &DetectOptions::default())?;
    assert_eq!(detection.files["code"].len(), 1);
    assert!(detection.files["code"][0].ends_with("main.rs"));
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
fn detection_ignores_git_worktree_pointer_files() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::write(root.join(".git"), "gitdir: /tmp/example\n")?;
    fs::write(root.join("main.rs"), "fn main() {}\n")?;

    let detection = compass_files::detect(
        root,
        &DetectOptions {
            ignore_policy: IgnorePolicy::HistoricalCommit,
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(detection.files["code"].len(), 1);
    assert!(detection.files["code"][0].ends_with("main.rs"));
    assert!(
        detection
            .unclassified
            .iter()
            .all(|path| !path.ends_with(".git"))
    );
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
        ("script.pl", "sub main {}\n", Some(FileType::Code)),
        ("Module.pm", "package Module;\n", Some(FileType::Code)),
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

    let play_routes = directory.path().join("conf/routes");
    fs::create_dir_all(play_routes.parent().ok_or("missing routes parent")?)?;
    fs::write(&play_routes, "GET / controllers.Home.index\n")?;
    assert_eq!(classify_file(&play_routes), Some(FileType::Code));
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
        root.join(".compassignore"),
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

    let memory = root.join("compass-out/memory/nested");
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
    assert!(detection.compassignore_patterns >= 4);
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
    let mut cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;
    cache.save(&source, &value, &CacheKind::Ast, None)?;
    assert!(
        fs::read_dir(cache.directory(&CacheKind::Ast, None))?
            .filter_map(Result::ok)
            .any(|entry| entry
                .path()
                .extension()
                .is_some_and(|value| value == "msgpack"))
    );
    assert_eq!(
        cache.load(&source, &CacheKind::Ast, None, false)?,
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
        cache.load(&source, &CacheKind::Semantic, None, false)?,
        None
    );
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, true)?,
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
    let mut cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;

    cache.save_batch(
        &[
            (first.clone(), first_value.clone()),
            (second.clone(), second_value.clone()),
        ],
        &CacheKind::Ast,
        None,
    )?;
    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false)?,
        Some(first_value.clone())
    );
    assert_eq!(
        cache.load(&second, &CacheKind::Ast, None, false)?,
        Some(second_value)
    );

    let portable_first = json!({
        "nodes":[{"id":"first","source_file":"first.rs"}],
        "edges":[],
        "framework_facts":[{
            "type":"route",
            "fact":{
                "anchor":{
                    "sourceFile":"first.rs",
                    "startByte":0,
                    "endByte":1,
                    "startLine":1,
                    "startColumn":0,
                    "endLine":1,
                    "endColumn":1
                }
            }
        }]
    });
    cache.save_portable_ast_batch(&[(first.clone(), portable_first)])?;
    let canonical_first_value = json!({
        "nodes":[{
            "id":"first",
            "source_file":fs::canonicalize(&first)?.to_string_lossy()
        }],
        "edges":[],
        "framework_facts":[{
            "type":"route",
            "fact":{
                "anchor":{
                    "sourceFile":fs::canonicalize(&first)?.to_string_lossy(),
                    "startByte":0,
                    "endByte":1,
                    "startLine":1,
                    "startColumn":0,
                    "endLine":1,
                    "endColumn":1
                }
            }
        }]
    });
    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false)?,
        Some(canonical_first_value)
    );

    fs::write(&first, "fn first_changed() {}\n")?;
    let changed =
        json!({"nodes":[{"id":"first_changed","source_file":first.to_string_lossy()}],"edges":[]});
    cache.save_batch(&[(first.clone(), changed.clone())], &CacheKind::Ast, None)?;
    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false)?,
        Some(changed)
    );
    Ok(())
}

#[test]
fn cache_keeps_distinct_extractions_for_identical_source_bytes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("AGENTS.md");
    let second = directory.path().join("CLAUDE.md");
    fs::write(&first, "# Shared instructions\n")?;
    fs::write(&second, "# Shared instructions\n")?;
    let first_value =
        json!({"nodes":[{"id":"agents","source_file":first.to_string_lossy()}],"edges":[]});
    let second_value =
        json!({"nodes":[{"id":"claude","source_file":second.to_string_lossy()}],"edges":[]});
    let mut cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;

    cache.save_batch(
        &[
            (first.clone(), first_value.clone()),
            (second.clone(), second_value.clone()),
        ],
        &CacheKind::Ast,
        None,
    )?;

    assert_eq!(
        cache.load(&first, &CacheKind::Ast, None, false)?,
        Some(first_value)
    );
    assert_eq!(
        cache.load(&second, &CacheKind::Ast, None, false)?,
        Some(second_value)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn cache_normalizes_root_aliases_without_collapsing_leaf_symlinks() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let requested_root = directory.path().join("repository");
    let aliased_root = directory.path().join("repository-alias");
    fs::create_dir(&requested_root)?;
    symlink(&requested_root, &aliased_root)?;
    let canonical_root = fs::canonicalize(&requested_root)?;

    let canonical_source = canonical_root.join("main.py");
    let aliased_source = aliased_root.join("main.py");
    fs::write(&canonical_source, "def main(): pass\n")?;
    let value = json!({"nodes":[{"id":"main","source_file":canonical_source.to_string_lossy()}],"edges":[]});
    let mut cache = Cache::open(&aliased_root, CacheOptions::output_directory(None))?;

    cache.save(&canonical_source, &value, &CacheKind::Ast, None)?;
    assert_eq!(
        cache.load(&aliased_source, &CacheKind::Ast, None, false)?,
        Some(value)
    );

    let target = canonical_root.join("AGENTS.md");
    let link = canonical_root.join("CLAUDE.md");
    fs::write(&target, "# Shared instructions\n")?;
    symlink(&target, &link)?;
    let target_value =
        json!({"nodes":[{"id":"agents","source_file":target.to_string_lossy()}],"edges":[]});
    let link_value =
        json!({"nodes":[{"id":"claude","source_file":link.to_string_lossy()}],"edges":[]});

    cache.save(&target, &target_value, &CacheKind::Ast, None)?;
    cache.save(&link, &link_value, &CacheKind::Ast, None)?;
    assert_eq!(
        cache.load(&target, &CacheKind::Ast, None, false)?,
        Some(target_value)
    );
    assert_eq!(
        cache.load(&link, &CacheKind::Ast, None, false)?,
        Some(link_value)
    );
    Ok(())
}

#[test]
fn malformed_and_non_object_cache_entries_fail_closed() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.py");
    fs::write(&source, "def main(): pass\n")?;
    let mut cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;

    cache.save(&source, &json!("scalar"), &CacheKind::Semantic, None)?;
    assert_eq!(
        cache.load(&source, &CacheKind::Semantic, None, false)?,
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
        cache.load(&source, &CacheKind::Semantic, None, false)?,
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
    let manifest_path = directory.path().join("compass-out/manifest.json");
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
    BuildGuard::ensure_complete(directory.path())?;
    guard.commit()?;
    BuildGuard::ensure_complete(directory.path())?;
    let not_a_directory = directory.path().join("not-a-directory");
    fs::write(&not_a_directory, "file")?;
    assert!(BuildGuard::begin(&not_a_directory.join("output")).is_err());

    let broken_guard = BuildGuard::begin(directory.path())?;
    let marker = broken_guard
        .staging_directory()
        .join(".compass-build-incomplete");
    fs::remove_file(&marker)?;
    fs::create_dir(&marker)?;
    assert!(broken_guard.commit().is_err());
    Ok(())
}

#[test]
fn build_guard_publishes_one_complete_generation_at_a_time() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let first = BuildGuard::begin(directory.path())?;
    fs::write(first.staging_directory().join("graph.json"), "graph-one")?;
    fs::write(
        first.staging_directory().join("program.json"),
        "program-one",
    )?;
    first.commit_with_artifacts(&["graph.json", "program.json"])?;
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "graph.json"
        )?)?,
        "graph-one"
    );

    {
        let failed = BuildGuard::begin(directory.path())?;
        fs::write(failed.staging_directory().join("graph.json"), "graph-two")?;
    }
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "graph.json"
        )?)?,
        "graph-one"
    );
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "program.json"
        )?)?,
        "program-one"
    );

    let incomplete = BuildGuard::begin(directory.path())?;
    fs::remove_file(incomplete.staging_directory().join("program.json"))?;
    assert!(
        incomplete
            .commit_with_artifacts(&["graph.json", "program.json"])
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "graph.json"
        )?)?,
        "graph-one"
    );

    let second = BuildGuard::begin(directory.path())?;
    fs::write(second.staging_directory().join("graph.json"), "graph-two")?;
    fs::write(
        second.staging_directory().join("program.json"),
        "program-two",
    )?;
    second.commit_with_artifacts(&["graph.json", "program.json"])?;
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "program.json"
        )?)?,
        "program-two"
    );
    Ok(())
}

#[test]
fn build_guard_publishes_atomic_artifacts_without_resealing_them() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let guard = BuildGuard::begin(directory.path())?;
    write_text_atomic(&guard.staging_directory().join("graph.json"), "graph")?;
    guard.commit_with_presealed_artifacts(&["graph.json"])?;
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "graph.json"
        )?)?,
        "graph"
    );
    Ok(())
}

#[test]
fn build_guard_can_exclude_a_large_generation_sidecar() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let first = BuildGuard::begin(directory.path())?;
    fs::write(first.staging_directory().join("graph.json"), "graph-one")?;
    fs::write(first.staging_directory().join("large.sqlite3"), "database")?;
    first.commit_with_artifacts(&["graph.json", "large.sqlite3"])?;

    let second = BuildGuard::begin_excluding(directory.path(), &["large.sqlite3"])?;
    assert_eq!(
        fs::read_to_string(second.staging_directory().join("graph.json"))?,
        "graph-one"
    );
    assert!(!second.staging_directory().join("large.sqlite3").exists());
    assert!(BuildGuard::begin_excluding(directory.path(), &["../unsafe"]).is_err());
    Ok(())
}

#[test]
fn build_guard_retains_only_two_complete_generations() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    for version in 0..4 {
        let guard = BuildGuard::begin(directory.path())?;
        fs::write(
            guard.staging_directory().join("graph.json"),
            version.to_string(),
        )?;
        guard.commit_with_artifacts(&["graph.json"])?;
    }
    let generations = fs::read_dir(directory.path().join(".compass-generations"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    assert_eq!(generations.len(), 2);
    assert_eq!(
        fs::read_to_string(BuildGuard::resolve_artifact(
            directory.path(),
            "graph.json"
        )?)?,
        "3"
    );
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
    fs::create_dir_all(cache_root.join("compass-out/cache/ast/v0.9.21"))?;
    fs::write(
        cache_root.join("compass-out/cache/ast/v0.9.21/stale.json"),
        "{}",
    )?;
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

    let default_cache = Cache::open(&root, CacheOptions::output_directory(Some(&cache_root)))?;
    assert!(
        default_cache
            .directory(&CacheKind::Ast, None)
            .ends_with(format!("ast/v5/e{CACHE_ENCODING_VERSION}"))
    );
    assert!(!cache_root.join("compass-out/cache/ast/v0.9.21").exists());

    let mut cache = Cache::open(&root, CacheOptions::output_directory(Some(&cache_root)))?;
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
        cache.load(&source, &CacheKind::Semantic, Some("new"), true)?,
        None
    );
    let stored = cache
        .load(&source, &CacheKind::Semantic, None, true)?
        .ok_or("current cache entry")?;
    assert!(
        stored["nodes"][0]["source_file"]
            .as_str()
            .is_some_and(|path| path.ends_with("main.md"))
    );
    assert!(
        stored["nodes"][1]["source_file"]
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
    assert!(
        Cache::open(
            root.join("missing-root"),
            CacheOptions::output_directory(None)
        )
        .is_err()
    );
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
    fs::write(root.join(".compassignore"), "excluded.rs\n")?;
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
    let mut index = StatHashIndex::load(directory.path(), "compass-out");
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
    fs::remove_file(guard.staging_directory().join(".compass-build-incomplete"))?;
    assert!(guard.commit().is_err());
    Ok(())
}

#[test]
fn program_cache_is_path_sensitive_and_namespace_isolated() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;
    let syntax = CacheKind::ProgramSyntax {
        ir_schema: 1,
        provider_version: "tree-sitter-rust/1".to_owned(),
    };
    let artifact = CacheKind::ProgramArtifact {
        ir_schema: 1,
        decoder_version: "scip/1".to_owned(),
    };
    cache.save_program(
        &syntax,
        "src/a.rs:aaaaaaaa",
        &json!({"source_file":"src/a.rs"}),
    )?;
    cache.save_program(
        &syntax,
        "src/b.rs:aaaaaaaa",
        &json!({"source_file":"src/b.rs"}),
    )?;
    cache.save_program(&artifact, "index.scip:bbbbbbbb", &json!({"kind":"scip"}))?;
    let syntax_directory = cache.directory(&syntax, None);
    assert!(syntax_directory.ends_with(format!("e{CACHE_ENCODING_VERSION}")));
    assert!(
        fs::read_dir(&syntax_directory)?
            .filter_map(Result::ok)
            .all(|entry| entry
                .path()
                .extension()
                .is_some_and(|value| value == "msgpack"))
    );

    let first: serde_json::Value = cache
        .load_program(&syntax, "src/a.rs:aaaaaaaa")?
        .ok_or("missing first syntax entry")?;
    let second: serde_json::Value = cache
        .load_program(&syntax, "src/b.rs:aaaaaaaa")?
        .ok_or("missing second syntax entry")?;
    assert_eq!(first["source_file"], "src/a.rs");
    assert_eq!(second["source_file"], "src/b.rs");

    let program_entry = |logical_key: &str| {
        let digest = Sha256::digest(logical_key.as_bytes());
        syntax_directory.join(format!("{digest:x}.msgpack"))
    };
    let first_entry = program_entry("src/a.rs:aaaaaaaa");
    assert!(
        first_entry.is_file(),
        "missing first MessagePack syntax entry"
    );
    let legacy_directory = syntax_directory.parent().ok_or("missing encoding parent")?;
    let legacy_entry = legacy_directory.join(
        first_entry
            .file_stem()
            .ok_or("missing MessagePack cache stem")?,
    );
    let legacy_entry = legacy_entry.with_extension("json");
    fs::write(&legacy_entry, serde_json::to_vec(&first)?)?;
    fs::remove_file(&first_entry)?;
    assert!(
        cache
            .load_program::<serde_json::Value>(&syntax, "src/a.rs:aaaaaaaa")?
            .is_none(),
        "hard cutover must not decode the legacy JSON cache"
    );

    let second_entry = program_entry("src/b.rs:aaaaaaaa");
    assert!(
        second_entry.is_file(),
        "missing second MessagePack syntax entry"
    );
    fs::write(&second_entry, b"not-messagepack")?;
    assert!(
        cache
            .load_program::<serde_json::Value>(&syntax, "src/b.rs:aaaaaaaa")?
            .is_none()
    );
    cache.save_program(&syntax, "src/b.rs:aaaaaaaa", &second)?;
    assert!(
        cache
            .load_program::<serde_json::Value>(&artifact, "src/a.rs:aaaaaaaa")?
            .is_none()
    );

    let live = ["src/a.rs:aaaaaaaa".to_owned()].into_iter().collect();
    assert_eq!(cache.prune_program(&syntax, &live)?, 1);
    assert!(
        cache
            .load_program::<serde_json::Value>(&syntax, "src/b.rs:aaaaaaaa")?
            .is_none()
    );
    assert!(
        cache
            .load_program::<serde_json::Value>(&artifact, "index.scip:bbbbbbbb")?
            .is_some()
    );
    Ok(())
}
