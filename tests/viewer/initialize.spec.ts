import { expect, test } from "@playwright/test";

test("initialization reviews scope before starting file-level progress", async ({ page }) => {
  await page.goto("/initialize.html");

  await expect(page.getByRole("heading", { name: "Build a map of compass" })).toBeVisible();
  await page.getByRole("radio", { name: /custom scope/i }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByLabel("Include paths and globs").fill("src\npackages/**");
  await page.getByLabel("Exclude paths and globs").fill("**/generated/**");
  await page.getByRole("button", { name: "Review configuration" }).click();
  await expect(page.getByText("packages/**")).toBeVisible();
  await page.getByRole("button", { name: "Build Compass index" }).click();

  await expect(page.getByText("1 of 3 files")).toBeVisible();
  await expect(page.getByText("src/commands/init.ts")).toBeVisible();
  await expect(page.getByText("3 of 3 files")).toBeVisible();
  await expect(page.getByText("src/views/graph.ts")).toBeVisible();
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
