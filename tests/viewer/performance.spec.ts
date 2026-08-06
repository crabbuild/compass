import { expect, test } from "@playwright/test";

test("small graph reaches its useful controls within one second", async ({ page }) => {
  const started = Date.now();
  await page.goto("/graph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();
  expect(Date.now() - started).toBeLessThan(1000);
});

test("Django-sized community overview opens directly with the static profile", async ({ page }) => {
  const started = Date.now();
  await page.goto("/largeGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  await expect(page.getByRole("region", {
    name: "Interactive Compass code graph"
  })).toHaveAttribute("data-rendering-profile", "static");
  await expect(page.getByText("Arranging graph layout")).toHaveCount(0);
  await expect(page.getByRole("searchbox", { name: "Filter communities" })).toBeVisible();
  await expect(page.locator(".compass-community-item")).toHaveCount(200);
  await expect(page.locator(".compass-graph-stats")).toContainText("4,000 shown");
  expect(Date.now() - started).toBeLessThan(3000);
});
