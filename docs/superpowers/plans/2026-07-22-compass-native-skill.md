# Native Compass skill implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `compass install` create and manage a new skill named `compass` whose content may borrow from Graphify but uses only native Compass commands and artifacts.

**Architecture:** Add one canonical Agent Skills package under `crates/compass-cli/assets/compass-skill/`. Keep platform routing in `install_commands.rs`, but point every platform at the canonical package and rename every generated integration from Graphify to Compass. Replace Python parity tests with native black-box contracts that inspect isolated installation trees.

**Tech stack:** Rust 2024, Cargo integration tests, embedded Markdown assets, `serde_json`, Agent Skills `SKILL.md`

## Global constraints

- The installed skill frontmatter name is exactly `compass`
- Installed skill directories end in `/skills/compass`
- Installed content uses `compass`, `/compass`, `compass-out/`, and `COMPASS_*`
- Installed content contains no `graphify`, `graphifyy`, `GRAPHIFY_*`, or `graphify-out`
- Graphify installations remain unchanged during Compass install and uninstall
- `compass uninstall --purge` removes `compass-out/` but preserves `graphify-out/`
- The supported platform set remains unchanged
- Installer writes remain atomic through `compass_files::write_text_atomic`
- No Python runtime or Graphify package becomes a Compass skill dependency

## File map

- Create `crates/compass-cli/assets/compass-skill/SKILL.md`: canonical native skill workflow
- Create `crates/compass-cli/assets/compass-skill/references/*.md`: native command guidance loaded on demand
- Create `crates/compass-cli/assets/compass-integrations/*.md`: always-on platform registrations and Kilo command
- Modify `crates/compass-cli/src/install_commands.rs`: Compass ownership, destinations, hooks, plugins, install and uninstall behavior
- Replace `crates/compass-cli/tests/install_cli.rs`: native installer lifecycle and content contracts
- Modify `crates/compass-cli/tests/compass_product.rs`: product-level help and stale-brand assertions
- Modify `README.md`: document native skill registration

### Task 1: Establish failing native installer contracts

**Files:**

- Replace: `crates/compass-cli/tests/install_cli.rs`
- Test: `crates/compass-cli/tests/install_cli.rs`

**Interfaces:**

- Consumes: `CARGO_BIN_EXE_compass`
- Produces: `InstallFixture::run(&[&str]) -> Result<Output, Box<dyn Error>>`, `directory_tree(&Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, Box<dyn Error>>`, and the native installation acceptance contract

- [ ] **Step 1: Replace the Python parity fixture with an isolated Compass fixture**

Use the native executable and isolated `HOME`, `USERPROFILE`, and project directories:

```rust
struct InstallFixture {
    _directory: TempDir,
    project: PathBuf,
    home: PathBuf,
}

impl InstallFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let home = directory.path().join("home");
        fs::create_dir_all(&project)?;
        fs::create_dir_all(&home)?;
        Ok(Self {
            _directory: directory,
            project,
            home,
        })
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
            .args(arguments)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .output()?)
    }
}

fn directory_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, Box<dyn Error>> {
    fn visit(
        root: &Path,
        directory: &Path,
        output: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, output)?;
            } else if path.is_file() {
                output.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}
```

- [ ] **Step 2: Add a project-scoped Codex acceptance test**

```rust
#[test]
fn project_codex_install_creates_native_compass_skill() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let output = fixture.run(&["install", "--platform", "codex", "--project"])?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let skill = fixture.project.join(".codex/skills/compass/SKILL.md");
    let body = fs::read_to_string(&skill)?;
    assert!(body.starts_with("---\nname: compass\n"));
    assert!(body.contains("compass query"));
    assert!(body.contains("compass-out/"));
    assert_native(&body);
    assert!(skill.with_file_name(".compass_version").is_file());
    Ok(())
}

fn assert_native(value: &str) {
    for forbidden in ["graphify", "graphifyy", "GRAPHIFY_", "graphify-out"] {
        assert!(!value.contains(forbidden), "stale token {forbidden}: {value}");
    }
}
```

