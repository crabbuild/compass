import { expect, test } from "@playwright/test";

/** The fixture mirrors a standalone export of a repository too large to embed. */
test("the control rail shares the workbench header instead of the canvas", async ({ page }) => {
  await page.goto("/exportWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const header = page.locator(".visualization-context");
  const rail = header.locator(".compass-graph-toolbar");
  await expect(rail).toBeVisible();
  await expect(page.locator(".compass-graph-stage .compass-graph-toolbar")).toHaveCount(0);
  await expect(page.locator(".compass-graph-stage")).toHaveAttribute("data-controls", "header");

  // Every control stays inside the header row: the trailing buttons are not
  // scrolled out of reach.
  const headerBox = await header.boundingBox();
  const filtersBox = await header.locator(".workbench-filter-trigger").boundingBox();
  const settingsBox = await header.getByRole("button", { name: "Graph settings" }).boundingBox();
  if (!headerBox || !filtersBox || !settingsBox) throw new Error("header controls are missing");
  expect(filtersBox.x).toBeGreaterThanOrEqual(headerBox.x);
  expect(settingsBox.x + settingsBox.width).toBeLessThanOrEqual(
    headerBox.x + headerBox.width + 1
  );
});

test("every community opens, and a bounded window says so", async ({ page }) => {
  await page.goto("/exportWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const communities = page.locator(".compass-community-panel");
  await expect(communities).toHaveAttribute("data-collapsed", "false");
  await expect(page.locator(".compass-community-item")).toHaveCount(2);
  await expect(page.locator(".compass-info-content .compass-empty")).toHaveCount(0);

  // Core holds 1,200 symbols and the export could embed two of them.
  await page.getByRole("button", { name: "Open group Core" }).click();
  await expect(page.locator(".compass-bounded-notice")).toContainText(
    "Bounded community detail"
  );
  await expect(page.locator(".compass-bounded-notice"))
    .toContainText("2 most connected of 1,200 symbols");
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("Community Core · 2 symbols");

  // The community list gives the inspector the whole column while one is open.
  await expect(communities).toHaveCount(0);
  await expect(page.getByText("Select all")).toHaveCount(0);

  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(communities).toHaveCount(1);
  await expect(page.getByText("Select all")).toBeVisible();

  // Data is complete in the export, so it opens without a bound.
  await page.getByRole("button", { name: "Open group Data" }).click();
  await expect(page.locator(".compass-bounded-notice")).toHaveCount(0);
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("Community Data · 2 symbols");
});

/** The single-model export (`compass update`'s graph.html) discloses bounds too. */
test("a single-model export names its bounded community window", async ({ page }) => {
  await page.goto("/exportCommunity.html");
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.waitFor();
  await search.fill("Core");
  await page.getByRole("option", { name: /Core/i }).click();
  await page.waitForTimeout(300);
  await page.locator("canvas").dblclick();

  await expect(page.locator(".compass-bounded-notice"))
    .toContainText("2 most connected of 3 symbols");
  await expect(page.locator(".compass-bounded-notice"))
    .toContainText("compass export json --community 0");
  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(page.locator(".compass-bounded-notice")).toHaveCount(0);
});

for (const width of [1440, 768, 390]) {
  test(`filters stay anchored and reachable at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/exportWorkbench.html");
    await page.getByRole("button", { name: "Open group Core" }).click();
    const back = page.getByRole("button", { name: "Back to community overview" });
    await expect(back).toBeVisible();
    expect(await back.evaluate((element) => element.parentElement?.firstElementChild === element)).toBe(true);
    const trigger = page.getByRole("button", { name: "Graph filters", exact: true });
    await trigger.click();
    const panel = page.getByRole("region", { name: "Graph filter options" });
    await expect(panel).toBeVisible();
    for (const viewportWidth of [width, width + 80]) {
      await page.setViewportSize({ width: viewportWidth, height: 900 });
      await expect.poll(async () => {
        const anchor = await trigger.boundingBox();
        const box = await panel.boundingBox();
        if (!anchor || !box) return false;
        const expectedLeft = Math.max(12, Math.min(anchor.x, viewportWidth - box.width - 12));
        return Math.abs(box.x - expectedLeft) < 2
          && Math.abs(box.y - anchor.y - anchor.height - 8) < 2
          && box.x + box.width <= viewportWidth - 10;
      }).toBe(true);
    }
    await panel.getByRole("combobox", { name: /^Relationship/ }).selectOption("calls");
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await expect(back).toBeVisible();
  });
}

for (const width of [320, 390, 558, 768, 1440]) {
  test(`the graph toolbar uses the available width at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/hierarchyWorkbench.html");
    await page.getByRole("button", { name: "Symbols", exact: true }).click();
    const toolbar = page.getByRole("toolbar", { name: "Graph controls" });
    const rail = toolbar.locator(".compass-toolbar-actions");
    // Read all geometry together: automatic layout can update the status text
    // and move the rail while separate asynchronous measurements are running.
    const geometry = await rail.evaluate((element) => {
      const rect = (node: Element) => {
        const { x, y, width, height } = node.getBoundingClientRect();
        return { x, y, width, height };
      };
      return {
        rail: rect(element), toolbar: rect(element.closest('[role="toolbar"]')!),
        controls: [...element.querySelectorAll("button, select")]
          .filter((node) => node.getClientRects().length > 0).map(rect)
      };
    });
    const { rail: railBox, toolbar: toolbarBox } = geometry;
    if (width <= 768) expect(railBox.width).toBeGreaterThan(toolbarBox.width * 0.95);
    expect(toolbarBox.height).toBeLessThanOrEqual(width <= 390 ? 200 : 140);
    for (const box of geometry.controls) {
      expect(box.x).toBeGreaterThanOrEqual(railBox.x - 1);
      expect(box.x + box.width).toBeLessThanOrEqual(railBox.x + railBox.width + 1);
      expect(box.y + box.height).toBeLessThanOrEqual(toolbarBox.y + toolbarBox.height + 1);
    }
    await page.getByRole("button", { name: "Graph settings", exact: true }).click();
    const spacing = page.getByRole("combobox", { name: "Layout spacing" });
    await expect(spacing).toHaveValue("2");
    await expect(spacing.locator("option")).toHaveText([
      "Compact · 75%", "Default · 100%", "Airy · 150%", "Wide · 200%", "Extra wide · 300%"
    ]);
    await spacing.selectOption({ label: "Extra wide · 300%" });
    await expect(page.getByRole("region", { name: "Interactive Compass code graph" })).toHaveAttribute("data-layout-spacing", "6");
  });
}

for (const hostWidth of [320, 558, 768]) {
  test(`icons keep their size inside a ${hostWidth}px host on a wide screen`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/hierarchyWorkbench.html");
    await page.locator(".visualization-workbench").evaluate((element, width) => {
      (element as HTMLElement).style.width = `${width}px`;
    }, hostWidth);
    const toolbar = page.getByRole("toolbar", { name: "Graph controls" });
    for (const name of ["Zoom out", "Zoom in", "Fit graph in view", "Graph settings"]) {
      const control = toolbar.getByRole("button", { name, exact: true });
      const icon = control.locator("svg");
      await expect.poll(() => icon.evaluate((element) => {
        const { width, height } = element.getBoundingClientRect();
        return [width, height];
      })).toEqual([16, 16]);
      const metrics = await control.evaluate((element) => {
        const button = element.getBoundingClientRect();
        const glyph = element.querySelector("svg")!.getBoundingClientRect();
        const panel = element.closest('[role="toolbar"]')!.getBoundingClientRect();
        return { width: button.width, height: button.height,
          contained: glyph.left >= button.left && glyph.right <= button.right,
          reachable: button.left >= panel.left && button.right <= panel.right };
      });
      expect(metrics.width).toBeGreaterThanOrEqual(30);
      expect(metrics.height).toBeGreaterThanOrEqual(28);
      expect(metrics.contained).toBe(true);
      expect(metrics.reachable).toBe(true);
    }
    const coverage = page.locator(".visualization-coverage");
    expect(await coverage.evaluate((element) => {
      const badge = element.getBoundingClientRect();
      const header = element.closest(".visualization-context")!.getBoundingClientRect();
      return badge.left >= header.left && badge.right <= header.right;
    })).toBe(true);
    // Compact labels must follow the host width, even though the viewport is wide.
    await expect(toolbar.locator(".compass-physics-button span")).toBeHidden();
    await toolbar.getByRole("button", { name: "Graph settings", exact: true }).click();
    await expect(page.getByRole("combobox", { name: "Layout spacing" })).toBeVisible();
  });
}
