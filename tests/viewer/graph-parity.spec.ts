import { expect, test } from "@playwright/test";

test("VS Code graph mirrors Compass export structure and exposes source metadata", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4178")) external.push(request.url());
  });
  await page.goto("/graph.html");
  await expect.poll(() => page.locator("canvas").evaluate((canvas) => {
    const context = canvas.getContext("2d");
    if (!context) return 0;
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let painted = 0;
    for (let index = 3; index < pixels.length; index += 4) {
      if (pixels[index] > 0) painted += 1;
    }
    return painted;
  })).toBeGreaterThan(0);
  await expect(page.locator(".compass-inspector-header")).toContainText("Compass");
  await expect(page.getByRole("toolbar", { name: "Graph controls" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Graph inspector" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Communities" })).toBeVisible();
  await expect(page.locator(".compass-graph-stats")).toContainText("4 nodes");
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("helper");
  await page.getByRole("option", { name: /helper/i }).click();
  await expect(page.getByRole("status")).toContainText("Inspecting helper");
  await expect(page.locator(".compass-metadata-grid")).toContainText("5–7");
  await expect(page.locator(".compass-signature-block")).toContainText("fn helper()");
  const source = page.getByRole("button", {
    name: "Open source src/lib.rs at lines 5–7"
  });
  await expect(source).toBeVisible();
  await expect(source.locator(".compass-source-path")).toHaveText("src/lib.rs");
  await expect(source.locator(".compass-source-range")).toHaveText("Lines 5–7");
  const neighbors = page.locator(".compass-neighbor-link");
  await expect(neighbors).toHaveCount(2);
  await expect(neighbors.locator(".compass-neighbor-dot")).toHaveCount(2);
  await expect(neighbors.first()).toHaveCSS("border-left-width", "0px");

  await page.evaluate(() => {
    window.addEventListener("compass:open-source", ((event: CustomEvent) => {
      (window as typeof window & { openedSource?: unknown }).openedSource = event.detail;
    }) as EventListener);
  });
  await source.click();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedSource?: unknown }).openedSource
  )).toEqual({ file: "src/lib.rs", startLine: 5, endLine: 7 });
  expect(external).toEqual([]);
});

test("file-only graph nodes stay inspectable without source navigation", async ({ page }) => {
  await page.goto("/graph.html");
  await page.evaluate(() => {
    window.addEventListener("compass:open-source", ((event: CustomEvent) => {
      (window as typeof window & { openedSource?: unknown }).openedSource = event.detail;
    }) as EventListener);
  });
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("README");
  await page.getByRole("option", { name: /README/i }).click();
  await expect(page.locator(".compass-metadata-grid")).toContainText("README.md");
  await expect(page.getByRole("button", { name: /open source/i })).toHaveCount(0);
  await page.waitForTimeout(300);
  await page.locator("canvas").dblclick();
  expect(await page.evaluate(
    () => (window as typeof window & { openedSource?: unknown }).openedSource
  )).toBeUndefined();

  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("Store");
  await page.getByRole("option", { name: /Store/i }).click();
  await expect(page.locator(".compass-source-metadata")).toContainText("Not recorded");
  await expect(page.getByRole("button", { name: /open source/i })).toHaveCount(0);
});

test("single-click inspects and double-click opens the selected node's exact range", async ({
  page
}) => {
  await page.goto("/graph.html");
  await page.evaluate(() => {
    window.addEventListener("compass:open-source", ((event: CustomEvent) => {
      (window as typeof window & { openedSource?: unknown }).openedSource = event.detail;
    }) as EventListener);
  });
  await page.getByRole("combobox", { name: "Search graph nodes" }).fill("run");
  await page.getByRole("option", { name: /^run/i }).click();
  await page.waitForTimeout(300);
  const canvas = page.locator("canvas");
  await canvas.click();
  expect(await page.evaluate(
    () => (window as typeof window & { openedSource?: unknown }).openedSource
  )).toBeUndefined();
  await canvas.dblclick();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedSource?: unknown }).openedSource
  )).toEqual({ file: "src/lib.rs", startLine: 1, endLine: 3 });
});

