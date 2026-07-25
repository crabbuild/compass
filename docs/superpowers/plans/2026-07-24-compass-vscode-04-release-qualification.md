# Compass VS Code Release Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Qualify, document, package, and smoke-test the complete Compass VS Code v1 across supported platforms, themes, accessibility modes, large graphs, multi-root workspaces, and remote extension hosts.

**Architecture:** Add layered automated gates around the Rust contracts, shared browser viewer, extension host, real VS Code, and packaged VSIX. Release automation builds viewer assets before Rust packaging, verifies deterministic manifests, and publishes no extension until every prior milestone passes.

**Tech Stack:** Cargo, npm, Vitest, Playwright, `@vscode/test-electron`, `@vscode/vsce`, axe-core, GitHub Actions, VSIX

## Global Constraints

- Plans 01, 02, and 03 are mandatory prerequisites.
- Version 1 is incomplete until all qualification tasks pass.
- Test trusted/untrusted, light/dark/high-contrast, reduced motion, multi-root, large graph, cancellation, corruption, and remote-host assumptions.
- No test may require production credentials or network access at viewer runtime.
- VSIX contains no native Compass binary and declares workspace extension kind.
- Generated viewer assets must match their checked manifest.
- Marketplace artwork uses the Lucide Compass mark and includes license attribution.
- Run `graphify update .` after code changes.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `tests/viewer/*` | Real-browser export/viewer interaction, CSP, accessibility, and performance. |
| `editors/vscode/src/test/suite/*` | Real VS Code extension-host integration tests. |
| `editors/vscode/scripts/smoke-vsix.mjs` | Install and activate the packaged extension. |
| `scripts/check_viewer_assets.mjs` | Deterministic viewer asset and no-network verification. |
| `.github/workflows/compass-ci.yml` | JavaScript, browser, extension, and packaging gates. |
| `.github/workflows/compass-vscode-release.yml` | Explicit Marketplace-ready VSIX build artifact. |
| `docs/guides/vscode.md` | User setup and workflow guide. |

### Task 1: Add real-browser parity, CSP, and accessibility gates

**Files:**
- Create: `tests/viewer/package.json`
- Create: `tests/viewer/playwright.config.ts`
- Create: `tests/viewer/graph-parity.spec.ts`
- Create: `tests/viewer/history.spec.ts`
- Create: `tests/viewer/callflow.spec.ts`
- Create: `tests/viewer/accessibility.spec.ts`
- Create: `tests/viewer/fixtures/generate.ts`
- Modify: `package-lock.json`

**Interfaces:**
- Consumes: generated current graph HTML, history HTML, call-flow HTML, and shared viewer fixtures.
- Produces: Chromium/Firefox/WebKit acceptance evidence.

- [ ] **Step 1: Install and configure browser tests**

Create `tests/viewer/package.json` with name `@compass/viewer-tests`,
`private: true`, and scripts `test: playwright test` and
`test:performance: playwright test performance.spec.ts`.

Run:

```bash
npm install --save-dev -w tests/viewer @playwright/test @axe-core/playwright
npx playwright install
```

Serve generated fixtures from a loopback server with network routing blocked for every other origin. Also open offline exports through `file://`.

- [ ] **Step 2: Test shared graph behavior**

```ts
test("export and hosted viewer share focus semantics", async ({ page }) => {
  await page.goto(fixtureUrl("graph.html"));
  await page.getByRole("searchbox").fill("helper");
  await page.getByRole("option", { name: /helper/i }).click();
  await expect(page.getByRole("status")).toContainText("Inspecting helper");
  await expect(page.locator('[data-node-id="unrelated"]')).toHaveAttribute("data-dimmed", "true");
  await page.getByRole("button", { name: "Resume layout" }).click();
  await page.getByRole("button", { name: /caller run/i }).click();
  await expect(page.getByRole("button", { name: "Resume layout" })).toBeVisible();
});
```

- [ ] **Step 3: Test CSP, no-network, themes, and accessibility**

Fail on any request outside the fixture origin. Run axe against graph, call graph, architecture, query, and history states. Exercise keyboard-only navigation, reduced motion, 320 CSS pixel layout, light/dark/high-contrast token fixtures, and corrupt history payload isolation.

- [ ] **Step 4: Verify and commit**

Run: `npm run test -w @compass/viewer-tests`

Expected: all three browser engines pass with zero serious/critical axe violations.

Run: `git add tests/viewer package-lock.json && git commit -m "test(viewer): qualify shared Compass interactions"`

### Task 2: Add real VS Code extension-host integration tests

