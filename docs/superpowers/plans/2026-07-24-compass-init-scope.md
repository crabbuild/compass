# Compass Init and Persisted Build Scope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an interactive and scriptable `compass init` command that persists a repository build scope and performs the first structural rebuild, with the same scope automatically reused by `update`, `extract`, and `watch`.

**Architecture:** `compass-files` owns the strict versioned project configuration and one compiled scope matcher shared by discovery and watcher filtering. `compass-cli` loads that configuration after resolving a project root, exposes a stream-oriented init workflow, and delegates the first build to the existing structural update pipeline. Scope is positive-first (`include`) and then subtractive (`exclude` plus ephemeral CLI exclusions).

**Tech Stack:** Rust 2024 (MSRV 1.97), serde, toml 1.1, glob 0.3, existing atomic file helpers, Cargo integration tests, shell completion scripts.

## Global Constraints

- The public configuration path is exactly `<project-root>/.compass/config.toml`.
- The only accepted schema version is `version = 1`; unknown keys and sections are errors.
- Missing or empty `build.include` means every otherwise eligible file.
- Scope entries are project-root-relative and serialize with `/` separators on every platform.
- Absolute entries, root escapes, malformed globs, escaping symlinks, unmatched init includes, and an empty final corpus are rejected.
- Filter order is built-in safety skips, Git ignores, configured includes, configured excludes, then CLI excludes.
- Includes never resurrect built-in skipped, Git-ignored, or excluded paths.
- `--yes` is required for non-terminal initialization; `--force` is required to replace existing configuration.
- A successful config write remains in place if the initial graph build fails.
- Do not persist credentials, provider settings, timestamps, or unrelated build tuning.
- Graphify compatibility behavior, including `graphify init` remaining unknown, must not change.
- Preserve every pre-existing uncommitted change in the working tree.
- Before every commit, inspect `git diff` and use `git add -p` for any file that was already modified before this feature; stage only feature-owned hunks.
- After code changes, run `graphify update .` from `/Users/haipingfu/graphify` as required by the repository instructions.

---

## File structure

Create or modify these focused units:

```text
crates/compass-files/
├── Cargo.toml                         add toml/glob dependencies
├── src/
│   ├── lib.rs                         export config and scope APIs/errors
│   ├── project_config.rs              strict v1 load/render/atomic write
│   ├── scope.rs                       normalize and compile include/exclude rules
│   └── detect.rs                      apply ScopeMatcher in scan and watcher paths
└── tests/
    ├── project_config.rs              schema/path/write contracts
    └── scope_detection.rs             discovery/matcher precedence contracts

crates/compass-core/
├── src/
│   ├── pipeline.rs                    carry BuildScope into every DetectOptions
│   └── watch.rs                       carry the same scope into WatchPathFilter
└── tests/program_pipeline.rs          constructor regression coverage

crates/compass-cli/
├── src/
│   ├── lib.rs                         dispatch, config loading, typed structural build helper
│   ├── init_commands.rs               init parsing, preview, persistence, prompting
│   ├── help.rs                        init help page and root catalog entry
│   └── bin/compass.rs                 route stream-oriented interactive init
└── tests/
    ├── init_cli.rs                    end-to-end noninteractive init/reconfigure tests
    ├── init_interactive.rs            injected-input prompt/cancel tests
    ├── update_cli.rs                  saved scope reuse
    ├── watch_cli.rs                   configured watcher behavior
    └── help_cli.rs                    help/catalog coverage

completions/
├── _compass
├── compass.bash
├── compass.fish
└── compass.ps1

docs/
├── getting-started.md
└── reference/
    ├── commands.md
    └── configuration.md

README.md
Cargo.lock
```

`project_config.rs` owns storage format only. `scope.rs` owns path semantics
only. `detect.rs` consumes the matcher but does not parse TOML.
`init_commands.rs` owns CLI interaction but receives all filesystem behavior
through the public `compass-files` APIs.

---

### Task 1: Strict versioned project configuration

**Files:**
- Modify: `crates/compass-files/Cargo.toml`
- Create: `crates/compass-files/src/project_config.rs`
- Create: `crates/compass-files/src/scope.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Create: `crates/compass-files/tests/project_config.rs`
- Modify: `Cargo.lock`

**Interfaces:**
- Produces: `pub const PROJECT_CONFIG_RELATIVE_PATH: &str`
- Produces: `pub struct ProjectConfig { pub version: u32, pub build: BuildScope }`
- Produces: `pub struct BuildScope { pub include: Vec<String>, pub exclude: Vec<String> }`
- Produces: `ProjectConfig::load(root: &Path) -> Result<Option<ProjectConfig>, FileError>`
- Produces: `ProjectConfig::write(&self, root: &Path) -> Result<PathBuf, FileError>`
- Produces: `ProjectConfig::normalize(self, root: &Path) -> Result<ProjectConfig, FileError>`

- [ ] **Step 1: Add failing schema and deterministic serialization tests**

Create `crates/compass-files/tests/project_config.rs` with focused tests:

```rust
use std::error::Error;
use std::fs;

