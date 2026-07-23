# Compass CLI help implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every Compass command descriptive, example-driven, terminal-aware help while preserving command behavior and the Graphify compatibility frontend.

**Architecture:** Add a typed help catalog, deterministic renderer, and command-path router under `compass-cli/src/help/`. Keep existing parsers and executors unchanged. The library renders plain help by default; the `compass` binary requests ANSI styling only for an interactive terminal that permits color.

**Tech Stack:** Rust 1.97.1, standard library `IsTerminal`, existing `compass-cli` outcome and test infrastructure, Cargo tests.

## Global constraints

- Preserve all normal command output, machine-readable output, and exit codes.
- Preserve Graphify help and command routing byte for byte.
- Add no command-line framework or styling dependency.
- Render ANSI styling only for an interactive terminal when `NO_COLOR` is absent and `TERM` is not `dumb`.
- Keep plain output deterministic and free of escape bytes.
- Document every public command, nested command, accepted option, accepted value, default, conflict, requirement, and repeatability rule.
- Keep internal worker and hook subprocess commands out of public root help.
- Write a failing behavior test before each production change.
- Run `graphify update .` from `/Users/haipingfu/graphify` after the final code change.

---

## File structure

- Create `crates/compass-cli/src/help/mod.rs`: public help entry points, routing, usage hints, and typo suggestions
- Create `crates/compass-cli/src/help/model.rs`: immutable help-page metadata and const builders
- Create `crates/compass-cli/src/help/render.rs`: deterministic plain rendering, wrapping, and semantic ANSI styling
- Create `crates/compass-cli/src/help/catalog/mod.rs`: page registry, root groups, aliases, and structural validation
- Create `crates/compass-cli/src/help/catalog/build.rs`: build, extraction, and semantic-maintenance pages
- Create `crates/compass-cli/src/help/catalog/explore.rs`: query, navigation, and benchmark pages
- Create `crates/compass-cli/src/help/catalog/history.rs`: history, diff, and every history subcommand page
- Create `crates/compass-cli/src/help/catalog/output.rs`: tree, export, and every export-format page
- Create `crates/compass-cli/src/help/catalog/integrate.rs`: serve, global, clone, add, PR, hook, install, provider, result, and reflection pages
- Create `crates/compass-cli/src/help/catalog/support.rs`: diagnose, update-check, merge-driver, and internal hook pages
- Create `crates/compass-cli/tests/help_cli.rs`: public CLI help, Graphify regression, and error-routing tests
- Modify `crates/compass-cli/src/lib.rs`: invoke the new plain help router and remove Compass-only string help dispatch
- Modify `crates/compass-cli/src/bin/compass.rs`: resolve help before streaming commands and select terminal styling
- Modify `README.md`: point the command-surface section to the new discoverable help forms

### Task 1: Define help metadata and deterministic rendering

**Files:**

- Create: `crates/compass-cli/src/help/model.rs`
- Create: `crates/compass-cli/src/help/render.rs`
- Create: `crates/compass-cli/src/help/mod.rs`
- Modify: `crates/compass-cli/src/lib.rs:1-15`

**Interfaces:**

- Produces: `HelpPage`, `HelpArgument`, `HelpOption`, `HelpVisibility`, `HelpGroup`, and const builder methods
- Produces: public `HelpStyle::{Plain, Ansi}` for the binary entry point
- Produces: `render_page(page: &HelpPage, children: &[&HelpPage], style: HelpStyle) -> String`
- Produces: `render_root(groups: &[HelpGroup], pages: &[HelpPage], style: HelpStyle) -> String`
- Consumes: no command parser state

- [ ] **Step 1: Write failing model and renderer tests**

Add `#[cfg(test)]` tests in `help/render.rs` that construct this page:

```rust
const SAMPLE: HelpPage = HelpPage::new(
    &["sample"],
    "Describe the sample command",
    &["compass sample [PATH] [OPTIONS]"],
)
.description("A sentence long enough to verify deterministic wrapping without changing words.")
.arguments(&[HelpArgument::new("[PATH]", "Project directory").default(".")])
.options(&[
    HelpOption::new(&["-o", "--out"], Some("DIR"), "Output directory")
        .default("compass-out"),
    HelpOption::new(&["--exclude"], Some("PATTERN"), "Exclude a matching path")
        .repeatable(),
])
.examples(&["compass sample", "compass sample ./api --out build"])
.tips(&["Use `compass watch` to refresh continuously."]);
```