- [ ] **Step 3: Add a Graphify-preservation test**

```rust
#[test]
fn compass_lifecycle_preserves_adjacent_graphify_install() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    let graphify = fixture.project.join(".codex/skills/graphify/SKILL.md");
    fs::create_dir_all(graphify.parent().ok_or("graphify parent")?)?;
    fs::write(&graphify, "---\nname: graphify\n---\n")?;
    fs::create_dir_all(fixture.project.join("graphify-out"))?;

    assert!(fixture.run(&["install", "--platform", "codex", "--project"])?.status.success());
    assert!(fixture.run(&["uninstall", "--platform", "codex", "--project"])?.status.success());

    assert_eq!(fs::read_to_string(graphify)?, "---\nname: graphify\n---\n");
    assert!(fixture.project.join("graphify-out").is_dir());
    Ok(())
}
```

- [ ] **Step 4: Run the focused test and verify the RED state**

Run:

```bash
cargo test -p compass-cli --test install_cli project_codex_install_creates_native_compass_skill -- --exact
```

Expected: FAIL because the current installer writes `.codex/skills/graphify/SKILL.md`.

- [ ] **Step 5: Commit the failing contracts**

```bash
git add crates/compass-cli/tests/install_cli.rs
git commit -m "test: define native Compass skill contracts"
```

### Task 2: Add the canonical native skill package

**Files:**

- Create: `crates/compass-cli/assets/compass-skill/SKILL.md`
- Create: `crates/compass-cli/assets/compass-skill/references/add-watch.md`
- Create: `crates/compass-cli/assets/compass-skill/references/extraction-spec.md`
- Create: `crates/compass-cli/assets/compass-skill/references/exports.md`
- Create: `crates/compass-cli/assets/compass-skill/references/github-and-merge.md`
- Create: `crates/compass-cli/assets/compass-skill/references/hooks.md`
- Create: `crates/compass-cli/assets/compass-skill/references/query.md`
- Create: `crates/compass-cli/assets/compass-skill/references/update.md`
- Test: `crates/compass-cli/src/install_commands.rs`

**Interfaces:**

- Consumes: native `compass update`, `query`, `path`, `explain`, `reflect`, `add`, `watch`, `export`, and `hook` command surfaces
- Produces: embedded asset `compass-skill/SKILL.md` and reference prefix `compass-skill/references/`

- [ ] **Step 1: Add a failing embedded-package test**

Add this unit test to `install_commands.rs`:

```rust
#[test]
fn canonical_compass_skill_package_is_native() {
    let body = asset_text("compass-skill/SKILL.md").expect("canonical Compass skill");
    assert!(body.starts_with("---\nname: compass\n"));
    assert!(body.contains("references/query.md"));
    assert!(body.contains("compass query"));
    assert!(body.contains("compass update"));
    for forbidden in ["graphify", "graphifyy", "GRAPHIFY_", "graphify-out", "python -m"] {
        assert!(!body.contains(forbidden), "stale token {forbidden}");
    }
    assert!(EMBEDDED_ASSETS.iter().any(|asset| {
        asset.path.starts_with("compass-skill/references/")
    }));
}
```

- [ ] **Step 2: Run the package test and verify the RED state**

Run:

```bash
cargo test -p compass-cli install_commands::tests::canonical_compass_skill_package_is_native -- --exact
```

Expected: FAIL because `compass-skill/SKILL.md` does not exist.

- [ ] **Step 3: Create the canonical `SKILL.md`**

Use this workflow shape and keep the finished file below 500 words:

```markdown
---
name: compass
description: Use when answering questions about a codebase, its architecture, dependencies, impact, or project artifacts, especially when compass-out exists or the user invokes /compass.
---

# Compass

Use Compass as the first navigation layer for codebase and architecture work.

## Existing graph

When `compass-out/graph.json` exists:

1. Run `compass reflect --if-stale`.
2. Read `compass-out/reflections/LESSONS.md` when it exists.
3. Run `compass query "<question>"` for scoped context.
4. Use `compass path "<source>" "<target>"` for dependency paths.
5. Use `compass explain "<concept>"` for one concept and its neighbors.

Read `compass-out/GRAPH_REPORT.md` for broad architecture only. Navigate `compass-out/wiki/index.md` when the wiki exists.

## Build or refresh

Run `compass update INPUT_PATH`. Use `.` when the user gives no path. After code changes, run `compass update .`.

Load only the reference needed for the request:

- Query and navigation: `references/query.md`
- Incremental refresh: `references/update.md`
- Hooks and assistant setup: `references/hooks.md`
- Watch mode and added sources: `references/add-watch.md`
- Export formats: `references/exports.md`
- Repository cloning and merged graphs: `references/github-and-merge.md`
- Graph schema and provenance: `references/extraction-spec.md`

If `compass` is unavailable, report that the Compass CLI must be installed. Do not install Python or another package.
```

- [ ] **Step 4: Create concise native references**

Each reference must state its trigger in the first paragraph, use only commands present in `compass <command> --help`, and defer option enumeration to CLI help. Use these exact contracts:

```markdown
# Query and navigate

Load this reference for codebase questions when `compass-out/graph.json` exists.

- `compass query "<question>"`: return a relevance-ranked scoped subgraph
- `compass path "<source>" "<target>"`: show the shortest dependency path
- `compass explain "<concept>"`: show one node and its connected context
- `compass affected <path-or-symbol>`: estimate downstream impact
- `compass tree`: render repository structure with graph context

Run `compass <command> --help` before using options not shown here.
```

`references/update.md`:

```markdown
# Refresh a graph

Load this reference when source files changed or the user requests a rebuild.

Run `compass update INPUT_PATH`. Use `.` when no path was supplied. Run `compass reflect --if-stale` after the update before answering a codebase question.

Run `compass update --help` before adding build options.
```

`references/hooks.md`:

```markdown
# Manage hooks and assistant setup

Load this reference when the user asks for automatic refresh or assistant registration.

- `compass hook install`: register repository refresh hooks
- `compass hook status`: inspect managed hooks
- `compass hook uninstall`: remove managed hooks
- `compass install --project --platform PLATFORM`: register the Compass skill in this repository
- `compass uninstall --project --platform PLATFORM`: remove the project skill

Run the relevant command with `--help` before adding options.
```

`references/add-watch.md`:

```markdown
# Add sources and watch changes

Load this reference when the user adds an external source or requests continuous graph refresh.

- `compass add URL`: add a supported external source
- `compass watch INPUT_PATH`: watch a directory and refresh its graph

Run `compass add --help` or `compass watch --help` before adding options.
```

`references/exports.md`:

```markdown
# Export graph artifacts

Load this reference when the user needs a wiki, Obsidian vault, SVG, GraphML, or graph database output.

- `compass export wiki`
- `compass export obsidian`
- `compass export svg`
- `compass export graphml`
- `compass export neo4j`

Run `compass export --help` for output paths and format-specific options.
```

`references/github-and-merge.md`:

```markdown
# Clone and merge repositories

Load this reference for GitHub repositories, multiple repositories, or merged graphs.

- `compass clone URL`: clone a repository for local graphing
- `compass update PATH`: build or refresh one repository graph
- `compass merge-graphs`: merge completed graph artifacts

Run each command with `--help` before supplying branch, input, or output options.
```

`references/extraction-spec.md`:

```markdown
# Graph schema and provenance

Load this reference when interpreting graph structure or extraction confidence.

Compass represents files, symbols, document sections, and project entities as nodes. Directed relationships connect nodes. Communities group densely connected nodes.

Treat `EXTRACTED` edges as direct structural evidence, `INFERRED` edges as resolved relationships with recorded confidence, and `AMBIGUOUS` edges as unresolved alternatives. Preserve direction and provenance when explaining a path.
```

- [ ] **Step 5: Validate the skill folder**

Run:

