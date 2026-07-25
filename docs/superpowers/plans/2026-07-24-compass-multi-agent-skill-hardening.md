# Compass Multi-Agent Skill Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `compass install` safely auto-detect and configure multiple coding agents through shared and native Agent Skills locations while preserving explicit selection, user configuration, and Graphify-style graph-first guidance.

**Architecture:** Keep `install_commands.rs` as the CLI facade and split its current responsibilities into focused registry, request/detection, planning/reporting, storage, and adapter modules. Every invocation builds an immutable plan, preflights and executes independent target transactions, verifies owned artifacts, and emits a stable text or JSON report.

**Tech Stack:** Rust 2024, `serde`, `serde_json`, `toml`, `sha2`, `compass-files` atomic writes, Cargo integration tests, embedded Agent Skills Markdown

## Global Constraints

- Plain `compass install` uses project scope at the Git root and user scope outside Git.
- Plain installation always includes the portable `.agents/skills/compass` target.
- `--project` and `--user` conflict; `--all` and `--platform` conflict.
- `--platform` is repeatable and explicit platform selection bypasses detection.
- Existing `--strict` remains the Claude Code hook option; `--require-all` controls partial-success exit behavior.
- Codex, Gemini CLI, OpenCode, and GitHub Copilot share `.agents/skills/compass`.
- Claude Code, Kiro, and Cline keep documented native skill roots.
- Unowned or user-modified files are never overwritten or deleted.
- Malformed JSON or TOML is preserved byte-for-byte.
- Install and uninstall ownership uses exact identities and digests, never substring matching.
- Graphify skills, instructions, and `graphify-out/` remain untouched.
- Installation does not build a graph; it recommends `compass update .` when needed.
- Text and versioned JSON reports contain the same statuses, reasons, paths, and next actions.
- After code changes, run `graphify update .` from `/Users/haipingfu/graphify`.

---

## File map

- Keep `crates/compass-cli/src/install_commands.rs` as the public command facade and legacy direct-command compatibility layer.
- Create `crates/compass-cli/src/install_commands/model.rs` for scope, request, target, action, status, and report data types.
- Create `crates/compass-cli/src/install_commands/registry.rs` for platform records, aliases, destinations, documentation metadata, and registry validation.
- Create `crates/compass-cli/src/install_commands/request.rs` for argument parsing and project/user scope resolution.
- Create `crates/compass-cli/src/install_commands/detect.rs` for `PATH`, config, instruction, and environment evidence.
- Create `crates/compass-cli/src/install_commands/plan.rs` for target expansion, shared-destination deduplication, and exit semantics.
- Create `crates/compass-cli/src/install_commands/report.rs` for stable human and JSON rendering.
- Create `crates/compass-cli/src/install_commands/storage.rs` for ownership manifests, digests, scoped locking, staging, strict config parsing, and exact managed sections.
- Create `crates/compass-cli/src/install_commands/adapters.rs` for native instruction, hook, plugin, command, and rule adapters.
- Expand `crates/compass-cli/tests/install_cli.rs` for supported destination and lifecycle acceptance tests.
- Create `crates/compass-cli/tests/install_detection.rs` for auto-detection, scope, deduplication, dry-run, and JSON tests.
- Create `crates/compass-cli/tests/install_failures.rs` for malformed configuration, conflicts, concurrency, rollback, and migration tests.
- Modify `crates/compass-cli/assets/compass-skill/SKILL.md` and integration assets to make graph existence conditional and add portable metadata.
- Modify `tools/skillgen/mod.rs` to validate Agent Skills metadata, reference depth, and the activation budget.
- Modify `crates/compass-cli/src/help.rs`, `README.md`, and `docs/guides/assistant-setup.md` to document the new lifecycle.
- Create `crates/compass-cli/tests/install_support/mod.rs` for one shared isolated-home/project/PATH integration fixture.

---

### Task 1: Define the installer model and agent registry

**Files:**

- Create: `crates/compass-cli/src/install_commands/model.rs`
- Create: `crates/compass-cli/src/install_commands/registry.rs`
- Modify: `crates/compass-cli/src/install_commands.rs`
- Test: `crates/compass-cli/src/install_commands/registry.rs`

**Interfaces:**

- Consumes: `std::path::{Path, PathBuf}`, `serde::{Serialize, Deserialize}`
- Produces: `InstallScope`, `ScopeKind`, `SupportTier`, `OutputFormat`, `InstallRequest`, `AgentDescriptor`, `AgentRegistry::new()`, `AgentRegistry::resolve()`, and `AgentDescriptor::skill_destination()`

- [ ] **Step 1: Write failing registry tests**

Add `mod model; mod registry;` to `install_commands.rs`, then add these tests to
`registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AgentRegistry, SupportTier};
    use crate::install_commands::model::InstallScope;

    #[test]
    fn registry_has_unique_ids_aliases_and_verified_sources() {
        let registry = AgentRegistry::new().expect("valid registry");
        assert_eq!(registry.resolve("skills").map(|agent| agent.id), Some("agents"));
        assert_eq!(registry.resolve("claude-code").map(|agent| agent.id), Some("claude"));
        for agent in registry.iter() {
            assert!(!agent.documentation_url.is_empty(), "{}", agent.id);
            assert_eq!(agent.verified_on, "2026-07-24", "{}", agent.id);
        }
    }

    #[test]
    fn documented_shared_consumers_resolve_to_one_portable_path() {
        let registry = AgentRegistry::new().expect("valid registry");
        let scope = InstallScope::Project(Path::new("/repo").to_path_buf());
        for id in ["codex", "gemini", "opencode", "copilot", "agents"] {
            let agent = registry.resolve(id).expect("registered agent");
            assert_eq!(agent.tier, SupportTier::SharedSkill);
            assert_eq!(
                agent.skill_destination(&scope).expect("skill destination"),
                Path::new("/repo/.agents/skills/compass/SKILL.md")
            );
        }
    }

    #[test]
    fn native_consumers_keep_their_documented_roots() {
        let registry = AgentRegistry::new().expect("valid registry");
        let scope = InstallScope::User(Path::new("/home/test").to_path_buf());
        let expected = [
            ("claude", "/home/test/.claude/skills/compass/SKILL.md"),
            ("kiro", "/home/test/.kiro/skills/compass/SKILL.md"),
            ("cline", "/home/test/.cline/skills/compass/SKILL.md"),
        ];
        for (id, path) in expected {
            let agent = registry.resolve(id).expect("registered agent");
            assert_eq!(agent.tier, SupportTier::NativeSkill);
            assert_eq!(
                agent.skill_destination(&scope).expect("skill destination"),
                Path::new(path)
            );
        }
    }
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test -p compass-cli install_commands::registry::tests --lib
```

Expected: compilation fails because `model` and registry types do not exist.

- [ ] **Step 3: Add the installer model**

Create `model.rs` with these exact public-to-module types:

