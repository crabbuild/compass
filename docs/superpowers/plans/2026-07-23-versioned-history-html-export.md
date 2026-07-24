# Versioned History HTML Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export every preferred materialized Compass realization as one offline HTML file where selecting a Git commit displays that commit’s complete code graph.

**Architecture:** The immutable history store stays authoritative. The CLI selects validated preferred realizations, derives topology-aware timeline metadata and parent diffs, and streams them into a no-clobber staged export. The HTML contains a small manifest plus one independently compressed payload per commit, so the browser can validate and decode only the selected graph, isolate corruption, and retain at most a small LRU of decoded snapshots. The viewer reuses the current Compass graph canvas behavior instead of creating a second divergent renderer.

**Tech Stack:** Rust (`compass-files`, `compass-cli`, `compass-history`, `compass-output`), `serde_json`, SHA-256, `flate2`, `base64`, static HTML/CSS/JavaScript, vendored `vis-network` 9.1.6 and a pinned MIT-licensed DEFLATE decoder.

## Global Constraints

- `compass history export --output PATH` defaults to HTML only with no revision; `--format html` is equivalent.
- Existing `history export REV --format graph-json|compass-out --output PATH` behavior is unchanged.
- Export only preferred, validated materializations. Do not build absent commits or expose alternates.
- The HTML embeds data and renderer; it makes no network request.
- An existing output fails. `--force` confirms only output over 256 MiB and never overwrites.
- Writes use a staged, atomic, no-clobber publish primitive and leave no partial destination.
- Default view is a selected commit’s full graph; parent comparison is opt-in and merge parents are explicit.
- Full SHA and realization IDs are embedded. `#commit=<full-sha>` and browser history restore selection.
- Timeline order follows the embedded parent DAG, not lexicographic SHA order. Select materialized `HEAD` by default; otherwise select the nearest materialized first-parent ancestor, then the newest topological leaf.
- Missing Git objects after a history rewrite do not make export fail. Stored SHA/parents/graph data remain authoritative and missing subject/author/date use explicit “unavailable” presentation.
- Never embed the absolute repository path. Use a sanitized repository basename unless the user supplies `--title`.
- Keep the full exact graph payload for every commit. Above the current 5,000-node rendering limit, open in community overview mode and allow search/drill-down without discarding exact data.
- Parent comparison uses Compass `diff_records` semantics and is disabled with an explanatory warning when the parent is absent or build profiles are not comparable.
- The archive verifies each compressed payload before decoding it. A broken payload disables only that commit.
- Commit rail, graph controls, and inspector meet WCAG 2.2 AA, remain usable at 320 CSS px, use 44×44 px touch targets, and respect reduced motion.
- Use CSS custom-property tokens for canvas, panels, text, focus, additions, removals, and changed records. Honor system light/dark preference and provide a keyboard-accessible in-session theme toggle; both themes meet AA contrast.
- Treat the parent-DAG rail as the visual signature: crisp topology lanes and commit strata, not generic nested cards, glass blur, gradient text, or decorative metric tiles.
- Switching commits preserves positions for stable node IDs, keeps a surviving node selection, destroys old network listeners, and maintains an LRU of at most three decoded graphs.
- A strict Content Security Policy enforces the offline boundary; string searches for `https://` are not a security test.
- Treat commit metadata and every graph attribute as untrusted. Validate structural IDs, use `textContent`/DOM construction for labels, and never interpolate payload strings into HTML, CSS, selectors, or event-handler source.
- Manifest corruption is a page-level fatal error with recovery guidance; payload corruption remains commit-local.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/compass-files/src/atomic.rs`, `src/lib.rs`, `tests/atomic_contract.rs` | Staged no-clobber publication used by single-file history export. |
| `crates/compass-history/src/git.rs`, `src/lib.rs`, `tests/git_contract.rs` | Optional presentation metadata and topology/default-selection contracts. |
| `crates/compass-output/assets/vis-network-9.1.6.min.js`, `assets/fflate-0.8.2.min.js` | Pinned offline graph and payload-decompression libraries. |
| `crates/compass-output/src/html.rs`, `src/history_html.rs`, `src/lib.rs`, `tests/history_html.rs` | Shared graph canvas, streamed archive, lazy viewer, and renderer contracts. |
| `crates/compass-cli/src/history_commands.rs`, `tests/history_cli.rs` | New no-revision syntax and validated snapshot collection. |
| `crates/compass-cli/src/help.rs`, `tests/coverage_paths.rs`, `docs/reference/commands.md`, `docs/reference/outputs.md`, `docs/guides/versioned-history.md`, `crates/compass-cli/assets/compass-skill/references/history.md` | Command documentation and assertions. |
| `tests/history-html/package.json`, `tests/history-html/time-travel.spec.mjs`, `.github/workflows/compass-ci.yml` | Real Chromium `file://` acceptance coverage. |
| `THIRD_PARTY_NOTICES.md` | `vis-network` and DEFLATE decoder source/version/license notices. |

## Canonical Terms

