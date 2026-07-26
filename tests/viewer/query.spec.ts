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

test("query modes use a full-width tab rail", async ({ page }) => {
  await page.goto("/query.html");
  const tabs = page.getByRole("tablist", { name: "Query mode" });
  await expect(tabs).toBeVisible();
  expect((await tabs.boundingBox())?.width).toBeGreaterThan(500);
  const natural = page.getByRole("tab", { name: "Ask the codebase" });
  const cql = page.getByRole("tab", { name: "CompassQL" });
  await expect(natural).toHaveAttribute("aria-selected", "true");
  expect(await natural.evaluate((element) => getComputedStyle(element).borderTopColor))
    .not.toBe("rgba(0, 0, 0, 0)");
  expect(await cql.evaluate((element) => getComputedStyle(element).borderTopColor))
    .toBe("rgba(0, 0, 0, 0)");
  await cql.click();
  await expect(cql).toHaveAttribute("aria-selected", "true");
  expect(await cql.evaluate((element) => getComputedStyle(element).borderTopColor))
    .not.toBe("rgba(0, 0, 0, 0)");
  expect(await natural.evaluate((element) => getComputedStyle(element).borderTopColor))
    .toBe("rgba(0, 0, 0, 0)");
  await expect(page.getByRole("textbox", { name: "CompassQL query" })).toBeVisible();
});

test("CompassQL actions and parameters share the bottom composer footer", async ({ page }) => {
  await page.goto("/query.html");
  await page.getByRole("tab", { name: "CompassQL" }).click();

  const shell = await page.locator(".query-editor-shell").boundingBox();
  const editor = await page.getByRole("textbox", { name: "CompassQL query" }).boundingBox();
  const params = await page.getByRole("textbox", { name: "CompassQL parameters" })
    .boundingBox();
  const run = await page.getByRole("button", { name: "Run query" }).boundingBox();

  expect(shell).not.toBeNull();
  expect(editor).not.toBeNull();
  expect(params).not.toBeNull();
  expect(run).not.toBeNull();
  expect(params!.x).toBeLessThan(run!.x);
  expect(run!.y).toBeGreaterThan(editor!.y + editor!.height);
  expect(run!.y + run!.height).toBeLessThanOrEqual(shell!.y + shell!.height);
});

test("narrow CompassQL composer preserves parameters and the bottom action", async ({
  page
}) => {
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto("/query.html");
  await page.getByRole("tab", { name: "CompassQL" }).click();

  const editor = page.getByRole("textbox", { name: "CompassQL query" });
  const params = page.getByRole("textbox", { name: "CompassQL parameters" });
  const run = page.getByRole("button", { name: "Run query" });
  await expect(editor).toBeVisible();
  await expect(params).toBeVisible();
  await expect(run).toBeVisible();

  const editorBox = await editor.boundingBox();
  const paramsBox = await params.boundingBox();
  const runBox = await run.boundingBox();
  expect(editorBox).not.toBeNull();
  expect(paramsBox).not.toBeNull();
  expect(runBox).not.toBeNull();
  expect(paramsBox!.height).toBeLessThanOrEqual(32);
  expect(paramsBox!.width).toBeGreaterThan(200);
  expect(runBox!.y).toBeGreaterThan(editorBox!.y + editorBox!.height);
  expect(await page.evaluate(
    () => document.documentElement.scrollWidth <= window.innerWidth
  )).toBe(true);
});

test("traversal answers expose graph and source actions", async ({ page }) => {
  await page.goto("/query.html?result=traversal");
  await page.getByRole("textbox", { name: "Natural-language query" }).fill("What is Pipeline?");
  await page.getByRole("button", { name: "Run query" }).click();

  await expect(page.getByRole("heading", { name: "146 graph matches" })).toBeVisible();
  await expect(page.getByText("Breadth-first · depth 2")).toBeVisible();
  await expect(page.getByRole("button", { name: "Open code graph" })).toBeVisible();

  await page.getByRole("button", {
    name: "Open Pipeline at caching/util/src/Pipeline.scala line 154"
  }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { openedQuerySource?: unknown }
  ).openedQuerySource)).toEqual({
    file: "caching/util/src/Pipeline.scala",
    startLine: 154,
    endLine: 154
  });

  await page.getByRole("button", { name: "Open code graph" }).click();
  expect(await page.evaluate(() => (
    window as typeof window & { openedQueryGraph?: boolean }
  ).openedQueryGraph)).toBe(true);
});