test("canvas colors adapt to VS Code theme variables", async ({ page }) => {
  await page.goto("/graph.html");
  const stage = page.locator(".compass-graph-stage");
  const canvas = page.locator("canvas");
  const initialCanvas = await canvas.evaluate((element) => element.toDataURL());
  await page.evaluate(() => {
    (window as typeof window & { initialNetwork?: Element | null }).initialNetwork =
      document.querySelector(".vis-network");
  });
  await page.getByRole("button", { name: "Show labels" }).click();
  await expect(page.getByRole("button", { name: "Hide labels" })).toBeVisible();
  expect(await page.evaluate(() => (
    window as typeof window & { initialNetwork?: Element | null }
  ).initialNetwork === document.querySelector(".vis-network"))).toBe(true);
  await page.getByRole("button", { name: "Hide labels" }).click();
  await expect(page.getByRole("button", { name: "Show labels" })).toBeVisible();
  await page.getByRole("button", { name: "Show labels" }).click();
  await page.evaluate(() => {
    document.documentElement.style.setProperty("--vscode-editor-background", "#f8fafc");
    document.documentElement.style.setProperty("--vscode-sideBar-background", "#eef2f7");
    document.documentElement.style.setProperty("--vscode-editor-foreground", "#172033");
    document.documentElement.style.setProperty("--vscode-descriptionForeground", "#334155");
    document.documentElement.style.setProperty("--vscode-contrastBorder", "#ff00ff");
    document.body.classList.add("vscode-high-contrast");
  });
  const light = await stage.evaluate((element) => getComputedStyle(element).backgroundImage);
  await expect.poll(() => canvas.evaluate((element) => element.toDataURL()))
    .not.toBe(initialCanvas);
  expect(await page.evaluate(() => (
    window as typeof window & { initialNetwork?: Element | null }
  ).initialNetwork === document.querySelector(".vis-network"))).toBe(true);
  await expect(page.getByRole("button", { name: "Resume layout" })).toBeVisible();
  await page.evaluate(() => {
    document.documentElement.style.setProperty("--vscode-editor-background", "#08111f");
    document.documentElement.style.setProperty("--vscode-sideBar-background", "#101b2d");
  });
  const dark = await stage.evaluate((element) => getComputedStyle(element).backgroundImage);
  expect(light).not.toBe(dark);
});

test("community double-click enters lazy detail, source opens, and Back restores overview", async ({
  page
}) => {
  await page.goto("/community.html");
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.fill("Core");
  await page.getByRole("option", { name: /Core/i }).click();
  await page.waitForTimeout(300);
  await page.locator("canvas").dblclick();
  await expect(page.getByRole("heading", { name: "Opening Core" })).toBeVisible();
  await expect(page.locator(".compass-workspace-content")).toHaveAttribute("inert", "");
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedCommunity?: number }).openedCommunity
  )).toBe(0);
  await expect(page.getByRole("button", { name: "Back to community overview" })).toBeVisible();

  await search.fill("run");
  await page.getByRole("option", { name: /^run/i }).click();
  await page.waitForTimeout(300);
  await page.locator("canvas").dblclick();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedSource?: unknown }).openedSource
  )).toEqual({ file: "src/lib.rs", startLine: 1, endLine: 3 });

  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(page.getByRole("button", { name: "Back to community overview" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Communities" })).toBeVisible();
  await expect(page.getByText("Data", { exact: true })).toBeVisible();
});

test("community failure preserves the overview and permits retry", async ({
  page
}) => {
  await page.goto("/community.html");
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.fill("Data");
  await page.getByRole("option", { name: /Data/i }).click();
  const openCommunity = page.getByRole("button", { name: "Open community" });
  await openCommunity.click();
  await expect(page.getByRole("heading", { name: "Opening Data" })).toBeVisible();
  await expect(page.locator(".compass-workspace-content")).toHaveAttribute("inert", "");
  await page.waitForTimeout(50);
  expect(await page.evaluate(
    () => (window as typeof window & { communityRequestCount?: number }).communityRequestCount
  )).toBe(1);

  await expect(page.getByRole("alert")).toContainText("Community detail failed");
  await expect(
    page.getByRole("complementary", { name: "Graph inspector" })
      .getByText("Core", { exact: true })
      .last()
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Back to community overview" })).toHaveCount(0);

  await openCommunity.click();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { communityRequestCount?: number }).communityRequestCount
  )).toBe(2);
  await expect(page.getByRole("button", { name: "Back to community overview" })).toBeVisible();
});
