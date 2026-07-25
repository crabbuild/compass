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
  await expect(page.getByRole("button", { name: /open source/i })).toBeVisible();
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