Assert that plain output has the required section order, includes `[default: .]` and `[repeatable]`, wraps at 88 columns, and contains no `\x1b`. Assert that stripping ANSI sequences from styled output returns the plain output exactly.

- [ ] **Step 2: Run the renderer test and verify the red state**

Run: `cargo test -p compass-cli help::render::tests --lib -- --nocapture`

Expected: FAIL because `help`, `HelpPage`, and `render_page` do not exist.

- [ ] **Step 3: Implement immutable model types and const builders**

Use these exact public shapes inside the crate:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HelpVisibility { Public, Internal }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HelpArgument {
    pub(crate) syntax: &'static str,
    pub(crate) description: &'static str,
    pub(crate) default: Option<&'static str>,
    pub(crate) required: bool,
    pub(crate) repeatable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HelpOption {
    pub(crate) names: &'static [&'static str],
    pub(crate) value: Option<&'static str>,
    pub(crate) description: &'static str,
    pub(crate) accepted: &'static [&'static str],
    pub(crate) default: Option<&'static str>,
    pub(crate) conflicts: &'static [&'static str],
    pub(crate) requires: &'static [&'static str],
    pub(crate) repeatable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HelpPage {
    pub(crate) path: &'static [&'static str],
    pub(crate) summary: &'static str,
    pub(crate) description: Option<&'static str>,
    pub(crate) usages: &'static [&'static str],
    pub(crate) arguments: &'static [HelpArgument],
    pub(crate) options: &'static [HelpOption],
    pub(crate) examples: &'static [&'static str],
    pub(crate) tips: &'static [&'static str],
    pub(crate) visibility: HelpVisibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HelpGroup {
    pub(crate) title: &'static str,
    pub(crate) commands: &'static [&'static str],
}
```

Implement `const fn` builders for every field used by the sample. Builders return a modified copy so all catalog pages remain compile-time constants.

Expose the style choice with this exact type:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpStyle { Plain, Ansi }
```

- [ ] **Step 4: Implement the plain renderer and semantic styling pass**

Render at an 88-column maximum. Build plain output first, then style known line classes so ANSI removal cannot change text. Use these style codes:

```rust
const BOLD_CYAN: &str = "\x1b[1;36m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
```

Color section headings with `BOLD_CYAN`, command and option terms with `CYAN`, tips with `YELLOW`, and annotations such as defaults with `DIM`. Do not style description prose.

- [ ] **Step 5: Run the renderer tests and verify green**

Run: `cargo test -p compass-cli help::render::tests --lib -- --nocapture`

Expected: PASS with no warnings.

- [ ] **Step 6: Commit the rendering foundation**

```bash
git add crates/compass-cli/src/help crates/compass-cli/src/lib.rs
git commit -m "feat(cli): add structured help renderer"
```

### Task 2: Add the complete command registry, root help, and path routing

**Files:**

- Create: `crates/compass-cli/src/help/catalog/mod.rs`
- Create: `crates/compass-cli/src/help/catalog/build.rs`
- Create: `crates/compass-cli/src/help/catalog/explore.rs`
- Create: `crates/compass-cli/src/help/catalog/history.rs`
- Create: `crates/compass-cli/src/help/catalog/output.rs`
- Create: `crates/compass-cli/src/help/catalog/integrate.rs`
- Create: `crates/compass-cli/src/help/catalog/support.rs`
- Modify: `crates/compass-cli/src/help/mod.rs`
- Create: `crates/compass-cli/tests/help_cli.rs`
- Modify: `crates/compass-cli/src/lib.rs:145-242,3703-3749`

**Interfaces:**

- Consumes: model and rendering interfaces from Task 1
- Produces: `catalog::pages() -> &'static [HelpPage]`
- Produces: `catalog::groups() -> &'static [HelpGroup]`
- Produces: `request(args: &[String], style: HelpStyle) -> Option<Outcome>`
- Produces: `closest_sibling(parent: &[&str], unknown: &str) -> Option<&'static str>`

- [ ] **Step 1: Write failing registry and routing tests**

