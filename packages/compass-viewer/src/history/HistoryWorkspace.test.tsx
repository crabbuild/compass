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
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {}
  }));
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
  it("puts a zero-topology comparison explanation before the graph canvas", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const parent = "b".repeat(40);
    const timeline = {
      schema: "compass.history.timeline/1" as const,
      repositoryId: "repo",
      selectedHead: commit,
      historyEnabled: true,
      totalEntries: 2,
      hasMore: false,
      nextCursor: null,
      entries: [{
        commit,
        parents: [parent],
        authorName: "Compass",
        authorEmail: "test@example.invalid",
        authoredAtSeconds: 2,
        subject: "Version bump",
        graphState: "graph_available" as const,
        presentationAvailable: true,
        realization: "current-realization",
        fingerprint: "current-fingerprint",
        job: null
      }, {
        commit: parent,
        parents: [],
        authorName: "Compass",
        authorEmail: "test@example.invalid",
        authoredAtSeconds: 1,
        subject: "Parent",
        graphState: "graph_available" as const,
        presentationAvailable: true,
        realization: "parent-realization",
        fingerprint: "parent-fingerprint",
        job: null
      }]
    };
    const emptyGraph = {
      schema: "compass.viewer.graph/1" as const,
      title: "No visible topology changes",
      stats: { nodes: 0, edges: 0, communities: 0, aggregated: false },
      nodes: [],
      edges: [],
      communities: [],
      hyperedges: []
    };
    flushSync(() => root.render(
      <HistoryWorkspace
        timeline={timeline}
        selectedCommit={commit}
        revisionLoadState="ready"
        graph={emptyGraph}
        graphCommit={commit}
        comparison={{
          parent,
          graph: emptyGraph,
          addedNodes: 0,
          removedNodes: 0,
          changedNodes: 0,
          addedEdges: 0,
          removedEdges: 0,
          changedEdges: 0
        }}
        changeCounts={{
          schema: "compass.history.change_counts/1",
          commit,
          parent,
          counts: {
            nodes: { added: 0, removed: 0, changed: 0 },
            edges: { added: 0, removed: 0, changed: 0 },
            hyperedges: { added: 0, removed: 0, changed: 0 }
          }
        }}
        onSelectCommit={vi.fn()}
        onExitComparison={vi.fn()}
        host={host()}
      />
    ));

    const explanation = Array.from(container.querySelectorAll("*"))
      .find((element) => element.textContent === "No structural graph changes");
    const graphFrame = container.querySelector(".history-graph-frame");
    expect(explanation).toBeDefined();
    expect(graphFrame).not.toBeNull();
    if (!explanation || !graphFrame) throw new Error("comparison layout did not render");
    expect(explanation.compareDocumentPosition(graphFrame) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy();
    expect(container.textContent).toContain(`Comparing ${commit.slice(0, 9)} to ${parent.slice(0, 9)}`);
    expect(container.textContent).toContain(
      "No structural changes from the first parent. Source or configuration changes may still exist"
    );
    root.unmount();
  });

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