```bash
python /Users/haipingfu/.codex/skills/.system/skill-creator/scripts/quick_validate.py crates/compass-cli/assets/compass-skill
rg -n -i 'graphify|graphifyy|GRAPHIFY_|graphify-out|python -m' crates/compass-cli/assets/compass-skill
```

Expected: validator exits `0`; `rg` prints no matches.

- [ ] **Step 6: Run the package test and verify the GREEN state**

Run:

```bash
cargo test -p compass-cli install_commands::tests::canonical_compass_skill_package_is_native -- --exact
```

Expected: PASS.

- [ ] **Step 7: Commit the canonical package**

```bash
git add crates/compass-cli/assets/compass-skill crates/compass-cli/src/install_commands.rs
git commit -m "feat: add native Compass skill package"
```

### Task 3: Make installer ownership fully Compass-native

**Files:**

- Modify: `crates/compass-cli/src/install_commands.rs`
- Create: `crates/compass-cli/assets/compass-integrations/agents-md.md`
- Create: `crates/compass-cli/assets/compass-integrations/antigravity-rules.md`
- Create: `crates/compass-cli/assets/compass-integrations/antigravity-workflow.md`
- Create: `crates/compass-cli/assets/compass-integrations/claude-md.md`
- Create: `crates/compass-cli/assets/compass-integrations/gemini-md.md`
- Create: `crates/compass-cli/assets/compass-integrations/kiro-steering.md`
- Create: `crates/compass-cli/assets/compass-integrations/kilo-command.md`
- Create: `crates/compass-cli/assets/compass-integrations/vscode-instructions.md`
- Test: `crates/compass-cli/src/install_commands.rs`
- Test: `crates/compass-cli/tests/install_cli.rs`

**Interfaces:**

- Consumes: canonical assets from Task 2
- Produces: `compass_executable() -> String`, Compass-owned paths, sections, JSON hooks, plugins, rules, workflows, and uninstall cleanup

- [ ] **Step 1: Add failing ownership and coexistence unit tests**

Add assertions that generated files use Compass names while Graphify fixtures survive:

```rust
#[test]
fn plugin_round_trip_owns_only_compass_entries() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    write(
        &root.join(".opencode/opencode.json"),
        r#"{"plugin":[".opencode/plugins/graphify.js"]}"#,
    )?;
    let mut lines = Vec::new();

    install_opencode(root, &mut lines)?;
    assert!(root.join(".opencode/plugins/compass.js").is_file());
    remove_opencode(root, &mut lines);

    let config = load_json_object(&root.join(".opencode/opencode.json"));
    assert_eq!(
        config["plugin"].as_array().and_then(|values| values.first()),
        Some(&Value::String(".opencode/plugins/graphify.js".to_owned()))
    );
    assert!(!root.join(".opencode/plugins/compass.js").exists());
    Ok(())
}

#[test]
fn unowned_compass_skill_is_not_overwritten() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let skill = directory.path().join(".codex/skills/compass/SKILL.md");
    write(&skill, "user-owned")?;
    let config = platform("codex").ok_or("codex platform")?;

    let error = install_skill(config, true, directory.path()).expect_err("must reject");
    assert!(error.contains("not managed by Compass"));
    assert_eq!(fs::read_to_string(skill)?, "user-owned");
    Ok(())
}

#[test]
fn missing_embedded_asset_reports_compass_reinstall() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let config = Platform::new(
        "codex",
        "missing-compass-skill.md",
        ".codex/skills/compass/SKILL.md",
        None,
    );
    let error = install_skill(config, true, directory.path()).expect_err("must reject");
    assert!(error.contains("reinstall Compass"));
    assert!(!error.to_ascii_lowercase().contains("graphify"));
    Ok(())
}
```

- [ ] **Step 2: Run the ownership tests and verify the RED state**

Run:

```bash
cargo test -p compass-cli install_commands::tests::plugin_round_trip_owns_only_compass_entries -- --exact
cargo test -p compass-cli install_commands::tests::unowned_compass_skill_is_not_overwritten -- --exact
cargo test -p compass-cli install_commands::tests::missing_embedded_asset_reports_compass_reinstall -- --exact
```