Add unit tests that require the exact public root inventory:

```rust
const ROOT_COMMANDS: &[&str] = &[
    "update", "extract", "watch", "cluster-only", "label", "merge-graphs",
    "cache-check", "merge-chunks", "merge-semantic", "query", "path", "explain",
    "affected", "benchmark", "history", "diff", "tree", "export", "serve",
    "global", "clone", "add", "prs", "hook", "install", "uninstall", "provider",
    "save-result", "reflect", "diagnose", "check-update", "merge-driver",
    "hook-check", "hook-guard",
];
```

Assert each name occurs once across the six groups and resolves to a public page. Assert `history-worker`, `hook-spawn`, and `hook-refresh` resolve as internal pages but never occur in a group. Assert these routes return the same text:

```rust
request(&s(&["history", "build", "--help"]), HelpStyle::Plain)
request(&s(&["history", "build", "-h"]), HelpStyle::Plain)
request(&s(&["help", "history", "build"]), HelpStyle::Plain)
```

In `tests/help_cli.rs`, assert root help contains all six headings and summaries for `update`, `query`, `history`, `export`, `serve`, and `diagnose`. Require root options for `-h, --help` and `-V, --version`. Require every command page to end its option section with `-h, --help` without repeating that option in page metadata.

- [ ] **Step 2: Run the registry tests and verify the red state**

Run: `cargo test -p compass-cli help -- --nocapture`

Expected: FAIL because the catalog and request router do not exist.

- [ ] **Step 3: Add root groups and one base page for every public command**

Use these exact groups:

```rust
pub(crate) const GROUPS: &[HelpGroup] = &[
    HelpGroup::new("Build and maintain", &[
        "update", "extract", "watch", "cluster-only", "label", "merge-graphs",
        "cache-check", "merge-chunks", "merge-semantic",
    ]),
    HelpGroup::new("Explore", &["query", "path", "explain", "affected", "benchmark"]),
    HelpGroup::new("History", &["history", "diff"]),
    HelpGroup::new("Visualize and export", &["tree", "export"]),
    HelpGroup::new("Integrate and automate", &[
        "serve", "global", "clone", "add", "prs", "hook", "install", "uninstall",
        "provider", "save-result", "reflect",
    ]),
    HelpGroup::new("Diagnose and support", &[
        "diagnose", "check-update", "merge-driver", "hook-check", "hook-guard",
    ]),
];
```

Each base page must contain its approved one-line summary, at least one current usage form, and at least one valid example. Add the nested paths from the design specification and internal pages for `history-worker`, `hook-spawn`, and `hook-refresh`.

- [ ] **Step 4: Implement help request recognition and conservative suggestions**

Recognize only these request shapes:

```rust
[]
["--help"]
["-h"]
["help"]
["help", command_path @ ..]
[command_path @ .., "--help"]
[command_path @ .., "-h"]
```

Resolve the longest registered command path. Map direct platform aliases such as `claude`, `codex`, `gemini`, and `vscode` to the public `install` page. Use case-sensitive Damerau-Levenshtein distance and suggest only when `distance <= max(1, unknown.chars().count() / 3)`.

- [ ] **Step 5: Route Compass library help through the catalog**

At the start of `run`, call `help::request(&args, HelpStyle::Plain)` only for `Frontend::Compass`. Remove `compass_help()` and `compass_command_help()`. Retain every Graphify help function because compatibility calls still use them.

Change the unknown Compass command branch to:

```rust
_ => Outcome::failure(help::unknown_command(&command)),
```

Keep the existing exit code `1` for an unknown command. Update the existing `not-real --help` coverage assertion from success to exit code `2` because an unknown help path is a new routed usage error.

- [ ] **Step 6: Run focused tests and verify green**

Run: `cargo test -p compass-cli help -- --nocapture`

Expected: PASS with no warnings.

- [ ] **Step 7: Commit registry and routing**

```bash
git add crates/compass-cli/src/help crates/compass-cli/src/lib.rs crates/compass-cli/tests/help_cli.rs crates/compass-cli/tests/coverage_paths.rs
git commit -m "feat(cli): route Compass help through command catalog"
```

### Task 3: Complete build and exploration help pages

**Files:**

