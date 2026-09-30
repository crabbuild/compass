import { expect, test, type Page } from "@playwright/test";

/**
 * The three things a reader asked for while reading a real `compass export
 * html` page: the export reads as the graph rather than a one-item menu,
 * communities carry the repository's own names, and inspecting something hands
 * the room the community list was using to the evidence about it.
 */

async function selectNode(page: Page, name: string): Promise<void> {
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.fill(name);
  const result = page.locator(".compass-search-item").first();
  await expect(result).toBeVisible();
  await result.click();
}

test("a one-view export reads as the graph, not as a one-item menu", async ({ page }) => {
  await page.goto("/hierarchyWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const rail = page.getByLabel("Compass navigation");
  await expect(rail).toHaveAttribute("data-collapsed", "true");
  await expect(rail.getByRole("button", { name: /Code graph/ })).toHaveCount(0);
  await expect(page.locator(".visualization-context-title")).toHaveCount(0);

  // The rail is still there for the snapshot identity when a reader wants it.
  await page.getByRole("button", { name: "Expand graph navigation" }).click();
  await expect(rail).toHaveAttribute("data-collapsed", "false");
  await expect(rail).toContainText("Snapshot");
});

test("communities carry the hierarchy's names instead of their numbers", async ({ page }) => {
  await page.goto("/hierarchyWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  await page.getByRole("button", { name: "Symbols" }).click();
  const panel = page.locator(".compass-community-panel");
  await expect(panel).toContainText("src/runtime");
  await expect(panel).toContainText("docs");
  await expect(panel).not.toContainText("Community 0");
});

test("selecting a community hands the room to its evidence", async ({ page }) => {
  await page.goto("/hierarchyWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const panel = page.locator(".compass-community-panel");
  await expect(panel).not.toHaveAttribute("data-collapsed", "true");

  await selectNode(page, "src/runtime");
  // The list is not merely folded: the inspector takes the whole column.
  await expect(panel).toHaveCount(0);
  const inspector = page.locator(".compass-graph-inspector");
  await expect(inspector).toHaveAttribute("data-focused", "true");
  await expect(inspector.locator(".compass-graph-stats")).toHaveCount(0);
  await expect(inspector).toContainText("Community evidence");
  await expect(inspector).toContainText("Symbols");
  await expect(inspector).toContainText("Level 0");
  const infoBottom = await inspector.locator(".compass-info-panel").first()
    .evaluate((element) => element.getBoundingClientRect().bottom);
  const inspectorBottom = await inspector.evaluate((element) => element.getBoundingClientRect().bottom);
  expect(inspectorBottom - infoBottom).toBeLessThan(2);

  // Escape steps back out of the selection and the list returns with it.
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(1);
  await expect(page.locator(".compass-viewer-status-text"))
    .not.toContainText("Inspecting");
});

test("a group that is not a community offers no community to open", async ({ page }) => {
  await page.goto("/hierarchyDrilldownWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  // Level 1 holds groups of the partition, not communities. A published child
  // projection can be opened, while no community detail is invented for it.
  await page.getByRole("group", { name: "Graph scope" })
    .getByRole("button", { name: "Level 1" }).click();
  await selectNode(page, "area0/part0");
  const inspector = page.locator(".compass-graph-inspector");
  await expect(inspector).toContainText("Sub-groups");
  await expect(inspector).toContainText("Community evidence");
  await expect(inspector.getByRole("button", { name: /Open subgraph/ })).toBeVisible();
  await expect(inspector).not.toContainText("Open community");

  await inspector.getByRole("button", { name: /Open subgraph/ }).click();
  await expect(page.getByRole("navigation", { name: "Graph path" }))
    .toContainText("area0/part0");

  // A group of the published partition does name one.
  await page.getByRole("group", { name: "Graph scope" })
    .getByRole("button", { name: "Level 2" }).click();
  await selectNode(page, "area1/module5");
  await expect(inspector).toContainText("Open community");
});
