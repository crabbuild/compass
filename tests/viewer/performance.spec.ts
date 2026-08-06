import { expect, test } from "@playwright/test";

test("small graph reaches its useful controls within one second", async ({ page }) => {
  const started = Date.now();
  await page.goto("/graph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();
  expect(Date.now() - started).toBeLessThan(1000);
});

test("large graph opens directly with the static rendering profile", async ({ page }) => {
  const started = Date.now();
  await page.goto("/largeGraph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();

  await expect(page.getByRole("region", {
    name: "Interactive Compass code graph"
  })).toHaveAttribute("data-rendering-profile", "static");
  await expect(page.getByText("Arranging graph layout")).toHaveCount(0);
  expect(Date.now() - started).toBeLessThan(3000);
});
