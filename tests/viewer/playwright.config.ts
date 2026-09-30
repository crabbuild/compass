import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  fullyParallel: true,
  forbidOnly: true,
  retries: process.env.CI ? 2 : 0,
  reporter: "line",
  globalSetup: "./fixtures/generate.ts",
  use: {
    baseURL: "http://127.0.0.1:4178",
    trace: "retain-on-failure"
  },
  webServer: {
    command: "node serve.mjs",
    port: 4178,
    reuseExistingServer: !process.env.CI
  },
  projects: [
    {
      name: "chromium",
      testIgnore: "performance.spec.ts",
      use: { browserName: "chromium" }
    },
    {
      name: "chromium-performance",
      testMatch: "performance.spec.ts",
      // Wall-clock qualification must not share CPU with other browser tests.
      dependencies: ["chromium"],
      workers: 1,
      use: { browserName: "chromium" }
    }
  ]
});