Expected: FAIL because plugins and destinations remain Graphify-named, unmarked content is overwritten, and the missing-asset error names Graphify.

- [ ] **Step 3: Introduce native ownership constants**

Add:

```rust
const SKILL_VERSION: &str = env!("CARGO_PKG_VERSION");
const SKILL_ASSET: &str = "compass-skill/SKILL.md";
const REFERENCE_BUNDLE: &str = "compass-skill";
const SECTION_HEADING: &str = "## compass";
const SKILL_DIRECTORY: &str = "compass";
```

Replace `COMPAT_VERSION` with `SKILL_VERSION`. Point every `Platform` at `SKILL_ASSET`, a destination ending in `/compass/SKILL.md`, and `Some(REFERENCE_BUNDLE)`.

- [ ] **Step 4: Protect unowned skill destinations before any write**

Add and call this helper before creating the skill directory or references:

```rust
fn require_owned_or_absent(destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return Ok(());
    }
    let marker = destination
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?
        .join(".compass_version");
    if marker.is_file() {
        Ok(())
    } else {
        Err(format!(
            "error: {} exists but is not managed by Compass",
            destination.display()
        ))
    }
}
```

- [ ] **Step 5: Convert all generated integration ownership**

Apply this exact ownership table throughout install and uninstall branches:

| Legacy value | Native value |
| --- | --- |
| `## graphify` | `## compass` |
| `# graphify` | `# compass` |
| `/graphify` | `/compass` |
| `graphify.md` | `compass.md` |
| `graphify.mdc` | `compass.mdc` |
| `graphify.js` | `compass.js` |
| `GraphifyPlugin` | `CompassPlugin` |
| `graphify-manager` | `compass-manager` |
| `graphify_executable()` | `compass_executable()` |

Hook cleanup predicates must match `compass`, not `graphify`, so adjacent Graphify hook entries remain.

- [ ] **Step 6: Resolve the native executable**

```rust
fn compass_executable() -> String {
    executable_on_path("compass")
        .or_else(|| env::current_exe().ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "compass".to_owned())
}
```

- [ ] **Step 7: Add Compass-native integration assets**

Use this exact body for `agents-md.md`, `claude-md.md`, `gemini-md.md`, `kiro-steering.md`, and `antigravity-rules.md`:

```markdown
## compass

This project has a Compass knowledge graph at `compass-out/`.

Rules:

- Run `compass query "<question>"` before broad source searches
- Use `compass path "<source>" "<target>"` for dependency paths
- Use `compass explain "<concept>"` for one concept and its neighbors
- Read `compass-out/GRAPH_REPORT.md` for broad architecture
- Navigate `compass-out/wiki/index.md` when the wiki exists
- Run `compass update .` after code changes
```

Use this exact workflow for `antigravity-workflow.md` and `kilo-command.md`:

```markdown
---
name: compass
description: Use when the user invokes /compass or requests knowledge-graph navigation for a project.
---

# Compass

Invoke the installed `compass` skill immediately. Use `.` when the user gives no path.
```

Use this exact body for `vscode-instructions.md`:

```markdown
## compass

Use the Compass knowledge graph at `compass-out/` before broad workspace searches. Run `compass query "<question>"` for scoped context and `compass update .` after code changes.
```

Point every `asset_text("always_on/...")` and `asset_text("command-kilo.md")` call at the matching `compass-integrations/...` asset.

- [ ] **Step 8: Run ownership and project install tests**

Run:

```bash
cargo test -p compass-cli install_commands::tests::plugin_round_trip_owns_only_compass_entries -- --exact
cargo test -p compass-cli install_commands::tests::unowned_compass_skill_is_not_overwritten -- --exact
cargo test -p compass-cli install_commands::tests::missing_embedded_asset_reports_compass_reinstall -- --exact
cargo test -p compass-cli --test install_cli project_codex_install_creates_native_compass_skill -- --exact
```

Expected: all PASS.