```rust
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InstallScope {
    Project(PathBuf),
    User(PathBuf),
}

impl InstallScope {
    pub(super) fn root(&self) -> &Path {
        match self {
            Self::Project(root) | Self::User(root) => root,
        }
    }

    pub(super) fn kind(&self) -> ScopeKind {
        match self {
            Self::Project(_) => ScopeKind::Project,
            Self::User(_) => ScopeKind::User,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScopeKind {
    Project,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SupportTier {
    SharedSkill,
    NativeSkill,
    AdapterOnly,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct InstallRequest {
    pub platforms: Vec<String>,
    pub all: bool,
    pub project: bool,
    pub user: bool,
    pub strict: bool,
    pub dry_run: bool,
    pub require_all: bool,
    pub format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InstallStatus {
    Installed,
    Updated,
    Current,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct TargetResult {
    pub id: String,
    pub consumers: BTreeSet<String>,
    pub status: InstallStatus,
    pub paths: Vec<PathBuf>,
    pub reason: Option<String>,
    pub rollback: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct InstallReport {
    pub schema: u32,
    pub scope: ScopeKind,
    pub root: PathBuf,
    pub detected: BTreeMap<String, Vec<String>>,
    pub results: Vec<TargetResult>,
    pub graph_exists: bool,
    pub next_actions: Vec<String>,
}
```

- [ ] **Step 4: Add the declarative registry**

Create `registry.rs` with `AgentDescriptor` fields for `id`, `aliases`,
`tier`, `commands`, `config_paths`, project/user destinations,
`documentation_url`, and `verified_on`. Populate every current platform from
`PLATFORM_NAMES`, add `cline`, and use these verified core records:

```rust
AgentDescriptor::shared(
    "codex",
    &[],
    &["codex"],
    &[".codex", "AGENTS.md"],
    "https://developers.openai.com/codex/concepts/customization#skills",
),
AgentDescriptor::native(
    "claude",
    &["claude-code", "windows"],
    &["claude"],
    &[".claude", "CLAUDE.md"],
    ".claude/skills/compass/SKILL.md",
    "https://code.claude.com/docs/en/skills",
),
AgentDescriptor::shared(
    "gemini",
    &[],
    &["gemini"],
    &[".gemini", "GEMINI.md"],
    "https://geminicli.com/docs/cli/skills/",
),
AgentDescriptor::shared(
    "opencode",
    &[],
    &["opencode"],
    &[".opencode", "opencode.json"],
    "https://opencode.ai/docs/skills/",
),
AgentDescriptor::shared(
    "copilot",
    &["vscode"],
    &["copilot", "code"],
    &[".github/copilot-instructions.md"],
    "https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/add-skills",
),
AgentDescriptor::native(
    "kiro",
    &[],
    &["kiro", "kiro-cli"],
    &[".kiro"],
    ".kiro/skills/compass/SKILL.md",
    "https://kiro.dev/docs/skills/",
),
AgentDescriptor::native(
    "cline",
    &[],
    &["cline"],
    &[".cline", ".clinerules"],
    ".cline/skills/compass/SKILL.md",
    "https://docs.cline.bot/customization/skills",
),
AgentDescriptor::shared(
    "agents",
    &["skills"],
    &[],
    &[".agents"],
    "https://agentskills.io/specification",
),
```

Use `.agents/skills/compass/SKILL.md` for every shared record. Preserve each
existing adapter destination for the remaining records and use
`https://github.com/crabbuild/compass/blob/main/docs/guides/assistant-setup.md`
as its compatibility-contract URL when no agent vendor documents an Agent
Skills root.
`AgentRegistry::new()` must reject duplicate ids/aliases and invalid empty
metadata.

- [ ] **Step 5: Run registry tests and existing install tests**

Run:

```bash
cargo test -p compass-cli install_commands::registry::tests --lib
cargo test -p compass-cli --test install_cli
```

Expected: both commands pass; legacy behavior remains unchanged because the new
registry is not yet the execution path.

- [ ] **Step 6: Commit**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands/model.rs crates/compass-cli/src/install_commands/registry.rs
git commit -m "refactor: define Compass agent registry"
```

---

### Task 2: Parse installation modes and detect scope and agents

**Files:**

- Create: `crates/compass-cli/src/install_commands/request.rs`
- Create: `crates/compass-cli/src/install_commands/detect.rs`
- Modify: `crates/compass-cli/src/install_commands.rs`
- Test: `crates/compass-cli/src/install_commands/request.rs`
- Test: `crates/compass-cli/src/install_commands/detect.rs`

**Interfaces:**

- Consumes: `InstallRequest`, `InstallScope`, `AgentRegistry`
- Produces: `parse_install_request(args: &[String]) -> Result<InstallRequest, String>`, `resolve_scope(request: &InstallRequest, cwd: &Path, home: Option<&Path>) -> Result<InstallScope, String>`, and `detect_agents(registry: &AgentRegistry, scope: &InstallScope, environment: &DetectionEnvironment) -> BTreeMap<String, Vec<String>>`

- [ ] **Step 1: Write failing request and scope tests**

```rust
#[test]
fn parses_repeatable_platforms_and_automation_options() {
    let args = strings(&[
        "--platform", "codex", "-p", "claude", "--project", "--dry-run",
        "--require-all", "--format", "json",
    ]);
    let request = parse_install_request(&args).expect("valid request");
    assert_eq!(request.platforms, ["codex", "claude"]);
    assert!(request.project);
    assert!(request.dry_run);
    assert!(request.require_all);
    assert_eq!(request.format, OutputFormat::Json);
}

#[test]
fn rejects_conflicting_modes_before_writes() {
    for args in [
        strings(&["--project", "--user"]),
        strings(&["--all", "--platform", "codex"]),
        strings(&["--format", "yaml"]),
    ] {
        assert!(parse_install_request(&args).is_err(), "{args:?}");
    }
}

#[test]
fn automatic_scope_uses_git_root_then_user_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let nested = repo.join("crates/app");
    std::fs::create_dir_all(repo.join(".git")).expect("git marker");
    std::fs::create_dir_all(&nested).expect("nested");
    let request = parse_install_request(&[]).expect("default request");
    assert_eq!(
        resolve_scope(&request, &nested, Some(temp.path())).expect("project scope"),
        InstallScope::Project(repo)
    );

    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    assert_eq!(
        resolve_scope(&request, &outside, Some(temp.path())).expect("user scope"),
        InstallScope::User(temp.path().to_path_buf())
    );
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
```

- [ ] **Step 2: Write failing detection tests**

```rust
#[test]
fn detection_records_strong_evidence_without_treating_a_name_as_proof() {
    let fixture = DetectionFixture::new();
    fixture.executable("codex");
    fixture.config(".claude/settings.json", "{}");
    fixture.directory(".kiro");
    let registry = AgentRegistry::new().expect("registry");
    let detected = detect_agents(&registry, &fixture.scope(), &fixture.environment());
    assert_eq!(detected.get("codex"), Some(&vec!["executable:codex".to_owned()]));
    assert_eq!(
        detected.get("claude"),
        Some(&vec!["config:.claude/settings.json".to_owned()])
    );
    assert!(!detected.contains_key("kiro"));
}

#[test]
fn agent_environment_overrides_are_strong_evidence() {
    let fixture = DetectionFixture::new();
    fixture.environment_variable("CODEX_HOME", fixture.path(".codex-home"));
    fixture.environment_variable("CLAUDE_CONFIG_DIR", fixture.path(".claude-home"));
    let detected = detect_agents(
        &AgentRegistry::new().expect("registry"),
        &fixture.scope(),
        &fixture.environment(),
    );
    assert!(detected["codex"].iter().any(|value| value.starts_with("env:CODEX_HOME=")));
    assert!(detected["claude"].iter().any(|value| value.starts_with("env:CLAUDE_CONFIG_DIR=")));
}
```

Define the unit-test fixture in `detect.rs` before these tests:

```rust
#[cfg(test)]
struct DetectionFixture {
    directory: tempfile::TempDir,
    variables: std::cell::RefCell<BTreeMap<String, OsString>>,
}

