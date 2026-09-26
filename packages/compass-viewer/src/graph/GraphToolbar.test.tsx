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
  const view = render(<GraphToolbar
    status="Layout paused"
    physicsRunning={false}
    layoutStyle="automatic"
    forceLabels={false}
    showEdgeLabels={false}
    hasSelection={false}
    isolateSelection={false}
    neighborhoodDepth={1}
    edgeDirection="both"
    layoutSpacing={2}
    showMinimap={true}
    {...callbacks}
    {...overrides}
  />);
  return { ...callbacks, ...view };
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
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));
    expect(screen.getByRole("button", {
      name: "Fit selected neighborhood"
    })).toBeDisabled();
    cleanup();

    const available = renderToolbar({ hasSelection: true });
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));
    fireEvent.click(screen.getByRole("button", {
      name: "Fit selected neighborhood"
    }));

    expect(unavailable.onFitSelection).not.toHaveBeenCalled();
    expect(available.onFitSelection).toHaveBeenCalledOnce();
  });

  it("keeps node and relationship labels as independent settings toggles", () => {
    const callbacks = renderToolbar({
      forceLabels: true,
      showEdgeLabels: true
    });
    // Labels live in the graph settings panel so the toolbar row stays readable.
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));
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
      target: { value: "3" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Show minimap" }));

    expect(callbacks.onToggleIsolation).toHaveBeenCalledOnce();
    expect(callbacks.onNeighborhoodDepthChange).toHaveBeenCalledWith(2);
    expect(callbacks.onEdgeDirectionChange).toHaveBeenCalledWith("outgoing");
    expect(callbacks.onLayoutSpacingChange).toHaveBeenCalledWith(3);
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
    const layout = screen.getByRole("button", { name: "Run layout" });
    expect(layout).toHaveTextContent("Layout");
    fireEvent.click(layout);
    expect(callbacks.onTogglePhysics).toHaveBeenCalledOnce();
  });

  it("removes depth layers and leads with the host filters", () => {
    renderToolbar({
      leadingControls: (
        <button type="button" aria-label="Graph filters">
          <span>Filters</span>
        </button>
      )
    });

    expect(screen.queryByRole("option", { name: "Depth layers" })).toBeNull();
    const settings = screen.getByRole("button", { name: "Graph settings" });
    const filters = screen.getByRole("button", { name: "Graph filters" });
    expect(settings.querySelector(".lucide-settings")).not.toBeNull();
    // The rail can scroll or wrap in a narrow host, so the view's own control
    // leads it instead of hiding behind the trailing canvas controls.
    expect(filters.compareDocumentPosition(settings) & Node.DOCUMENT_POSITION_FOLLOWING)
      .not.toBe(0);
    expect(filters.closest(".compass-toolbar-actions")?.firstElementChild)
      .toBe(filters.closest(".compass-toolbar-leading"));
  });

  it("puts Overview before Filters and offers five spacing levels on the roomier baseline", () => {
    renderToolbar({ onBack: vi.fn(), leadingControls: <button>Filters</button> });
    const back = screen.getByRole("button", { name: "Back to community overview" });
    expect(back.closest(".compass-toolbar-actions")?.firstElementChild).toBe(back);
    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));
    const spacing = screen.getByRole("combobox", { name: "Layout spacing" }) as HTMLSelectElement;
    expect([...spacing.options].map((option) => [option.value, option.text])).toEqual([
      ["1.5", "Compact · 75%"], ["2", "Default · 100%"],
      ["3", "Airy · 150%"], ["4", "Wide · 200%"], ["6", "Extra wide · 300%"]
    ]);
    expect(spacing.value).toBe("2");
  });

  it("opens the shortcut guide with question mark", () => {
    renderToolbar();
    fireEvent.keyDown(document, { key: "?" });
    expect(screen.getByLabelText("Graph keyboard shortcuts")).toBeVisible();
  });

  it("returns focus to the settings trigger when Escape closes its panel", () => {
    renderToolbar();
    const trigger = screen.getByRole("button", { name: "Graph settings" });
    fireEvent.click(trigger);
    screen.getByRole("combobox", { name: "Layout spacing" }).focus();

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.queryByRole("region", { name: "Graph exploration controls" })).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("requests that a competing leading panel close before settings open", () => {
    const onLeadingPanelClose = vi.fn();
    renderToolbar({
      leadingPanelOpen: true,
      onLeadingPanelClose
    });

    fireEvent.click(screen.getByRole("button", { name: "Graph settings" }));

    expect(onLeadingPanelClose).toHaveBeenCalledOnce();
    expect(screen.getByRole("region", { name: "Graph exploration controls" })).toBeVisible();
  });
});
