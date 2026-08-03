import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { GraphNode } from "../contracts/graph";
import { NodeHoverCard } from "./NodeHoverCard";

const node: GraphNode = {
  id: "node-1",
  label: "requestHandler",
  kind: "function",
  community: 3,
  source: {
    file: "src/request.ts",
    startLine: 12,
    endLine: 18
  }
};

const hover = { nodeId: node.id, x: 100, y: 80 };

describe("NodeHoverCard", () => {
  it("hints that an aggregated community opens its subgraph", () => {
    const markup = renderToStaticMarkup(
      <NodeHoverCard
        node={{ ...node, memberCount: 24 }}
        hover={hover}
        activation={{ type: "community", communityId: 3 }}
      />
    );

    expect(markup).toContain("Double-click");
    expect(markup).toContain("to open community subgraph");
    expect(markup).toContain("lucide-network");
  });

  it("hints that a concrete node opens its source code", () => {
    const markup = renderToStaticMarkup(
      <NodeHoverCard
        node={node}
        hover={hover}
        activation={{ type: "source", source: node.source! }}
      />
    );

    expect(markup).toContain("Double-click");
    expect(markup).toContain("to open source code");
    expect(markup).toContain("lucide-file-code-2");
  });

  it("does not show an action hint for non-navigable nodes", () => {
    const markup = renderToStaticMarkup(
      <NodeHoverCard
        node={{ ...node, source: undefined }}
        hover={hover}
        activation={{ type: "none" }}
      />
    );

    expect(markup).not.toContain("Double-click");
    expect(markup).not.toContain("compass-hover-hint");
  });
});