#[cfg(test)]
impl DetectionFixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().expect("tempdir"),
            variables: std::cell::RefCell::new(BTreeMap::new()),
        }
    }

    fn root(&self) -> PathBuf {
        self.directory.path().join("project")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.directory.path().join(relative)
    }

    fn scope(&self) -> InstallScope {
        fs::create_dir_all(self.root()).expect("project root");
        InstallScope::Project(self.root())
    }

    fn executable(&self, name: &str) {
        let path = self.path("bin").join(name);
        fs::create_dir_all(path.parent().expect("bin parent")).expect("bin");
        fs::write(path, b"fixture").expect("executable");
    }

    fn config(&self, relative: &str, content: &str) {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("config parent")).expect("config parent");
        fs::write(path, content).expect("config");
    }

    fn directory(&self, relative: &str) {
        fs::create_dir_all(self.root().join(relative)).expect("directory");
    }

    fn environment_variable(&self, name: &str, value: PathBuf) {
        self.variables
            .borrow_mut()
            .insert(name.to_owned(), value.into_os_string());
    }

    fn environment(&self) -> DetectionEnvironment {
        DetectionEnvironment {
            path: vec![self.path("bin")],
            variables: self.variables.borrow().clone(),
            project_root: Some(self.root()),
            user_home: Some(self.directory.path().join("home")),
        }
    }
}
```

- [ ] **Step 3: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli install_commands::request::tests --lib
cargo test -p compass-cli install_commands::detect::tests --lib
```

Expected: compilation fails because request and detection functions are absent.

- [ ] **Step 4: Implement the parser and scope resolver**

Support positional platform compatibility, `--platform NAME`,
`--platform=NAME`, and `-p NAME`. Deduplicate repeated identical platforms
while preserving order. Reject unknown options, conflicting scope, `--all`
with a platform, missing values, and unsupported formats.

Use this Git-root resolver:

```rust
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.canonicalize().ok()?;
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
```

Explicit `--project` returns an error when no Git root exists. Explicit
`--user` returns an error when home is unavailable. Default mode chooses a Git
root when present and otherwise requires a home.

- [ ] **Step 5: Implement evidence-based detection**

Define:

```rust
pub(super) struct DetectionEnvironment {
    pub path: Vec<PathBuf>,
    pub variables: BTreeMap<String, OsString>,
    pub project_root: Option<PathBuf>,
    pub user_home: Option<PathBuf>,
}
```

Check command names against `PATH` and Windows `PATHEXT`. Treat a regular file
at a registry config path as strong evidence only when it is valid UTF-8 and,
for `.json`, valid JSON. Treat agent environment overrides as strong evidence.
Record existing directories only as supporting evidence after one strong signal
exists. Sort and deduplicate evidence for deterministic reports.

- [ ] **Step 6: Run tests**

Run:

```bash
cargo test -p compass-cli install_commands::request::tests --lib
cargo test -p compass-cli install_commands::detect::tests --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands/request.rs crates/compass-cli/src/install_commands/detect.rs
git commit -m "feat: detect Compass assistant install modes"
```

---

### Task 3: Build immutable plans and stable reports

**Files:**

- Create: `crates/compass-cli/src/install_commands/plan.rs`
- Create: `crates/compass-cli/src/install_commands/report.rs`
- Modify: `crates/compass-cli/src/install_commands/model.rs`
- Modify: `crates/compass-cli/src/install_commands.rs`
- Test: `crates/compass-cli/src/install_commands/plan.rs`
- Test: `crates/compass-cli/src/install_commands/report.rs`

**Interfaces:**

- Consumes: `InstallRequest`, `InstallScope`, registry records, detection map
- Produces: `InstallPlan`, `InstallTarget`, `TargetKind`, `build_install_plan(registry: &AgentRegistry, request: InstallRequest, scope: InstallScope, detected: BTreeMap<String, Vec<String>>) -> Result<InstallPlan, String>`, `InstallReport::exit_code(require_all: bool) -> u8`, `render_text(&InstallReport) -> String`, and `render_json(&InstallReport) -> Result<String, String>`

- [ ] **Step 1: Write failing planning tests**

```rust
#[test]
fn automatic_plan_always_has_one_shared_target_and_detected_native_targets() {
    let registry = AgentRegistry::new().expect("registry");
    let request = InstallRequest::default();
    let scope = InstallScope::Project(PathBuf::from("/repo"));
    let detected = BTreeMap::from([
        ("codex".to_owned(), vec!["executable:codex".to_owned()]),
        ("gemini".to_owned(), vec!["executable:gemini".to_owned()]),
        ("claude".to_owned(), vec!["executable:claude".to_owned()]),
    ]);
    let plan = build_install_plan(&registry, request, scope, detected).expect("plan");
    assert_eq!(plan.targets.len(), 2);
    assert_eq!(
        plan.targets[0].consumers,
        BTreeSet::from(["agents".to_owned(), "codex".to_owned(), "gemini".to_owned()])
    );
    assert_eq!(
        plan.targets[0].skill_path.as_deref(),
        Some(Path::new("/repo/.agents/skills/compass/SKILL.md"))
    );
    assert_eq!(plan.targets[1].consumers, BTreeSet::from(["claude".to_owned()]));
}

#[test]
fn explicit_platforms_bypass_detection_and_deduplicate_shared_path() {
    let request = InstallRequest {
        platforms: vec!["codex".into(), "copilot".into()],
        ..InstallRequest::default()
    };
    let plan = build_install_plan(
        &AgentRegistry::new().expect("registry"),
        request,
        InstallScope::User(PathBuf::from("/home/test")),
        BTreeMap::from([("claude".into(), vec!["executable:claude".into()])]),
    )
    .expect("plan");
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(
        plan.targets[0].consumers,
        BTreeSet::from(["codex".to_owned(), "copilot".to_owned()])
    );
}
```

- [ ] **Step 2: Write failing report tests**

```rust
#[test]
fn partial_success_is_visible_and_require_all_changes_the_exit_code() {
    let report = fixture_report(vec![
        target("shared", InstallStatus::Installed, None),
        target("claude-hook", InstallStatus::Skipped, Some("invalid JSON")),
    ]);
    assert_eq!(report.exit_code(false), 0);
    assert_eq!(report.exit_code(true), 1);
    let text = render_text(&report);
    assert!(text.contains("INSTALLED"));
    assert!(text.contains("SKIPPED"));
    assert!(text.contains("invalid JSON"));
}

#[test]
fn json_report_is_versioned_and_matches_text_data() {
    let report = fixture_report(vec![target("shared", InstallStatus::Current, None)]);
    let value: serde_json::Value =
        serde_json::from_str(&render_json(&report).expect("json report")).expect("valid JSON");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["results"][0]["status"], "current");
    assert_eq!(value["graph_exists"], false);
    assert_eq!(
        value["next_actions"][0],
        "Run `compass update .` to build the graph."
    );
}

fn target(id: &str, status: InstallStatus, reason: Option<&str>) -> TargetResult {
    TargetResult {
        id: id.to_owned(),
        consumers: BTreeSet::from([id.to_owned()]),
        status,
        paths: vec![PathBuf::from(format!("/repo/{id}"))],
        reason: reason.map(str::to_owned),
        rollback: None,
    }
}

fn fixture_report(results: Vec<TargetResult>) -> InstallReport {
    InstallReport {
        schema: 1,
        scope: ScopeKind::Project,
        root: PathBuf::from("/repo"),
        detected: BTreeMap::new(),
        results,
        graph_exists: false,
        next_actions: vec!["Run `compass update .` to build the graph.".to_owned()],
    }
}
```

