// @vitest-environment jsdom

import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { GraphSemanticLegend } from "./GraphSemanticLegend";

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Semantic fixture",
  stats: { nodes: 3, edges: 2, communities: 1, aggregated: false },
  nodes: [
    { id: "run", label: "run", community: 0, kind: "function" },
    { id: "store", label: "Store", community: 0, kind: "class" },
    { id: "api", label: "api", community: 0, kind: "module" }
  ],
  edges: [
    { id: "run-store", source: "run", target: "store", relation: "calls" },
    { id: "api-run", source: "api", target: "run", relation: "imports" }
  ],
  communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("GraphSemanticLegend", () => {
  afterEach(cleanup);

  it("explains only the node and edge categories present in the subgraph", () => {
    render(<GraphSemanticLegend model={model} />);
    const legend = screen.getByRole("complementary", { name: "Graph visual legend" });
    expect(within(legend).getByText("Callable")).toBeTruthy();
    expect(within(legend).getByText("Type")).toBeTruthy();
    expect(within(legend).getByText("Module / file")).toBeTruthy();
    expect(within(legend).getByText("Execution")).toBeTruthy();
    expect(within(legend).getByText("Dependency")).toBeTruthy();
    expect(within(legend).queryByText("Boundary / data")).toBeNull();
    expect(within(legend).queryByText("Data / event flow")).toBeNull();
  });
});
