// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GraphToolbar } from "./GraphToolbar";

function renderToolbar(overrides: Partial<Parameters<typeof GraphToolbar>[0]> = {}) {
  const callbacks = {
    onTogglePhysics: vi.fn(),
    onLayoutChange: vi.fn(),
    onZoomOut: vi.fn(),
    onResetZoom: vi.fn(),
    onZoomIn: vi.fn(),
    onFit: vi.fn(),
    onFitSelection: vi.fn(),
    onReset: vi.fn(),
    onToggleLabels: vi.fn(),
    onToggleEdgeLabels: vi.fn(),
    onToggleIsolation: vi.fn(),
    onNeighborhoodDepthChange: vi.fn(),
    onEdgeDirectionChange: vi.fn(),
    onLayoutSpacingChange: vi.fn(),
    onToggleMinimap: vi.fn()
  };
  render(<GraphToolbar
    status="Layout paused"
    physicsRunning={false}
    layoutStyle="automatic"
    forceLabels={false}
    showEdgeLabels={false}
    hasSelection={false}
    isolateSelection={false}
    neighborhoodDepth={1}
    edgeDirection="both"
    layoutSpacing={1}
    showMinimap={true}
    {...callbacks}
    {...overrides}
  />);
  return callbacks;
}

describe("GraphToolbar", () => {
  afterEach(cleanup);

  it("provides explicit camera controls", () => {
    const callbacks = renderToolbar();

    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    fireEvent.click(screen.getByRole("button", { name: "Reset zoom to 100%" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    fireEvent.click(screen.getByRole("button", { name: "Fit graph in view" }));

    expect(callbacks.onZoomOut).toHaveBeenCalledOnce();
    expect(callbacks.onResetZoom).toHaveBeenCalledOnce();
    expect(callbacks.onZoomIn).toHaveBeenCalledOnce();
    expect(callbacks.onFit).toHaveBeenCalledOnce();
  });

  it("only fits a selected neighborhood when a node is selected", () => {
    const unavailable = renderToolbar();
    expect(screen.getByRole("button", {
      name: "Fit selected neighborhood"
    })).toBeDisabled();
    cleanup();

    const available = renderToolbar({ hasSelection: true });
    fireEvent.click(screen.getByRole("button", {
      name: "Fit selected neighborhood"
    }));

    expect(unavailable.onFitSelection).not.toHaveBeenCalled();
    expect(available.onFitSelection).toHaveBeenCalledOnce();
  });

  it("exposes node and relationship labels as independent toggles", () => {
    const callbacks = renderToolbar({
      forceLabels: true,
      showEdgeLabels: true
    });
    const nodeLabels = screen.getByRole("button", { name: "Hide labels" });
    const edgeLabels = screen.getByRole("button", {
      name: "Hide relationship labels"
    });

    expect(nodeLabels).toHaveAttribute("aria-pressed", "true");
    expect(edgeLabels).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(nodeLabels);
    fireEvent.click(edgeLabels);

    expect(callbacks.onToggleLabels).toHaveBeenCalledOnce();
    expect(callbacks.onToggleEdgeLabels).toHaveBeenCalledOnce();
  });

  it("groups advanced exploration controls in a discoverable panel", () => {
    const callbacks = renderToolbar({ hasSelection: true });
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));

    fireEvent.click(screen.getByRole("button", { name: "Isolate selection" }));
    fireEvent.click(screen.getByRole("button", { name: "2 hops" }));
    fireEvent.click(screen.getByRole("button", { name: "Outgoing edges" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Layout spacing" }), {
      target: { value: "1.25" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Show minimap" }));

    expect(callbacks.onToggleIsolation).toHaveBeenCalledOnce();
    expect(callbacks.onNeighborhoodDepthChange).toHaveBeenCalledWith(2);
    expect(callbacks.onEdgeDirectionChange).toHaveBeenCalledWith("outgoing");
    expect(callbacks.onLayoutSpacingChange).toHaveBeenCalledWith(1.25);
    expect(callbacks.onToggleMinimap).toHaveBeenCalledOnce();
  });

  it("lets users prepare neighborhood settings before selecting a node", () => {
    const callbacks = renderToolbar();
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));

    expect(screen.getByRole("note")).toHaveTextContent("Select a node to isolate it");
    expect(screen.getByRole("button", { name: "Isolate selection" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "2 hops" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Outgoing edges" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "2 hops" }));
    fireEvent.click(screen.getByRole("button", { name: "Outgoing edges" }));
    expect(callbacks.onNeighborhoodDepthChange).toHaveBeenCalledWith(2);
    expect(callbacks.onEdgeDirectionChange).toHaveBeenCalledWith("outgoing");
  });

  it("makes layout motion an explicit labeled action", () => {
    const callbacks = renderToolbar();
    const resume = screen.getByRole("button", { name: "Resume layout" });
    expect(resume).toHaveTextContent("Resume");
    fireEvent.click(resume);
    expect(callbacks.onTogglePhysics).toHaveBeenCalledOnce();
  });

  it("opens the shortcut guide with question mark", () => {
    renderToolbar();
    fireEvent.keyDown(document, { key: "?" });
    expect(screen.getByLabelText("Graph keyboard shortcuts")).toBeVisible();
  });
});