- **Materialized commit:** a Git commit with at least one published Compass realization.
- **Preferred realization:** the single realization selected by the history store for a materialized commit.
- **Timeline entry:** exported metadata for one preferred realization, whether or not its Git object still exists.
- **Graph payload:** the independently compressed, digested exact `GraphDocument` for one timeline entry.
- **Comparable parent:** an embedded parent realization whose normalized build profile is semantically comparable to the selected realization.
- **Overview mode:** a derived community meta-graph used only for rendering large graphs; the exact payload remains embedded and searchable.

## Task 0: Add staged atomic no-clobber file publication

**Files:**
- Modify: `crates/compass-files/src/atomic.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Create: `crates/compass-files/tests/atomic_contract.rs`

**Interfaces:** Produces `PreparedFile::new`, `PreparedFile::writer`, `PreparedFile::len`, and `PreparedFile::publish_noclobber`. Task 2 streams the HTML into it; Task 3 decides whether to publish after seeing the exact size.

- [ ] **Step 1: Write failing no-clobber and cleanup tests**

```rust
#[test]
fn prepared_file_publishes_without_overwriting() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("history.html");
    std::fs::write(&destination, "existing")?;
    let mut prepared = PreparedFile::new(&destination)?;
    prepared.writer().write_all(b"replacement")?;
    assert!(matches!(prepared.publish_noclobber(), Err(FileError::DestinationExists(_))));
    assert_eq!(std::fs::read_to_string(destination)?, "existing");
    Ok(())
}

