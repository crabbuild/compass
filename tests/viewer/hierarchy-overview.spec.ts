import { expect, test } from "@playwright/test";

/**
 * `compass export html` opens a clustered repository on the published
 * hierarchy. These specs read the two shapes a real repository publishes: a
 * level that splits the repository, and the edge case where the only published
 * level holds the whole repository as one node.
 */
test("an export opens on the published level that splits the repository", async ({ page }) => {
  await page.goto("/hierarchyWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  const controls = page.getByRole("group", { name: "Graph scope" });
  await expect(controls.getByRole("button", { name: "Level 0" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-viewer-status-text"))
    .toContainText("2 communities · 4 symbols");
  await expect(page.locator(".compass-graph-stats"))
    .toContainText("2 communities");
  await expect(page.locator(".compass-graph-breadcrumb")).toContainText("Repository");

  await controls.getByRole("button", { name: "Symbols" }).click();
  await expect(controls.getByRole("button", { name: "Symbols" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-graph-stats")).toContainText("4 nodes");
});

test("a single-group level never opens as the repository overview", async ({ page }) => {
  await page.goto("/singleGroupHierarchyWorkbench.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  // One group holding every symbol is not an overview, so the page keeps the
  // canvas the export published instead of drawing a single node.
  const controls = page.getByRole("group", { name: "Graph scope" });
  await expect(controls.getByRole("button", { name: "Symbols" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-graph-stats")).toContainText("4 nodes");

  // The published level is still reachable by hand.
  await controls.getByRole("button", { name: "Level 0" }).click();
  await expect(controls.getByRole("button", { name: "Level 0" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".compass-viewer-status-text"))
    .toContainText("1 communities · 4 symbols");
});
