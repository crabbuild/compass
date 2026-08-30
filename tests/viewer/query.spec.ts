import { expect, test } from "@playwright/test";

test("query supports keyboard execution and cancellation", async ({ page }) => {
  await page.goto("/query.html?delay=1");
  const editor = page.getByRole("combobox", { name: "Ask input" });
  await editor.fill("Who calls Pipe");
  const completions = page.getByRole("listbox", { name: "Ask suggestions" });
  await expect(completions).toBeVisible();
  await expect(completions.getByRole("option")).toContainText([
    "caching::util::Pipeline"
  ]);
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      queryHostMessages: Array<{ type: string; request?: { term?: string } }>;
    }).queryHostMessages;
    return messages.filter((message) => message.type === "complete").at(-1)?.request?.term;
  })).toBe("Pipe");
  await editor.press("Tab");
  await expect(editor).toHaveValue("Who calls caching::util::Pipeline");
  await expect(completions).toBeHidden();
  await editor.press("Shift+Enter");
  await expect(editor).toHaveValue("Who calls caching::util::Pipeline\n");
  await editor.press("Enter");
  await expect(page.getByRole("button", { name: "Cancel Ask" })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("resolving the question");
  await page.getByRole("button", { name: "Cancel Ask" }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { queryHostMessages: Array<{ type: string }> }
  ).queryHostMessages.at(-1)?.type)).toBe("cancel");
  await expect(page.getByRole("button", { name: "Ask", exact: true })).toBeVisible();
  await expect(page.getByText("Run cancelled")).toBeVisible();
});

test("graph completion failures degrade without blocking query execution", async ({ page }) => {
  await page.goto("/query.html?completionError=1");
  const editor = page.getByRole("combobox", { name: "Ask input" });
  await editor.fill("What is Pipe");
  await expect(page.getByRole("status")).toContainText(
    "Graph suggestions are unavailable"
  );
  await editor.press("Enter");
  await expect(page.getByRole("heading", { name: "1 graph match" })).toBeVisible();
});

test("late graph completions cannot replace a newer input generation", async ({ page }) => {
  await page.goto("/query.html?completionDelay=1");
  const editor = page.getByRole("combobox", { name: "Ask input" });
  await editor.fill("Explain Pipe");
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { queryHostMessages: Array<{ type: string }> }
  ).queryHostMessages.filter((message) => message.type === "complete").length)).toBe(1);
  await editor.fill("Explain MissingSymbol");
  await expect(page.getByRole("status")).toContainText(
    "No graph symbols match “MissingSymbol”"
  );
  await expect(page.getByRole("option", { name: /Pipeline/ })).toHaveCount(0);
});

test("query renders structured columns", async ({ page }) => {
  await page.goto("/query.html?result=rows");
  await page.getByRole("tab", { name: /CompassQL/ }).click();
  await page.getByRole("combobox", { name: "CompassQL input" }).fill("MATCH (n) RETURN n");
  await page.getByRole("button", { name: "Run query" }).click();
  await expect(page.getByRole("columnheader", { name: "symbol" })).toBeVisible();
  await expect(page.getByRole("columnheader", { name: "calls" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "run" })).toBeVisible();
});

test("query errors keep the editor available for recovery", async ({ page }) => {
  await page.goto("/query.html?error=1");
  const editor = page.getByRole("combobox", { name: "Ask input" });
  await editor.fill("broken query");
  await page.getByRole("button", { name: "Ask" }).click();
  await expect(page.getByRole("alert")).toContainText("CompassQL could not parse this query");
  await page.getByRole("button", { name: "Edit input" }).click();
  await expect(editor).toBeFocused();
});

test("query commands use a full-width tab rail", async ({ page }) => {
  await page.goto("/query.html");
  const tabs = page.getByRole("tablist", { name: "Query command" });
  await expect(tabs).toBeVisible();
  expect((await tabs.boundingBox())?.width).toBeGreaterThan(500);
  const ask = page.getByRole("tab", { name: /Ask/ });
  const explain = page.getByRole("tab", { name: /Explain/ });
  const cql = page.getByRole("tab", { name: /CompassQL/ });
  await expect(tabs.getByRole("tab")).toHaveCount(3);
  await expect(ask).toHaveAttribute("aria-selected", "true");
  await expect(explain).toHaveAttribute("aria-selected", "false");
  await expect(ask.locator(".query-mode-indicator")).toHaveCSS("opacity", "1");
  await expect(cql.locator(".query-mode-indicator")).toHaveCSS("opacity", "0");
  await cql.click();
  await expect(cql).toHaveAttribute("aria-selected", "true");
  await expect(cql.locator(".query-mode-indicator")).toHaveCSS("opacity", "1");
  await expect(ask.locator(".query-mode-indicator")).toHaveCSS("opacity", "0");
  await expect(page.getByRole("combobox", { name: "CompassQL input" })).toBeVisible();
});

