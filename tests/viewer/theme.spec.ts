import { expect, test, type Page } from "@playwright/test";

const themeCases = [
  {
    name: "light",
    bodyClass: "vscode-light",
    background: "#f4f4f4",
    foreground: "#202020",
    sidebar: "#ebebeb",
    input: "#ffffff"
  },
  {
    name: "dark",
    bodyClass: "vscode-dark",
    background: "#181818",
    foreground: "#d6d6d6",
    sidebar: "#202020",
    input: "#292929"
  },
  {
    name: "high contrast dark",
    bodyClass: "vscode-high-contrast",
    background: "#000000",
    foreground: "#ffffff",
    sidebar: "#000000",
    input: "#000000"
  },
  {
    name: "high contrast light",
    bodyClass: "vscode-high-contrast-light",
    background: "#ffffff",
    foreground: "#000000",
    sidebar: "#ffffff",
    input: "#ffffff"
  }
] as const;

for (const theme of themeCases) {
  test(`Ask Codebase inherits ${theme.name} VS Code tokens`, async ({ page }) => {
    await page.goto("/query.html");
    await applyTheme(page, theme);

    await expect(page.locator(".query-shell")).toHaveCSS(
      "background-color",
      hexToRgb(theme.background)
    );
    await expect(page.locator(".query-shell")).toHaveCSS("color", hexToRgb(theme.foreground));
    await expect(page.getByRole("textbox", { name: "Natural-language query" }))
      .toHaveCSS("background-color", hexToRgb(theme.input));
  });

  test(`version comparison inherits ${theme.name} VS Code tokens`, async ({ page }) => {
    await page.goto("/history.html");
    await applyTheme(page, theme);

    const picker = page.getByRole("combobox", { name: "Comparison revision" });
    await expect(picker).toHaveCSS("background-color", hexToRgb(theme.input));
    await expect(picker).toHaveCSS("color", hexToRgb(theme.foreground));
    await expect(picker).toHaveCSS(
      "color-scheme",
      theme.name.includes("light") ? "light" : "dark"
    );
    await expect(picker).toHaveCSS("border-top-color", hexToRgb(theme.foreground));
    if (theme.name.includes("high contrast")) {
      await expect(picker).toHaveCSS("border-top-width", "2px");
    }
  });
}

test("high-contrast themes use the VS Code contrast border", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.locator(".history-commit-details")).toBeVisible();
  await applyTheme(page, {
    bodyClass: "vscode-high-contrast",
    background: "#000000",
    foreground: "#ffffff",
    sidebar: "#000000",
    input: "#000000",
    contrastBorder: "#ff00ff"
  });

  await expect(page.locator(".history-commit-details"))
    .toHaveCSS("border-top-color", "rgb(255, 0, 255)");
  await expect(page.locator(".history-commit-details")).toHaveCSS("border-top-width", "2px");
});

test("history comparison and source diffs follow the light VS Code theme", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision B graph/i }).click();
  await page.getByRole("button", { name: /Compare revisions/i }).click();
  await expect(page.locator(".history-source-diff")).toBeVisible();

  await applyTheme(page, themeCases[0]);

  await expect(page.locator(".history-comparison"))
    .toHaveCSS("background-color", "rgb(244, 244, 244)");
  await expect(page.locator(".history-comparison"))
    .toHaveCSS("color", "rgb(32, 32, 32)");
  await expect(page.locator(".history-source-diff"))
    .toHaveCSS("background-color", "rgb(244, 244, 244)");
  await expect(page.locator(".history-source-diff"))
    .toHaveCSS("color-scheme", "light");
  await expect(page.getByRole("button", { name: "Split" }))
    .toHaveCSS("background-color", "rgb(244, 244, 244)");
  await expect(page.getByRole("button", { name: "Split" }))
    .toHaveCSS("color", "rgb(32, 32, 32)");
  await page.getByRole("tab", { name: /Changed graph/ }).click();
  await expect(page.locator(".compass-graph-stage"))
    .toHaveAttribute("data-comparison", "true");
  await expect(page.locator(".compass-graph-stage"))
    .toHaveCSS("background-color", "rgb(244, 244, 244)");
  await expect(page.locator(".compass-graph-stage"))
    .toHaveCSS("background-image", "none");
});

test("Architecture symbol titles use editor foreground in light themes", async ({ page }) => {
  await page.goto("/architecture.html");
  await applyTheme(page, themeCases[0]);
  await page.evaluate(() => {
    document.documentElement.style.setProperty("--vscode-sideBar-foreground", "#f2f2f2");
  });

  await expect(page.locator(".architecture-symbol-list strong").first())
    .toHaveCSS("color", "rgb(32, 32, 32)");
});

