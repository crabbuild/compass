import type { GraphHover } from "./NodeHoverCard";

export type GraphNetworkEvent = {
  nodes: Array<string | number>;
  node?: string | number;
  pointer: { DOM: { x: number; y: number } };
};

export type GraphNetworkEventSource = {
  on(event: string, callback: (parameters: GraphNetworkEvent) => void): void;
};

export type GraphNetworkHandlers = {
  onFocus(nodeId: string): void;
  onOpenSource(nodeId: string): void;
  onHover(change: GraphHover | null): void;
  onClear(): void;
};

export function bindGraphNetworkEvents(
  network: GraphNetworkEventSource,
  handlers: GraphNetworkHandlers
): void {
  network.on("click", (parameters) => {
    handlers.onHover(null);
    const selected = parameters.nodes[0];
    if (selected !== undefined) handlers.onFocus(String(selected));
    else handlers.onClear();
  });
  network.on("doubleClick", (parameters) => {
    const selected = parameters.nodes[0];
    if (selected !== undefined) handlers.onOpenSource(String(selected));
  });
  network.on("hoverNode", (parameters) => {
    if (parameters.node === undefined) return;
    handlers.onHover({
      nodeId: String(parameters.node),
      x: parameters.pointer.DOM.x,
      y: parameters.pointer.DOM.y
    });
  });
  network.on("blurNode", () => handlers.onHover(null));
  network.on("dragStart", () => handlers.onHover(null));
  network.on("zoom", () => handlers.onHover(null));
}