- Modify: `crates/compass-cli/src/help/catalog/build.rs`
- Modify: `crates/compass-cli/src/help/catalog/explore.rs`
- Modify: `crates/compass-cli/tests/help_cli.rs`

**Interfaces:**

- Consumes: `HelpPage`, `HelpArgument`, and `HelpOption` const builders
- Produces: complete pages for 14 build and exploration commands

- [ ] **Step 1: Add failing table-driven option coverage tests**

Require these option names on the named page:

```rust
const REQUIRED: &[(&[&str], &[&str])] = &[
    (&["update"], &["--out", "--force", "--no-cluster", "--no-viz", "--no-gitignore", "--exclude", "--resolution", "--exclude-hubs"]),
    (&["extract"], &["--code-only", "--cargo", "--google-workspace", "--postgres", "--backend", "--model", "--mode", "--token-budget", "--max-concurrency", "--max-workers", "--api-timeout", "--allow-partial", "--dedup-llm", "--timing", "--out", "--no-cluster", "--force", "--no-viz", "--no-gitignore", "--exclude", "--resolution", "--exclude-hubs", "--global", "--as"]),
    (&["watch"], &["--debounce", "--out", "--no-cluster", "--no-viz", "--no-gitignore", "--exclude", "--poll"]),
    (&["cluster-only"], &["--graph", "--no-viz", "--no-label", "--resolution", "--exclude-hubs", "--min-community-size"]),
    (&["label"], &["--graph", "--backend", "--model", "--missing-only", "--no-viz", "--resolution", "--exclude-hubs", "--max-concurrency", "--batch-size", "--min-community-size", "--timing"]),
    (&["merge-graphs"], &["--out"]),
    (&["cache-check"], &["--root", "--mode", "--deep", "--prompt-file"]),
    (&["merge-chunks"], &["--out"]),
    (&["merge-semantic"], &["--cached", "--new", "--out"]),
    (&["query"], &["--dfs", "--context", "--budget", "--graph", "--at", "--cql", "--file", "--stdin", "--repl", "--param", "--params-file", "--format", "--output", "--timeout-ms", "--max-rows", "--max-path-depth", "--max-expanded-relationships", "--max-memory-bytes"]),
    (&["path"], &["--graph", "--at"]),
    (&["explain"], &["--graph", "--at"]),
    (&["affected"], &["--relation", "--depth", "--graph"]),
];
```

Also assert `query --format` accepts `table`, `json`, and `jsonl`; `extract --mode` accepts only `deep`; and repeatable options mark `--exclude` and `--param` as repeatable.

- [ ] **Step 2: Run the focused test and verify the red state**

Run: `cargo test -p compass-cli --test help_cli build_and_explore_pages_document_options -- --exact --nocapture`

Expected: FAIL on the first missing option.

- [ ] **Step 3: Fill build and semantic-maintenance metadata**

Read the match arms in `command_build_with_validation`, `run_watch`, `command_cluster_only`, `command_label`, `command_merge_graphs`, and `semantic_commands.rs`. Encode each accepted spelling once, identify values with uppercase placeholders, and add current defaults. Include examples for local-only extraction, semantic extraction, PostgreSQL extraction, continuous watch, manual clustering, graph merging, cache inspection, chunk merging, and semantic merge.

- [ ] **Step 4: Fill exploration metadata**

Document natural-language and CompassQL usage separately on `query`. Record source conflicts among inline CQL, `--file`, `--stdin`, and `--repl`; record `--output` requirements; and document the five CompassQL limits with current defaults. Add examples for natural queries, parameters, revision selection, shortest paths, explanations, affected-code traversal, and benchmark execution.

- [ ] **Step 5: Run focused tests and verify green**

Run: `cargo test -p compass-cli --test help_cli build_and_explore_pages_document_options -- --exact --nocapture`

Expected: PASS.

- [ ] **Step 6: Commit build and exploration pages**

```bash
git add crates/compass-cli/src/help/catalog/build.rs crates/compass-cli/src/help/catalog/explore.rs crates/compass-cli/tests/help_cli.rs
git commit -m "feat(cli): document build and exploration commands"
```

### Task 4: Complete history and export help pages

**Files:**