**Files:**
- Create: `editors/vscode/src/test/runTest.ts`
- Create: `editors/vscode/src/test/suite/index.ts`
- Create: `editors/vscode/src/test/suite/setup.test.ts`
- Create: `editors/vscode/src/test/suite/graph.test.ts`
- Create: `editors/vscode/src/test/suite/calls.test.ts`
- Create: `editors/vscode/src/test/suite/history.test.ts`
- Create: `editors/vscode/src/test/suite/multiroot.test.ts`
- Create: `editors/vscode/src/test/fixtures/fake-compass.mjs`
- Modify: `editors/vscode/package.json`
- Modify: `package-lock.json`

**Interfaces:**
- Consumes: packaged extension code and deterministic fake/real fixture CLIs.
- Produces: extension-host acceptance suite.

- [ ] **Step 1: Install the VS Code test harness**

Run:

```bash
npm install --save-dev -w editors/vscode @vscode/test-electron mocha @types/mocha
```

- [ ] **Step 2: Add deterministic fake CLI scenarios**

The fake CLI implements capabilities, JSONL progress, viewer-json, call-graph, callflow-json, query, timeline, history export/build, and diff. Select scenarios through a fixture file, not environment code injection. Record all received argument arrays for assertions.

- [ ] **Step 3: Test critical workflows**

Assert:

- missing/incompatible CLI setup;
- untrusted workspace command disablement;
- init/update/watch start-stop and cancellation;
- current graph hydration and source reveal;
- UTF-8 cursor call graph and lazy expansion;
- architecture and query results;
- all-commit timeline without implicit build;
- explicit history build, exact revision load, parent comparison, and failure isolation;
- multi-root repository targeting and tab identity; and
- disconnected process/remote-like restart behavior.

Run the same packaged workspace extension in Remote SSH, WSL, and Dev
Container smoke environments (or their official CI harnesses) and assert that
CLI discovery, process working directory, public artifacts, and source opening
all occur on the remote extension host rather than the local UI host.

- [ ] **Step 4: Verify and commit**

Run: `npm run test:integration -w @compass/vscode`

Expected: a clean VS Code profile completes every workflow.

Run: `git add editors/vscode package-lock.json && git commit -m "test(vscode): cover Compass editor workflows"`

### Task 3: Add performance and resource gates

**Files:**
- Create: `tests/viewer/performance.spec.ts`
- Create: `editors/vscode/src/test/performance/processManager.perf.test.ts`
- Create: `scripts/generate_viewer_benchmarks.mjs`
- Modify: `editors/vscode/package.json`
- Modify: `tests/viewer/package.json`

**Interfaces:**
- Consumes: small, medium, and large generated graphs/timelines.
- Produces: repeatable budgets and diagnostic artifacts.

- [ ] **Step 1: Generate deterministic benchmark fixtures**

Generate 500, 5,000, and 25,000-node graphs with seeded communities; 10,000-commit timelines with merges; and call expansions of 100/1,000 nodes. Store generators, not huge fixtures.

- [ ] **Step 2: Enforce interaction budgets**

On the pinned CI runner profile:

- extension activation without an opened Compass view: under 150 ms median;
- 500-node first useful render: under 1 s;
- 5,000-node overview first useful render: under 2.5 s;
- 10,000-commit timeline scroll: no long task above 100 ms after warm-up;
- lazy 100-node call expansion merge: under 100 ms;
- cancellation acknowledgement: under 500 ms; and
- decoded historical cache: never above three entries.

Record traces on failure and use five measured runs after one warm-up. Compare medians, not one noisy sample.

- [ ] **Step 3: Verify and commit**

Run:

```bash
npm run test:performance -w @compass/viewer-tests
npm run test:performance -w @compass/vscode
```

Run: `git add tests/viewer editors/vscode scripts/generate_viewer_benchmarks.mjs && git commit -m "test: enforce Compass viewer performance budgets"`

### Task 4: Integrate JavaScript, browser, and VSIX gates into CI

**Files:**
- Create: `scripts/check_viewer_assets.mjs`
- Modify: `Makefile`
- Modify: `.github/workflows/compass-ci.yml`
- Create: `.github/workflows/compass-vscode-release.yml`
- Modify: `scripts/package_macos.sh`
- Modify: `scripts/test_release_scripts.sh`

**Interfaces:**
- Produces: deterministic asset check, `make test-js`, `make test-vscode`, and VSIX artifact workflow.
- Consumes: all prior test commands.

- [ ] **Step 1: Add deterministic asset verification**

`check_viewer_assets.mjs` rebuilds assets in a temporary directory, compares SHA-256 and filenames with `crates/compass-output/assets/viewer/manifest.json`, scans source maps/bundles for remote URLs, and fails on differences without overwriting the checkout.

- [ ] **Step 2: Extend CI**

