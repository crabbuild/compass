import { expect, test } from "@playwright/test";

test("small graph reaches its useful controls within one second", async ({ page }) => {
  const started = Date.now();
  await page.goto("/graph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).waitFor();
  expect(Date.now() - started).toBeLessThan(1000);
});
