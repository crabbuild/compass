import { expect, test } from "@playwright/test";

test("timeline exposes explicit graph state without implicit build", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
  await expect(page.getByText("graph available")).toBeVisible();
  await expect(page.getByRole("button", { name: /open graph/i })).toBeVisible();
});

test("historical graph lazily enters a community and returns to its overview", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("button", { name: /open graph/i }).click();
  const search = page.getByRole("combobox", { name: "Search graph nodes" });
  await search.fill("Core");
  await page.getByRole("option", { name: /Core/i }).click();
  await expect(
    page.getByRole("toolbar", { name: "Graph controls" }).getByRole("status")
  ).toContainText("Inspecting Core");
  await page.getByRole("button", { name: "Open community" }).click();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { openedHistoricalCommunity?: number })
      .openedHistoricalCommunity
  )).toBe(0);
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.find((message) => message.type === "openCommunity");
  })).toMatchObject({
    commit: "a".repeat(40),
    realization: "r-a",
    fingerprint: "f-a"
  });
  await expect(page.getByRole("button", { name: "Back to community overview" })).toBeVisible();
  await page.getByRole("button", { name: "Back to community overview" }).click();
  await expect(page.getByRole("button", { name: "Back to community overview" })).toHaveCount(0);
  await expect(page.getByText("Data", { exact: true })).toBeVisible();
});

test("late revision and derived responses cannot replace the selected commit", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("button", { name: /open graph/i }).click();
  await page.getByRole("option", { name: /Revision B graph/i }).click();
  await page.getByRole("button", { name: /open graph/i }).click();

  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();
  await expect(page.locator("small").filter({ hasText: "Revision B graph" })).toBeVisible();
  await expect(page.getByText("nodes +2 −0 ~1", { exact: true })).toBeVisible();

  await page.evaluate(() => {
    const fixture = window as typeof window & {
      emitHistoryMessage(message: unknown): void;
      historyGraphs: Record<string, unknown>;
    };
    const commitA = "a".repeat(40);
    fixture.emitHistoryMessage({
      type: "comparison",
      commit: commitA,
      parent: "",
      realization: "r-a",
      fingerprint: "f-a",
      currentGraph: fixture.historyGraphs[commitA],
      parentGraph: fixture.historyGraphs[commitA],
      semanticDiff: { findings: [{ summary: "Stale comparison" }] }
    });
    fixture.emitHistoryMessage({
      type: "changeCounts",
      commit: commitA,
      counts: {
        schema: "compass.history.change_counts/1",
        commit: commitA,
        parent: "",
        counts: {
          nodes: { added: 99, removed: 0, changed: 0 },
          edges: { added: 99, removed: 0, changed: 0 },
          hyperedges: { added: 99, removed: 0, changed: 0 }
        }
      }
    });
    fixture.emitHistoryMessage({
      type: "communityError",
      requestId: "stale",
      commit: commitA,
      communityId: 0,
      message: "Stale community failure"
    });
    fixture.emitHistoryMessage({
      type: "error",
      operation: "Compare revisions",
      commit: commitA,
      message: "Revision A comparison failed"
    });
  });

  await page.waitForTimeout(250);
  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();
  await expect(page.locator("small").filter({ hasText: "Revision B graph" })).toBeVisible();
  await expect(page.getByText("nodes +2 −0 ~1", { exact: true })).toBeVisible();
  await expect(page.getByText("Semantic change findings")).toHaveCount(0);
  await expect(page.getByText("Stale community failure")).toHaveCount(0);
  await expect(page.getByText("Revision A comparison failed")).toHaveCount(0);

  await page.getByRole("option", { name: /Revision A graph/i }).click();
  await expect(page.getByText("Revision A comparison failed")).toHaveCount(0);
  await expect(page.getByText("Semantic change findings")).toHaveCount(0);
});

test("unavailable comparison explains how to recover", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("option", { name: /Revision C needs build/i }).click();
  await expect(page.getByRole("button", { name: /Compare parent 1/i })).toBeDisabled();
  await expect(page.getByText("Comparison unavailable: build this revision first.")).toBeVisible();
});

test("selected operation errors stay beside the commit and out of semantic findings", async ({ page }) => {
  await page.goto("/history.html");
  await page.evaluate(() => {
    const fixture = window as typeof window & {
      emitHistoryMessage(message: unknown): void;
    };
    fixture.emitHistoryMessage({
      type: "error",
      operation: "Load graph",
      commit: "a".repeat(40),
      message: "Fixture graph load failed"
    });
  });
  await expect(page.getByRole("alert")).toContainText(
    "Load graph: Fixture graph load failed"
  );
  await expect(page.getByText("Semantic change findings")).toHaveCount(0);
});

test("cancelled build returns the revision action to idle", async ({ page }) => {
  await page.goto("/history.html?build=cancel");
  await page.getByRole("option", { name: /Revision C needs build/i }).click();
  await page.getByRole("button", { name: "Build graph" }).click();
  await expect(page.getByRole("button", { name: "Choosing profile…" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Build graph" })).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("failed build reports recovery and permits retry", async ({ page }) => {
  await page.goto("/history.html?build=fail");
  await page.getByRole("option", { name: /Revision C needs build/i }).click();
  await page.getByRole("button", { name: "Build graph" }).click();
  await expect(page.getByRole("button", { name: "Choosing profile…" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Building…" })).toBeDisabled();
  await expect(page.getByRole("alert")).toContainText("Build failed: Fixture build failed");
  await expect(page.getByRole("button", { name: "Retry build" })).toBeEnabled();
});

test("successful build refreshes availability and comparison controls", async ({ page }) => {
  await page.goto("/history.html?build=success");
  await page.getByRole("option", { name: /Revision C needs build/i }).click();
  await page.getByRole("button", { name: "Build graph" }).click();
  await expect(page.getByRole("button", { name: "Building…" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Open graph" })).toBeEnabled();
  await expect(page.getByRole("button", { name: /Compare parent 1/i })).toBeEnabled();
  await expect(page.getByText(/Comparison unavailable/)).toHaveCount(0);
});