#[test]
fn dropping_unpublished_prepared_file_removes_staging() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("history.html");
    { let mut prepared = PreparedFile::new(&destination)?; prepared.writer().write_all(b"partial")?; }
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
    Ok(())
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test -p compass-files --test atomic_contract prepared_file`

Expected: compilation fails because `PreparedFile` is not defined.

- [ ] **Step 3: Implement the staged writer**

Create the temporary file in the destination directory with owner-only permissions. `publish_noclobber` must flush and `sync_all`, atomically link/persist without replacing an existing destination, sync the parent directory, and delete the staging name. `Drop` removes unpublished staging. Keep the platform-specific no-clobber operation behind one private helper and exercise it on the existing Linux, macOS, and Windows CI matrix.

```rust
#[error("destination already exists: {0}")]
DestinationExists(PathBuf),
pub struct PreparedFile { destination: PathBuf, staging: PathBuf, file: Option<File>, bytes: u64 }
pub struct PreparedWriter<'a> { file: &'a mut File, bytes: &'a mut u64 }
impl Write for PreparedWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize>;
    fn flush(&mut self) -> std::io::Result<()>;
}
impl PreparedFile {
    pub fn new(destination: &Path) -> Result<Self, FileError>;
    pub fn writer(&mut self) -> PreparedWriter<'_>;
    pub fn len(&self) -> u64;
    pub fn publish_noclobber(mut self) -> Result<(), FileError>;
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo test -p compass-files --test atomic_contract`

Expected: all atomic contract tests pass on the host platform.

Run: `git add crates/compass-files/src/atomic.rs crates/compass-files/src/lib.rs crates/compass-files/tests/atomic_contract.rs && git commit -m "feat(files): add atomic no-clobber publication"`

## Task 1: Add safe Git presentation metadata

**Files:**
- Modify: `crates/compass-history/src/git.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Create: `crates/compass-history/tests/git_contract.rs`

**Interfaces:** Produces `CommitPresentation` and `Repository::presentation(&CommitId) -> Result<Option<CommitPresentation>, HistoryError>`; Task 3 consumes both. `None` means the immutable realization exists but Git no longer has the commit object.

- [ ] **Step 1: Write the failing contract test**

```rust
#[test]
fn presentation_returns_safe_commit_display_fields() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Historian"])?;
    git(directory.path(), &["config", "user.email", "history@example.invalid"])?;
    std::fs::write(directory.path().join("lib.rs"), "pub fn v1() {}\n")?;
    git(directory.path(), &["add", "lib.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feat: add history view"])?;
    let repository = Repository::discover(directory.path())?;
    let presentation = repository.presentation(&repository.resolve("HEAD")?)?.ok_or("metadata")?;
    assert_eq!(presentation.subject, "feat: add history view");
    assert_eq!(presentation.author, "Compass Historian");
    assert!(presentation.authored_at.contains('T'));
    let missing = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?;
    assert_eq!(repository.presentation(&missing)?, None);
    Ok(())
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p compass-history --test git_contract presentation_returns_safe_commit_display_fields`

Expected: compilation fails because optional `Repository::presentation` does not exist.

- [ ] **Step 3: Implement NUL-delimited Git parsing**

Add the type and method below. Use `git_output`, not `git_line`, since the format includes NUL delimiters. Require exactly five split fields, an empty trailing field, and the requested SHA. Reject non-UTF-8 or multiline display fields with `HistoryError::Git`.

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitPresentation { pub subject: String, pub author: String, pub authored_at: String }

pub fn presentation(&self, commit: &CommitId) -> Result<Option<CommitPresentation>, HistoryError> {
    if !self.has_commit_object(commit)? { return Ok(None); }
    let output = git_output(&self.root, &["show", "-s", "--format=%H%x00%s%x00%an%x00%aI%x00", "--end-of-options", commit.as_str()])?;
    let fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    let [actual, subject, author, authored_at, trailing] = fields.as_slice() else {
        return Err(HistoryError::Git("Git returned malformed commit presentation".to_owned()));
    };
    if !trailing.is_empty() || std::str::from_utf8(actual).ok() != Some(commit.as_str()) {
        return Err(HistoryError::Git("Git returned an unexpected commit presentation".to_owned()));
    }
    let field = |bytes: &[u8], name: &str| -> Result<String, HistoryError> {
        let value = std::str::from_utf8(bytes).map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 {name}: {error}")))?;
        if value.contains(['\r', '\n']) { return Err(HistoryError::Git(format!("Git returned multiline {name}"))); }
        Ok(value.to_owned())
    };
    Ok(Some(CommitPresentation { subject: field(subject, "subject")?, author: field(author, "author")?, authored_at: field(authored_at, "timestamp")? }))
}
```

Implement `has_commit_object` with `git cat-file --batch-check=%(objectname) %(objecttype)` and pass `<sha>^{commit}\n` on stdin through a new private `git_output_with_input` helper. Treat the machine-readable `<expression> missing` response as `false`, accept only `<full-sha> commit` as `true`, and reject every other response as `HistoryError::Git`; do not infer absence from process exit codes or localized stderr. Re-export with `pub use git::{CommitPresentation, GitTargetLimitation, Repository, WorktreeGuard};`.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo test -p compass-history --test git_contract presentation_returns_safe_commit_display_fields`

Expected: both commands pass.

Run: `git add crates/compass-history/src/git.rs crates/compass-history/src/lib.rs crates/compass-history/tests/git_contract.rs && git commit -m "feat(history): expose commit presentation metadata"`

## Task 2: Implement the self-contained history renderer

**Files:**
- Create: `crates/compass-output/assets/vis-network-9.1.6.min.js`
- Create: `crates/compass-output/assets/fflate-0.8.2.min.js`
- Modify: `THIRD_PARTY_NOTICES.md`
- Modify: `Cargo.toml`
- Modify: `crates/compass-output/Cargo.toml`
- Modify: `crates/compass-output/src/html.rs`
- Create: `crates/compass-output/src/history_html.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Create: `crates/compass-output/tests/history_html.rs`

**Interfaces:** Produces shared `GraphViewModel`, `HistoryTimelineEntry`, `HistorySnapshot`, `HistoryArchiveBuilder`, and `PreparedHistoryHtml`; Task 3 consumes them. `PreparedHistoryHtml::len` exposes exact staged size and `publish_noclobber` delegates to Task 0.

- [ ] **Step 1: Vendor the pinned library and record its license**

Run: `curl --fail --location --silent --show-error https://unpkg.com/vis-network@9.1.6/standalone/umd/vis-network.min.js --output crates/compass-output/assets/vis-network-9.1.6.min.js && curl --fail --location --silent --show-error https://unpkg.com/fflate@0.8.2/umd/index.js --output crates/compass-output/assets/fflate-0.8.2.min.js && shasum -a 256 crates/compass-output/assets/vis-network-9.1.6.min.js crates/compass-output/assets/fflate-0.8.2.min.js`

Expected SHA-256 digests:

```text
576bb887733eb01bb52ee75b90ef46d818454de5fddb5b616fb8a298d307ca12  crates/compass-output/assets/vis-network-9.1.6.min.js
c3b34f2e9f5e74d4d7d64e01cac7a0c01954c6c406414d42185c7b53d6875ddf  crates/compass-output/assets/fflate-0.8.2.min.js
```

Add both MIT licenses, versions, and source URLs to `THIRD_PARTY_NOTICES.md`; retain source headers. Add workspace `flate2 = "1.1.9"` and `base64 = "0.22.1"` dependencies to `compass-output`.

- [ ] **Step 2: Write the failing renderer contracts**

```rust
#[test]
fn history_html_is_offline_deterministic_and_embeds_all_commits() -> Result<(), Box<dyn std::error::Error>> {
    let rendered = render_fixture(vec![entry("b".repeat(40), "Second"), entry("a".repeat(40), "First")])?;
    let html = rendered.text();
    assert!(html.contains("default-src 'none'"));
    assert!(!html.contains("<script src="));
    assert!(html.contains("vis.Network"));
    assert!(html.contains("compass-history-manifest/v1"));
    assert_eq!(html.matches("data-compass-payload=").count(), 2);
    assert!(html.contains("function decodeSelectedPayload"));
    Ok(())
}

#[test]
fn history_html_escapes_script_terminators() -> Result<(), Box<dyn std::error::Error>> {
    let rendered = render_fixture(vec![entry_with_label("</script><img src=x>")])?;
    let html = rendered.text();
    assert!(!html.contains("</script><img"));
    assert_eq!(decode_fixture_payload(html)?.nodes[0].label(), "</script><img src=x>");
    Ok(())
}

#[test]
fn corrupt_payload_is_isolated_from_other_commits() -> Result<(), Box<dyn std::error::Error>> {
    let mut rendered = render_fixture(two_entries())?;
    rendered.corrupt_payload(FIRST_COMMIT);
    assert!(rendered.manifest().entry(FIRST_COMMIT).is_err());
    assert!(rendered.manifest().entry(SECOND_COMMIT).is_ok());
    Ok(())
}
```

- [ ] **Step 3: Add typed payload rendering and safe serialization**

```rust
#[derive(Clone, Debug, Serialize)]
pub struct HistoryArchiveHeader {
    pub title: String, pub exported_at: String, pub default_commit: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistoryDiffCounts {
    pub nodes_added: u64, pub nodes_removed: u64, pub nodes_changed: u64,
    pub edges_added: u64, pub edges_removed: u64, pub edges_changed: u64,
    pub hyperedges_added: u64, pub hyperedges_removed: u64, pub hyperedges_changed: u64,
}
#[derive(Clone, Debug, Serialize)]
pub enum HistoryRecordKind { Node, Edge, Hyperedge }
#[derive(Clone, Debug, Serialize)]
pub enum HistoryChangeKind { Added, Removed, Changed }
#[derive(Clone, Debug, Serialize)]
pub struct HistoryChangedRecord {
    pub record: HistoryRecordKind, pub change: HistoryChangeKind, pub key: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistoryParentDiff {
    pub parent: String, pub comparable: bool, pub unavailable_reason: Option<String>,
    pub counts: HistoryDiffCounts, pub changed: Vec<HistoryChangedRecord>, pub truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistoryTimelineEntry {
    pub commit: String, pub parents: Vec<String>, pub realization: String,
    pub profile_digest: String, pub fingerprint: String,
    pub subject: Option<String>, pub author: Option<String>, pub authored_at: Option<String>,
    pub lane: usize, pub node_count: u64, pub edge_count: u64, pub overview: bool,
    pub parent_diffs: Vec<HistoryParentDiff>,
}
#[derive(Clone, Debug, Serialize)]
pub struct GraphViewModel {
    pub nodes: Vec<Value>, pub edges: Vec<Value>, pub legend: Vec<Value>,
    pub hyperedges: Value, pub source_nodes: usize, pub source_edges: usize,
}
#[derive(Clone, Debug, Serialize)]
struct HistoryPayloadManifest {
    timeline: HistoryTimelineEntry, element_id: String, sha256: String,
    compressed_bytes: u64, uncompressed_bytes: u64,
}
pub struct HistorySnapshot {
    pub timeline: HistoryTimelineEntry,
    pub document: GraphDocument,
    pub overview: Option<GraphViewModel>,
}
pub struct HistoryArchiveBuilder {
    prepared: PreparedFile, header: HistoryArchiveHeader,
    entries: Vec<HistoryPayloadManifest>, seen: BTreeSet<String>,
}
pub struct PreparedHistoryHtml { prepared: PreparedFile, entries: usize }
impl HistoryArchiveBuilder {
    pub fn new(destination: &Path, header: HistoryArchiveHeader) -> Result<Self, OutputError>;
    pub fn append(&mut self, snapshot: HistorySnapshot) -> Result<(), OutputError>;
    pub fn finish(self) -> Result<PreparedHistoryHtml, OutputError>;
}
impl PreparedHistoryHtml {
    pub fn len(&self) -> u64;
    pub fn publish_noclobber(self) -> Result<(), OutputError>;
}
```

Extract the current node/edge/community transformation and graph-control script from `html.rs` into a reusable `GraphViewModel`/shared template section; keep existing static HTML output byte-compatible where its parity tests require it. For graphs above 5,000 nodes, compute the same community overview model during export and store it beside the exact document. Each `append` canonicalizes one payload envelope containing the exact document plus optional overview, computes SHA-256 over the uncompressed bytes, compresses it with `flate2::write::ZlibEncoder` at the default level, base64-encodes it, writes an inert per-commit payload element, records compressed/uncompressed sizes, and drops the document before the next snapshot. The browser must use fflate's `unzlibSync` for the matching zlib wrapper. `finish` writes the small `compass-history-manifest/v1` separately from payloads. Escape `<`, `>`, `&`, U+2028, and U+2029 in the manifest. Duplicate commits fail. Inline both pinned assets and a CSP with `default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; worker-src blob:`.

- [ ] **Step 4: Implement browser-side behavior**

```javascript
async function selectCommit(commit, { updateHistory = true } = {}) {
  const entry = entriesByCommit.get(commit) || entriesByCommit.get(manifest.defaultCommit);
  setViewerBusy(true, `Loading ${shortSha(entry.commit)}`);
  const previousPositions = captureStableNodePositions();
  const graphDocument = await decodeSelectedPayload(entry);
  selectedCommit = entry.commit;
  renderCommitRail(entry.commit);
  replaceGraph(graphDocument, { previousPositions, selectedNodeId });
  renderInspector(entry);
  document.title = `Compass history — ${entry.commit.slice(0, 12)}`;
  if (updateHistory) history.pushState({ commit: entry.commit }, "", `#commit=${entry.commit}`);
  setViewerBusy(false, `Showing ${shortSha(entry.commit)}`);
}
function restoreCommitFromLocation() {
  const requested = new URLSearchParams(location.hash.slice(1)).get("commit");
  const commit = entriesByCommit.has(requested) ? requested : manifest.defaultCommit;
  selectCommit(commit, { updateHistory: false });
}
function compareWithParent(parentCommit) {
  const diff = entriesByCommit.get(selectedCommit).parentDiffs.find(value => value.parent === parentCommit);
  if (!diff || !diff.comparable) return showComparisonUnavailable(diff);
  applyEmbeddedDiff(diff); renderComparisonSummary(diff);
}
```

`decodeSelectedPayload` locates only the selected payload element, base64-decodes and inflates it, checks exact uncompressed length and SHA-256 before parsing, then inserts it into a three-entry LRU. Digest verification may use Web Crypto when available but must include a local fallback that works from `file://`. Decode/inflate in a Blob Worker for payloads over 5 MiB so the commit rail remains responsive.

