import { expect, test } from "@playwright/test";

test("timeline automatically opens an available revision without building", async ({ page }) => {
  await page.goto("/history.html?load=manual");
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.filter((message) => message.type === "loadRevision").length;
  })).toBe(1);
  const loadingState = page.locator(
    ".history-graph-frame > .workbench-state[data-kind='running']"
  );
  await expect(
    loadingState.getByRole("heading", { name: "Loading Revision A graph", exact: true })
  ).toBeVisible();
  await expect(loadingState).toContainText("Compass is opening the stored graph");
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
  await expect(page.getByLabel("Revision A graph").getByText("graph available")).toBeVisible();
  await page.evaluate(() => {
    (window as typeof window & { releaseHistoryGraph(): void }).releaseHistoryGraph();
  });
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
});

test("historical graph lazily enters a community and returns to its overview", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
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
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision B graph/i }).click();

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

  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision A graph/i }).click();
  await expect(page.getByText("Revision A comparison failed")).toHaveCount(0);
  await expect(page.getByText("Semantic change findings")).toHaveCount(0);
});

test("comparison replaces the full graph with a readable focused delta", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision B graph/i }).click();
  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();

  await page.getByRole("button", { name: /Compare revisions/i }).click();

  await expect(page.getByText("Comparison mode")).toBeVisible();
  await expect(page.getByText(/Comparing bbbbbbbbb to aaaaaaaaa/)).toBeVisible();
  await expect(page.getByLabel("Visible graph delta")).toContainText(
    "nodesAdded 0Removed 0Changed 1"
  );
  await expect(page.getByText("Cargo.toml")).toBeVisible();
  await expect(page.locator(".history-source-diff")).toBeVisible();
  await expect(
    page.locator(".history-source-diff").getByText('version = "3.1.7"', { exact: false })
  ).toBeVisible();
  await expect(
    page.locator(".history-source-diff").getByText('name = "compass"', { exact: false }).first()
  ).toBeVisible();
  await expect(page.locator(".history-source-diff [data-line]")).toHaveCount(6);
  const diffLayout = await page.locator(".history-source-diff").evaluate((element) => {
    const shadowRoot = element.shadowRoot;
    const gutter = shadowRoot?.querySelector<HTMLElement>("[data-gutter]");
    const content = shadowRoot?.querySelector<HTMLElement>("[data-content]");
    const lineTops = [...(shadowRoot?.querySelectorAll<HTMLElement>(
      "[data-additions] [data-line]"
    ) ?? [])].map((line) => line.getBoundingClientRect().top);
    return {
      gutterDisplay: gutter ? getComputedStyle(gutter).display : null,
      contentDisplay: content ? getComputedStyle(content).display : null,
      distinctLineTops: new Set(lineTops).size,
      height: element.getBoundingClientRect().height
    };
  });
  expect(diffLayout).toMatchObject({
    gutterDisplay: "grid",
    contentDisplay: "grid",
    distinctLineTops: 3
  });
  expect(diffLayout.height).toBeGreaterThan(75);
  await expect(page.getByRole("button", { name: "Split" })).toHaveAttribute(
    "aria-pressed",
    "true"
  );

  await page.getByRole("tab", { name: /Changed graph/ }).click();
  await expect(page.getByText(/Viewing changed subgraph for bbbbbbbbb/)).toBeVisible();
  await expect(page.getByLabel("Graph change filters")).toContainText("Changed1");

  const graphSearch = page.getByRole("combobox", { name: "Search graph nodes" });
  await graphSearch.fill("run");
  await page.getByRole("option", { name: /run/i }).click();
  await expect(page.getByRole("heading", { name: "What changed" })).toBeVisible();
  await expect(page.locator(".compass-record-evidence").getByText("signature")).toBeVisible();
  await expect(page.getByRole("region", { name: "Inspector" }).getByText("src/lib.rs"))
    .toBeVisible();

  await page.getByRole("tab", { name: /Semantic findings/ }).click();
  await expect(page.getByText("Fixture comparison", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Exit comparison" }).click();
  await expect(page.getByText("Comparison mode")).toHaveCount(0);
  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.find((message) => message.type === "exitComparison");
  })).toMatchObject({
    type: "exitComparison",
    commit: "b".repeat(40)
  });
});

test("rejects an unknown semantic report before drawing a versioned graph delta", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision B graph/i }).click();
  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();
  await page.getByRole("button", { name: /Compare revisions/i }).click();
  await expect(page.getByText("Comparison mode")).toBeVisible();

  await page.evaluate(() => {
    const fixture = window as typeof window & {
      emitHistoryMessage(message: unknown): void;
      historyGraphs: Record<string, unknown>;
    };
    const commit = "b".repeat(40);
    const parent = "a".repeat(40);
    fixture.emitHistoryMessage({
      type: "comparison",
      commit,
      parent,
      realization: "r-b",
      fingerprint: "f-b",
      parentRealization: "r-a",
      parentFingerprint: "f-a",
      currentGraph: fixture.historyGraphs[commit],
      parentGraph: fixture.historyGraphs[parent],
      semanticDiff: {
        schema: "compass.semantic_diff.report/2",
        graph_delta: {
          added_nodes: [{ id: "invented", label: "Invented" }]
        }
      }
    });
  });

  await expect(page.getByRole("alert")).toContainText(
    "unsupported or malformed versioned graph comparison"
  );
  await expect(page.getByText("Comparison mode")).toHaveCount(0);
  await expect(page.getByText("Invented", { exact: true })).toHaveCount(0);
});

