import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ArchitectureOverview } from "../contracts/architecture";
import { ArchitectureMap } from "./ArchitectureMap";

const overview: ArchitectureOverview = {
  title: "Fixture",
  scope: "production",
  evidence: "all",
  sections: [
    {
      id: "api", name: "API", nodeCount: 20, totalNodeCount: 22,
      internalCallCount: 12, incomingCalls: 0, outgoingCalls: 30,
      scopes: { production: 20, test: 2, generated: 0, vendor: 0, unknown: 0 }
    },
    {
      id: "storage", name: "Storage", nodeCount: 10, totalNodeCount: 10,
      internalCallCount: 5, incomingCalls: 30, outgoingCalls: 0,
      scopes: { production: 10, test: 0, generated: 0, vendor: 0, unknown: 0 }
    }
  ],
  routes: [{
    id: "api→storage", sourceSection: "api", targetSection: "storage",
    calls: 30, extracted: 24, inferred: 6, ambiguous: 0
  }],
  statistics: {
    visibleNodes: 30, totalNodes: 32, visibleCalls: 47, totalCalls: 49,
    communities: 2, extracted: 40, inferred: 9, ambiguous: 0
  },
  coverage: { internal: 19, crossSection: 30, unassigned: 0 },
  provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
};

describe("ArchitectureMap", () => {
  it("renders keyboard-accessible directed routes and a table alternative", () => {
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={vi.fn()} />);
    expect(screen.getByRole("img", { name: /2 subsystems and 1 directed routes/i }))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: /api to storage, 30 calls/i }))
      .toBeInTheDocument();
    expect(screen.getByText("View routes as a table")).toBeInTheDocument();
  });

  it("reports route selection from pointer activation", () => {
    const onSelect = vi.fn();
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: /api to storage, 30 calls/i }));
    expect(onSelect).toHaveBeenCalledWith({ kind: "route", id: "api→storage" });
  });
});
