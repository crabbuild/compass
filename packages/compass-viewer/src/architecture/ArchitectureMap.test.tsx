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
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: memoryStorage()
    });
  });

  it("renders keyboard-accessible directed routes and a table alternative", () => {
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={vi.fn()} />);
    expect(screen.getByRole("region", { name: "Scrollable architecture flow diagram" }))
      .toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("group", { name: /2 subsystems and 1 directed routes/i }))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: /api to storage, 30 calls/i }))
      .toBeInTheDocument();
    expect(screen.getByText("View routes as a table")).toBeInTheDocument();
  });

  it("keeps an intrinsic canvas width and zooms without fitting it to the panel", () => {
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={vi.fn()} />);
    const map = screen.getByRole("group", { name: /2 subsystems and 1 directed routes/i });
    const canvas = map.parentElement;

    expect(map).toHaveAttribute("viewBox", expect.stringMatching(/^0 0 1280 /));
    expect(canvas).toHaveStyle({ width: "1280px" });
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(canvas).toHaveStyle({ width: "1600px" });
    expect(screen.getByRole("button", { name: "Reset zoom and scroll position" }))
      .toBeInTheDocument();
  });

  it("reports route selection from pointer activation", () => {
    const onSelect = vi.fn();
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: /api to storage, 30 calls/i }));
    expect(onSelect).toHaveBeenCalledWith({ kind: "route", id: "api→storage" });
  });

  it("repositions a dragged subsystem, reconnects its route, and remembers the drop", () => {
    render(<ArchitectureMap overview={overview} selection={undefined} onSelect={vi.fn()} />);
    const map = screen.getByRole("group", { name: /2 subsystems and 1 directed routes/i });
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
      "compass.architecture.layout.v2:Fixture:production:all"
    )).toContain("\"api\"");
    expect(screen.getByRole("button", { name: "Reset subsystem positions" }))
      .not.toBeDisabled();
  });

  it("starts with key routes and reveals the complete map on demand", () => {
    const denseOverview: ArchitectureOverview = {
      ...overview,
      sections: [
        overview.sections[0]!,
        ...Array.from({ length: 18 }, (_, index) => ({
          ...overview.sections[1]!,
          id: `storage-${index}`,
          name: `Storage ${index}`
        }))
      ],
      routes: Array.from({ length: 18 }, (_, index) => ({
        ...overview.routes[0]!,
        id: `api→storage-${index}`,
        targetSection: `storage-${index}`,
        calls: index + 1
      }))
    };
    render(
      <ArchitectureMap
        overview={denseOverview}
        selection={undefined}
        onSelect={vi.fn()}
      />
    );

    expect(screen.getByRole("group", {
      name: /19 subsystems and 16 of 18 directed routes visible/i
    })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "All routes · 18" }));
    expect(screen.getByRole("group", {
      name: /19 subsystems and 18 directed routes/i
    })).toBeInTheDocument();
  });

  it("shows only the selected subsystem neighborhood in focused mode", () => {
    const extraSections = Array.from({ length: 8 }, (_, index) => ({
      ...overview.sections[1]!,
      id: `service-${index}`,
      name: `Service ${index}`
    }));
    const selectedRoutes = extraSections.slice(0, 2).map((target, index) => ({
      ...overview.routes[0]!,
      id: `api-service-${index}`,
      targetSection: target.id,
      calls: 2 + index
    }));
    const unrelatedRoutes = extraSections.slice(2).map((target, index) => ({
      ...overview.routes[0]!,
      id: `unrelated-${index}`,
      sourceSection: extraSections[0]!.id,
      targetSection: target.id,
      calls: 100 + index
    }));
    render(
      <ArchitectureMap
        overview={{
          ...overview,
          sections: [overview.sections[0]!, ...extraSections],
          routes: [...selectedRoutes, ...unrelatedRoutes]
        }}
        selection={{ kind: "section", id: "api" }}
        onSelect={vi.fn()}
      />
    );

    expect(screen.getByRole("button", { name: "Neighbors" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    expect(screen.getByRole("group", {
      name: /3 subsystems and 2 of 8 directed routes visible/i
    })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Service 2,/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /service-0 to service-2/i }))
      .not.toBeInTheDocument();
  });

  it("explains an empty filtered architecture instead of leaving a blank canvas", () => {
    render(
      <ArchitectureMap
        overview={{
          ...overview,
          sections: overview.sections.map((section) => ({
            ...section,
            nodeCount: 0,
            incomingCalls: 0,
            outgoingCalls: 0
          })),
          routes: []
        }}
        selection={undefined}
        onSelect={vi.fn()}
      />
    );

    expect(screen.getByText("No architecture to draw")).toBeInTheDocument();
    expect(screen.getByText(/No subsystems match the current scope/i)).toBeInTheDocument();
  });
});

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key) {
      return values.get(key) ?? null;
    },
    key(index) {
      return [...values.keys()][index] ?? null;
    },
    removeItem(key) {
      values.delete(key);
    },
    setItem(key, value) {
      values.set(key, value);
    }
  };
}
