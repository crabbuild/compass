import { expect, test } from "@playwright/test";

test("architecture and call graph have separate purpose-built views", async ({ page }) => {
  await page.goto("/architecture.html");
  await expect(page.getByRole("heading", { name: "System call flow" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Architecture sections" })).toBeVisible();
  await page.goto("/calls.html");
  await expect(page.getByText("depth 1")).toBeVisible();
  await expect(page.getByText("Calls from run")).toBeVisible();
});