- [ ] **Step 3: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli install_commands::plan::tests --lib
cargo test -p compass-cli install_commands::report::tests --lib
```

Expected: compilation fails because plan and rendering functions are absent.

- [ ] **Step 4: Implement plan expansion and deduplication**

Add `InstallPlan` and `InstallTarget` to `model.rs`. Key targets by normalized
skill path plus target kind. Merge consumer ids in a `BTreeSet`. In automatic
mode, seed the `agents` portable consumer, add all detected records, and retain
adapter-only targets separately. In explicit mode, resolve only requested ids.
In `--all` mode, resolve every registry record.

Set `graph_exists` from `<scope-root>/compass-out/graph.json`. Add
`Run \`compass update .\` to build the graph.` only when absent. Add each
agent’s documented reload instruction once.

- [ ] **Step 5: Implement stable report rendering**

Serialize `InstallReport` with `serde_json::to_string_pretty`. Text output must
sort targets by id and use fixed status labels:

```rust
fn status_label(status: InstallStatus) -> &'static str {
    match status {
        InstallStatus::Installed => "INSTALLED",
        InstallStatus::Updated => "UPDATED",
        InstallStatus::Current => "CURRENT",
        InstallStatus::Skipped => "SKIPPED",
        InstallStatus::Failed => "FAILED",
    }
}
```

`exit_code(false)` returns `0` only if at least one result is installed,
updated, or current. `exit_code(true)` additionally requires zero skipped or
failed results.

- [ ] **Step 6: Run tests**

Run:

```bash
cargo test -p compass-cli install_commands::plan::tests --lib
cargo test -p compass-cli install_commands::report::tests --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands/model.rs crates/compass-cli/src/install_commands/plan.rs crates/compass-cli/src/install_commands/report.rs
git commit -m "feat: plan and report multi-agent installs"
```

---

### Task 4: Add exact ownership, locking, and safe configuration storage

**Files:**

- Create: `crates/compass-cli/src/install_commands/storage.rs`
- Modify: `crates/compass-cli/src/install_commands.rs`
- Modify: `crates/compass-cli/Cargo.toml`
- Modify: `Cargo.lock`
- Test: `crates/compass-cli/src/install_commands/storage.rs`

**Interfaces:**

- Consumes: `compass_files::write_text_atomic`, embedded skill assets
- Produces: `OwnershipManifest`, `OwnershipState`, `TargetTransaction`, `content_digest(&[u8]) -> String`, `InstallLock::acquire(&Path) -> Result<InstallLock, String>`, `preflight_skill(&Path) -> Result<OwnershipState, String>`, `install_skill_tree(destination: &Path, scope: ScopeKind, consumers: BTreeSet<String>, files: &BTreeMap<String, Vec<u8>>) -> Result<InstallStatus, String>`, `remove_owned_tree(destination: &Path) -> Result<InstallStatus, String>`, `load_json_object_strict(path: &Path) -> Result<Map<String, Value>, String>`, `load_toml_table_strict(path: &Path) -> Result<toml::Table, String>`, `replace_managed_section(content: &str, section: &str) -> Result<String, String>`, and `remove_managed_section(content: &str) -> Result<String, String>`

- [ ] **Step 1: Add dependencies and write failing manifest tests**

Add:

```toml
sha2.workspace = true
toml.workspace = true
```

Then add:

```rust
#[test]
fn manifest_detects_current_modified_and_unowned_content() {
    let fixture = StorageFixture::new();
    let installed = fixture.install_skill(&["codex", "gemini"]).expect("install");
    assert_eq!(installed, InstallStatus::Installed);
    assert_eq!(fixture.preflight().expect("preflight"), OwnershipState::Current);

    fs::write(fixture.skill_dir().join("SKILL.md"), "user edit").expect("edit");
    assert_eq!(fixture.preflight().expect("preflight"), OwnershipState::Modified);

    fs::remove_file(fixture.skill_dir().join(MANIFEST_NAME)).expect("remove manifest");
    assert_eq!(fixture.preflight().expect("preflight"), OwnershipState::Unowned);
}

#[test]
fn malformed_json_and_toml_are_preserved_byte_for_byte() {
    let fixture = StorageFixture::new();
    let json = fixture.path("settings.json");
    let toml = fixture.path("config.toml");
    fs::write(&json, b"{ broken").expect("json");
    fs::write(&toml, b"[broken").expect("toml");
    assert!(load_json_object_strict(&json).is_err());
    assert!(load_toml_table_strict(&toml).is_err());
    assert_eq!(fs::read(json).expect("json bytes"), b"{ broken");
    assert_eq!(fs::read(toml).expect("toml bytes"), b"[broken");
}

#[test]
fn managed_sections_use_exact_markers() {
    let original = "# Notes\n\n## compass\nuser-owned heading\n";
    let updated = replace_managed_section(original, "graph-first").expect("section");
    assert!(updated.contains("## compass\nuser-owned heading"));
    assert!(updated.contains("<!-- compass:managed:start -->"));
    assert_eq!(remove_managed_section(&updated).expect("remove"), original);
}
```

- [ ] **Step 2: Write failing lock and staging tests**

```rust
#[test]
fn lock_excludes_a_second_installer_and_drop_releases_it() {
    let fixture = StorageFixture::new();
    let first = InstallLock::acquire(fixture.root()).expect("first lock");
    assert!(InstallLock::acquire(fixture.root()).is_err());
    drop(first);
    assert!(InstallLock::acquire(fixture.root()).is_ok());
}

#[test]
fn failed_stage_does_not_replace_current_skill() {
    let fixture = StorageFixture::new();
    fixture.install_skill(&["agents"]).expect("initial install");
    let before = fixture.tree();
    let error = fixture.install_with_missing_asset().expect_err("missing asset");
    assert!(error.contains("missing embedded asset"));
    assert_eq!(fixture.tree(), before);
}
```

Define the test fixture in the same unit-test module:

```rust
struct StorageFixture {
    directory: tempfile::TempDir,
}

impl StorageFixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn skill_dir(&self) -> PathBuf {
        self.root().join(".agents/skills/compass")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root().join(relative)
    }

    fn package() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (
                "SKILL.md".to_owned(),
                b"---\nname: compass\ndescription: fixture\n---\n".to_vec(),
            ),
            (
                "references/query.md".to_owned(),
                b"# Query\n\nRun `compass query`.\n".to_vec(),
            ),
        ])
    }

    fn install_skill(&self, consumers: &[&str]) -> Result<InstallStatus, String> {
        install_skill_tree(
            &self.skill_dir(),
            ScopeKind::Project,
            consumers.iter().map(|value| (*value).to_owned()).collect(),
            &Self::package(),
        )
    }

    fn install_with_missing_asset(&self) -> Result<InstallStatus, String> {
        install_skill_tree(
            &self.skill_dir(),
            ScopeKind::Project,
            BTreeSet::from(["agents".to_owned()]),
            &BTreeMap::from([(
                "references/query.md".to_owned(),
                b"# Query\n".to_vec(),
            )]),
        )
    }

    fn preflight(&self) -> Result<OwnershipState, String> {
        preflight_skill(&self.skill_dir())
    }

    fn tree(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        collect_tree(self.root()).expect("directory tree")
    }
}
```

Add `collect_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String>`
as a test-only recursive helper that sorts `read_dir` entries before reading
regular files.