test("revision actions share a common control baseline", async ({ page }) => {
  await page.goto("/history.html");

  const alignment = await page.evaluate(() => {
    const query = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Query this revision"));
    const picker = document.querySelector<HTMLSelectElement>(
      '[aria-label="Comparison revision"]'
    );
    const compare = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Compare revisions"));
    if (!query || !picker || !compare) throw new Error("revision actions did not render");
    return {
      query: query.getBoundingClientRect(),
      picker: picker.getBoundingClientRect(),
      compare: compare.getBoundingClientRect()
    };
  });

  expect(Math.abs(alignment.query.top - alignment.picker.top)).toBeLessThanOrEqual(1);
  expect(Math.abs(alignment.query.bottom - alignment.picker.bottom)).toBeLessThanOrEqual(1);
  expect(Math.abs(alignment.compare.top - alignment.picker.top)).toBeLessThanOrEqual(1);
  expect(Math.abs(alignment.compare.bottom - alignment.picker.bottom)).toBeLessThanOrEqual(1);
});

test("unavailable comparison offers to build the selected graph", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision C needs build/i }).click();
  await expect(page.getByText("Graph not built for this revision")).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.some((message) => message.type === "buildRevision");
  })).toBe(false);
  await expect(page.getByRole("button", { name: /Build selected graph/i })).toBeEnabled();
  await expect(page.getByText(/Build the selected revision graph/)).toBeVisible();
});

test("selected operation errors stay beside the commit and out of semantic findings", async ({ page }) => {
  await page.goto("/history.html?load=error");
  await expect(page.getByRole("alert")).toContainText("Fixture graph load failed");
  await expect(page.getByRole("button", { name: "Retry load" })).toBeVisible();
  await expect(page.getByText("Semantic change findings")).toHaveCount(0);
});

test("cancelled build returns the revision action to idle", async ({ page }) => {
  await page.goto("/history.html?build=cancel");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision C needs build/i }).click();
  await page.getByRole("button", { name: "Build graph" }).click();
  await expect(page.getByRole("status")).toContainText("Choosing a build profile");
  await expect(page.getByRole("button", { name: "Build graph" })).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("failed build reports recovery and permits retry", async ({ page }) => {
  await page.goto("/history.html?build=fail");
  await page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision C needs build/i }).click();
  await page.getByRole("button", { name: "Build graph" }).click();
  await expect(page.getByRole("status")).toContainText("Choosing a build profile");
  await expect(page.getByRole("status")).toContainText("Building revision graph");
  await expect(page.getByRole("alert")).toContainText("Fixture build failed");
  await expect(page.getByRole("button", { name: "Retry build" })).toBeEnabled();
});

test("a missing arbitrary baseline can be built and compared in place", async ({ page }) => {
  await page.goto("/history.html?build=success");
  const comparisonRevision = page.getByRole("combobox", { name: "Comparison revision" });
  await comparisonRevision.selectOption("c".repeat(40));
  await page.getByRole("button", { name: /Build baseline graph/i }).click();
  await expect(page.getByRole("button", { name: /Building baseline graph/i })).toBeDisabled();
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
  await expect(page.getByRole("button", { name: /Compare revisions/i })).toBeEnabled();
  await page.getByRole("button", { name: /Compare revisions/i }).click();
  await expect(page.getByText(/Comparing aaaaaaaaa to ccccccccc/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.findLast((message) => message.type === "compare");
  })).toMatchObject({
    type: "compare",
    commit: "a".repeat(40),
    parent: "c".repeat(40)
  });
});

test("history bootstrap failure offers a working retry", async ({ page }) => {
  await page.goto("/history.html?bootstrap=error");
  await expect(page.getByRole("alert")).toContainText("Fixture history unavailable");
  await page.getByRole("button", { name: "Retry history" }).click();
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.filter((message) => message.type === "retryTimeline").length;
  })).toBe(1);
});

test("timeline loads another cached page on demand", async ({ page }) => {
  await page.goto("/history.html?pagination=true");
  await expect(page.getByText("2 loaded commits")).toBeVisible();
  await page.getByRole("button", { name: "Load more commits" }).click();
  await expect(page.getByText("3 reachable commits")).toBeVisible();
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision C needs build/i })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.filter((message) => message.type === "loadMoreTimeline").length;
  })).toBe(1);
});

test("a late commit page cannot overwrite a newer timeline generation", async ({ page }) => {
  await page.goto("/history.html?pagination=true");
  await page.getByRole("button", { name: "Load more commits" }).click();
  await page.evaluate(() => {
    const fixture = window as typeof window & {
      fixtureTimeline: { entries: unknown[] };
      historyGeneration: number;
      emitHistoryMessage(message: unknown): void;
    };
    fixture.historyGeneration = 2;
    fixture.emitHistoryMessage({
      type: "timeline",
      repositoryId: "fixture",
      generation: 2,
      timeline: {
        ...fixture.fixtureTimeline,
        totalEntries: null,
        hasMore: true,
        nextCursor: "new-cursor",
        entries: fixture.fixtureTimeline.entries.slice(0, 2)
      }
    });
  });
  await page.waitForTimeout(120);
  await expect(page.getByRole("listbox", { name: "Git commit timeline" })
    .getByRole("option", { name: /Revision C needs build/i })).toHaveCount(0);
  await expect(page.getByText("2 loaded commits")).toBeVisible();
});

test("disabled history can be enabled from Codebase Evolution", async ({ page }) => {
  await page.goto("/history.html?historyEnabled=false");
  await expect(page.getByText("Revision graphs are not enabled")).toBeVisible();
  await page.getByRole("button", { name: "Enable revision graphs" }).click();
  await expect(page.getByRole("status")).toContainText("Enabling revision graphs");
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const messages = (window as typeof window & {
      historyHostMessages: Array<Record<string, unknown>>;
    }).historyHostMessages;
    return messages.filter((message) => message.type === "enableHistory").length;
  })).toBe(1);
});