test("query result surfaces honor the high-contrast border token", async ({ page }) => {
  await page.goto("/query.html?result=traversal");
  await page.getByRole("textbox", { name: "Natural-language query" }).fill("What is Pipeline?");
  await page.getByRole("button", { name: "Run query" }).click();
  await applyTheme(page, {
    bodyClass: "vscode-high-contrast",
    background: "#000000",
    foreground: "#ffffff",
    sidebar: "#000000",
    input: "#000000",
    contrastBorder: "#ff00ff"
  });

  await expect(page.locator(".query-traversal-summary"))
    .toHaveCSS("border-top-color", "rgb(255, 0, 255)");
  await expect(page.locator(".query-node-results")).toHaveCSS("border-top-width", "2px");
});

test("query composer focus follows the VS Code focus token", async ({ page }) => {
  await page.goto("/query.html");
  await page.evaluate(() => {
    document.documentElement.style.setProperty("--vscode-focusBorder", "#ff00ff");
  });
  await page.getByRole("textbox", { name: "Natural-language query" }).focus();

  await expect(page.locator(".query-editor-shell"))
    .toHaveCSS("border-top-color", "rgb(255, 0, 255)");
  await expect(page.locator(".query-editor-shell"))
    .toHaveCSS("outline-color", "rgb(255, 0, 255)");
});

test("loading respects reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/architecture.html?delay=1");

  await expect(page.locator(".compass-load-progress i")).toHaveCSS("animation-name", "none");
  await expect(page.locator(".architecture-load-skeleton span").first())
    .toHaveCSS("animation-name", "none");
});

test("graph chrome stays flat and token-driven", async ({ page }) => {
  await page.goto("/graph.html");
  const toolbar = page.getByRole("toolbar", { name: "Graph controls" });
  await expect(toolbar).toHaveCSS("backdrop-filter", "none");
  await expect(toolbar).toHaveCSS("box-shadow", "none");
  await expect(toolbar).toHaveCSS("border-radius", "4px");
  await expect(page.getByRole("complementary", { name: "Graph inspector" }))
    .toHaveCSS("box-shadow", "none");
});

test("narrow Architecture, Ask Codebase, and Evolution views preserve core actions", async ({
  page
}) => {
  await page.setViewportSize({ width: 420, height: 780 });

  await page.goto("/architecture.html");
  await expect(page.getByRole("searchbox", {
    name: "Search the complete architecture"
  })).toBeVisible();
  await expect(page.getByRole("button", { name: /API/ }).first()).toBeVisible();
  await expectNoHorizontalDocumentOverflow(page);

  await page.goto("/query.html");
  await expect(page.getByRole("button", { name: "Run query" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Natural-language query" }))
    .toBeVisible();
  await expectNoHorizontalDocumentOverflow(page);

  await page.goto("/history.html");
  await expect(page.getByRole("combobox", { name: "Select revision" })).toBeVisible();
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeHidden();
  await expect(page.getByRole("button", { name: "Query this revision" })).toBeVisible();
  const graphSearch = page.getByRole("combobox", { name: "Search graph nodes" });
  await graphSearch.scrollIntoViewIfNeeded();
  await expect(graphSearch).toBeInViewport();
  await expectNoHorizontalDocumentOverflow(page);
});

async function applyTheme(
  page: Page,
  theme: {
    bodyClass: string;
    background: string;
    foreground: string;
    sidebar: string;
    input: string;
    contrastBorder?: string;
  }
): Promise<void> {
  await page.evaluate((tokens) => {
    document.body.className = tokens.bodyClass;
    const root = document.documentElement.style;
    root.setProperty("--vscode-editor-background", tokens.background);
    root.setProperty("--vscode-editor-foreground", tokens.foreground);
    root.setProperty("--vscode-sideBar-background", tokens.sidebar);
    root.setProperty("--vscode-sideBar-foreground", tokens.foreground);
    root.setProperty("--vscode-input-background", tokens.input);
    root.setProperty("--vscode-input-foreground", tokens.foreground);
    root.setProperty("--vscode-dropdown-background", tokens.input);
    root.setProperty("--vscode-dropdown-foreground", tokens.foreground);
    root.setProperty("--vscode-dropdown-border", tokens.foreground);
    root.setProperty("--vscode-panel-border", tokens.foreground);
    root.setProperty("--vscode-focusBorder", tokens.foreground);
    if (tokens.contrastBorder) {
      root.setProperty("--vscode-contrastBorder", tokens.contrastBorder);
    }
  }, theme);
}

async function expectNoHorizontalDocumentOverflow(page: Page): Promise<void> {
  await expect.poll(() => page.evaluate(
    () => document.documentElement.scrollWidth <= window.innerWidth
  )).toBe(true);
}

function hexToRgb(hex: string): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `rgb(${value >> 16}, ${(value >> 8) & 255}, ${value & 255})`;
}