test("CompassQL actions and parameters share the bottom composer footer", async ({ page }) => {
  await page.goto("/query.html");
  await page.getByRole("tab", { name: "CompassQL" }).click();

  const shell = await page.locator(".query-editor-shell").boundingBox();
  const editor = await page.getByRole("combobox", { name: "CompassQL input" }).boundingBox();
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

  const editor = page.getByRole("combobox", { name: "CompassQL input" });
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

test("typed Ask answers expose graph and source actions", async ({ page }) => {
  await page.goto("/query.html");
  await page.getByRole("combobox", { name: "Ask input" }).fill("What is Pipeline?");
  await page.getByRole("button", { name: "Ask" }).click();

  await expect(page.getByRole("heading", { name: "1 graph match" })).toBeVisible();
  await expect(page.getByText("Search evidence")).toBeVisible();
  await expect(page.getByRole("button", { name: "Open code graph" })).toBeVisible();

  await page.getByRole("button", {
    name: "Open Pipeline at caching/util/src/Pipeline.scala line 154"
  }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { openedQuerySource?: unknown }
  ).openedQuerySource)).toEqual({
    file: "caching/util/src/Pipeline.scala",
    startByte: 0,
    endByte: 8,
    startLine: 154,
    startColumn: 0,
    endLine: 154,
    endColumn: 8
  });

  await page.getByRole("button", { name: "Open code graph" }).click();
  expect(await page.evaluate(() => (
    window as typeof window & { openedQueryGraph?: boolean }
  ).openedQueryGraph)).toBe(true);
});

test("Ask diagnostics are itemized instead of rendered as a JSON blob", async ({ page }) => {
  await page.goto("/query.html?diagnostic=1");
  await page.getByRole("combobox", { name: "Ask input" }).fill("Find a missing node");
  await page.getByRole("button", { name: "Ask" }).click();

  const guidance = page.getByRole("region", { name: "Query guidance" });
  await expect(guidance).toContainText("No Match");
  await expect(guidance).toContainText("Try a qualified name");
  await expect(guidance).not.toContainText('"diagnostics"');
});

test("Ask and Explain results remain available in separate tabs", async ({ page }) => {
  await page.goto("/query.html");
  await page.getByRole("combobox", { name: "Ask input" }).fill("What is Pipeline?");
  await page.getByRole("button", { name: "Ask" }).click();
  await expect(page.getByRole("heading", { name: "1 graph match" })).toBeVisible();

  await page.getByRole("tab", { name: /Explain/ }).click();
  await page.getByRole("combobox", { name: "Explain input" }).fill("Pipeline");
  await page.getByRole("button", { name: "Explain" }).click();
  await expect(page.getByRole("heading", { name: "Symbol explanation" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Relationships 2" })).toBeVisible();
  await expect(page.getByText("save")).toBeVisible();

  const results = page.getByRole("tablist", { name: "Query results" });
  await expect(results.getByRole("tab")).toHaveCount(2);
  await results.getByRole("tab", { name: /Ask.*What is Pipeline/ }).click();
  await expect(page.getByRole("heading", { name: "1 graph match" })).toBeVisible();
});

test("typed graph query evidence exposes exact and heuristic provenance", async ({ page }) => {
  await page.goto("/graph.html");
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("helper");
  await page.getByRole("option", { name: /helper/i }).click();
  await expect(page.getByRole("region", { name: "Node evidence" }))
    .toContainText("Exact");
  await expect(page.getByRole("region", { name: "Node evidence" }))
    .toContainText("rust.functions");
  const relationships = page.getByRole("region", { name: "Relationship evidence" });
  await expect(relationships).toContainText("Ambiguous");
  await expect(relationships).toContainText("trait-object-call");
  await expect(relationships).toContainText("Wired at src/lib.rs:6");
  await expect(relationships).toContainText("compatible receiver type");
});
