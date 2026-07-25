import { expect, test } from "@playwright/test";

test("query supports keyboard execution and cancellation", async ({ page }) => {
  await page.goto("/query.html?delay=1");
  const editor = page.getByRole("textbox", { name: "Natural-language query" });
  await editor.fill("How does authentication reach storage?");
  await editor.press("Control+Enter");
  await expect(page.getByRole("button", { name: "Cancel query" })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Traversing the code graph");
  await page.getByRole("button", { name: "Cancel query" }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { queryHostMessages: Array<{ type: string }> }
  ).queryHostMessages.at(-1)?.type)).toBe("cancel");
  await expect(page.getByRole("button", { name: "Run query" })).toBeVisible();
});

test("query renders structured columns", async ({ page }) => {
  await page.goto("/query.html?result=rows");
  await page.getByRole("textbox", { name: "Natural-language query" }).fill("List symbols");
  await page.getByRole("button", { name: "Run query" }).click();
  await expect(page.getByRole("columnheader", { name: "symbol" })).toBeVisible();
  await expect(page.getByRole("columnheader", { name: "calls" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "run" })).toBeVisible();
});

test("query errors keep the editor available for recovery", async ({ page }) => {
  await page.goto("/query.html?error=1");
  const editor = page.getByRole("textbox", { name: "Natural-language query" });
  await editor.fill("broken query");
  await page.getByRole("button", { name: "Run query" }).click();
  await expect(page.getByRole("alert")).toContainText("CompassQL could not parse this query");
  await page.getByRole("button", { name: "Revise query" }).click();
  await expect(editor).toBeFocused();
});