Implement a virtualized searchable commit rail (SHA, subject, author, unavailable-metadata fallback), full graph canvas, and inspector. Use a real listbox or button list with `aria-current`, Up/Down/Home/End navigation, visible focus, an `aria-live` load/error status, and a canvas text alternative summarizing node/edge counts and selected node relationships. At 760 px collapse the inspector into a drawer; at 520 px collapse the rail into a modal sheet. Touch targets are at least 44×44 px. Stop physics under `prefers-reduced-motion`. Drive light/dark/highlight colors through named CSS variables, initialize from `prefers-color-scheme`, and update both DOM and `vis-network` colors from an accessible theme toggle without using storage APIs.

`replaceGraph` must destroy the old `vis.Network` and its listeners, reuse positions for IDs present in both commits, seed new nodes near connected surviving nodes, keep a selected node when its ID survives, and show “node not present in this commit” otherwise. Embedded Compass diffs highlight added/changed current records and list removed records in the inspector. Missing or incomparable parents render disabled controls with reasons. Register `popstate` and `hashchange` to restore fragments without duplicate history entries.

- [ ] **Step 5: Verify and commit renderer work**

Add duplicate-SHA, independent-payload, digest, compressed round-trip, CSP, named-viewer-hook, stable-layout, fragment, accessibility-marker, theme-token, responsive-breakpoint, external-reference, and escaping tests. Test 0, 1, 5,001, and 50,000-node snapshots; the latter two must retain exact payloads while selecting overview rendering. Run: `cargo test -p compass-output --test history_html && cargo test -p compass-output`

