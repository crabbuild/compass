import { expect, test, type Page } from "@playwright/test";

/**
 * The overview canvas is a real canvas, so a spec cannot address an edge
 * directly. Sweep a coarse grid over the graph until an aggregated edge hover
 * card appears and return its text.
 */
async function hoverCommunityEdge(page: Page): Promise<string> {
  const canvas = page.locator(".compass-canvas");
  const box = await canvas.boundingBox();
  if (!box) throw new Error("the graph canvas has no layout box");
  for (let row = 1; row <= 8; row += 1) {
    for (let column = 1; column <= 12; column += 1) {
      await page.mouse.move(
        box.x + (box.width * column) / 13,
        box.y + (box.height * row) / 9
      );
      const card = page.locator(".compass-edge-hover-card");
      if (await card.count() > 0) {
        return (await card.first().innerText()).replace(/\s+/g, " ");
      }
    }
  }
  throw new Error("no aggregated community edge was reachable by hover");
}

// The fixture is a 900-symbol, 12-community repository the export did not
// aggregate, so the viewer has to derive the community overview itself.
test("a large symbol export opens as a labelled community overview", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const controls = page.getByRole("group", { name: "Graph scope" });
  await expect(controls.getByRole("button", { name: "Communities" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("12 communities · 900 symbols");
  await expect(page.locator(".compass-graph-stats"))
    .toContainText("900 symbols");
  await expect(page.locator(".compass-community-item").first())
    .toContainText("75");

  // Search reaches every symbol, not only the communities on screen.
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("symbol_451");
  const result = page.locator(".compass-search-item").first();
  await expect(result).toBeVisible();
  await expect(result).toContainText("module_");
  await result.click();
  // The result opens the community that holds it and focuses the symbol.
  await expect(page.getByRole("button", { name: "Back to community overview" }))
    .toBeVisible();
  await expect(page.locator(".compass-viewer-status-text"))
    .toContainText("Inspecting symbol_451");

  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(controls.getByRole("button", { name: "Communities" }))
    .toHaveAttribute("aria-pressed", "true");
});

test("the symbol scope keeps the unmodified graph reachable", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  await page.getByRole("button", { name: "Symbols" }).click();
  await expect(page.getByRole("button", { name: "Symbols" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-viewer-status-text"))
    .not.toHaveText("12 communities · 900 symbols");
  await expect(page.locator(".compass-graph-stats"))
    .toContainText("900 nodes");

  await page.getByRole("button", { name: "Communities" }).click();
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("12 communities · 900 symbols");
});

test("the overview states what couples two communities", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const hoverText = await hoverCommunityEdge(page);
  // Exact counts of at least two relationship kinds, never one opaque total.
  expect(hoverText).toMatch(/\d+ (calls|imports|contains)/);
  expect(hoverText).toMatch(/· \d+ (calls|imports|contains)/);
  expect(hoverText).not.toContain("cross-community edges");
});

test("the repository path returns from a community detail", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const path = page.getByRole("navigation", { name: "Graph path" });
  await expect(path).toContainText("Repository");
  await expect(path.getByRole("button")).toHaveCount(0);

  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("symbol_451");
  await page.locator(".compass-search-item").first().click();
  await expect(path).toContainText("module_");

  await path.getByRole("button", { name: "Repository" }).click();
  await expect(page.getByRole("button", { name: "Back to community overview" }))
    .toHaveCount(0);
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("12 communities · 900 symbols");
});

test("a community list row opens the same group as the canvas", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const open = page.getByRole("button", { name: /^Open group / }).first();
  await expect(open).toBeEnabled();
  await open.click();

  await expect(page.getByRole("button", { name: "Back to community overview" }))
    .toBeVisible();
  const path = page.getByRole("navigation", { name: "Graph path" });
  await expect(path.getByRole("button", { name: "Repository" })).toBeVisible();
});

test("the reader can switch between overview designs", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const designs = page.getByRole("group", { name: "Overview design" });
  await expect(designs.getByRole("button", { name: "Bubbles" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("region", { name: "Interactive Compass code graph" }))
    .toBeVisible();

  await designs.getByRole("button", { name: "Matrix" }).click();
  await expect(page.getByRole("grid", { name: "Community coupling matrix" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Interactive Compass code graph" }))
    .toHaveCount(0);
  await expect(page.locator(".compass-viewer-status-text"))
    .toContainText("Matrix · 12 communities · 900 symbols");

  await designs.getByRole("button", { name: "Area" }).click();
  await expect(page.getByRole("group", { name: "Community area map" })).toBeVisible();

  await designs.getByRole("button", { name: "Tiers" }).click();
  await expect(page.getByRole("group", { name: "Community tier map" })).toBeVisible();

  await designs.getByRole("button", { name: "Bubbles" }).click();
  await expect(page.getByRole("region", { name: "Interactive Compass code graph" }))
    .toBeVisible();
  await expect(page.locator(".compass-viewer-status-text"))
    .toHaveText("12 communities · 900 symbols");
});

test("a matrix cell opens the community it points at", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();
  await page.getByRole("group", { name: "Overview design" })
    .getByRole("button", { name: "Matrix" })
    .click();

  const firstCell = page.locator(".compass-matrix-cell:not([disabled])").first();
  await expect(firstCell).toBeVisible();
  await firstCell.click();

  await expect(page.getByRole("button", { name: "Back to community overview" }))
    .toBeVisible();
  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(page.getByRole("grid", { name: "Community coupling matrix" })).toBeVisible();
});

test("the area map labels its tiles and the tier map draws its ribbons", async ({ page }) => {
  await page.goto("/largeSymbolGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  await page.getByRole("group", { name: "Overview design" })
    .getByRole("button", { name: "Area" })
    .click();
  await expect(page.locator(".compass-treemap-label").first()).toBeVisible();
  await expect(page.getByRole("group", { name: "Community area map" }))
    .toContainText("symbols");

  await page.getByRole("group", { name: "Overview design" })
    .getByRole("button", { name: "Tiers" })
    .click();
  await expect(page.locator(".compass-lane-ribbon").first()).toBeVisible();
  await expect(page.locator(".compass-lane-tier").first()).toHaveText(/tier 1/i);
});
