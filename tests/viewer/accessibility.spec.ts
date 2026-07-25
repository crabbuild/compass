import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const pageName of ["graph", "loading", "architecture", "calls", "history", "query"]) {
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
