import { expect, test } from "@playwright/test";

test("shared graph focuses a searched symbol and exposes source", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4178")) external.push(request.url());
  });
  await page.goto("/graph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("helper");
  await page.getByRole("option", { name: /helper/i }).click();
  await expect(page.getByRole("status")).toContainText("Inspecting helper");
  await expect(page.getByRole("button", { name: /open source/i })).toBeVisible();
  expect(external).toEqual([]);
});