Add pinned Node setup, `npm ci`, typecheck, unit tests, viewer asset verification, Playwright Chromium on normal CI, full browser matrix on scheduled hardening, extension-host integration on Linux, and VSIX package smoke tests on Linux/macOS/Windows.

- [ ] **Step 3: Protect Cargo packaging**

Make Rust crate/package workflows run the viewer asset verifier before `cargo package`. Ensure packaged `compass-output` includes the asset manifest and bundles while the VSIX excludes Rust targets, history stores, fixture outputs, and any native Compass binary.

- [ ] **Step 4: Add an explicit VSIX release workflow**

The workflow accepts a version and confirmation, verifies the extension manifest version, runs all required gates, creates the VSIX, emits SHA-256 and provenance attestations, and uploads an artifact. Marketplace publication remains a separately authorized environment action.

- [ ] **Step 5: Verify and commit**

Run:

```bash
node scripts/check_viewer_assets.mjs
make test-js
make test-vscode
sh scripts/test_release_scripts.sh
npm run package -w @compass/vscode
```

Run: `git add scripts Makefile .github editors/vscode && git commit -m "ci: qualify Compass VS Code releases"`

### Task 5: Complete branding, packaging, and Marketplace metadata

**Files:**
- Create: `editors/vscode/media/icon.svg`
- Create: `editors/vscode/media/icon.png`
- Create: `editors/vscode/media/compass-dark.svg`
- Create: `editors/vscode/media/compass-light.svg`
- Create: `editors/vscode/README.md`
- Create: `editors/vscode/CHANGELOG.md`
- Create: `editors/vscode/SECURITY.md`
- Create: `editors/vscode/.vscodeignore`
- Modify: `editors/vscode/package.json`
- Modify: `THIRD_PARTY_NOTICES.md`

**Interfaces:**
- Produces: Marketplace-ready extension identity and minimal VSIX.

- [ ] **Step 1: Generate licensed Compass artwork**

Use Lucide's Compass geometry as the source, retain the ISC attribution, render a 128×128 PNG for Marketplace, and use monochrome theme-aware SVGs for VS Code surfaces. Verify legibility at 16, 24, 32, and 128 pixels.

- [ ] **Step 2: Finalize the manifest**

Set display name `Compass`, categories `Visualization`, `Programming Languages`, and `Other`; declare `extensionKind: ["workspace"]`; add commands, menus, views, walkthrough, configuration, repository/license links, and `capabilities.untrustedWorkspaces.supported: false`.

- [ ] **Step 3: Enforce package contents**

`.vscodeignore` excludes source tests, traces, fixture repos, maps, and development configs while retaining compiled host/webview bundles, icons, README, changelog, license, and notices. Assert the VSIX contains no `compass`/`compass.exe` binary.

- [ ] **Step 4: Package and commit**

Run:

```bash
npm run package -w @compass/vscode
npm run smoke:vsix -w @compass/vscode
unzip -l editors/vscode/*.vsix
```

Run: `git add editors/vscode THIRD_PARTY_NOTICES.md && git commit -m "build(vscode): package the Compass extension"`

### Task 6: Publish complete user and contributor documentation

**Files:**
- Create: `docs/guides/vscode.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/outputs.md`
- Modify: `docs/implementation/workspace-tour.md`
- Modify: `README.md`
- Modify: `CONTRIBUTING.md`

**Interfaces:**
- Documents: installation, setup, current graph, calls, architecture, query, history, diff, trust, remote hosts, troubleshooting, schemas, asset builds, and release gates.

- [ ] **Step 1: Write task-oriented user documentation**

Include exact command/command-palette names, required CLI version behavior, binary path selection, repository selection, source navigation limits, history materialization costs, partial call coverage, private storage, and no-telemetry policy.

- [ ] **Step 2: Document machine contracts**

Add capabilities, graph viewer, Program call graph, call-flow, timeline, and progress schemas to command/output references with unknown-major behavior and examples.

- [ ] **Step 3: Document contributor workflow**

Explain npm workspace setup, shared viewer boundaries, deterministic asset generation, extension-host tests, browser tests, VSIX packaging, Lucide attribution, and the rule against reading history SQLite from TypeScript.

- [ ] **Step 4: Run documentation and full release gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
npm run package -w @compass/vscode
npm run smoke:vsix -w @compass/vscode
graphify update .
```

- [ ] **Step 5: Commit the qualified v1 documentation**

Run:

```bash
git add README.md CONTRIBUTING.md docs editors/vscode/README.md editors/vscode/CHANGELOG.md
git commit -m "docs: publish the Compass VS Code guide"
```

The complete v1 is ready only when Tasks 1–6 pass on the required CI matrix and all acceptance criteria in `docs/superpowers/specs/2026-07-24-compass-vscode-extension-design.md` are checked.