use compass_files::{BuildScope, ProjectConfig, PROJECT_CONFIG_RELATIVE_PATH};

#[test]
fn project_config_round_trips_with_stable_normalized_text() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn run() {}\n")?;
    let config = ProjectConfig {
        version: 1,
        build: BuildScope {
            include: vec!["./src\\".to_owned(), "src/".to_owned(), "Cargo.toml".to_owned()],
            exclude: vec!["vendor\\**".to_owned()],
        },
    }
    .normalize(root.path())?;

    let path = config.write(root.path())?;
    assert_eq!(path, root.path().join(PROJECT_CONFIG_RELATIVE_PATH));
    assert_eq!(
        fs::read_to_string(&path)?,
        "version = 1\n\n[build]\ninclude = [\"src/\", \"Cargo.toml\"]\nexclude = [\"vendor/**\"]\n"
    );
    assert_eq!(ProjectConfig::load(root.path())?, Some(config));
    Ok(())
}

#[test]
fn project_config_rejects_unknown_fields_and_versions() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join(".compass");
    fs::create_dir(&directory)?;
    let path = directory.join("config.toml");

    fs::write(&path, "version = 1\nsecret = \"nope\"\n[build]\n")?;
    let error = ProjectConfig::load(root.path()).expect_err("unknown key must fail");
    assert!(error.to_string().contains("unknown field"));

    fs::write(&path, "version = 2\n[build]\n")?;
    let error = ProjectConfig::load(root.path()).expect_err("version 2 must fail");
    assert!(error.to_string().contains("unsupported Compass config version 2"));
    Ok(())
}
```

Add separate cases for empty entries, absolute paths, `../escape`, malformed
glob syntax, symlink escape, missing config returning `Ok(None)`, and exact
first-seen de-duplication after separator normalization.

- [ ] **Step 2: Run the new test target and confirm the API is missing**

Run:

```bash
cargo test -p compass-files --test project_config
```

Expected: compilation fails because `BuildScope`, `ProjectConfig`, and
`PROJECT_CONFIG_RELATIVE_PATH` are not exported.

- [ ] **Step 3: Add TOML/glob dependencies and the strict data model**

Add to `crates/compass-files/Cargo.toml`:

```toml
glob.workspace = true
toml.workspace = true
```

In `project_config.rs`, define strict serde structs and explicit version
checking:

```rust
use serde::{Deserialize, Serialize};

use crate::{BuildScope, FileError, write_text_atomic};

pub const PROJECT_CONFIG_RELATIVE_PATH: &str = ".compass/config.toml";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub version: u32,
    #[serde(default)]
    pub build: BuildScope,
}

impl ProjectConfig {
    pub fn new(build: BuildScope) -> Self {
        Self { version: 1, build }
    }

    pub fn load(root: &Path) -> Result<Option<Self>, FileError> {
        let path = root.join(PROJECT_CONFIG_RELATIVE_PATH);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(FileError::Io { path, source }),
        };
        let parsed: Self = toml::from_str(&text)
            .map_err(|source| FileError::ProjectConfigToml { path: path.clone(), source })?;
        if parsed.version != 1 {
            return Err(FileError::UnsupportedProjectConfig {
                path,
                version: parsed.version,
            });
        }
        parsed.normalize(root).map(Some)
    }

    pub fn write(&self, root: &Path) -> Result<PathBuf, FileError> {
        let normalized = self.clone().normalize(root)?;
        let path = root.join(PROJECT_CONFIG_RELATIVE_PATH);
        let text = normalized.render()?;
        write_text_atomic(&path, &text)?;
        Ok(path)
    }
}
```

Add typed `FileError` variants for TOML decoding/encoding, unsupported version,
and invalid scope entries. Render the fixed v1 key order explicitly rather
than relying on map order. Use `toml::Value::String(...).to_string()` when
escaping each array entry.

- [ ] **Step 4: Implement normalization and atomic write behavior**

In `scope.rs`, define the storage type used by Task 1:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BuildScope {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}
```

