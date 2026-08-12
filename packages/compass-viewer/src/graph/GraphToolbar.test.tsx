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
    onToggleEdgeLabels: vi.fn()
  };
  render(<GraphToolbar
    status="Layout paused"
    physicsRunning={false}
    layoutStyle="automatic"
    forceLabels={false}
    showEdgeLabels={false}
    hasSelection={false}
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
});
