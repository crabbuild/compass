import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  beforeEach(() => {
    window.localStorage.clear();
  });

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

  it("repositions a dragged subsystem, reconnects its route, and remembers the drop", () => {
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={vi.fn()} />);
    const map = screen.getByRole("img", { name: /2 subsystems and 1 directed routes/i });
    const api = screen.getByRole("button", { name: /^API, 20 visible symbols/i });
    const initialTransform = api.getAttribute("transform");
    Object.defineProperties(map, {
      getScreenCTM: {
        value: () => ({ inverse: () => ({}) })
      },
      createSVGPoint: {
        value: () => ({
          x: 0,
          y: 0,
          matrixTransform(this: { x: number; y: number }) {
            return { x: this.x, y: this.y };
          }
        })
      }
    });
    Object.defineProperties(api, {
      setPointerCapture: { value: vi.fn() },
      hasPointerCapture: { value: () => true },
      releasePointerCapture: { value: vi.fn() }
    });

    fireEvent.pointerDown(api, { pointerId: 7, clientX: 356, clientY: 392 });
    fireEvent.pointerMove(api, { pointerId: 7, clientX: 486, clientY: 472 });
    fireEvent.pointerUp(api, { pointerId: 7, clientX: 486, clientY: 472 });

    expect(api.getAttribute("transform")).not.toBe(initialTransform);
    expect(window.localStorage.getItem(
      "compass.architecture.layout.v1:Fixture:production:all"
    )).toContain("\"api\"");
    expect(screen.getByRole("button", { name: "Reset subsystem positions" }))
      .not.toBeDisabled();
  });
});
