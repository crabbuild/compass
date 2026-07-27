import { expect, test } from "@playwright/test";

test("compiled graph webview centers its loader and honors reduced motion", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/loading.html");

  await expect(page.getByRole("heading", { name: "Mapping your codebase" })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Preparing inspector");

  const shell = await page.locator(".compass-load-shell").boundingBox();
  const content = await page.locator(".compass-load-copy").boundingBox();
  expect(shell).not.toBeNull();
  expect(content).not.toBeNull();
  expect(Math.abs(
    (content!.x + content!.width / 2) - (shell!.x + shell!.width / 2)
  )).toBeLessThan(2);

  const mark = await page.locator(".compass-load-mark").boundingBox();
  expect(mark).not.toBeNull();
  expect(mark!.width).toBe(58);
  expect(mark!.height).toBe(58);
  await expect(page.locator(".compass-load-logo")).toHaveCount(1);
  await expect(page.locator(".compass-load-graph")).toHaveCount(0);
  await expect(page.locator(".compass-load-logo"))
    .toHaveCSS("animation-name", "none");
  await expect(page.locator(".compass-load-progress i"))
    .toHaveCSS("animation-name", "none");
});

test("compiled graph webview exposes working recovery actions", async ({ page }) => {
  await page.goto("/loading.html?error=1");

  await expect(page.getByRole("alert")).toContainText("The graph export could not be read.");
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await page.getByRole("button", { name: "Show Compass output" }).click();

  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { hostMessages: Array<{ type: string }> }
  ).hostMessages.map(({ type }) => type))).toEqual([
    "ready",
    "retry",
    "showOutput"
  ]);
});

test("compiled graph webview explains one-time preparation for a large graph", async ({
  page
}) => {
  await page.goto("/loading.html?large=1");

  await expect(
    page.getByRole("heading", { name: "Preparing a large code graph" })
  ).toBeVisible();
  await expect(page.getByRole("status")).toContainText("42.2 MB");
  await expect(page.getByRole("status")).toContainText("Building overview");
  await expect(page.getByText("Snapshot ready"))
    .toHaveAttribute("data-state", "complete");
  await expect(page.getByText("Building overview"))
    .toHaveAttribute("data-state", "active");
  await expect(page.getByText("Opening explorer"))
    .toHaveAttribute("data-state", "pending");
});

test("compiled graph webview reports the snapshot phase honestly", async ({ page }) => {
  await page.goto("/loading.html?snapshot=1");

  await expect(page.getByText("Securing snapshot"))
    .toHaveAttribute("data-state", "active");
  await expect(page.getByText("Building overview"))
    .toHaveAttribute("data-state", "pending");
});

test("loading logo follows the high-contrast token", async ({ page }) => {
  await page.goto("/loading.html");
  await page.evaluate(() => {
    document.body.classList.add("vscode-high-contrast");
    document.documentElement.style.setProperty("--vscode-contrastBorder", "#ff00ff");
    document.documentElement.style.setProperty("--vscode-editor-background", "#000000");
    document.documentElement.style.setProperty("--vscode-editor-foreground", "#ffffff");
  });

  await expect(page.locator(".compass-load-logo"))
    .toHaveCSS("color", "rgb(255, 0, 255)");
  await expect(page.locator(".compass-load-mark")).toHaveCSS("border-top-width", "0px");
  await expect(page.getByText("Reading graph")).toHaveCSS("color", "rgb(255, 255, 255)");
});
