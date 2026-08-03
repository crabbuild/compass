import type { GraphHover } from "./NodeHoverCard";
import type { GraphEdgeHover } from "./EdgeHoverCard";

export type GraphNetworkEvent = {
  nodes: Array<string | number>;
  edges: Array<string | number>;
  node?: string | number;
  edge?: string | number;
  scale?: number;
  pointer: { DOM: { x: number; y: number } };
};

export type GraphNetworkEventSource = {
  on(event: string, callback: (parameters: GraphNetworkEvent) => void): void;
};

export type GraphNetworkHandlers = {
  onFocus(nodeId: string): void;
  onOpenSource(nodeId: string): void;
  onOpenRelationshipSource(edgeId: string): void;
  onHover(change: GraphHover | null): void;
  onHoverEdge(change: GraphEdgeHover | null): void;
  onClear(): void;
};

export function bindGraphNetworkEvents(
  network: GraphNetworkEventSource,
  handlers: GraphNetworkHandlers
): void {
  network.on("click", (parameters) => {
    handlers.onHover(null);
    if (parameters.edges.length === 0) handlers.onHoverEdge(null);
    const selectedNode = parameters.nodes[0];
    if (selectedNode !== undefined) {
      handlers.onFocus(String(selectedNode));
    } else {
      handlers.onClear();
    }
  });
  network.on("doubleClick", (parameters) => {
    const selected = parameters.nodes[0];
    if (selected !== undefined) {
      handlers.onOpenSource(String(selected));
      return;
    }
    const edge = parameters.edges[0];
    if (edge !== undefined) handlers.onOpenRelationshipSource(String(edge));
  });
  network.on("hoverNode", (parameters) => {
    if (parameters.node === undefined) return;
    handlers.onHoverEdge(null);
    handlers.onHover({
      nodeId: String(parameters.node),
      x: parameters.pointer.DOM.x,
      y: parameters.pointer.DOM.y
    });
  });
  network.on("blurNode", () => handlers.onHover(null));
  network.on("hoverEdge", (parameters) => {
    if (parameters.edge !== undefined) {
      handlers.onHover(null);
      handlers.onHoverEdge({
        edgeId: String(parameters.edge),
        x: parameters.pointer.DOM.x,
        y: parameters.pointer.DOM.y
      });
    }
  });
  network.on("blurEdge", () => handlers.onHoverEdge(null));
  network.on("dragStart", () => {
    handlers.onHover(null);
    handlers.onHoverEdge(null);
  });
  network.on("zoom", () => {
    handlers.onHover(null);
    handlers.onHoverEdge(null);
  });
}
