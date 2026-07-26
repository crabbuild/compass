import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { CommitRail } from "./CommitRail";
import type { HistoryHost } from "./HistoryWorkspace";
import { HistoryWorkspace } from "./HistoryWorkspace";

const commit = "a".repeat(40);

beforeAll(() => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
});

function host(): HistoryHost {
  return {
    loadRevision: vi.fn(),
    buildRevision: vi.fn(),
    compare: vi.fn(),
    queryRevision: vi.fn(),
    loadChangeCounts: vi.fn(),
    openSource: vi.fn(),
    openCommunity: vi.fn(),
    enableHistory: vi.fn(),
    loadMore: vi.fn()
  };
}

describe("HistoryWorkspace", () => {
  it("lets the user enable revision graphs from the disabled state", () => {
    const historyHost = host();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <HistoryWorkspace
        timeline={{
          schema: "compass.history.timeline/1",
          repositoryId: "repo",
          selectedHead: commit,
          historyEnabled: false,
          totalEntries: 1,
          hasMore: false,
          nextCursor: null,
          entries: [{
            commit,
            parents: [],
            authorName: "Compass",
            authorEmail: "test@example.invalid",
            authoredAtSeconds: 1,
            subject: "Initial revision",
            graphState: "not_materialized",
            presentationAvailable: false,
            realization: null,
            fingerprint: null,
            job: null
          }]
        }}
        selectedCommit={commit}
        revisionLoadState="idle"
        onSelectCommit={vi.fn()}
        host={historyHost}
      />
    ));

    const button = Array.from(container.querySelectorAll("button"))
      .find((candidate) => candidate.textContent === "Enable revision graphs");
    button?.click();

    expect(historyHost.enableHistory).toHaveBeenCalledOnce();
    root.unmount();
  });

  it("loads another commit page when the user scrolls near the rail end", () => {
    const loadMore = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <CommitRail
        entries={Array.from({ length: 10 }, (_, index) => ({
          commit: String(index).padStart(40, "a"),
          parents: [],
          authorName: "Compass",
          authorEmail: "test@example.invalid",
          authoredAtSeconds: index,
          subject: `Revision ${index}`,
          graphState: "not_materialized" as const,
          presentationAvailable: false,
          realization: null,
          fingerprint: null,
          job: null
        }))}
        selected=""
        hasMore
        loadingMore={false}
        onLoadMore={loadMore}
        onSelect={vi.fn()}
      />
    ));
    const rail = container.querySelector<HTMLElement>('[role="listbox"]');
    expect(rail).not.toBeNull();
    if (!rail) return;
    Object.defineProperties(rail, {
      clientHeight: { value: 400 },
      scrollHeight: { value: 800 },
      scrollTop: { value: 350, configurable: true }
    });

    rail.dispatchEvent(new Event("scroll", { bubbles: true }));

    expect(loadMore).toHaveBeenCalledOnce();
    root.unmount();
  });
});