Expected: both commands pass.

Run: `git add Cargo.toml THIRD_PARTY_NOTICES.md crates/compass-output/Cargo.toml crates/compass-output/assets/vis-network-9.1.6.min.js crates/compass-output/assets/fflate-0.8.2.min.js crates/compass-output/src/html.rs crates/compass-output/src/history_html.rs crates/compass-output/src/lib.rs crates/compass-output/tests/history_html.rs && git commit -m "feat(output): render scalable offline history viewer"`

## Task 3: Wire preferred history to `compass history export`

**Files:**
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:** Consumes Tasks 0–2 plus `HistoryStore::{list,validate,artifacts,diff_records}`. Produces `compass history export --output PATH [--format html] [--title NAME] [--force]`.

- [ ] **Step 1: Write failing CLI coverage**

Extend `history_commands_inspect_prefer_and_export_published_realizations` with a second committed/preferred graph, then add:

```rust
let output = directory.path().join("history.html");
let result = run(compass, directory.path(), &["history", "export", "--output", output.to_str().unwrap()])?;
assert!(result.status.success());
let html = std::fs::read_to_string(&output)?;
assert!(html.contains("compass-history-manifest/v1"));
assert!(html.contains(first_commit.as_str()) && html.contains(second_commit.as_str()));
assert!(html.contains("default-src 'none'"));
assert!(!html.contains("<script src="));
assert_eq!(run(compass, directory.path(), &["history", "export", "HEAD", "--format", "html", "--output", "bad.html"])?.status.code(), Some(2));
assert_eq!(run(compass, directory.path(), &["history", "export", "--format", "graph-json", "--output", "bad.json"])?.status.code(), Some(2));
```

Add fixtures for a two-parent merge, an unmaterialized parent, different build profiles, and a stored commit whose Git object was deleted. Assert the export keeps all preferred entries, chooses materialized HEAD as `default_commit`, uses DAG order rather than SHA order, embeds a disabled reason for missing/incomparable parents, falls back to unavailable Git metadata, omits the absolute temporary directory, and leaves output absent for empty history.

- [ ] **Step 2: Verify the focused test fails**

Run: `cargo test -p compass-cli --test history_cli history_commands_inspect_prefer_and_export_published_realizations`

Expected: failure because the current handler requires `REV`.

- [ ] **Step 3: Make parsing format-aware**