- Modify: `crates/compass-cli/src/help/catalog/history.rs`
- Modify: `crates/compass-cli/src/help/catalog/output.rs`
- Modify: `crates/compass-cli/tests/help_cli.rs`

**Interfaces:**

- Consumes: catalog and renderer from Tasks 1 and 2
- Produces: 10 history child pages, 8 export child pages, `diff`, and `tree`

- [ ] **Step 1: Add failing nested-page coverage tests**

Require these exact child paths:

```rust
const HISTORY: &[&str] = &["enable", "disable", "status", "build", "rebuild", "list", "show", "prefer", "export", "gc"];
const EXPORTS: &[&str] = &["html", "callflow-html", "obsidian", "wiki", "svg", "graphml", "neo4j", "falkordb"];
```

For each child, assert both `compass parent child --help` and `compass help parent child` return exit code `0`, identical plain output, the full usage path, and an `Examples:` section.

Require these option contracts:

```rust
(&["history", "status"], &["--format"])
(&["history", "build"], &["--profile-from", "--format"])
(&["history", "rebuild"], &["--replace-corrupt", "--format"])
(&["history", "list"], &["--format"])
(&["history", "show"], &["--format"])
(&["history", "prefer"], &["--format"])
(&["history", "export"], &["--format", "--output"])
(&["history", "gc"], &["--prune-non-preferred", "--yes", "--format"])
(&["diff"], &["--detailed", "--format", "--topology-only", "--relation", "--path", "--community", "--limit", "--detect-renames", "--rename-threshold"])
(&["tree"], &["--graph", "--output", "--root", "--max-children", "--top-k-edges", "--label"])
```

- [ ] **Step 2: Run the nested tests and verify the red state**

Run: `cargo test -p compass-cli --test help_cli nested_history_and_export_help_is_complete -- --exact --nocapture`

Expected: FAIL on incomplete child metadata.

- [ ] **Step 3: Complete history pages**

Document revision and realization positionals precisely. Record `text|json`, `graph-json|compass-out`, `--yes` requiring `--prune-non-preferred`, `--replace-corrupt`, build-profile options, and `diff` filters. Include a warning that `history export --format compass-out` rejects an existing destination and that destructive garbage collection needs explicit confirmation.

- [ ] **Step 4: Complete tree and export pages**

Create distinct child pages because each format accepts a different option subset. Document Neo4j and FalkorDB credential environment variables without printing credential values. Document callflow defaults: 15 sections, scale `1.0`, 18 nodes, and 24 edges. Document HTML's node limit and Obsidian's output directory.

- [ ] **Step 5: Run the nested tests and verify green**

Run: `cargo test -p compass-cli --test help_cli nested_history_and_export_help_is_complete -- --exact --nocapture`

Expected: PASS.

- [ ] **Step 6: Commit history and export pages**

```bash
git add crates/compass-cli/src/help/catalog/history.rs crates/compass-cli/src/help/catalog/output.rs crates/compass-cli/tests/help_cli.rs
git commit -m "feat(cli): document history and export commands"
```

### Task 5: Complete integration, configuration, and internal help pages

**Files:**

- Modify: `crates/compass-cli/src/help/catalog/integrate.rs`
- Modify: `crates/compass-cli/src/help/catalog/support.rs`
- Modify: `crates/compass-cli/tests/help_cli.rs`

**Interfaces:**

- Consumes: catalog and renderer from Tasks 1 and 2
- Produces: complete remaining public and internal pages

- [ ] **Step 1: Add failing nested and option inventory tests**

Require child pages for `provider add|list|show|remove`, `global add|remove|list|path`, `hook install|uninstall|status`, and `diagnose multigraph`.

Require these important option sets:

```rust
(&["serve"], &["--graph", "--transport", "--host", "--port", "--api-key", "--path", "--json-response", "--stateless", "--session-timeout"])
(&["clone"], &["--branch", "--out"])
(&["add"], &["--author", "--contributor", "--dir"])
(&["prs"], &["--triage", "--worktrees", "--conflicts", "--wrong-base", "--base", "--repo", "--graph"])
(&["install"], &["--project", "--strict", "--platform"])
(&["uninstall"], &["--project", "--purge", "--platform"])
(&["provider", "add"], &["--base-url", "--default-model", "--env-key", "--pricing-input", "--pricing-output"])
(&["save-result"], &["--question", "--answer", "--answer-file", "--type", "--nodes", "--outcome", "--correction", "--memory-dir"])
(&["reflect"], &["--memory-dir", "--out", "--graph", "--analysis", "--labels", "--half-life-days", "--min-corroboration", "--if-stale"])
(&["diagnose", "multigraph"], &["--graph", "--json", "--max-examples", "--directed", "--undirected", "--extract-path"])
```

