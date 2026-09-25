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
