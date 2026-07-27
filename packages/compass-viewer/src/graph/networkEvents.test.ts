import { describe, expect, it, vi } from "vitest";
import {
  bindGraphNetworkEvents,
  type GraphNetworkEvent,
  type GraphNetworkEventSource
} from "./networkEvents";

function fixture() {
  const listeners = new Map<string, (event: GraphNetworkEvent) => void>();
  const network: GraphNetworkEventSource = {
    on(event, callback) {
      listeners.set(event, callback);
    }
  };
  const handlers = {
    onFocus: vi.fn(),
    onOpenSource: vi.fn(),
    onHover: vi.fn(),
    onHoverEdge: vi.fn(),
    onBlurEdge: vi.fn(),
    onZoom: vi.fn(),
    onClear: vi.fn()
  };
  bindGraphNetworkEvents(network, handlers);
  return { listeners, handlers };
}

const event = (nodes: Array<string | number>): GraphNetworkEvent => ({
  nodes,
  edges: [],
  pointer: { DOM: { x: 10, y: 20 } }
});

describe("bindGraphNetworkEvents", () => {
  it("keeps single-click selection separate from double-click source opening", () => {
    const { listeners, handlers } = fixture();
    listeners.get("click")?.(event(["run"]));
    expect(handlers.onFocus).toHaveBeenCalledWith("run");
    expect(handlers.onOpenSource).not.toHaveBeenCalled();

    listeners.get("doubleClick")?.(event(["run"]));
    expect(handlers.onOpenSource).toHaveBeenCalledWith("run");
  });

  it("ignores a double-click without a node", () => {
    const { listeners, handlers } = fixture();
    listeners.get("doubleClick")?.(event([]));
    expect(handlers.onOpenSource).not.toHaveBeenCalled();
  });

  it("forwards edge hover and clears it on blur", () => {
    const { listeners, handlers } = fixture();
    listeners.get("hoverEdge")?.({
      ...event([]),
      edge: 7,
      edges: [7]
    });
    expect(handlers.onHoverEdge).toHaveBeenCalledWith("7");

    listeners.get("blurEdge")?.(event([]));
    expect(handlers.onBlurEdge).toHaveBeenCalledTimes(1);
  });

  it("reports zoom scale and clears transient hover state", () => {
    const { listeners, handlers } = fixture();
    listeners.get("zoom")?.({
      ...event([]),
      scale: 1.25
    });

    expect(handlers.onHover).toHaveBeenCalledWith(null);
    expect(handlers.onBlurEdge).toHaveBeenCalledTimes(1);
    expect(handlers.onZoom).toHaveBeenCalledWith(1.25);
  });

  it("clears node and edge hover when dragging starts", () => {
    const { listeners, handlers } = fixture();
    listeners.get("dragStart")?.(event([]));

    expect(handlers.onHover).toHaveBeenCalledWith(null);
    expect(handlers.onBlurEdge).toHaveBeenCalledTimes(1);
  });
});
