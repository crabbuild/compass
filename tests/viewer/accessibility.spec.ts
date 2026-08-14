import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const pageName of [
  "graph",
  "workbench",
  "loading",
  "architecture",
  "calls",
  "history",
  "initialize",
  "query"
]) {
  test(`${pageName} has no serious accessibility violations`, async ({ page }) => {
    await page.goto(`/${pageName}.html`);
    const results = await new AxeBuilder({ page })
      .disableRules(["color-contrast"])
      .analyze();
    expect(results.violations.filter((violation) =>
      ["serious", "critical"].includes(violation.impact ?? "")
    )).toEqual([]);
  });
}

test("graph remains usable at 320 CSS pixels and reduced motion", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 720 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/graph.html");
  await expect(page.getByRole("combobox", { name: "Search graph nodes" })).toBeVisible();
  await expect(page.getByRole("toolbar")).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Graph inspector" })).toBeVisible();
  const columns = await page.locator(".compass-workspace")
    .evaluate((element) => getComputedStyle(element).gridTemplateColumns);
  expect(columns.split(" ")).toHaveLength(1);
});

test("workbench graph panels stay exclusive, bounded, and recover from empty filters", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto("/workbench.html");

  const navigation = page.getByRole("complementary", { name: "Compass navigation" });
  await page.getByRole("button", { name: "Collapse graph navigation" }).click();
  await expect(navigation).toHaveAttribute("data-collapsed", "true");
  await page.getByRole("button", { name: "Expand graph navigation" }).click();
  await expect(navigation).toHaveAttribute("data-collapsed", "false");

  const filters = page.getByRole("button", { name: "Graph filters" });
  const settings = page.getByRole("button", { name: "Graph settings" });
  await expect(settings.locator(".lucide-settings")).toHaveCount(1);
  await expect(filters.locator(".lucide-sliders-horizontal")).toHaveCount(1);
  expect(await settings.evaluate((element) => {
    const filter = document.querySelector('[aria-label="Graph filters"]');
    return filter !== null
      && Boolean(element.compareDocumentPosition(filter) & Node.DOCUMENT_POSITION_FOLLOWING);
  })).toBe(true);
  await settings.click();
  await expect(settings).toHaveAttribute("aria-expanded", "true");
  await filters.click();
  await expect(settings).toHaveAttribute("aria-expanded", "false");
  await expect(filters).toHaveAttribute("aria-expanded", "true");

  const panel = page.getByRole("region", { name: "Graph filter options" });
  const bounds = await panel.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(320);

  await page.getByRole("combobox", { name: "Relationship" }).selectOption("documents");
  await page.getByRole("combobox", { name: "Node kind" }).selectOption("function");
  await expect(page.getByText("No nodes match these filters")).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(page.getByText("No nodes match these filters")).toBeHidden();
  await expect(filters).toContainText("4 / 4");
});

test("graph inspector can be resized, collapsed, and expanded accessibly", async ({ page }) => {
  await page.goto("/graph.html");
  const inspector = page.getByRole("complementary", { name: "Graph inspector" });
  const separator = page.getByRole("separator", { name: "Resize graph inspector" });

  await expect(inspector).toBeVisible();
  await expect(separator).toHaveAttribute("aria-valuenow", "340");
  await separator.press("ArrowLeft");
  await expect(separator).toHaveAttribute("aria-valuenow", "364");

  await page.getByRole("button", { name: "Collapse graph inspector" }).click();
  await expect(separator).toBeHidden();
  await expect(page.getByRole("button", { name: "Expand graph inspector" })).toBeVisible();

  await page.getByRole("button", { name: "Expand graph inspector" }).click();
  await expect(page.getByRole("button", { name: "Collapse graph inspector" })).toBeVisible();
  await expect(page.getByRole("separator", { name: "Resize graph inspector" }))
    .toHaveAttribute("aria-valuenow", "364");
});
