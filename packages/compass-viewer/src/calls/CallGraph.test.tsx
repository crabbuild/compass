import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CallGraphResponse } from "../contracts/callGraph";
import { CallGraph, type CallGraphHost } from "./CallGraph";

vi.mock("./CallCanvas", () => ({
  CallCanvas: () => null
}));

const graph: CallGraphResponse = {
  schema: "compass.call_graph/1",
  rootSymbol: "resolve",
  direction: "callees",
  depth: 1,
  nodes: [{
    id: "resolve",
    symbol: "resolve",
    name: "ResolveControlPlaneTarget()",
    file: "cmd/entire/cli/auth/control_plane.go",
    startLine: 42,
    endLine: 55,
    startByte: null,
    endByte: null,
    graphNodeId: "resolve",
    unresolved: false,
    evidenceLayer: "structural_graph"
  }],
  edges: [],
  truncated: false,
  continuations: [],
  coverage: {
    resolved: 0,
    inferred: 0,
    ambiguous: 0,
    unresolved: 0,
    evidenceLayer: "structural_graph",
    partial: true,
    limitations: ["program_ir_unavailable"],
    warning: "Structural call evidence may omit unresolved calls."
  }
};

describe("CallGraph directions", () => {
  it("changes direction through the host and exposes the selected direction", async () => {
    const user = userEvent.setup();
    const host: CallGraphHost = {
      openSource: vi.fn(),
      expand: vi.fn(),
      changeDirection: vi.fn()
    };
    render(<CallGraph graph={graph} host={host} />);

    expect(screen.getByRole("button", { name: "Callees" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    await user.click(screen.getByRole("button", { name: "Callers" }));

    expect(host.changeDirection).toHaveBeenCalledWith("callers");
  });

  it("renders a valid partial empty state", () => {
    render(<CallGraph graph={graph} host={{
      openSource: vi.fn(),
      expand: vi.fn(),
      changeDirection: vi.fn()
    }} />);

    expect(screen.getByText("No callees found")).toBeInTheDocument();
    expect(screen.getByText("Partial call coverage")).toBeInTheDocument();
    expect(screen.getByText("Structural graph")).toBeInTheDocument();
  });
});
