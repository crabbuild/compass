import { expect, test } from "@playwright/test";

test("architecture and call graph have separate purpose-built views", async ({ page }) => {
  await page.goto("/architecture.html");
  await expect(page.getByRole("heading", { name: "System call flow" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Architecture sections" })).toBeVisible();
  await expect(page.getByText("Showing 24 of 25 flows")).toBeVisible();
  await expect(page.locator(".architecture-flow-grid > div")).toHaveCount(24);
  await page.getByRole("button", { name: "Show all 25 flows" }).click();
  await expect(page.locator(".architecture-flow-grid > div")).toHaveCount(25);
  await expect(page.getByText("Showing 24 of 25 flows")).toHaveCount(0);

  await page.goto("/calls.html");
  await expect(page.getByText("depth 1")).toBeVisible();
  await expect(page.getByText("Calls from run")).toBeVisible();
  await expect(page.getByText("2 nodes", { exact: true })).toBeVisible();
  await expect(page.getByText("1 edge", { exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("Partial call graph");
  await expect(page.getByRole("alert")).toContainText(
    "Compass reached the configured graph limit. Counts and paths may be incomplete."
  );
  await expect(page.getByText("Showing 20 of 21 continuations")).toBeVisible();
  await expect(page.getByRole("button", { name: /Expand (callers|callees)/ })).toHaveCount(20);
  await page.getByRole("button", { name: "Show all 21 continuations" }).click();
  await expect(page.getByRole("button", { name: /Expand (callers|callees)/ })).toHaveCount(21);
  await expect(page.getByText("Showing 20 of 21 continuations")).toHaveCount(0);
});