Assert internal pages have `HelpVisibility::Internal`, include “internal”, and name a public alternative.

- [ ] **Step 2: Run the inventory tests and verify the red state**

Run: `cargo test -p compass-cli --test help_cli integration_and_support_pages_document_options -- --exact --nocapture`

Expected: FAIL on incomplete metadata.

- [ ] **Step 3: Complete integration pages**

Document transport defaults and API key environment fallback for `serve`; global graph operations; GitHub clone forms; ingest attribution; PR views; managed hooks; every install platform; provider pricing units; result outcomes; and reflection defaults. Mark mutually exclusive pairs such as `--answer` with `--answer-file` through structured conflict metadata.

- [ ] **Step 4: Complete support and internal pages**

Document `diagnose multigraph`, `check-update`, and `merge-driver`. Explain that `hook-check`, `hook-guard`, `history-worker`, `hook-spawn`, and `hook-refresh` support installed integrations and are not normal interactive workflows. Keep the last three out of root groups.

- [ ] **Step 5: Add structural completeness validation**

Add a unit test that rejects duplicate paths, missing parents, empty summaries, empty usage forms, public leaf pages without examples, grouped internal pages, public root pages absent from a group, and grouped names without a page. Add an explicit `EXPECTED_PUBLIC_OPTIONS` map in `catalog/mod.rs`; compare each page's flattened option names to that map so future parser additions require a help update in the same change.

- [ ] **Step 6: Run integration and structural tests and verify green**

Run: `cargo test -p compass-cli help -- --nocapture`

Expected: PASS with no warnings.

- [ ] **Step 7: Commit remaining catalog pages**

```bash
git add crates/compass-cli/src/help/catalog crates/compass-cli/tests/help_cli.rs
git commit -m "feat(cli): document integration and support commands"
```

### Task 6: Enable terminal styling and focused usage recovery

**Files:**

- Modify: `crates/compass-cli/src/help/mod.rs`
- Modify: `crates/compass-cli/src/bin/compass.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/tests/help_cli.rs`

**Interfaces:**

- Produces: `HelpStyle::detect(is_terminal: bool, no_color: Option<&OsStr>, term: Option<&OsStr>) -> HelpStyle`
- Produces: `pub fn compass_help_request(arguments: &[OsString], style: HelpStyle) -> Option<Outcome>`
- Produces: `append_usage_hint(outcome: Outcome, path: &[&str]) -> Outcome`

- [ ] **Step 1: Add failing style-policy and error-recovery tests**

Use this exact table:

```rust
assert_eq!(HelpStyle::detect(false, None, None), HelpStyle::Plain);
assert_eq!(HelpStyle::detect(true, Some(OsStr::new("")), None), HelpStyle::Plain);
assert_eq!(HelpStyle::detect(true, None, Some(OsStr::new("dumb"))), HelpStyle::Plain);
assert_eq!(HelpStyle::detect(true, None, Some(OsStr::new("DUMB"))), HelpStyle::Plain);
assert_eq!(HelpStyle::detect(true, None, Some(OsStr::new("xterm-256color"))), HelpStyle::Ansi);
```

Assert `compass udpate` retains exit code `1`, suggests `update`, and points to `compass --help`. Assert a distant unknown command has no suggestion. Assert a history usage error with code `2` ends with `Run `compass history build --help` for usage.` and a runtime history failure with code `1` does not.

- [ ] **Step 2: Run focused tests and verify the red state**

Run: `cargo test -p compass-cli --test help_cli styling_and_usage_recovery_are_conservative -- --exact --nocapture`

Expected: FAIL because style detection and usage hints are not connected.

- [ ] **Step 3: Detect style before streaming dispatch**

Update `compass.rs` to import `std::io::IsTerminal`. Before the `diff`, `watch`, and `serve` branches, calculate:

```rust
let style = compass_cli::HelpStyle::detect(
    io::stdout().is_terminal(),
    std::env::var_os("NO_COLOR").as_deref(),
    std::env::var_os("TERM").as_deref(),
);
if let Some(outcome) = compass_cli::compass_help_request(&arguments, style) {
    return ExitCode::from(compass_cli::write_outcome(
        &outcome,
        &mut io::stdout(),
        &mut io::stderr(),
    ));
}
```

The early check ensures streaming commands receive the same styled help as ordinary commands.

- [ ] **Step 4: Append help hints only to usage outcomes**

After Compass dispatch, append a hint only when `Outcome.code == 2`, `stderr` is non-empty, and it does not already contain a help instruction. Resolve the longest valid command path from the supplied arguments. Do not modify code `1`, `3`, or `4` failures.

- [ ] **Step 5: Run focused tests and verify green**

Run: `cargo test -p compass-cli --test help_cli styling_and_usage_recovery_are_conservative -- --exact --nocapture`

Expected: PASS.

- [ ] **Step 6: Commit terminal behavior**

```bash
git add crates/compass-cli/src/help/mod.rs crates/compass-cli/src/bin/compass.rs crates/compass-cli/src/lib.rs crates/compass-cli/tests/help_cli.rs
git commit -m "feat(cli): add terminal styling and help recovery"
```

### Task 7: Preserve compatibility, update guidance, and verify the workspace

**Files:**

- Modify: `crates/compass-cli/tests/help_cli.rs`
- Modify: `README.md`
- Refresh: `/Users/haipingfu/graphify/graphify-out/`

**Interfaces:**

- Consumes: completed help system
- Produces: compatibility evidence, user guidance, and refreshed repository graph

- [ ] **Step 1: Add the failing Graphify byte-regression test**

Add this integration assertion before changing documentation:

```rust
#[test]
fn graphify_help_asset_remains_byte_for_byte_unchanged() {
    let outcome = invoke(Frontend::Graphify, &["--help"]);
    assert_eq!(outcome.code, 0);
    assert_eq!(outcome.stdout, include_str!("../assets/graphify-help.txt"));
}
```

Temporarily change the expected value by one byte, run the test to prove it fails, then restore the exact asset comparison.

- [ ] **Step 2: Run the compatibility test and verify green after restoration**

Run: `cargo test -p compass-cli --test help_cli graphify_help_asset_remains_byte_for_byte_unchanged -- --exact --nocapture`

Expected: PASS.

- [ ] **Step 3: Update README command discovery guidance**

Below “Current native command surface,” add:

```markdown
Run `compass --help` for commands grouped by workflow. Run
`compass help <command>` or `compass <command> --help` for arguments,
options, defaults, examples, and related-command tips. Nested help accepts
the full path, such as `compass help history build`.
```

- [ ] **Step 4: Format and run the complete CLI suite**

Run: `cargo fmt --all -- --check`

Expected: exit `0` with no output.

Run: `cargo test -p compass-cli --all-targets`

Expected: all `compass-cli` unit and integration tests pass.

- [ ] **Step 5: Run full workspace checks**

Run: `cargo test --workspace --all-targets`

Expected: all workspace tests pass.

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: exit `0` with no warnings.

- [ ] **Step 6: Inspect representative output**

Run:

```bash
target/debug/compass --help
target/debug/compass update --help
target/debug/compass help query
target/debug/compass history build --help
target/debug/compass export neo4j --help
NO_COLOR=1 target/debug/compass --help
target/debug/compass udpate
```

Expected: grouped root help, detailed option descriptions and examples, nested pages, no ANSI in the `NO_COLOR` run, and a conservative `update` suggestion.

- [ ] **Step 7: Refresh the Graphify knowledge graph**

Run from `/Users/haipingfu/graphify`:

```bash
graphify update .
```

Expected: exit `0` and `graphify-out/GRAPH_REPORT.md` reports a current graph.

- [ ] **Step 8: Review the final diff and commit**

Run: `git diff --check`

Expected: no output.

Run: `git status --short`

Expected: only the README, help tests, and any formatter changes from this task remain uncommitted.

```bash
git add README.md crates/compass-cli/tests/help_cli.rs
git commit -m "docs(cli): explain Compass help discovery"
```