Normalize `\` to `/`, remove leading `./`, preserve a meaningful trailing `/`,
reject `Component::RootDir`, `Component::Prefix`, and `Component::ParentDir`,
validate glob syntax with `glob::Pattern::new`, and de-duplicate via a
`BTreeSet` used only for membership while retaining first-seen vector order.
For existing literal paths, canonicalize and require the result to remain
under the canonical root.

Keep `BuildScope::default()` as:

```rust
impl Default for BuildScope {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}
```

- [ ] **Step 5: Export the API and run formatting/tests**

Add `mod project_config; mod scope;` only when each corresponding file exists,
and re-export the Task 1 types/constants from `lib.rs`.

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-files --test project_config
```

Expected: formatting succeeds and every project-config test passes.

- [ ] **Step 6: Commit the configuration layer**

```bash
git add crates/compass-files/Cargo.toml crates/compass-files/src/lib.rs crates/compass-files/src/project_config.rs crates/compass-files/src/scope.rs crates/compass-files/tests/project_config.rs
git add -p Cargo.lock
git commit -m "feat(files): add project scope configuration"
```

---

### Task 2: One include/exclude matcher for detection and watch filtering

**Files:**
- Modify: `crates/compass-files/src/scope.rs`
- Modify: `crates/compass-files/src/detect.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Create: `crates/compass-files/tests/scope_detection.rs`
- Modify: `crates/compass-files/tests/contracts.rs`

**Interfaces:**
- Consumes: `BuildScope` from Task 1
- Produces: `pub struct ScopeMatcher`
- Produces: `ScopeMatcher::new(root: &Path, scope: &BuildScope) -> Result<ScopeMatcher, FileError>`
- Produces: `ScopeMatcher::allows(&self, path: &Path) -> bool`
- Produces: `ScopeMatcher::may_match_descendant(&self, directory: &Path) -> bool`
- Produces: `ScopeMatcher::unmatched_includes<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) -> Vec<String>`
- Extends: `DetectOptions { pub scope: BuildScope }`

- [ ] **Step 1: Write failing positive-scope and precedence tests**

Create `scope_detection.rs`:

```rust
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_files::{BuildScope, DetectOptions, ScopeMatcher, detect};

#[test]
fn detection_includes_files_directories_and_globs_then_applies_excludes()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    for (path, text) in [
        ("src/lib.rs", "pub fn lib() {}\n"),
        ("services/api/src/main.rs", "fn api() {}\n"),
        ("services/api/src/generated.rs", "fn generated() {}\n"),
        ("tools/ignored.rs", "fn ignored() {}\n"),
        ("Cargo.toml", "[package]\nname='fixture'\n"),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
        fs::write(path, text)?;
    }

    let detection = detect(
        root.path(),
        &DetectOptions {
            scope: BuildScope {
                include: vec![
                    "src/".to_owned(),
                    "services/*/src".to_owned(),
                    "Cargo.toml".to_owned(),
                ],
                exclude: vec!["**/generated.rs".to_owned()],
            },
            ..DetectOptions::default()
        },
    )?;
    let code = detection.files["code"].join("\n");
    assert!(code.contains("src/lib.rs"));
    assert!(code.contains("services/api/src/main.rs"));
    assert!(code.contains("Cargo.toml"));
    assert!(!code.contains("generated.rs"));
    assert!(!code.contains("tools/ignored.rs"));
    Ok(())
}

#[test]
fn empty_include_means_all_and_includes_do_not_resurrect_ignored_files()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(root.path().join(".gitignore"), "src/ignored.rs\n")?;
    fs::write(root.path().join("src/live.rs"), "fn live() {}\n")?;
    fs::write(root.path().join("src/ignored.rs"), "fn ignored() {}\n")?;
    let detection = detect(
        root.path(),
        &DetectOptions {
            scope: BuildScope {
                include: vec!["src/".to_owned()],
                exclude: Vec::new(),
            },
            ..DetectOptions::default()
        },
    )?;
    assert_eq!(detection.files["code"].len(), 1);
    assert!(detection.files["code"][0].ends_with("src/live.rs"));
    Ok(())
}
```

Add matcher parity assertions for literal file, literal directory, glob-matched
directory descendants, configured exclude, hidden/built-in output directory,
CLI `extra_excludes`, and an unmatched include list.

- [ ] **Step 2: Run the tests and verify the missing field/type failures**

Run:

```bash
cargo test -p compass-files --test scope_detection
```

Expected: compilation fails because `ScopeMatcher` and
`DetectOptions::scope` do not exist.

- [ ] **Step 3: Implement compiled scope patterns**

In `scope.rs`, compile each normalized entry once into:

```rust
enum ScopePattern {
    LiteralFile(String),
    LiteralDirectory(String),
    Glob { raw: String, pattern: glob::Pattern },
}

