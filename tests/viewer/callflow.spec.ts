import { expect, test } from "@playwright/test";

test("architecture and call graph have separate purpose-built views", async ({ page }) => {
  await page.goto("/architecture.html");
  await expect(page.getByRole("heading", { name: "Fixture" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Architecture subsystems" })).toBeVisible();
  await expect(page.getByText("25 subsystem routes")).toBeVisible();
  await expect(page.getByText("16 of 25 routes · Show all")).toBeVisible();
  await expect(page.locator(".architecture-routes > g")).toHaveCount(16);
  await page.getByRole("button", { name: "All routes" }).click();
  await expect(page.locator(".architecture-routes > g")).toHaveCount(25);

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

test("architecture loading is informative and recoverable", async ({ page }) => {
  await page.goto("/architecture.html?delay=1");
  await expect(
    page.getByRole("heading", { name: "Preparing symbol index" })
  ).toBeVisible();
  await expect(page.locator(".architecture-load-skeleton")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Fixture" })).toBeVisible();

  await page.goto("/architecture.html?error=1");
  await expect(page.getByRole("alert")).toContainText("Architecture export failed");
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & {
      architectureHostMessages: Array<{ type: string }>;
    }
  ).architectureHostMessages.map(({ type }) => type))).toEqual(["ready", "retry"]);
});

test("architecture searches globally and bounds large symbol and call collections", async ({
  page
}) => {
  await page.goto("/architecture.html");
  const globalSearch = page.getByRole("searchbox", {
    name: "Search the complete architecture"
  });
  await globalSearch.fill("database");
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.getByRole("option", { name: /^database symbol\b/i }).click();
  await expect(page.getByRole("heading", { name: "API" })).toBeVisible();
  await expect(page.getByRole("tablist")).toBeVisible();

  const symbolFilter = page.getByRole("searchbox", { name: "Filter architecture selection" });
  await symbolFilter.fill("");
  await expect(page.locator(".architecture-symbol-list article")).toHaveCount(31);
  await expect(page.getByText("1–31 of 31 symbols")).toBeVisible();

  await page.getByRole("tab", { name: "Calls" }).click();
  await expect(page.getByText("1–53 of 53 calls")).toBeVisible();
  const callFilter = page.getByRole("searchbox", { name: "Filter architecture selection" });
  await callFilter.fill("database");
  await expect(page.locator(".architecture-call-list article", {
    hasText: /authenticate.*database/i
  }).first())
    .toBeVisible();
});

test("call graph uses a balanced cursor-resolution state and recoverable error", async ({
  page
}) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto("/calls.html");
  await expect(
    page.getByRole("heading", { name: "Resolving the function under your cursor" })
  ).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Locating symbol");
  await expect(page.getByRole("status")).toContainText("Tracing callers");
  await expect(page.getByRole("status")).toContainText("Tracing callees");

  const shell = await page.locator(".compass-load-shell").boundingBox();
  const content = await page.locator(".compass-load-copy").boundingBox();
  expect(shell).not.toBeNull();
  expect(content).not.toBeNull();
  expect(Math.abs(
    (content!.x + content!.width / 2) - (shell!.x + shell!.width / 2)
  )).toBeLessThan(2);
  await expect(page.getByText("Calls from run")).toBeVisible();
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("helper");
  await page.getByRole("option", { name: /helper/i }).click();
  const source = page.getByRole("button", {
    name: "Open source src/lib.rs at bytes 11–20"
  });
  await expect(source.locator(".compass-source-range")).toHaveText("Bytes 11–20");
  await source.click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { openedCallGraphSource?: unknown }
  ).openedCallGraphSource)).toEqual({
    file: "src/lib.rs",
    startByte: 11,
    endByte: 20
  });

  await page.goto("/calls.html?error=1");
  await expect(page.getByRole("alert")).toContainText(
    "No function could be resolved at this cursor position."
  );
  await page.getByRole("button", { name: "Show Compass output" }).click();
  expect(await page.evaluate(() => (
    window as typeof window & { showedCallGraphOutput?: boolean }
  ).showedCallGraphOutput)).toBe(true);
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { callGraphHostMessages: Array<{ type: string }> }
  ).callGraphHostMessages.map(({ type }) => type))).toEqual([
    "ready",
    "showOutput",
    "retry"
  ]);
});