- [ ] **Step 3: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli install_commands::storage::tests --lib
```

Expected: compilation fails because storage types and functions are absent.

- [ ] **Step 4: Implement manifests and digests**

Use:

```rust
pub(super) const MANIFEST_NAME: &str = ".compass-install.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct OwnershipManifest {
    pub schema: u32,
    pub compass_version: String,
    pub scope: ScopeKind,
    pub root: String,
    pub consumers: BTreeSet<String>,
    pub files: BTreeMap<String, String>,
    pub adapters: BTreeMap<String, String>,
}

pub(super) fn content_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
```

Project manifests store root `"."`; user manifests store root `"~"` so
checked-in files contain no machine-specific absolute path. Normalize managed
relative paths with `/`. Reject `..`, absolute paths, duplicate paths, and a
manifest whose schema is not `1`.

- [ ] **Step 5: Implement locking and atomic directory staging**

Acquire `<root>/.compass-install.lock` with `OpenOptions::create_new(true)`.
Write a token containing process id, UNIX nanoseconds, and an atomic sequence.
On drop, remove the file only when its token still matches. A lock older than
five minutes can be removed after re-reading metadata and content; two
contenders still race through `create_new`, so only one wins.

Stage the complete skill in
`.<directory-name>.<pid>.<sequence>.compass-stage`, validate every embedded
asset and digest, then rename it into place. Keep the existing tree unchanged
until the stage validates. On Windows, use the same backup-copy fallback
contract as `compass-files` atomic writes and restore the backup on failure.

- [ ] **Step 6: Implement strict config and exact sections**

`load_json_object_strict` and `load_toml_table_strict` return empty maps only
when the path does not exist or is an empty file. Existing nonempty malformed
content returns an error with its path and parser message.

Managed Markdown uses:

```text
<!-- compass:managed:start -->
## compass
Use `compass query` before broad source searches.
<!-- compass:managed:end -->
```

Reject one-sided or nested markers. Replace only the bytes between the exact
pair. Remove only that pair and normalize at most one surrounding blank line.

- [ ] **Step 7: Run tests**

Run:

```bash
cargo test -p compass-cli install_commands::storage::tests --lib
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.lock crates/compass-cli/Cargo.toml crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands/storage.rs
git commit -m "feat: add safe Compass install storage"
```

---

### Task 5: Route every platform through the planner and transactional adapters

**Files:**

- Create: `crates/compass-cli/src/install_commands/adapters.rs`
- Modify: `crates/compass-cli/src/install_commands.rs`
- Modify: `crates/compass-cli/src/install_commands/model.rs`
- Modify: `crates/compass-cli/src/install_commands/plan.rs`
- Create: `crates/compass-cli/tests/install_support/mod.rs`
- Modify: `crates/compass-cli/tests/install_cli.rs`
- Create: `crates/compass-cli/tests/install_detection.rs`

**Interfaces:**

- Consumes: `InstallPlan`, registry, storage primitives, embedded assets
- Produces: `execute_install(plan: &InstallPlan) -> InstallReport`, `apply_adapter(target: &InstallTarget, registry: &AgentRegistry, transaction: &mut TargetTransaction) -> Result<Vec<PathBuf>, String>`, and the new `command_install` execution path

- [ ] **Step 1: Extract one shared integration-test fixture**

Move the current isolated `InstallFixture` and sorted `directory_tree` helper
from `install_cli.rs` into `tests/install_support/mod.rs`. Use this exact public
surface so Tasks 5 and 6 share setup without copying behavior:

```rust
pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub struct InstallFixture {
    _directory: tempfile::TempDir,
    pub project: PathBuf,
    pub home: PathBuf,
    bin: PathBuf,
}

impl InstallFixture {
    fn new(git: bool) -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let home = directory.path().join("home");
        let bin = directory.path().join("bin");
        fs::create_dir_all(&project)?;
        fs::create_dir_all(&home)?;
        fs::create_dir_all(&bin)?;
        if git {
            fs::create_dir_all(project.join(".git"))?;
        }
        Ok(Self {
            _directory: directory,
            project,
            home,
            bin,
        })
    }

    pub fn new_git() -> TestResult<Self> {
        Self::new(true)
    }

    pub fn new_outside_git() -> TestResult<Self> {
        Self::new(false)
    }

    pub fn executable(&self, name: &str) -> TestResult<()> {
        let path = self.bin.join(name);
        fs::write(&path, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    fn command(&self, arguments: &[&str]) -> TestResult<Command> {
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(
                std::env::var_os("PATH")
                    .as_deref()
                    .map(std::env::split_paths)
                    .into_iter()
                    .flatten(),
            ),
        )?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
        command
            .args(arguments)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("PATH", path)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR");
        Ok(command)
    }

    pub fn run(&self, arguments: &[&str]) -> TestResult<Output> {
        Ok(self.command(arguments)?.output()?)
    }

    pub fn spawn(&self, arguments: &[&str]) -> TestResult<Child> {
        Ok(self.command(arguments)?.spawn()?)
    }

    pub fn write(&self, path: impl AsRef<Path>, bytes: &[u8]) -> TestResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
        Ok(())
    }

    pub fn write_json(&self, path: impl AsRef<Path>, value: Value) -> TestResult<()> {
        self.write(path, &serde_json::to_vec_pretty(&value)?)
    }

    pub fn tree(&self) -> TestResult<BTreeMap<PathBuf, Vec<u8>>> {
        directory_tree(self._directory.path())
    }

    pub fn find_files_named(&self, name: &str) -> TestResult<Vec<PathBuf>> {
        Ok(self
            .tree()?
            .into_keys()
            .filter(|path| path.file_name().is_some_and(|value| value == name))
            .map(|path| self._directory.path().join(path))
            .collect())
    }

    pub fn find_stage_directories(&self) -> TestResult<Vec<PathBuf>> {
        find_directories(self._directory.path(), ".compass-stage")
    }

    pub fn assert_complete_owned_skill(&self, relative: &str) -> TestResult<()> {
        let root = self.project.join(relative);
        assert!(root.join("SKILL.md").is_file());
        assert!(root.join("references/query.md").is_file());
        assert!(root.join(".compass-install.json").is_file());
        Ok(())
    }
}

pub fn assert_success(context: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{context}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
```

`new_git()` creates `project/.git`, `home`, and `bin`. `new_outside_git()`
creates the same directories without `.git` and uses the non-repository
directory as the command cwd. `executable()` writes a fixture file in `bin`
and, on Unix, sets mode `0o755`. Both `run()` and `spawn()` set `HOME`,
`USERPROFILE`, and `PATH`, remove `CODEX_HOME` and `CLAUDE_CONFIG_DIR`, use the
fixture project as cwd, and execute `CARGO_BIN_EXE_compass`. All recursive tree
walks sort entries. Implement
`directory_tree(root: &Path) -> TestResult<BTreeMap<PathBuf, Vec<u8>>>` and
`find_directories(root: &Path, suffix: &str) -> TestResult<Vec<PathBuf>>` as
recursive helpers that sort each `read_dir` result before descending.

Each integration test begins with:

```rust
mod install_support;

use install_support::{InstallFixture, TestResult, assert_success};
```

- [ ] **Step 2: Add failing no-argument and explicit CLI tests**

Create `install_detection.rs` with the existing isolated fixture pattern:

```rust
#[test]
fn plain_install_in_git_uses_project_scope_and_detected_agents() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.executable("codex")?;
    fixture.executable("claude")?;
    let output = fixture.run(&["install"])?;
    assert_success("automatic install", &output);
    assert!(fixture.project.join(".agents/skills/compass/SKILL.md").is_file());
    assert!(fixture.project.join(".claude/skills/compass/SKILL.md").is_file());
    assert!(!fixture.home.join(".agents/skills/compass/SKILL.md").exists());
    Ok(())
}

