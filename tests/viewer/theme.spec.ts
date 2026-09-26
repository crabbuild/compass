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
    await expect(page.getByRole("combobox", { name: "Ask input" }))
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
  await page.goto("/query.html");
  await page.getByRole("combobox", { name: "Ask input" }).fill("What is Pipeline?");
  await page.getByRole("button", { name: "Ask" }).click();
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
  await page.getByRole("combobox", { name: "Ask input" }).focus();

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

test("standalone HTML export follows the operating-system light theme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/exportCommunity.html");

  await expect(page.locator("body")).toHaveCSS("background-color", "rgb(246, 247, 249)");
  await expect.poll(() => page.locator(".compass-graph-stage").evaluate((element) =>
    getComputedStyle(element).getPropertyValue("--compass-canvas").trim()
  )).toBe("#fbfcfd");
  await expect(page.getByRole("complementary", { name: "Graph inspector" }))
    .toHaveCSS("background-color", "rgb(255, 255, 255)");
});

test("a standalone export pins light or dark on request", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/exportCommunity.html");
  const canvas = page.locator(".compass-graph-stage");
  const canvasToken = () => canvas.evaluate((element) =>
    getComputedStyle(element).getPropertyValue("--compass-canvas").trim());

  const theme = page.getByRole("group", { name: "Colour theme" });
  await expect(theme.getByRole("button", { name: "Auto" }))
    .toHaveAttribute("aria-pressed", "true");
  await expect.poll(canvasToken).toBe("#fbfcfd");

  // Readers may want the dark palette for a screenshot even on a light system.
  await theme.getByRole("button", { name: "Dark" }).click();
  await expect.poll(canvasToken).toBe("#08090c");
  await expect(page.locator("html")).toHaveAttribute("data-compass-theme", "dark");

  await theme.getByRole("button", { name: "Light" }).click();
  await expect.poll(canvasToken).toBe("#fbfcfd");
  await expect(page.locator("html")).toHaveAttribute("data-compass-theme", "light");

  await theme.getByRole("button", { name: "Auto" }).click();
  await expect(page.locator("html")).not.toHaveAttribute("data-compass-theme", /.+/);
});

for (const preference of ["Light", "Dark"] as const) {
  test(`layout menu follows the ${preference} override and shows layout icons`, async ({ page }) => {
    // Force the opposite OS palette to reproduce the native popup mismatch.
    await page.emulateMedia({ colorScheme: preference === "Dark" ? "light" : "dark" });
    await page.setViewportSize({ width: 390, height: 780 });
    await page.goto("/exportCommunity.html");
    await page.getByRole("group", { name: "Colour theme" })
      .getByRole("button", { name: preference, exact: true }).click();
    const trigger = page.getByRole("combobox", { name: "Graph layout", exact: true });
    await trigger.click();
    const menu = page.getByRole("listbox", { name: "Graph layout" });
    await expect(menu).toBeVisible();
    const palette = await menu.evaluate((element) => {
      const style = getComputedStyle(element);
      const luminance = (color: string) => {
        const [r, g, b] = color.match(/[\d.]+/g)!.slice(0, 3).map((value) => {
          const channel = Number(value) / 255;
          return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
      };
      const background = luminance(style.backgroundColor);
      const foreground = luminance(style.color);
      const rect = element.getBoundingClientRect();
      return { background, contrast: (Math.max(background, foreground) + 0.05)
        / (Math.min(background, foreground) + 0.05),
      contained: rect.left >= 0 && rect.right <= window.innerWidth && rect.bottom <= window.innerHeight };
    });
    expect(palette.contrast).toBeGreaterThanOrEqual(4.5);
    expect(palette.background < 0.1).toBe(preference === "Dark");
    expect(palette.contained).toBe(true);
    for (const [label, icon] of [
      ["Automatic", "network"], ["Circle", "circle"], ["Concentric", "target"],
      ["Spiral", "shell"], ["Square grid", "grid-2x2"]
    ]) {
      await expect(menu.getByRole("option", { name: label, exact: true })
        .locator(`svg.lucide-${icon}`)).toBeVisible();
    }
    await expect(menu.getByRole("option", { name: "Automatic", exact: true })
      .locator(".lucide-check")).toBeVisible();
    await page.keyboard.press("End");
    await expect(menu.getByRole("option", { name: "Square grid", exact: true })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(trigger).toContainText("Square grid");
    await expect(trigger.locator(".lucide-grid-2x2")).toBeVisible();
    await expect(trigger).toBeFocused();
    await expect(menu).toHaveCount(0);
    await trigger.press("ArrowDown");
    await expect(menu).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await expect(trigger).toContainText("Square grid");
  });
}

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
  await expect(page.getByRole("button", { name: "Ask" })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Ask input" }))
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
