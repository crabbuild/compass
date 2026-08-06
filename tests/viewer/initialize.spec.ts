import { expect, test } from "@playwright/test";

test("initialization step connectors stay centered on every marker", async ({ page }) => {
  await page.goto("/initialize.html");

  const offset = async (axis: "horizontal" | "vertical") => page.locator(
    ".init-step-nav li"
  ).first().evaluate((item, requestedAxis) => {
    const marker = item.querySelector(".init-step-marker")?.getBoundingClientRect();
    const bounds = item.getBoundingClientRect();
    const connector = getComputedStyle(item, "::after");
    if (!marker) throw new Error("Step marker is missing");
    return requestedAxis === "vertical"
      ? Math.abs(marker.x + marker.width / 2 - bounds.x - Number.parseFloat(connector.left))
      : Math.abs(marker.y + marker.height / 2 - bounds.y - Number.parseFloat(connector.top));
  }, axis);

  expect(await offset("vertical")).toBeLessThan(0.5);
  await page.setViewportSize({ width: 520, height: 900 });
  expect(await offset("horizontal")).toBeLessThan(0.5);
});

test("initialization reviews scope before starting file-level progress", async ({ page }) => {
  await page.goto("/initialize.html?manualSuccess=true");

  await expect(page.getByRole("heading", { name: "Build a map of compass" })).toBeVisible();
  await page.getByRole("radio", { name: /custom scope/i }).click();
  await page.getByRole("checkbox", { name: "src" }).check();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByLabel("Additional include globs").fill("packages/**");
  await page.getByLabel("Exclude paths and globs").fill("**/generated/**");
  await page.getByRole("button", { name: "Review configuration" }).click();
  await expect(page.getByText("packages/**")).toBeVisible();
  await page.getByRole("button", { name: "Build Compass index" }).click();

  await expect(page.getByText("1 of 3 files")).toBeVisible();
  await expect(page.getByText("src/commands/init.ts")).toBeVisible();
  await expect(page.getByText("3 of 3 files")).toBeVisible();
  await expect(page.getByText("src/views/graph.ts")).toBeVisible();
  await page.evaluate(() => {
    (window as typeof window & { completeInitialization: () => void })
      .completeInitialization();
  });
  await expect(page.getByRole("heading", { name: "Your Compass index is ready" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      initializationHostMessages: Array<Record<string, unknown>>;
    }).initializationHostMessages;
    return messages.find((message) => message.type === "start");
  })).toMatchObject({
    request: {
      includes: ["src", "packages/**"],
      excludes: ["**/generated/**"]
    }
  });
});

test("repository paths stay inside a scrollable pane", async ({ page }) => {
  await page.goto("/initialize.html?manyFiles=true");
  await page.getByRole("radio", { name: /custom scope/i }).click();
  await page.getByRole("button", { name: "Expand src" }).click();

  const tree = page.getByRole("tree", { name: "Repository scope" });
  await expect(tree).toBeVisible();
  await expect(page.getByText("181 paths · 0 selected")).toBeVisible();

  const dimensions = await tree.evaluate((element) => ({
    clientHeight: element.clientHeight,
    overflowY: getComputedStyle(element).overflowY,
    scrollHeight: element.scrollHeight
  }));
  expect(dimensions.overflowY).toBe("auto");
  expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);

  await tree.evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
  await expect.poll(() => tree.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect(page.getByRole("treeitem", { name: "src/module-179.ts" })).toBeVisible();
});

test("existing configuration requires explicit replacement confirmation", async ({ page }) => {
  await page.goto("/initialize.html?existing=true");
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Review configuration" }).click();

  const build = page.getByRole("button", { name: "Replace configuration and build" });
  await expect(build).toBeDisabled();
  await page.getByRole("checkbox", {
    name: /replace \.compass\/config\.toml/i
  }).check();
  await expect(build).toBeEnabled();
  await build.click();

  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      initializationHostMessages: Array<Record<string, unknown>>;
    }).initializationHostMessages;
    return messages.find((message) => message.type === "start");
  })).toMatchObject({
    request: { replaceExisting: true }
  });
});

test("cancelled builds announce recovery actions", async ({ page }) => {
  await page.goto("/initialize.html");
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Review configuration" }).click();
  await page.getByRole("button", { name: "Build Compass index" }).click();
  await page.getByRole("button", { name: "Cancel build" }).click();

  await expect(page.getByRole("heading", { name: "Build cancelled" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume build" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit configuration" })).toBeVisible();
});