#[test]
fn plain_install_without_git_or_agents_uses_user_portable_fallback() -> TestResult {
    let fixture = InstallFixture::new_outside_git()?;
    let output = fixture.run(&["install"])?;
    assert_success("portable fallback", &output);
    assert!(fixture.home.join(".agents/skills/compass/SKILL.md").is_file());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Run `compass update .`"));
    Ok(())
}

#[test]
fn repeated_platforms_install_only_explicit_consumers() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.executable("gemini")?;
    let output = fixture.run(&[
        "install", "--platform", "codex", "--platform", "claude", "--project",
    ])?;
    assert_success("explicit install", &output);
    assert!(fixture.project.join(".agents/skills/compass/SKILL.md").is_file());
    assert!(fixture.project.join(".claude/skills/compass/SKILL.md").is_file());
    assert!(!fixture.project.join(".gemini/settings.json").exists());
    Ok(())
}
```

- [ ] **Step 3: Add failing dry-run and JSON tests**

```rust
#[test]
fn dry_run_reports_without_mutating() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.executable("codex")?;
    let before = fixture.tree()?;
    let output = fixture.run(&["install", "--dry-run", "--format", "json"])?;
    assert_success("dry run", &output);
    assert_eq!(fixture.tree()?, before);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["schema"], 1);
    assert_eq!(report["scope"], "project");
    Ok(())
}

#[test]
fn shared_consumers_write_one_skill_manifest() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    for command in ["codex", "gemini", "opencode", "copilot"] {
        fixture.executable(command)?;
    }
    assert_success("shared install", &fixture.run(&["install"])?);
    let manifests = fixture.find_files_named(".compass-install.json")?;
    assert_eq!(manifests, [fixture.project.join(".agents/skills/compass/.compass-install.json")]);
    Ok(())
}
```

- [ ] **Step 4: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli --test install_detection
```

Expected: tests fail because no-argument install still defaults to Claude and
the new options are not wired to execution.

- [ ] **Step 5: Move platform mutation into adapters**

Move the current platform-specific functions from `install_commands.rs` into
`adapters.rs` in these groups:

- Claude/CodeBuddy/Gemini/Codex JSON hooks
- AGENTS/CLAUDE/GEMINI/Copilot managed sections
- OpenCode/Kilo plugins and config registration
- Cursor/Windsurf/Kiro/Antigravity rule or steering files
- Direct compatibility command registration

Replace permissive `load_json_object` calls with
`load_json_object_strict`. Replace heading-based sections with exact managed
markers. Give every hook and plugin a stable adapter id such as
`compass:codex:pre-tool-use`; remove entries by exact object equality or exact
registered path.

- [ ] **Step 6: Implement per-target execution**

For each planned target:

1. Acquire the scope lock.
2. Preflight the skill and every adapter path.
3. Return `Skipped` for unowned or modified conflicts.
4. Return `Current` when all digests and adapter identities match.
5. Stage and install the canonical skill if present.
6. Apply adapters with captured original bytes.
7. Re-read the skill, manifest, and adapter identities.
8. On error, restore captured bytes and remove only newly created owned paths.
9. Return `Installed`, `Updated`, `Failed`, and rollback details accurately.

For `dry_run`, execute only steps 1–4 and render planned paths with no writes.

- [ ] **Step 7: Wire `command_install` and direct aliases**

`command_install` must call, in order:

```rust
let request = request::parse_install_request(args)?;
let scope = request::resolve_scope(&request, &cwd, home.as_deref())?;
let registry = registry::AgentRegistry::new()?;
let detected = detect::detect_agents(&registry, &scope, &environment);
let plan = plan::build_install_plan(&registry, request.clone(), scope, detected)?;
let report = adapters::execute_install(&plan);
let code = report.exit_code(request.require_all);
let body = match request.format {
    OutputFormat::Text => report::render_text(&report),
    OutputFormat::Json => report::render_json(&report)?,
};
```

Translate errors into `Outcome::failure`. Use an `Outcome` constructor that can
return rendered stdout with a nonzero code for `--require-all`; do not move the
report to stderr. Direct commands create an explicit one-platform request and
call the same pipeline.

- [ ] **Step 8: Update current acceptance destinations**

Change the Codex assertions in `install_cli.rs` from
`.codex/skills/compass/SKILL.md` to `.agents/skills/compass/SKILL.md`. Add Cline
to project and global platform matrices. Assert every canonical skill contains
the ownership manifest and all references.

- [ ] **Step 9: Run focused tests**

Run:

```bash
cargo test -p compass-cli --test install_detection
cargo test -p compass-cli --test install_cli
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands crates/compass-cli/tests/install_support crates/compass-cli/tests/install_cli.rs crates/compass-cli/tests/install_detection.rs
git commit -m "feat: install Compass for detected agents"
```

---

### Task 6: Harden migration, uninstall, conflicts, and rollback

**Files:**

- Modify: `crates/compass-cli/src/install_commands.rs`
- Modify: `crates/compass-cli/src/install_commands/adapters.rs`
- Modify: `crates/compass-cli/src/install_commands/plan.rs`
- Modify: `crates/compass-cli/src/install_commands/storage.rs`
- Create: `crates/compass-cli/tests/install_failures.rs`
- Modify: `crates/compass-cli/tests/install_cli.rs`

**Interfaces:**

- Consumes: ownership manifests, legacy `.compass_version`, install plan/report
- Produces: `LegacyMigration`, `plan_legacy_migration(scope: &InstallScope, shared_target: &InstallTarget) -> Result<Option<LegacyMigration>, String>`, `execute_uninstall(plan: &InstallPlan) -> InstallReport`, exact rollback behavior, and `--require-all` partial-success lifecycle

- [ ] **Step 1: Extend the shared fixture for legacy installations**

Add this method in `tests/install_support/mod.rs`:

```rust
pub fn write_legacy_codex_skill(&self, modified: bool) -> TestResult<()> {
    assert_success(
        "seed current bundle",
        &self.run(&["install", "--project", "--platform", "codex"])?,
    );
    let shared = self.project.join(".agents/skills/compass");
    let legacy = self.project.join(".codex/skills/compass");
    copy_directory(&shared, &legacy)?;
    fs::remove_file(legacy.join(".compass-install.json"))?;
    fs::write(legacy.join(".compass_version"), env!("CARGO_PKG_VERSION"))?;
    if modified {
        fs::write(legacy.join("SKILL.md"), "user-modified legacy skill")?;
    }
    fs::remove_dir_all(shared)?;
    Ok(())
}
```

Add a sorted recursive
`copy_directory(source: &Path, destination: &Path) -> TestResult<()>` beside
the existing tree helpers. It creates destination directories and uses
`fs::copy` for regular files.

- [ ] **Step 2: Write failing malformed-config and partial-success tests**