pub struct ScopeMatcher {
    root: PathBuf,
    includes: Vec<ScopePattern>,
    excludes: Vec<ScopePattern>,
}
```

`ScopePattern::matches` must compare the complete normalized relative path and
each directory ancestor. This makes `services/*/src` admit
`services/api/src/main.rs`. `ScopeMatcher::allows` returns:

```rust
(self.includes.is_empty() || self.includes.iter().any(|rule| rule.matches(relative)))
    && !self.excludes.iter().any(|rule| rule.matches(relative))
```

Use `glob::MatchOptions { case_sensitive: true, require_literal_separator: true, require_literal_leading_dot: true }`.
Classify a non-glob entry as a directory when it ends in `/` or currently
resolves to a directory; classify every other non-glob entry as a literal
file. This preserves future matching for saved directory entries while keeping
literal file matching exact.
For pruning, return `true` for every glob-containing scope; for literal rules,
return true only when the directory is an ancestor/descendant candidate of at
least one included literal. Correctness takes priority over pruning.

- [ ] **Step 4: Apply scope after built-in and Git filters in normal detection**

Add `scope: BuildScope::default()` to `DetectOptions::default()`. Compile one
`ScopeMatcher` in `detect`. In `WalkState::walk`, retain existing noise and
Git-ignore checks, then skip a directory when
`!scope.may_match_descendant(&path)`. Before classification in the file loop,
skip files for which `!scope.allows(&path)`.

Stop appending `extra_excludes` to the Git-ignore pattern vector. Compile them
as their own existing `IgnorePattern` vector and apply `ignored` to that vector
after `scope.allows`. This preserves the current CLI pattern and negation
semantics. Keep Git patterns, configured scope, and ephemeral CLI exclusions
in separate fields so precedence and reporting remain explicit. Walk-time
pruning may use an exclusion early as an optimization, but the per-file
decision must still evaluate the documented order.

- [ ] **Step 5: Reuse the matcher in `WatchPathFilter`**

Store `scope: ScopeMatcher` in `WatchPathFilter`. In
`WatchPathFilter::new`, compile it from `options.scope`; in `allows`, keep the
existing root/noise/classification checks, then evaluate Git patterns,
`scope.allows(&absolute)`, and the separate CLI `IgnorePattern` vector in that
order.

Add a contracts test:

```rust
let options = DetectOptions {
    scope: BuildScope {
        include: vec!["src/".to_owned()],
        exclude: vec!["src/generated/**".to_owned()],
    },
    ..DetectOptions::default()
};
let filter = WatchPathFilter::new(root, &options)?;
assert!(filter.allows(&root.join("src/new.rs")));
assert!(!filter.allows(&root.join("tests/new.rs")));
assert!(!filter.allows(&root.join("src/generated/new.rs")));
```

- [ ] **Step 6: Run the complete filesystem crate**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-files
```

Expected: every unit and integration test passes.

- [ ] **Step 7: Commit matcher and discovery integration**

```bash
git add crates/compass-files/src/lib.rs crates/compass-files/src/scope.rs crates/compass-files/src/detect.rs crates/compass-files/tests/contracts.rs crates/compass-files/tests/scope_detection.rs
git commit -m "feat(files): apply persisted build scope"
```

---

### Task 3: Load saved scope for update, extract, and watch

**Files:**
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/watch.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/tests/update_cli.rs`
- Modify: `crates/compass-cli/tests/watch_cli.rs`

**Interfaces:**
- Consumes: `BuildScope`, `ProjectConfig::load`
- Extends: `BuildOptions { pub scope: BuildScope }`
- Produces: `fn load_compass_scope(frontend: Frontend, root: &Path) -> Result<BuildScope, Outcome>`
- Guarantees: all `DetectOptions` derived from one `BuildOptions` clone its scope

- [ ] **Step 1: Add a failing update integration test**

Append to `update_cli.rs`:

```rust
#[test]
fn update_reuses_saved_project_scope() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join(".compass"))?;
    std::fs::create_dir(root.path().join("src"))?;
    std::fs::create_dir(root.path().join("tools"))?;
    std::fs::write(
        root.path().join(".compass/config.toml"),
        "version = 1\n\n[build]\ninclude = [\"src/\"]\nexclude = []\n",
    )?;
    std::fs::write(root.path().join("src/in_scope.rs"), "pub fn in_scope() {}\n")?;
    std::fs::write(root.path().join("tools/out.rs"), "pub fn out() {}\n")?;

    run_update(root.path(), |_| {})?;
    let graph = compass_model::GraphDocument::load(&root.path().join("compass-out/graph.json"))?;
    let sources = graph
        .nodes
        .iter()
        .map(|node| node.string("source_file"))
        .collect::<Vec<_>>();
    assert!(sources.iter().any(|path| path.ends_with("src/in_scope.rs")));
    assert!(!sources.iter().any(|path| path.ends_with("tools/out.rs")));
    Ok(())
}
```

Add another test that writes an unsupported config version and asserts update
fails without producing `compass-out/graph.json`.

- [ ] **Step 2: Run the focused update test and observe the out-of-scope file**

Run:

```bash
cargo test -p compass-cli --test update_cli update_reuses_saved_project_scope -- --exact
```

Expected: FAIL because the graph still contains `tools/out.rs`.

- [ ] **Step 3: Carry BuildScope through core build options**

Add `pub scope: BuildScope` to `BuildOptions`, initialize it with
`BuildScope::default()`, and set `scope: options.scope.clone()` in every
`DetectOptions` created from build options:

- the main deterministic pipeline;
- incremental manifest detection;
- semantic pending-count detection;
- semantic failure reporting;
- semantic build detection;
- watcher event filtering.

Use `..DetectOptions::default()` for unrelated direct detection call sites so
history and tests retain whole-repository behavior unless they explicitly set
a scope.

- [ ] **Step 4: Load config only for the Compass frontend after root resolution**

In `compass-cli/src/lib.rs`, add:

```rust
fn load_compass_scope(frontend: Frontend, root: &Path) -> Result<BuildScope, Outcome> {
    if frontend == Frontend::Graphify {
        return Ok(BuildScope::default());
    }
    ProjectConfig::load(root)
        .map(|config| config.map_or_else(BuildScope::default, |value| value.build))
        .map_err(|error| Outcome::failure(format!("error: {error}")))
}
```

After `command_build_with_validation` resolves `root`, assign the loaded scope
to `options.scope`. At the end of `parse_watch_options`, load the same scope
into `options.build.scope`. Keep CLI exclusions in
`options.extra_excludes`; do not mutate the persisted scope.

- [ ] **Step 5: Add watcher scope regression coverage**

In `watch_cli.rs`, create a fixture with `src/in.rs` and `tools/out.rs`, persist
`include = ["src/"]`, start `compass watch --poll`, modify the tools file, and
assert no rebuild status appears during two debounce windows. Then create
`src/new.rs` and assert a rebuild occurs and the new function enters
`graph.json`.

Unit-test `collect_event` in `compass-core/src/watch.rs` with a configured
`WatchPathFilter` as the fast deterministic oracle; retain one end-to-end poll
test for wiring.

- [ ] **Step 6: Run core and focused CLI suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-core
cargo test -p compass-cli --test update_cli
cargo test -p compass-cli --test watch_cli
```

Expected: all commands succeed and invalid config stops rather than widening
the graph.

- [ ] **Step 7: Commit build/watch config consumption**

```bash
git add crates/compass-core/src/pipeline.rs crates/compass-core/src/watch.rs crates/compass-core/tests/program_pipeline.rs crates/compass-cli/src/lib.rs crates/compass-cli/tests/update_cli.rs crates/compass-cli/tests/watch_cli.rs
git commit -m "feat(cli): reuse project scope for graph builds"
```

---

### Task 4: Scriptable `compass init` with preview, overwrite safety, and first build

**Files:**
- Create: `crates/compass-cli/src/init_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/bin/compass.rs`
- Create: `crates/compass-cli/tests/init_cli.rs`

**Interfaces:**
- Produces: `pub fn run_init(arguments: &[OsString], input: &mut impl BufRead, stdout: &mut impl Write, stderr: &mut impl Write, input_is_terminal: bool) -> u8`
- Produces: private `InitOptions { root, includes, excludes, yes, force }`
- Produces: private `InitPreview { detection, config_path, output_path }`
- Produces: `pub(crate) fn run_structural_update(frontend: Frontend, options: BuildOptions) -> Outcome`
- Produces: `fn format_compass_structural_result(result: &BuildResult, no_cluster: bool) -> Outcome`

- [ ] **Step 1: Write failing non-interactive CLI lifecycle tests**

Create `init_cli.rs` with a helper that executes `CARGO_BIN_EXE_compass`.
Cover:

```rust
#[test]
fn init_yes_writes_scope_and_builds_only_matching_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("src"))?;
    std::fs::create_dir(root.path().join("tools"))?;
    std::fs::write(root.path().join("src/lib.rs"), "pub fn included() {}\n")?;
    std::fs::write(root.path().join("tools/task.rs"), "pub fn excluded() {}\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "src/", "--yes"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(root.path().join(".compass/config.toml").is_file());
    let graph = compass_model::GraphDocument::load(&root.path().join("compass-out/graph.json"))?;
    assert!(graph.nodes.iter().any(|node| node.label() == "included()"));
    assert!(!graph.nodes.iter().any(|node| node.label() == "excluded()"));
    Ok(())
}
```

Add exact cases for whole-repository `--yes`, unmatched include, empty final
corpus, existing config without/with `--force`, `--include=value`,
`--exclude=value`, malformed options, and build failure retaining config.

- [ ] **Step 2: Run the lifecycle test and verify `init` is unknown**

Run:

```bash
cargo test -p compass-cli --test init_cli init_yes_writes_scope_and_builds_only_matching_files -- --exact
```

Expected: FAIL with unknown command `init`.

- [ ] **Step 3: Parse init options without mutating the filesystem**

Implement a hand-written parser consistent with the existing CLI:

```rust
struct InitOptions {
    root: PathBuf,
    includes: Vec<String>,
    excludes: Vec<String>,
    yes: bool,
    force: bool,
}
```

Accept one positional root, repeatable split/inline include/exclude options,
`--yes`, `--force`, and `-h|--help`. Reject missing values, unknown flags, and
a second root with exit code `2`.

- [ ] **Step 4: Build and validate a preview before writing**

Normalize `ProjectConfig::new(BuildScope { ... })`, run `detect` with that
scope, feed all detected file paths to
`ScopeMatcher::unmatched_includes`, and fail with code `2` if any include is
unmatched or `Detection::total_files == 0`.

Render this deterministic preview to stdout:

```text
Project root: <absolute root>
Scope: 3 include rule(s), 1 exclude rule(s)
Matched: 42 files (35 code, 5 documents, 2 papers, 0 images, 0 video)
Config: <root>/.compass/config.toml
Output: <root>/compass-out
```

- [ ] **Step 5: Extract a typed structural-update helper**

Move the existing non-semantic update execution into:

```rust
pub(crate) fn run_structural_update(
    frontend: Frontend,
    mut options: BuildOptions,
) -> Outcome {
    options.purpose = BuildPurpose::Update;
    match build_graph_with_layers(&options, None, &[]) {
        Ok(result) => format_compass_structural_result(&result, options.no_cluster),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}
```

Have the normal Compass `update` branch and init both call this helper. Preserve
the Graphify compatibility formatter in its existing branch. Extract the
existing Compass success string and Program IR summary into
`format_compass_structural_result`; do not change its bytes for normal
`compass update`. Before init calls the helper, set `force = true`,
`program_analysis = true`, the validated scope, and the resolved root on
`BuildOptions`.

- [ ] **Step 6: Atomically write config, run the build, and distinguish partial success**

Write only after validation and (for Task 4, `--yes`) confirmation bypass.
If writing succeeds but `run_structural_update` fails, emit:

```text
Compass configuration saved to .compass/config.toml.
Initial build failed: <bounded diagnostic>
Fix the reported issue, then run `compass update`.
```

Return the build's nonzero status and leave the config. Refuse an existing
config before preview unless `--force`.

- [ ] **Step 7: Route init through the binary and keep the library dispatcher safe**

Add `mod init_commands;` and make `run(Frontend::Compass, ["init", ...])`
return the same “must be run from the compass binary” failure pattern used by
streaming commands. Leave `Frontend::Graphify` on the unknown-command path.

In `bin/compass.rs`, route init after help handling and before watch:

```rust
if !compatibility && arguments.first().and_then(|value| value.to_str()) == Some("init") {
    let stdin = io::stdin();
    let input_is_terminal = stdin.is_terminal();
    let mut locked = stdin.lock();
    return ExitCode::from(compass_cli::run_init(
        &arguments[1..],
        &mut locked,
        &mut io::stdout(),
        &mut io::stderr(),
        input_is_terminal,
    ));
}
```

Compatibility mode bypasses the branch so `graphify init` remains unknown.

- [ ] **Step 8: Run init and update regression suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-cli --test init_cli
cargo test -p compass-cli --test update_cli
```

Expected: all tests pass; Graphify-specific compatibility tests remain
unchanged.

- [ ] **Step 9: Commit non-interactive init**

```bash
git add crates/compass-cli/src/bin/compass.rs crates/compass-cli/src/init_commands.rs crates/compass-cli/src/lib.rs crates/compass-cli/tests/init_cli.rs crates/compass-cli/tests/update_cli.rs
git commit -m "feat(cli): add scriptable compass init"
```

---

### Task 5: Interactive init prompts and cancellation

**Files:**
- Modify: `crates/compass-cli/src/init_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Create: `crates/compass-cli/tests/init_interactive.rs`

**Interfaces:**
- Consumes: `run_init` from Task 4
- Produces: one prompt implementation shared by pre-populated flags and manual entries
- Guarantees: cancellation returns `0` without config/build mutation

- [ ] **Step 1: Write failing injected-input prompt tests**

Call `run_init` directly with `Cursor<Vec<u8>>`, output buffers, and
`input_is_terminal = true`:

```rust
#[test]
fn interactive_custom_scope_matches_flag_configuration() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("src"))?;
    std::fs::write(root.path().join("src/lib.rs"), "pub fn run() {}\n")?;
    let args = vec![root.path().as_os_str().to_owned()];
    let mut input = Cursor::new(b"custom\nsrc/\n\n**/generated/**\n\ny\n".to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run_init(&args, &mut input, &mut stdout, &mut stderr, true);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert!(String::from_utf8(stdout)?.contains("Matched:"));
    assert_eq!(
        ProjectConfig::load(root.path())?.expect("saved").build.include,
        ["src/"]
    );
    Ok(())
}
```

Add cancellation (`no` at final confirmation), whole-repository selection,
pre-populated flags, EOF, invalid answer retry, and non-terminal-without-yes
cases. For cancellation, assert neither `.compass` nor `compass-out` exists.

- [ ] **Step 2: Run the interactive test and verify the binary route is absent**

Run:

```bash
cargo test -p compass-cli --test init_interactive
```

Expected: FAIL because `run_init` rejects the non-`--yes` path instead of
prompting.

- [ ] **Step 3: Implement line-oriented prompt helpers**

Use injected `BufRead`/`Write` values:

```rust
fn prompt_line(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> Result<String, InitError> {
    write!(output, "{prompt}")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Err(InitError::UnexpectedEof);
    }
    Ok(line.trim().to_owned())
}
```

Prompt for `all|custom`; for custom mode, read includes until a blank line,
then exclusions until a blank line. Display detected common vendor/generated
directories only as suggestions. At confirmation accept case-insensitive
`y|yes` and `n|no`, and retry every other answer.

- [ ] **Step 4: Enforce terminal policy**

When `--yes` is absent and `input_is_terminal` is false, return `2` with:

```text
error: compass init requires an interactive terminal; pass --yes for non-interactive setup
```

Do not use stdout terminal state for this decision; input capability controls
whether answers can be read.

- [ ] **Step 5: Verify interactive mode uses the existing real-binary route**

Add a binary test that starts `compass init` with piped input and no `--yes`;
because piped stdin is not a terminal, assert exit `2` and the exact
non-terminal guidance. Keep successful prompt behavior covered through the
injected `run_init` interface, where `input_is_terminal = true` is explicit
and deterministic.

- [ ] **Step 6: Run prompt and lifecycle suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-cli --test init_interactive
cargo test -p compass-cli --test init_cli
```

Expected: all tests pass.

- [ ] **Step 7: Commit interactive init**

```bash
git add crates/compass-cli/src/init_commands.rs crates/compass-cli/src/lib.rs crates/compass-cli/tests/init_interactive.rs
git commit -m "feat(cli): add interactive init setup"
```

---

### Task 6: Public help, completions, and documentation

**Files:**
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/help_cli.rs`
- Modify: `completions/_compass`
- Modify: `completions/compass.bash`
- Modify: `completions/compass.fish`
- Modify: `completions/compass.ps1`
- Modify: `README.md`
- Modify: `docs/getting-started.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/configuration.md`

**Interfaces:**
- Consumes: finalized CLI/schema behavior from Tasks 1–5
- Produces: one public `init` help page and discoverable completion entries

- [ ] **Step 1: Add failing help coverage**

Add `"init"` to the root command coverage list and test:

```rust
let init = invoke(&["init", "--help"]);
assert_eq!(init.code, 0);
for text in [
    "compass init [PATH] [OPTIONS]",
    "--include <PATH_OR_GLOB>",
    "--exclude <GLOB>",
    "--yes",
    "--force",
    ".compass/config.toml",
] {
    assert!(init.stdout.contains(text), "missing {text}");
}
```

Run:

```bash
cargo test -p compass-cli --test help_cli
```

Expected: FAIL because root help and the page catalog do not list init.

- [ ] **Step 2: Add the help catalog entry**

Place `init` first in the “Build and maintain” group. Add a page using the
existing `page!` macro with:

```text
Arguments:
  [PATH]                       Project root [default: .]

Options:
  --include <PATH_OR_GLOB>     Include a file, directory, or glob; repeatable
  --exclude <GLOB>             Exclude a project-relative glob; repeatable
  --yes                        Accept the preview without prompting
  --force                      Replace existing .compass/config.toml

Examples:
  compass init
  compass init . --include src --exclude '**/generated/**' --yes

Notes:
  Init writes .compass/config.toml and performs a forced structural build.
```

- [ ] **Step 3: Update every shipped shell completion**

Add top-level `init` completion and init-specific
`--include --exclude --yes --force` options. Do not expose `init` on the
Graphify compatibility executable; these completion files are Compass-only.

Run syntax checks:

```bash
bash -n completions/compass.bash
zsh -n completions/_compass
```

Expected: both commands exit `0`.

- [ ] **Step 4: Update user documentation**

In `README.md` and `docs/getting-started.md`, make `compass init` the first-run
path and retain `compass update` as the repeatable refresh command. Document
the whole-repository and custom-scope examples.

In `commands.md`, add the exact command contract and exit behavior. In
`configuration.md`, add the v1 TOML schema, matching precedence, strict error
behavior, and the rule that CLI exclusions are additive and ephemeral.

- [ ] **Step 5: Run help and documentation checks**

Run:

```bash
cargo test -p compass-cli --test help_cli
cargo test -p compass-cli --test compass_product
git diff --check
```

Expected: tests pass and no whitespace errors are reported.

- [ ] **Step 6: Commit public surface documentation**

```bash
git add docs/getting-started.md docs/reference/commands.md docs/reference/configuration.md crates/compass-cli/tests/help_cli.rs completions/_compass completions/compass.bash completions/compass.fish completions/compass.ps1
git add -p README.md crates/compass-cli/src/help.rs
git commit -m "docs: document compass init workflow"
```

---

### Task 7: Full verification and graph refresh

**Files:**
- Refresh generated graph artifacts through the repository-required command
- Modify no source files unless verification exposes a feature regression

**Interfaces:**
- Consumes: all preceding tasks
- Produces: fresh evidence that the complete workspace and graph are current

- [ ] **Step 1: Verify the feature-focused crates**

Run:

```bash
cargo test -p compass-files
cargo test -p compass-core
cargo test -p compass-cli --test init_cli
cargo test -p compass-cli --test init_interactive
cargo test -p compass-cli --test update_cli
cargo test -p compass-cli --test watch_cli
cargo test -p compass-cli --test help_cli
```

Expected: every command exits `0` with zero failed tests.

- [ ] **Step 2: Verify the full Rust workspace**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: formatting, Clippy, and all workspace tests succeed.

- [ ] **Step 3: Exercise the installed command contract in a temporary fixture**

Build the binary, create a temporary repository containing `src/`,
`tools/`, and generated code, then run:

```bash
cargo build -p compass-cli --bin compass
<workspace>/target/debug/compass init . --include src --exclude '**/generated/**' --yes
<workspace>/target/debug/compass update
```

Inspect `.compass/config.toml` and `compass-out/graph.json`; confirm the graph
contains only eligible `src` sources and that the second update reports a
successful scoped refresh.

- [ ] **Step 4: Refresh the parent knowledge graph**

From `/Users/haipingfu/graphify`, run:

```bash
graphify update .
```

Expected: exit `0` and refreshed `graphify-out/` metadata reflecting the code
changes.

- [ ] **Step 5: Inspect final diff and repository state**

Run:

```bash
git diff --check
git status --short
git log --oneline -8
```

Verify every changed tracked file belongs to the approved scope or is a
pre-existing user change. Do not stage or alter unrelated work.

- [ ] **Step 6: Inspect the parent graph refresh without staging unrelated work**

Run:

```bash
git -C /Users/haipingfu/graphify status --short -- graphify-out compass
```

The parent repository may record the Compass submodule pointer and may ignore
generated graph artifacts. Do not create a parent-repository commit as part of
this feature. Report the exact verification commands, outcomes, refreshed
artifact state, and the Compass commit sequence in the handoff.
