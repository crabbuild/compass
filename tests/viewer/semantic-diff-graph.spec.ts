import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/semanticDiffGraph.html");
});

test("renders a bounded collision-free capsule graph safely", async ({ page }) => {
  const nodes = page.locator("#graph-canvas [data-node-id]");
  await expect(nodes).toHaveCount(42);
  await expect(page.locator('[data-node-id="changed-core"] .graph-node-label')).toBeVisible();
  await expect(page.locator('[data-node-id="added-leaf"] .graph-node-label')).toBeVisible();
  await expect(page.locator('[data-node-id="removed-caller"] .graph-node-label')).toBeVisible();
  await expect(page.locator("#graph-canvas img")).toHaveCount(0);
  await expect(page.locator("body img")).toHaveCount(0);
  await expect(page.locator("#graph-note")).toContainText("42 of");

  const boxes = await nodes.evaluateAll((elements) =>
    elements.map((element) => {
      const box = element.getBoundingClientRect();
      return {
        left: box.left,
        right: box.right,
        top: box.top,
        bottom: box.bottom
      };
    })
  );
  for (let left = 0; left < boxes.length; left += 1) {
    for (let right = left + 1; right < boxes.length; right += 1) {
      const overlaps = boxes[left].left < boxes[right].right
        && boxes[left].right > boxes[right].left
        && boxes[left].top < boxes[right].bottom
        && boxes[left].bottom > boxes[right].top;
      expect(overlaps).toBe(false);
    }
  }
});

test("focuses the selected neighborhood and navigates inspector relationships", async ({ page }) => {
  const changed = page.locator('[data-node-id="changed-core"]');
  await changed.click();

  await expect(changed).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-node-id="added-leaf"]')).toHaveClass(/is-neighbor/);
  await expect(page.locator('[data-node-id="unrelated"]')).toHaveClass(/is-dimmed/);
  await expect(page.getByRole("heading", { name: "changed-core" })).toBeVisible();
  await expect(page.locator("#graph-inspector")).toContainText("implementation");
  await expect(page.locator('#graph-inspector a[href="#source-change-0"]')).toBeVisible();
  await expect(page.locator('#graph-inspector a[href="#sd1-fixture"]')).toBeVisible();

  await page.locator('[data-neighbor-id="context-api"]').click();
  await expect(page.locator("#graph-live")).toContainText("Inspecting context-api");
  await expect(page.getByRole("heading", { name: "context-api" })).toBeVisible();
  await expect(page.locator("#graph-inspector dt")).toHaveCount(0);

  await page.locator('[data-neighbor-id="changed-core"]').click();
  await page.locator('[data-neighbor-id="zz-outside-sample"]').click();
  await expect(page.locator("#graph-live")).toContainText("Inspecting zz-outside-sample");
  await expect(page.locator("#graph-note")).toContainText("outside the bounded visual sample");

  await page.locator("#graph-canvas svg").click({ position: { x: 4, y: 4 } });
  await expect(changed).toHaveAttribute("aria-pressed", "false");
});

test("supports keyboard selection, clearing, and narrow inspector layout", async ({ page }) => {
  const changed = page.locator('[data-node-id="changed-core"]');
  await changed.focus();
  await page.keyboard.press("Enter");
  await expect(changed).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await expect(changed).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("#graph-live")).toContainText("Graph selection cleared");

  await page.setViewportSize({ width: 720, height: 900 });
  const canvas = await page.locator("#graph-canvas").boundingBox();
  const inspector = await page.locator("#graph-inspector").boundingBox();
  expect(canvas).not.toBeNull();
  expect(inspector).not.toBeNull();
  expect(inspector!.y).toBeGreaterThanOrEqual(canvas!.y + canvas!.height - 1);
});

test("falls back safely when graph data is unavailable", async ({ page }) => {
  await page.evaluate(() => {
    const runtime = globalThis as typeof globalThis & {
      graphFixture: { destroy(): void };
      CompassSemanticDiffGraph: {
        mount(options: Record<string, unknown>): unknown;
      };
    };
    runtime.graphFixture.destroy();
    const data = document.getElementById("semantic-diff-data")?.textContent || "{}";
    const report = JSON.parse(data);
    delete report.graph_delta;
    runtime.CompassSemanticDiffGraph.mount({
      report,
      host: document.getElementById("graph-canvas"),
      inspector: document.getElementById("graph-inspector"),
      liveRegion: document.getElementById("graph-live"),
      note: document.getElementById("graph-note")
    });
  });

  await expect(page.locator("#graph-canvas")).toContainText("Interactive graph unavailable.");
  await expect(page.locator("#graph-note")).toContainText("exhaustive graph-change lists");
  await expect(page.locator("#exhaustive-list")).toContainText("changed-core");
});

test("rejects unknown semantic report versions without rendering guessed topology", async ({ page }) => {
  await page.evaluate(() => {
    const runtime = globalThis as typeof globalThis & {
      graphFixture: { destroy(): void };
      CompassSemanticDiffGraph: {
        mount(options: Record<string, unknown>): unknown;
      };
    };
    runtime.graphFixture.destroy();
    const data = document.getElementById("semantic-diff-data")?.textContent || "{}";
    const report = JSON.parse(data);
    report.schema = "compass.semantic_diff.report/2";
    runtime.CompassSemanticDiffGraph.mount({
      report,
      host: document.getElementById("graph-canvas"),
      inspector: document.getElementById("graph-inspector"),
      liveRegion: document.getElementById("graph-live"),
      note: document.getElementById("graph-note")
    });
  });

  await expect(page.locator("#graph-canvas")).toContainText("Interactive graph unavailable.");
  await expect(page.locator("#graph-note")).toContainText("embedded report data");
  await expect(page.locator("#graph-canvas [data-node-id]")).toHaveCount(0);
});