```rust
#[test]
fn malformed_claude_json_is_preserved_while_shared_target_succeeds() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.executable("codex")?;
    fixture.executable("claude")?;
    let settings = fixture.project.join(".claude/settings.json");
    fixture.write(&settings, b"{ broken")?;
    let output = fixture.run(&["install", "--format", "json"])?;
    assert_success("best effort", &output);
    assert_eq!(fs::read(&settings)?, b"{ broken");
    assert!(fixture.project.join(".agents/skills/compass/SKILL.md").is_file());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(report["results"].as_array().expect("results").iter().any(|result| {
        result["status"] == "skipped"
            && result["reason"].as_str().is_some_and(|value| value.contains("invalid JSON"))
    }));
    Ok(())
}

#[test]
fn require_all_returns_nonzero_without_hiding_the_report() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.executable("codex")?;
    fixture.executable("claude")?;
    fixture.write(fixture.project.join(".claude/settings.json"), b"{ broken")?;
    let output = fixture.run(&["install", "--require-all", "--format", "json"])?;
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["schema"], 1);
    assert!(output.stderr.is_empty());
    Ok(())
}
```

- [ ] **Step 3: Write failing ownership and uninstall tests**

```rust
#[test]
fn modified_managed_skill_is_skipped_and_uninstall_preserves_it() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    assert_success(
        "install",
        &fixture.run(&["install", "--project", "--platform", "codex"])?,
    );
    let skill = fixture.project.join(".agents/skills/compass/SKILL.md");
    fs::write(&skill, "user-modified")?;
    let reinstall = fixture.run(&["install", "--project", "--platform", "codex"])?;
    assert!(reinstall.status.success());
    assert!(String::from_utf8_lossy(&reinstall.stdout).contains("SKIPPED"));
    assert_success(
        "uninstall",
        &fixture.run(&["uninstall", "--project", "--platform", "codex"])?,
    );
    assert_eq!(fs::read_to_string(skill)?, "user-modified");
    Ok(())
}

#[test]
fn uninstall_removes_only_exact_compass_entries() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    assert_success(
        "install",
        &fixture.run(&["install", "--project", "--platform", "codex"])?,
    );
    let settings = fixture.project.join(".codex/hooks.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&settings)?)?;
    document["hooks"]["PreToolUse"]
        .as_array_mut()
        .expect("PreToolUse array")
        .push(json!({
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": "my-compass-wrapper"}]
        }));
    fixture.write_json(&settings, document)?;
    assert_success(
        "uninstall",
        &fixture.run(&["uninstall", "--project", "--platform", "codex"])?,
    );
    let document: serde_json::Value = serde_json::from_slice(&fs::read(settings)?)?;
    assert!(document.to_string().contains("my-compass-wrapper"));
    assert!(!document.to_string().contains("\"compass hook-check\""));
    Ok(())
}
```

- [ ] **Step 4: Write failing legacy migration and concurrency tests**

```rust
#[test]
fn managed_legacy_codex_skill_moves_only_after_shared_verification() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.write_legacy_codex_skill(false)?;
    assert_success("migration", &fixture.run(&["install", "--project", "--platform", "codex"])?);
    assert!(fixture.project.join(".agents/skills/compass/SKILL.md").is_file());
    assert!(!fixture.project.join(".codex/skills/compass").exists());
    Ok(())
}

#[test]
fn modified_legacy_codex_skill_is_preserved_and_reported() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    fixture.write_legacy_codex_skill(true)?;
    let output = fixture.run(&["install", "--project", "--platform", "codex"])?;
    assert_success("safe migration", &output);
    assert!(fixture.project.join(".agents/skills/compass/SKILL.md").is_file());
    assert!(fixture.project.join(".codex/skills/compass/SKILL.md").is_file());
    assert!(String::from_utf8_lossy(&output.stdout).contains("legacy Codex skill was modified"));
    Ok(())
}

#[test]
fn concurrent_installers_never_produce_a_partial_skill() -> TestResult {
    let fixture = InstallFixture::new_git()?;
    let first = fixture.spawn(&["install", "--project", "--platform", "codex"])?;
    let second = fixture.spawn(&["install", "--project", "--platform", "codex"])?;
    let outputs = [first.wait_with_output()?, second.wait_with_output()?];
    assert!(outputs.iter().any(|output| output.status.success()));
    fixture.assert_complete_owned_skill(".agents/skills/compass")?;
    assert!(fixture.find_stage_directories()?.is_empty());
    Ok(())
}
```

- [ ] **Step 5: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli --test install_failures
```

Expected: failures expose permissive config loading, broad substring removal,
missing migration, and missing lock behavior in the live command path.

- [ ] **Step 6: Implement verified legacy migration**

Recognize a legacy skill only when `.compass_version` exists beside
`.codex/skills/compass/SKILL.md`. Compare its files with the embedded bundle.
Install and verify the shared target first. Delete the legacy tree only when
every legacy managed file matches; otherwise preserve it and add a skipped
migration result with the exact path.

- [ ] **Step 7: Route uninstall through manifests and exact adapters**

Resolve the same registry targets as install. Remove a skill only when its
manifest parses, every target path stays within the resolved scope, and every
file digest still matches. Remove a managed section only between exact markers.
Remove hook/plugin values only when equal to the registered managed value.
Preserve modified files and return a skipped result.

Keep `--purge` limited to the resolved `COMPASS_OUT`/`compass-out` directory
inside the project root. Reject absolute `COMPASS_OUT` values and `..`
components before deletion.

- [ ] **Step 8: Finish rollback and concurrent behavior**

Ensure every adapter captures `Option<Vec<u8>>` for existing files before
mutation. Rollback writes original bytes atomically or removes a newly created
owned file. Include a second failure message when rollback cannot restore a
path. Hold one scope lock through target verification and manifest write.

- [ ] **Step 9: Run failure and lifecycle tests**

Run:

```bash
cargo test -p compass-cli --test install_failures
cargo test -p compass-cli --test install_cli
cargo test -p compass-cli --test install_detection
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/compass-cli/src/install_commands.rs crates/compass-cli/src/install_commands crates/compass-cli/tests/install_support crates/compass-cli/tests/install_cli.rs crates/compass-cli/tests/install_failures.rs
git commit -m "fix: harden Compass install lifecycle"
```

---

### Task 7: Tighten the canonical skill and build-time contracts

**Files:**

- Modify: `crates/compass-cli/assets/compass-skill/SKILL.md`
- Modify: `crates/compass-cli/assets/compass-integrations/agents-md.md`
- Modify: `crates/compass-cli/assets/compass-integrations/antigravity-rules.md`
- Modify: `crates/compass-cli/assets/compass-integrations/claude-md.md`
- Modify: `crates/compass-cli/assets/compass-integrations/gemini-md.md`
- Modify: `crates/compass-cli/assets/compass-integrations/kiro-steering.md`
- Modify: `crates/compass-cli/assets/compass-integrations/vscode-instructions.md`
- Modify: `tools/skillgen/mod.rs`
- Test: `tools/skillgen/mod.rs`
- Test: `crates/compass-cli/tests/install_cli.rs`

**Interfaces:**

- Consumes: Agent Skills specification and canonical embedded bundle
- Produces: portable `compatibility`/`metadata`, conditional graph guidance, activation-budget validation, and trigger fixtures

- [ ] **Step 1: Add failing skill-contract tests**

Add to the skill generator tests:

```rust
#[test]
fn canonical_frontmatter_is_portable_and_versioned() {
    let skill = fixture_skill();
    let metadata = parse_frontmatter(&skill).expect("frontmatter");
    assert_eq!(metadata["name"], "compass");
    assert!(metadata["description"].contains("architecture"));
    assert!(metadata["description"].contains("compass-out"));
    assert!(metadata["compatibility"].contains("compass CLI"));
    assert_eq!(metadata["metadata"]["author"], "crabbuild");
    assert!(metadata["metadata"]["version"].as_str().is_some());
}