Replace the parser tuple with `HistoryOptions { positionals, format: Option<String>, output: Option<PathBuf>, title: Option<String>, force: bool }`; only the no-revision HTML export accepts `title` and `force`. Reject duplicates and empty/over-200-scalar titles. Use this exact decision function while retaining `text` as the default for all non-export commands:

```rust
fn export_format(options: &HistoryOptions) -> Result<&str, CommandFailure> {
    match (options.positionals.is_empty(), options.format.as_deref()) {
        (true, None | Some("html")) => Ok("html"),
        (true, Some(_)) => Err(usage("history export without REV only supports --format html")),
        (false, Some("graph-json" | "compass-out")) => Ok(options.format.as_deref().unwrap()),
        (false, Some("html")) => Err(usage("history export --format html accepts no REV")),
        (false, None) => Err(usage("history export REV requires --format graph-json or compass-out")),
    }
}
```

- [ ] **Step 4: Implement collection and size-gated atomic export**

Add `const HISTORY_HTML_CONFIRM_BYTES: u64 = 256 * 1024 * 1024;`. The `export_history_html` helper collects `history.list(None)` entries with `preferred == true`, validates every ID, and builds an index by commit. Compute the default commit as: embedded `HEAD`; otherwise nearest embedded first-parent ancestor of `HEAD`; otherwise a parent-DAG leaf ordered by available authored timestamp and full SHA. Topologically order the rail newest-first while preserving merge lanes; use SHA only as a deterministic tie-breaker. Never call `resolve_or_materialize`.

Fail fast when `output.exists()`, then still rely on Task 0's atomic no-clobber publication to close the race where another process creates the destination during export.

```rust
struct ExportCandidate {
    published: PublishedVersion,
    commit: CommitId,
    presentation: Option<CommitPresentation>,
}
struct OrderedCommit { commit: String, lane: usize }
fn choose_default_commit(repository: &Repository, entries: &BTreeMap<String, ExportCandidate>) -> Result<String, CommandFailure>;
fn order_timeline(entries: &BTreeMap<String, ExportCandidate>, default: &str) -> Result<Vec<OrderedCommit>, CommandFailure>;
fn collect_parent_diffs(history: &HistoryStore, selected: &PublishedVersion, entries: &BTreeMap<String, ExportCandidate>) -> Result<Vec<HistoryParentDiff>, CommandFailure>;
fn timeline_entry(selected: &PublishedVersion, lane: usize, presentation: Option<CommitPresentation>, parent_diffs: Vec<HistoryParentDiff>) -> HistoryTimelineEntry;
fn build_history_overview(document: &GraphDocument, node_limit: usize) -> Result<Option<GraphViewModel>, OutputError>;
```

`order_timeline` uses reverse Kahn sorting: start with commits that have no embedded children, emit the default commit’s component first, and release a parent only after all embedded children were considered. Order other leaf components by authored timestamp and SHA. Detect and fail on an impossible cycle rather than silently dropping entries. Assign deterministic lane numbers in a second pass so merges and disconnected/rewrite-retained histories are visually distinguishable.

```rust
let header = HistoryArchiveHeader {
    title: title.unwrap_or_else(|| repository.root().file_name().and_then(OsStr::to_str).unwrap_or("Compass history").to_owned()),
    exported_at: OffsetDateTime::now_utc().format(&Rfc3339).map_err(runtime)?,
    default_commit: default_commit.clone(),
};
let entry_count = ordered.len();
let mut archive = HistoryArchiveBuilder::new(output, header).map_err(runtime)?;
for ordered_commit in ordered {
    let candidate = preferred_by_commit.get(&ordered_commit.commit).ok_or_else(|| runtime("ordered commit disappeared"))?;
    let published = &candidate.published;
    history.validate(&published.id).map_err(runtime)?;
    let parent_diffs = collect_parent_diffs(&history, published, &preferred_by_commit)?;
    let document = history.artifacts(&published.id).map_err(runtime)?.artifacts.document;
    archive.append(HistorySnapshot {
        timeline: timeline_entry(published, ordered_commit.lane, candidate.presentation.clone(), parent_diffs),
        overview: build_history_overview(&document, 5_000).map_err(runtime)?,
        document,
    }).map_err(runtime)?;
}
let prepared = archive.finish().map_err(runtime)?;
if prepared.len() > HISTORY_HTML_CONFIRM_BYTES && !force {
    return Err(runtime(format!("history HTML is {} bytes; rerun with --force for exports larger than 256 MiB", prepared.len())));
}
let bytes = prepared.len();
prepared.publish_noclobber().map_err(runtime)?;
Ok(format!("exported {entry_count} preferred history realizations to {} ({bytes} bytes)", output.display()))
```

`collect_parent_diffs` calls `normalized_profile_for_comparison` for each embedded parent. For comparable pairs, stream only `Node`, `Edge`, and `Hyperedge` changes through `diff_records` into bounded summary/detail records; cap detailed changed IDs at 5,000 per parent while retaining exact counts and a `truncated` flag. Missing or incompatible parents get a reason and no misleading diff. Refuse an empty entry set with `no preferred materialized realizations to export`. Dropping an over-threshold unforced `PreparedHistoryHtml` removes staging.

