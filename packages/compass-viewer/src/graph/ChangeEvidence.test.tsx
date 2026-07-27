import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { GraphEdge, GraphNode } from "../contracts/graph";
import { ChangeEvidence } from "./ChangeEvidence";
import { ChangedSymbolList } from "./ChangedSymbolList";

const changedNode: GraphNode = {
  id: "changed",
  label: "render",
  kind: "function",
  community: 7,
  change: "changed",
  source: { file: "src/render.ts", startLine: 8 },
  evidence: {
    before: {
      source: { file: "src/render.ts", startLine: 3 },
      signature: "render(oldValue)"
    },
    after: {
      source: { file: "src/render.ts", startLine: 8 },
      signature: "render(newValue)"
    },
    fields: [{
      field: "signature",
      before: "render(oldValue)",
      after: "render(newValue)"
    }, {
      field: "source.startLine",
      before: 3,
      after: 8
    }]
  }
};
const neighbor: GraphNode = {
  id: "caller",
  label: "main",
  community: 7,
  change: "added"
};
const edge: GraphEdge = {
  id: "calls",
  source: "caller",
  target: "changed",
  relation: "calls",
  confidence: "extracted",
  change: "changed",
  evidence: {
    before: { relation: "references" },
    after: { relation: "calls" },
    fields: [{
      field: "relation",
      before: "references",
      after: "calls"
    }]
  }
};

describe("ChangeEvidence", () => {
  it("shows field and relationship evidence and opens both source revisions", () => {
    const onOpenSource = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <ChangeEvidence
          node={changedNode}
          edges={[edge]}
          nodes={new Map([[changedNode.id, changedNode], [neighbor.id, neighbor]])}
          sourceRevisions={{ before: "parent", after: "current" }}
          onFocus={vi.fn()}
          onOpenSource={onOpenSource}
        />
      ));

    expect(container.querySelector("h3")?.textContent).toBe("What changed");
    expect(Array.from(container.querySelectorAll("th")).map((cell) => cell.textContent))
      .toContain("Before");
    expect(container.textContent).toContain("render(oldValue)");
    expect(container.textContent).toContain("render(newValue)");
    expect(container.textContent).toContain("extracted");
    expect(container.textContent).toContain("calls");

    const sourceButtons = Array.from(container.querySelectorAll<HTMLButtonElement>("button"));
    flushSync(() => sourceButtons.find((button) => button.textContent?.includes("Open before"))?.click());
    flushSync(() => sourceButtons.find((button) => button.textContent?.includes("Open after"))?.click());
    expect(onOpenSource).toHaveBeenNthCalledWith(
      1,
      { file: "src/render.ts", startLine: 3 },
      "parent"
    );
    expect(onOpenSource).toHaveBeenNthCalledWith(
      2,
      { file: "src/render.ts", startLine: 8 },
      "current"
    );
    root.unmount();
  });

  it("shows every retained field for added and removed symbols", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const renderNode = (node: GraphNode) => flushSync(() => root.render(
      <ChangeEvidence
        node={node}
        edges={[]}
        nodes={new Map([[node.id, node]])}
        onFocus={vi.fn()}
        onOpenSource={vi.fn()}
      />
    ));
    const added: GraphNode = {
      id: "added",
      label: "Added",
      community: 7,
      change: "added",
      size: 42,
      learningStatus: "learned",
      evidence: {
        after: {
          id: "added",
          label: "Added",
          community: 7,
          size: 42,
          learningStatus: "learned"
        },
        fields: [
          { field: "id", after: "added" },
          { field: "label", after: "Added" },
          { field: "community", after: 7 },
          { field: "size", after: 42 },
          { field: "learningStatus", after: "learned" }
        ]
      }
    };

    renderNode(added);
    expect(container.querySelector("h3")?.textContent).toBe("Added symbol metadata");
    expect(container.textContent).toContain("learningStatus");
    expect(container.textContent).toContain("learned");
    expect(container.textContent).toContain("size");
    expect(container.textContent).toContain("42");

    const removed: GraphNode = {
      ...added,
      id: "removed",
      label: "Removed",
      change: "removed",
      evidence: {
        before: {
          id: "removed",
          label: "Removed",
          community: 7,
          memberCount: 12
        },
        fields: [
          { field: "id", before: "removed" },
          { field: "label", before: "Removed" },
          { field: "community", before: 7 },
          { field: "memberCount", before: 12 }
        ]
      }
    };
    renderNode(removed);
    expect(container.querySelector("h3")?.textContent).toBe("Removed symbol metadata");
    expect(container.textContent).toContain("memberCount");
    expect(container.textContent).toContain("12");
    root.unmount();
  });
});

describe("ChangedSymbolList", () => {
  it("orders affected symbols by status and filters by source", () => {
    const onFocus = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <ChangedSymbolList
          nodes={[
            { ...neighbor, id: "added", label: "Zulu", source: { file: "src/z.ts" } },
            { ...changedNode, id: "changed-b", label: "Beta" },
            { ...changedNode, id: "changed-a", label: "Alpha" },
            { id: "removed", label: "Gone", community: 7, change: "removed" }
          ]}
          query=""
          onFocus={onFocus}
        />
      ));

    expect(Array.from(container.querySelectorAll("button")).map((button) => button.textContent))
      .toEqual([
      "ChangedAlphasrc/render.ts",
      "ChangedBetasrc/render.ts",
      "AddedZulusrc/z.ts",
      "RemovedGoneGraph symbol"
    ]);

    flushSync(() => root.render(
        <ChangedSymbolList
          nodes={[{ ...neighbor, id: "added", label: "Zulu", source: { file: "src/z.ts" } }]}
          query="src/z"
          onFocus={onFocus}
        />
      ));
    flushSync(() => container.querySelector<HTMLButtonElement>("button")?.click());
    expect(onFocus).toHaveBeenCalledWith("added");
    root.unmount();
  });
});
