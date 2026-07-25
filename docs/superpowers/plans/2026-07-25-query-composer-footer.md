# Query Composer Footer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Place Query Codebase actions in a unified bottom composer footer and align CompassQL parameters from the left.

**Architecture:** Keep query state and host messaging unchanged. Restructure only `QueryWorkspace` presentation so the textarea and footer share one semantic container, then adapt that container with VS Code-token CSS for wide and narrow editor columns.

**Tech Stack:** TypeScript 5.9, React 19, VS Code webview CSS variables, Vite 7, Vitest 3, and Playwright.

## Global Constraints

- Implement the layout before adding regression coverage; do not use TDD.
- Preserve the existing query request, cancellation, result, and source-navigation behavior.
- Use only VS Code semantic tokens for ordinary chrome.
- Keep the parameter input left-aligned.
- Keep Run or Cancel in the composer footer.
- Preserve every core action at 320 CSS pixels.
- Preserve unrelated uncommitted workspace changes.
- Run `graphify update .` after code changes.

---

### Task 1: Build the unified query composer

**Files:**

- Modify: `packages/compass-viewer/src/query/QueryWorkspace.tsx`
- Modify: `packages/compass-viewer/src/theme.css`

**Interfaces:**

- Consumes: existing `QueryWorkspace`, `QueryHost`, `QueryRequest`, `mode`, `params`, `running`, and `execute`
- Produces: `.query-editor-shell`, `.query-composer-footer`, `.query-footer-actions`, and the existing `.query-run`

- [ ] **Step 1: Restructure the composer markup**

Inside `.query-composer`, wrap the textarea and footer in one
`.query-editor-shell`. Move `.query-run` into `.query-composer-footer`. In
CompassQL mode, move `.query-params` into the left side of that footer. Render
`.query-footer-actions` on the right with `.query-shortcut` followed by Run or
Cancel.

The resulting structure is:

```tsx
<div className="query-composer">
  <div className="query-editor-shell">
    <div className="query-editor">
      <textarea />
    </div>
    <div className="query-composer-footer">
      {mode === "cql" && (
        <label className="query-params">
          <span>Parameters</span>
          <input aria-label="CompassQL parameters" />
        </label>
      )}
      <div className="query-footer-actions">
        <span className="query-shortcut">⌘ Enter</span>
        <button className="query-run">Run</button>
      </div>
    </div>
  </div>
</div>
```

Keep `.query-examples` after `.query-composer` in natural-language mode.

- [ ] **Step 2: Style the unified shell**

Give `.query-editor-shell` the input border, radius, background, and
`focus-within` outline. Remove the textarea border and absolute shortcut
positioning. Add a semantic top border to `.query-composer-footer`. Keep the
footer compact, with parameters flexing left and `.query-footer-actions`
aligned right.

- [ ] **Step 3: Add narrow-column behavior**

Within the existing `@media (max-width: 760px)` block:

```css
.query-composer-footer {
  flex-wrap: wrap;
}

.query-params {
  flex: 1 0 100%;
}

.query-footer-actions {
  width: 100%;
  justify-content: flex-end;
}
```

Allow the parameter label and input to stack below 420 CSS pixels if their
combined intrinsic width would overflow.

- [ ] **Step 4: Verify types and production CSS**

Run:

```bash
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
```

Expected: both commands pass.

---

### Task 2: Add post-implementation regression coverage

**Files:**

- Modify: `tests/viewer/query.spec.ts`
- Modify: `tests/viewer/theme.spec.ts`

**Interfaces:**

- Consumes: compiled Query fixture and the new composer class names
- Produces: wide, narrow, parameter-alignment, and action-placement regression coverage

- [ ] **Step 1: Cover wide CompassQL placement**

Extend `query.spec.ts` after implementation with a test that switches to
CompassQL and compares bounding boxes:

```ts
const shell = await page.locator(".query-editor-shell").boundingBox();
const params = await page.getByRole("textbox", {
  name: "CompassQL parameters"
}).boundingBox();
const run = await page.getByRole("button", { name: "Run query" }).boundingBox();

expect(params!.x).toBeLessThan(run!.x);
expect(run!.y).toBeGreaterThan(shell!.y + shell!.height / 2);
```

- [ ] **Step 2: Cover narrow placement**

At a 320 by 720 viewport, verify that the parameter input and Run button are
visible, the Run button remains below the textarea, and the document has no
horizontal overflow.

- [ ] **Step 3: Cover token-driven focus**

Extend `theme.spec.ts` to focus the query textarea and assert that
`.query-editor-shell` uses the injected VS Code focus border.

- [ ] **Step 4: Run focused browser coverage**

From `tests/viewer` run:

```bash
npx playwright test query.spec.ts theme.spec.ts
```

Expected: all tests pass.

---

### Task 3: Complete verification and graph refresh

**Files:**

- Generated by existing scripts only: `crates/compass-output/assets/viewer/*`

**Interfaces:**

- Consumes: completed composer source and tests
- Produces: verified viewer and extension artifacts

- [ ] **Step 1: Run unit suites and type checks**

Run:

```bash
npm test -w @compass/viewer
npm test -w editors/vscode
npm run typecheck -w @compass/viewer
npm run typecheck -w editors/vscode
```

Expected: all commands pass.

- [ ] **Step 2: Run production builds**

Run:

```bash
npm run build -w @compass/viewer
npm run build -w editors/vscode
```

Expected: both builds pass.

- [ ] **Step 3: Refresh the code graph**

From `/Users/haipingfu/graphify` run:

```bash
graphify update .
```

Expected: the graph update completes successfully.

- [ ] **Step 4: Inspect the final diff**

Confirm that the composer changes are limited to the approved layout and that
all unrelated pre-existing modifications remain intact and unstaged.