- [ ] **Step 9: Commit native installer ownership**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/assets/compass-integrations
git commit -m "feat: install Compass-owned assistant integrations"
```

### Task 4: Cover every platform and uninstall mode

**Files:**

- Modify: `crates/compass-cli/tests/install_cli.rs`
- Modify: `crates/compass-cli/tests/compass_product.rs`
- Modify: `README.md`

**Interfaces:**

- Consumes: native install and uninstall behavior from Task 3
- Produces: full platform lifecycle coverage and user-facing installation documentation

- [ ] **Step 1: Add a platform matrix test**

For every entry in `PROJECT_PLATFORMS`, install project scope, inspect every text file, uninstall, and assert no Compass-owned skill remains:

```rust
#[test]
fn every_project_platform_installs_native_content() -> Result<(), Box<dyn Error>> {
    for platform in PROJECT_PLATFORMS {
        let fixture = InstallFixture::new()?;
        let output = fixture.run(&["install", "--platform", platform, "--project"])?;
        assert!(output.status.success(), "{platform}: {}", String::from_utf8_lossy(&output.stderr));

        for (path, bytes) in directory_tree(&fixture.project)? {
            if let Ok(text) = String::from_utf8(bytes) {
                assert_native(&text);
                if path.ends_with("SKILL.md") {
                    assert!(text.starts_with("---\nname: compass\n"), "{platform}: {}", path.display());
                }
            }
        }

        assert!(fixture.run(&["uninstall", "--platform", platform, "--project"])?.status.success());
        assert!(!directory_tree(&fixture.project)?
            .keys()
            .any(|path| path.to_string_lossy().contains("/skills/compass/")));
    }
    Ok(())
}
```

- [ ] **Step 2: Add a global platform matrix**

Keep the current global platform set and inspect both the isolated home and project because Cursor and Gemini own project files:

```rust
const GLOBAL_PLATFORMS: &[&str] = &[
    "claude", "codex", "opencode", "kilo", "aider", "copilot", "claw",
    "droid", "trae", "trae-cn", "hermes", "kiro", "pi", "codebuddy",
    "antigravity", "antigravity-windows", "windows", "kimi", "amp",
    "agents", "devin", "gemini", "cursor",
];

