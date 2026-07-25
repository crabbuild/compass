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
    onClear: vi.fn()
  };
  bindGraphNetworkEvents(network, handlers);
  return { listeners, handlers };
}

const event = (nodes: Array<string | number>): GraphNetworkEvent => ({
  nodes,
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
});