- [ ] **Step 5: Verify and commit CLI work**

Add explicit HTML, `--title`, legacy format, non-export `--force`, existing target with `--force`, exact threshold, staging cleanup, rewritten-history metadata fallback, DAG ordering, HEAD fallback, missing parent, merge parents, comparable diff, profile mismatch, and diff-truncation tests. Run: `cargo test -p compass-cli --test history_cli && cargo test -p compass-cli history_commands::tests`

Expected: both commands pass; existing `graph-json` and `compass-out` contracts stay green.

Run: `git add crates/compass-cli/src/history_commands.rs crates/compass-cli/tests/history_cli.rs && git commit -m "feat(history): export offline time-travel HTML"`

## Task 4: Update public help and documentation

**Files:**
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/coverage_paths.rs`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/outputs.md`
- Modify: `docs/guides/versioned-history.md`
- Modify: `crates/compass-cli/assets/compass-skill/references/history.md`

- [ ] **Step 1: Write the failing help assertion**

```rust
let help = invoke(Frontend::Compass, &["help", "history", "export"]);
assert_eq!(help.code, 0);
assert!(help.stdout.contains("history export [--format html] --output <PATH> [--title <NAME>] [--force]"));
assert!(help.stdout.contains("default: html"));
assert!(help.stdout.contains("256 MiB"));
```

- [ ] **Step 2: Verify it fails**

Run: `cargo test -p compass-cli --test coverage_paths history_export_help_describes_offline_time_travel`

Expected: failure because current help requires `REV`.

- [ ] **Step 3: Document exact usage and safety rules**

Use these examples in all five surfaces:

```text
compass history export --output history.html
compass history export --format html --output history.html
compass history export --output history.html --title "Payments architecture"
compass history export --output history.html --force
```

Explain that the bundle includes all preferred materialized commits, including entries retained after Git rewrites; missing Git presentation fields are labeled unavailable. It opens offline, defaults to the exact full graph when a row is clicked, enters overview mode above 5,000 rendered nodes without dropping exact payload data, and offers optional parent comparison only when an embedded parent is semantically comparable. Document topology ordering, HEAD fallback, `--title`, payload compression/lazy decoding, the three-snapshot LRU, CSP, the 256 MiB confirmation, no absolute local path, and the no-overwrite rule. Retain separate revision-specific examples.

- [ ] **Step 4: Verify and commit docs**

Run: `cargo test -p compass-cli --test coverage_paths history_export_help_describes_offline_time_travel && rg -n "history export \[--format html\] --output|256 MiB|offline|overview mode|Git rewrite|--title" crates/compass-cli/src/help.rs docs crates/compass-cli/assets/compass-skill/references/history.md`

Expected: test passes and `rg` returns each documentation surface.

Run: `git add crates/compass-cli/src/help.rs crates/compass-cli/tests/coverage_paths.rs docs/reference/commands.md docs/reference/outputs.md docs/guides/versioned-history.md crates/compass-cli/assets/compass-skill/references/history.md && git commit -m "docs(history): explain offline HTML time travel"`

## Task 5: Add real-browser accessibility and time-travel acceptance coverage

**Files:**
- Create: `crates/compass-output/examples/history_html_fixture.rs`
- Create: `tests/history-html/package.json`
- Create: `tests/history-html/package-lock.json`
- Create: `tests/history-html/time-travel.spec.mjs`
- Modify: `.github/workflows/compass-ci.yml`

**Interfaces:** Consumes the exported file exactly as a user does through `file://`; verifies behavior that Rust string-marker tests cannot prove.

- [ ] **Step 1: Create deterministic browser fixtures**

The Rust example writes three standalone files to a required output directory: `linear.html` (three comparable commits and one persistent node), `merge.html` (two materialized parents), and `corrupt.html` (one deliberately damaged payload plus one valid payload). It uses fixed SHAs/timestamps and 5,001 nodes in one commit to exercise overview mode.

Run: `cargo run -p compass-output --example history_html_fixture -- tests/history-html/generated`

Expected: all three files exist, and rerunning produces byte-identical files.

- [ ] **Step 2: Add pinned Playwright and axe dependencies**

```json
{
  "name": "compass-history-html-tests",
  "private": true,
  "type": "module",
  "scripts": { "test": "playwright test" },
  "devDependencies": {
    "@axe-core/playwright": "4.12.1",
    "@playwright/test": "1.61.1"
  }
}
```

Run: `npm install --prefix tests/history-html --package-lock-only`

Expected: `package-lock.json` pins every transitive dependency and `npm ci --prefix tests/history-html` succeeds.

- [ ] **Step 3: Write failing `file://` acceptance tests**