#[test]
fn every_global_platform_installs_native_content() -> Result<(), Box<dyn Error>> {
    for platform in GLOBAL_PLATFORMS {
        let fixture = InstallFixture::new()?;
        let output = fixture.run(&["install", "--platform", platform])?;
        assert!(output.status.success(), "{platform}: {}", String::from_utf8_lossy(&output.stderr));
        for root in [&fixture.home, &fixture.project] {
            for (_, bytes) in directory_tree(root)? {
                if let Ok(text) = String::from_utf8(bytes) {
                    assert_native(&text);
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Add direct-versus-generic equivalence**

Install `codex` through both command forms and compare their project trees:

```rust
#[test]
fn direct_and_generic_codex_installs_match() -> Result<(), Box<dyn Error>> {
    let generic = InstallFixture::new()?;
    let direct = InstallFixture::new()?;
    assert!(generic.run(&["install", "--platform", "codex", "--project"])?.status.success());
    assert!(direct.run(&["codex", "install", "--project"])?.status.success());
    assert_eq!(directory_tree(&generic.project)?, directory_tree(&direct.project)?);
    Ok(())
}
```

- [ ] **Step 4: Add idempotence and parser-mutation contracts**

```rust
#[test]
fn reinstall_is_idempotent_and_parser_errors_do_not_mutate() -> Result<(), Box<dyn Error>> {
    let fixture = InstallFixture::new()?;
    assert!(fixture.run(&["install", "--platform", "codex", "--project"])?.status.success());
    let first = directory_tree(&fixture.project)?;
    assert!(fixture.run(&["install", "--platform", "codex", "--project"])?.status.success());
    assert_eq!(directory_tree(&fixture.project)?, first);

    let rejected = fixture.run(&["install", "--unknown"])?;
    assert!(!rejected.status.success());
    assert_eq!(directory_tree(&fixture.project)?, first);
    Ok(())
}
```

- [ ] **Step 5: Add purge isolation**

Create both output directories, run `compass uninstall --project --purge`, and assert:

```rust
assert!(!fixture.project.join("compass-out").exists());
assert!(fixture.project.join("graphify-out").is_dir());
```

- [ ] **Step 6: Add product-level stale-brand checks**

Extend `compass_product.rs`:

```rust
#[test]
fn install_help_is_compass_native() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["install", "--help"])
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("compass install"));
    assert!(!stdout.to_ascii_lowercase().contains("graphify"));
    Ok(())
}
```

- [ ] **Step 7: Update the README**

Add this native registration flow after binary installation:

````markdown
Register the Compass skill with your coding assistant:

```bash
compass install
```

Use a project-scoped skill when the configuration should travel with the repository:

```bash
compass install --project --platform codex
```
````

- [ ] **Step 8: Run the platform suite and verify GREEN**

Run:

```bash
cargo test -p compass-cli --test install_cli
cargo test -p compass-cli --test compass_product
```

Expected: all tests PASS and no test invokes Python Graphify.

- [ ] **Step 9: Commit platform coverage and docs**

```bash
git add crates/compass-cli/tests/install_cli.rs crates/compass-cli/tests/compass_product.rs README.md
git commit -m "test: cover native Compass skill lifecycle"
```

### Task 5: Validate the skill with fresh agents and verify the repository

**Files:**

- Modify only if validation exposes a gap: `crates/compass-cli/assets/compass-skill/SKILL.md`
- Modify only if validation exposes a gap: `crates/compass-cli/assets/compass-skill/references/*.md`

**Interfaces:**

- Consumes: completed native skill package and installer
- Produces: forward-test evidence, formatted Rust, passing workspace tests, and a refreshed parent Graphify graph

- [ ] **Step 1: Run a baseline skill scenario without the new skill**

Give a fresh agent only this request and a small fixture repository:

```text
Use the repository’s Compass knowledge graph to explain how authentication reaches storage. Refresh the graph if needed. Do not use Graphify.
```

Record whether it discovers `compass query`, checks `compass-out/`, and refreshes with `compass update .`. This is the RED baseline.

- [ ] **Step 2: Run the same scenario with the installed Compass skill**

Install with `compass install --project --platform codex`, give a fresh agent the same request, and verify that it follows the native workflow without Python or Graphify.

- [ ] **Step 3: Close only observed skill gaps**

If the forward test omits a required action or uses a stale command, add the minimum positive instruction to `SKILL.md` or the relevant reference, rerun `quick_validate.py`, and repeat the same scenario.

- [ ] **Step 4: Run formatting and static checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p compass-cli --all-targets -- -D warnings
git diff --check
```

Expected: every command exits `0`.

- [ ] **Step 5: Run focused and full tests**

Run:

```bash
cargo test -p compass-cli --test install_cli
cargo test -p compass-cli install_commands
cargo test --workspace
```

Expected: every command exits `0` with zero failing tests.

- [ ] **Step 6: Audit the installed artifact**

Install into a temporary project and scan only Compass-owned output:

```bash
tmp_dir=$(mktemp -d)
(
  cd "$tmp_dir"
  /Users/haipingfu/graphify/compass/target/debug/compass install --project --platform codex
  rg -n -i 'graphify|graphifyy|GRAPHIFY_|graphify-out|python -m' \
    .codex/skills/compass AGENTS.md
)
```

Expected: install exits `0`; `rg` prints no matches.

- [ ] **Step 7: Refresh the parent repository graph**

Run from `/Users/haipingfu/graphify`:

```bash
graphify update .
```

Expected: exit `0` and updated graph metadata under `graphify-out/`.

- [ ] **Step 8: Commit any validation fixes**

If Step 3 changed the skill:

```bash
git add crates/compass-cli/assets/compass-skill
git commit -m "docs: tighten native Compass skill guidance"
```
