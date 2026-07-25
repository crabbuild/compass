import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const pageName of ["graph", "architecture", "calls", "history", "query"]) {
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