```javascript
test("selects exact commits, restores history, and never requests network", async ({ page }) => {
  const requests = [];
  page.on("request", request => requests.push(request.url()));
  await page.goto(pathToFileURL(resolve("generated/linear.html")).href);
  await page.getByRole("button", { name: /second commit/i }).click();
  await expect(page).toHaveURL(/#commit=[0-9a-f]{40}$/);
  await expect(page.getByRole("status")).toContainText("Showing");
  await page.goBack();
  await expect(page.getByRole("button", { name: /newest commit/i })).toHaveAttribute("aria-current", "true");
  expect(requests.filter(url => /^https?:/.test(url))).toEqual([]);
});

test("is keyboard, responsive, reduced-motion, and WCAG AA usable", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto(pathToFileURL(resolve("generated/linear.html")).href);
  await page.getByRole("button", { name: /open commit history/i }).click();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await expect(page.getByTestId("physics-state")).toHaveText("paused");
});
```

Add tests for lazy decode (`window.__COMPASS_DEBUG__.decodedCommits <= 1` after load), three-entry LRU after four selections, position reuse for a stable node, merge-parent choice, overview mode retaining exact search, light/dark theme contrast and toggle state, profile-mismatch disabled state, invalid fragment fallback with notice, corrupt-payload isolation, fatal manifest corruption guidance, and hostile commit/node labels that must render as text without executing or creating injected elements.

- [ ] **Step 4: Run the browser suite and verify it fails before viewer completion**

Run: `cargo run -p compass-output --example history_html_fixture -- tests/history-html/generated && npm ci --prefix tests/history-html && npm exec --prefix tests/history-html -- playwright install chromium && npm test --prefix tests/history-html`

Expected before Tasks 2–3 are complete: tests fail on missing time-travel controls. Expected afterward: all Chromium tests pass.

- [ ] **Step 5: Add a pinned CI browser-smoke step and commit**

In `quality-and-parity`, after Rust tests, run the fixture generator, `npm ci --prefix tests/history-html`, `npm exec --prefix tests/history-html -- playwright install --with-deps chromium`, and `npm test --prefix tests/history-html`. Keep generated HTML ignored and regenerated in CI.

Run: `git add crates/compass-output/examples/history_html_fixture.rs tests/history-html/package.json tests/history-html/package-lock.json tests/history-html/time-travel.spec.mjs .github/workflows/compass-ci.yml .gitignore && git commit -m "test(history): exercise HTML time travel in Chromium"`

## Task 6: Complete verification

**Files:** Modify only files where verification exposes a defect.

- [ ] **Step 1: Run test and formatting gates**

Run: `cargo fmt --check && cargo clippy --workspace --lib --bins --locked -- -D warnings && cargo test -p compass-files && cargo test -p compass-history && cargo test -p compass-output && cargo test -p compass-cli && npm test --prefix tests/history-html`

Expected: all commands exit 0.

- [ ] **Step 2: Audit a generated artifact offline**

Run: `rg -n "<script[^>]+src=|<link[^>]+href=|fetch\(|XMLHttpRequest|WebSocket" history.html; rg -n "Content-Security-Policy|compass-history-manifest/v1|data-compass-payload|#commit=|Compare with parent" history.html`

Expected: no result from the first search; the second finds CSP, manifest, independent payloads, deep-link, and comparison markers. The Task 5 browser suite supplies the required `file://` interaction evidence.

- [ ] **Step 3: Refresh Graphify after source changes**

Run: `graphify update .`

Expected: graph update completes successfully.

## Plan Self-Review

- **Spec coverage:** Task 0 guarantees staged no-clobber publication. Tasks 1 and 3 select validated preferred history, tolerate rewritten Git history, order the parent DAG, and embed comparable Compass diffs. Task 2 makes independent compressed payloads, lazy verification/decoding, shared graph controls, stable layout, overview mode, responsive accessibility, and CSP. Task 4 documents the exact contract. Task 5 proves `file://` behavior in Chromium. Task 6 runs project gates and refreshes Graphify.
- **Placeholder scan:** Each task contains exact paths, interfaces, test assertions, commands, expected results, and a commit boundary.
- **Type consistency:** `CommitPresentation` originates in `compass-history`; `PreparedFile` originates in `compass-files`; `HistoryTimelineEntry`, `HistorySnapshot`, `HistoryArchiveBuilder`, and `PreparedHistoryHtml` originate in `compass-output`; `history_commands.rs` is the adapter and never materializes during export.

## Deferred Time-Travel Opportunities

These are valuable follow-ups, deliberately excluded from the first release so the archive and parent-time-travel contract can stabilize:

1. **Pin any commit as comparison baseline.** Compute an on-demand browser topology comparison between two already decoded exact payloads, with a strong warning when profiles differ. Do not precompute all pairs.
2. **Follow a node through time.** Add a compact export-time index of commits where a stable node ID was added, changed, or removed, then let the inspector jump among those commits.
3. **Architecture growth track.** Plot node/edge/community counts along the commit rail and mark discontinuities caused by extraction-profile changes separately from code growth.
4. **Playback controls.** Add Previous/Next and optional autoplay while keeping the commit rail primary; respect reduced motion and pause when the tab is hidden.
5. **Signed exports.** Add an optional detached or embedded signature over manifest and payload digests for authenticity. The v1 SHA-256 fields detect corruption, not malicious replacement.
6. **Range exports.** If real repositories regularly exceed practical browser/file limits even after compression, add an explicit revision-range filter while keeping all materialized commits as the default.
