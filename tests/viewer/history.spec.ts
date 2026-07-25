import { expect, test } from "@playwright/test";

test("timeline exposes explicit graph state without implicit build", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
  await expect(page.getByText("graph available")).toBeVisible();
  await expect(page.getByRole("button", { name: /open graph/i })).toBeVisible();
});