#[test]
fn skill_stays_within_activation_budget_and_references_one_level_deep() {
    let root = fixture_skill_root();
    validate(&root, fixture_cli_source(), fixture_help_source()).expect("valid skill");
    let skill = fs::read_to_string(root.join("compass-skill/SKILL.md")).expect("skill");
    assert!(estimated_tokens(&skill) < 5_000);
    assert!(!skill.contains("references/nested/"));
}
```

Add an install assertion:

```rust
assert!(
    body.contains("When `compass-out/graph.json` exists"),
    "skill must not claim installation already built a graph"
);
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli skillgen --lib
cargo test -p compass-cli --test install_cli project_codex_install_creates_native_compass_skill
```

Expected: the metadata parser/activation check is absent and current integration
copy claims the graph exists unconditionally.

- [ ] **Step 3: Update skill frontmatter and graph-state wording**

Use:

```yaml
---
name: compass
description: "Navigate and verify codebase architecture, dependencies, history, and change impact with Compass. Use for codebase questions, when compass-out exists, or when the user invokes /compass."
compatibility: "Requires the native compass CLI for graph builds and queries."
metadata:
  author: crabbuild
  version: "${COMPASS_VERSION}"
---
```

Have `build.rs` replace only `${COMPASS_VERSION}` with
`CARGO_PKG_VERSION` before embedding. Change every always-on integration from
“This project has a Compass knowledge graph” to:

```text
Use the Compass knowledge graph at `compass-out/` when it exists. If codebase
navigation is requested and the graph is absent, recommend `compass update .`;
do not claim installation built the graph.
```

Keep the rules to read `GRAPH_REPORT.md`, navigate the wiki index, query before
broad searches, verify source, qualify inference, and update after code changes.

- [ ] **Step 4: Implement build-time metadata and budget validation**

Add a minimal frontmatter parser limited to the required scalar fields and the
`metadata` map. Validate Agent Skills name/description constraints,
`compatibility <= 500` characters, author/version presence, and parent
directory name. Estimate tokens conservatively as `(bytes + 2) / 3`; require
less than `5_000`. Reject reference paths with more than
`references/<file>.md`.

- [ ] **Step 5: Run skill and installer tests**

Run:

```bash
cargo test -p compass-cli skillgen --lib
cargo test -p compass-cli --test install_cli
cargo test -p compass-cli --test install_detection
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/compass-cli/assets crates/compass-cli/build.rs crates/compass-cli/tests/install_cli.rs tools/skillgen/mod.rs
git commit -m "docs: harden Compass agent guidance"
```

---

### Task 8: Update help, support documentation, and complete verification

**Files:**

- Modify: `crates/compass-cli/src/help.rs`
- Modify: `README.md`
- Modify: `docs/guides/assistant-setup.md`
- Modify: `crates/compass-cli/tests/compass_product.rs`
- Modify: `crates/compass-cli/tests/install_detection.rs`
- Modify: `crates/compass-cli/tests/install_failures.rs`

**Interfaces:**

- Consumes: registry platform catalog and final CLI behavior
- Produces: contract-tested help, verified support matrix, troubleshooting guide, final graph refresh

- [ ] **Step 1: Add failing help and documentation contracts**

Add product tests:

```rust
#[test]
fn install_help_documents_automatic_and_explicit_modes() {
    let output = compass(&["install", "--help"]);
    assert_success(&output);
    let help = String::from_utf8_lossy(&output.stdout);
    for text in [
        "auto-detect",
        "--project",
        "--user",
        "--platform <NAME>",
        "--all",
        "--dry-run",
        "--require-all",
        "--format <text|json>",
        "--strict",
    ] {
        assert!(help.contains(text), "missing {text}: {help}");
    }
}

#[test]
fn assistant_guide_uses_current_codex_and_native_skill_roots() {
    let guide = include_str!("../../../docs/guides/assistant-setup.md");
    for path in [
        ".agents/skills/compass",
        ".claude/skills/compass",
        ".kiro/skills/compass",
        ".cline/skills/compass",
    ] {
        assert!(guide.contains(path), "missing {path}");
    }
    assert!(!guide.contains(".codex/skills/compass"));
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test -p compass-cli --test compass_product install_help_documents_automatic_and_explicit_modes
cargo test -p compass-cli --test compass_product assistant_guide_uses_current_codex_and_native_skill_roots
```

Expected: help and the current guide still describe single-platform defaults
and the legacy Codex destination.

- [ ] **Step 3: Generate help platform text from the registry contract**

Expose `registry::platform_help()` and use it in the install help renderer so
the list cannot drift. Document automatic scope, repeated platforms, all new
options, `--strict` versus `--require-all`, direct aliases, and JSON exit
semantics. Update uninstall help with `--user`, repeatable platforms, and exact
ownership behavior when implemented by Task 6.

- [ ] **Step 4: Rewrite assistant setup around the new lifecycle**

Document:

- `compass install` automatic behavior
- Project/user selection and Git-root resolution
- Explicit one, many, and all-platform selection
- Shared versus native versus adapter support tiers
- Verified paths and source URLs dated 2026-07-24
- Detection evidence and `--dry-run`
- Text/JSON statuses and `--require-all`
- Claude `--strict`
- Reload/restart guidance, including Gemini `/skills reload`
- Legacy Codex migration and user-modified conflicts
- Exact uninstall and `--purge` boundaries
- Graph-first navigation and `compass update .`

Update the README quick start to lead with:

```bash
compass install
compass update .
```

and show explicit `--project --platform codex` as the deterministic CI
alternative.

- [ ] **Step 5: Run formatting and focused suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-cli --test compass_product
cargo test -p compass-cli --test install_cli
cargo test -p compass-cli --test install_detection
cargo test -p compass-cli --test install_failures
```

Expected: PASS.

- [ ] **Step 6: Run lint and full workspace tests**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Expected: PASS with no warnings or failed tests.

- [ ] **Step 7: Manually exercise the public command contract in an isolated home**

Run:

```bash
demo_root="$(mktemp -d)"
HOME="$demo_root/home" USERPROFILE="$demo_root/home" \
  cargo run -q -p compass-cli -- install --user --platform codex --platform claude --format json
HOME="$demo_root/home" USERPROFILE="$demo_root/home" \
  cargo run -q -p compass-cli -- install --user --platform codex --platform claude --dry-run
HOME="$demo_root/home" USERPROFILE="$demo_root/home" \
  cargo run -q -p compass-cli -- uninstall --user --platform codex --platform claude
```

Expected: JSON reports one shared and one Claude target; dry-run changes
nothing; uninstall removes only manifest-owned artifacts. Remove the explicit
temporary directory after recording the output.

- [ ] **Step 8: Refresh the repository knowledge graph**

Run:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Expected: successful incremental refresh of `graphify-out/` with the new
installer modules and relationships.

- [ ] **Step 9: Review final scope and commit**

Run:

```bash
git -C /Users/haipingfu/graphify/compass status --short
git -C /Users/haipingfu/graphify/compass diff --check
git -C /Users/haipingfu/graphify/compass diff --stat
```

Confirm no unrelated generated directories are staged. Then:

```bash
git add README.md docs/guides/assistant-setup.md crates/compass-cli/src/help.rs crates/compass-cli/tests
git commit -m "docs: explain automatic Compass agent setup"
```
