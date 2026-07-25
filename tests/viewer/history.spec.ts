import { expect, test } from "@playwright/test";

test("timeline exposes explicit graph state without implicit build", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
  await expect(page.getByText("graph available")).toBeVisible();
  await expect(page.getByRole("button", { name: /open graph/i })).toBeVisible();
});

test("historical graph lazily enters a community and returns to its overview", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("button", { name: /open graph/i }).click();
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.fill("Core");
  await page.getByRole("option", { name: /Core/i }).click();
  await expect(page.getByRole("status")).toContainText("Inspecting Core");
  await page.getByRole("button", { name: "Open community" }).click();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedHistoricalCommunity?: number })
      .openedHistoricalCommunity
  )).toBe(0);
  await expect(page.getByRole("button", { name: "Back to community overview" })).toBeVisible();
  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(page.getByRole("button", { name: "Back to community overview" })).toHaveCount(0);
  await expect(page.getByText("Data", { exact: true })).toBeVisible();
});
