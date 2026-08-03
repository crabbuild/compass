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
    onOpenRelationshipSource: vi.fn(),
    onHover: vi.fn(),
    onHoverEdge: vi.fn(),
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

  it("opens an edge relationship source on double-click", () => {
    const { listeners, handlers } = fixture();
    listeners.get("doubleClick")?.({
      ...event([]),
      edges: ["caller-callee"]
    });
    expect(handlers.onOpenRelationshipSource).toHaveBeenCalledWith("caller-callee");
    expect(handlers.onOpenSource).not.toHaveBeenCalled();
  });

  it("clears graph focus on edge clicks without ending the active hover", () => {
    const { listeners, handlers } = fixture();
    listeners.get("click")?.({
      ...event([]),
      edges: [7]
    });
    expect(handlers.onClear).toHaveBeenCalledTimes(1);
    expect(handlers.onHoverEdge).not.toHaveBeenCalledWith(null);
  });

  it("clears both graph focus and edge hover on background clicks", () => {
    const { listeners, handlers } = fixture();
    listeners.get("click")?.(event([]));
    expect(handlers.onClear).toHaveBeenCalledTimes(1);
    expect(handlers.onHoverEdge).toHaveBeenCalledWith(null);
  });

  it("forwards edge hover and clears it on blur", () => {
    const { listeners, handlers } = fixture();
    listeners.get("hoverEdge")?.({
      ...event([]),
      edge: 7,
      edges: [7]
    });
    expect(handlers.onHover).toHaveBeenCalledWith(null);
    expect(handlers.onHoverEdge).toHaveBeenCalledWith({ edgeId: "7", x: 10, y: 20 });

    listeners.get("blurEdge")?.(event([]));
    expect(handlers.onHoverEdge).toHaveBeenLastCalledWith(null);
  });

  it("clears transient hover on zoom without changing label visibility", () => {
    const { listeners, handlers } = fixture();
    listeners.get("zoom")?.({
      ...event([]),
      scale: 1.25
    });

    expect(handlers.onHover).toHaveBeenCalledWith(null);
    expect(handlers.onHoverEdge).toHaveBeenCalledWith(null);
  });

  it("clears node and edge hover when dragging starts", () => {
    const { listeners, handlers } = fixture();
    listeners.get("dragStart")?.(event([]));

    expect(handlers.onHover).toHaveBeenCalledWith(null);
    expect(handlers.onHoverEdge).toHaveBeenCalledWith(null);
  });

  it("clears an edge card before showing a node card", () => {
    const { listeners, handlers } = fixture();
    listeners.get("hoverNode")?.({
      ...event(["run"]),
      node: "run"
    });

    expect(handlers.onHoverEdge).toHaveBeenCalledWith(null);
    expect(handlers.onHover).toHaveBeenCalledWith({ nodeId: "run", x: 10, y: 20 });
  });
});
