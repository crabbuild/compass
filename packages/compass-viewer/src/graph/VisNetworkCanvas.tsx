import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef
} from "react";
import { DataSet, Network, type Edge, type Node, type Options } from "vis-network/standalone";
import type { GraphViewModel } from "@/contracts/graph";

export type GraphCanvasHandle = {
  fit(): void;
  reset(): void;
};

type Props = {
  model: GraphViewModel;
  focusedNodeId: string | null;
  physicsRunning: boolean;
  forceLabels: boolean;
  hiddenCommunities: ReadonlySet<number>;
  onFocus(nodeId: string): void;
  onClear(): void;
  onStabilized(): void;
};

const options: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    navigationButtons: false,
    keyboard: { enabled: true }
  },
  layout: { improvedLayout: true },
  nodes: {
    borderWidth: 1.5,
    font: { color: "#d7e1ed", face: "system-ui", size: 11 },
    shape: "dot",
    size: 11
  },
  edges: {
    arrows: { to: { enabled: true, scaleFactor: 0.45 } },
    color: { color: "#54657a", opacity: 0.72 },
    smooth: { enabled: true, type: "dynamic", roundness: 0.5 },
    width: 1
  },
  physics: {
    enabled: true,
    stabilization: { enabled: true, iterations: 220, updateInterval: 20 },
    barnesHut: {
      gravitationalConstant: -2600,
      centralGravity: 0.12,
      springLength: 105,
      springConstant: 0.025,
      damping: 0.22
    }
  }
};

function nodeColor(model: GraphViewModel, community: number) {
  const color = model.communities.find((candidate) => candidate.id === community)?.color
    ?? "#6688aa";
  return { background: color, border: color };
}

export const VisNetworkCanvas = forwardRef<GraphCanvasHandle, Props>(
  function VisNetworkCanvas({
    model,
    focusedNodeId,
    physicsRunning,
    forceLabels,
    hiddenCommunities,
    onFocus,
    onClear,
    onStabilized
  }, ref) {
    const containerRef = useRef<HTMLDivElement>(null);
    const networkRef = useRef<Network | null>(null);
    const initialViewRef = useRef<{ position: { x: number; y: number }; scale: number } | null>(null);
    const nodeData = useMemo(() => new DataSet<Node>(
      model.nodes.map((node) => ({
        id: node.id,
        label: node.label,
        title: node.label,
        group: String(node.community),
        color: node.color ?? nodeColor(model, node.community),
        value: Math.max(1, node.degree ?? 1)
      }))
    ), [model]);
    const edgeData = useMemo(() => new DataSet<Edge>(
      model.edges.map((edge) => ({
        id: edge.id,
        from: edge.source,
        to: edge.target,
        title: edge.relation,
        dashes: edge.confidence === "inferred",
        width: edge.confidence === "ambiguous" ? 2 : 1
      }))
    ), [model]);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;
      const network = new Network(container, {
        nodes: nodeData,
        edges: edgeData
      }, options);
      networkRef.current = network;
      network.on("click", (parameters) => {
        const selected = parameters.nodes[0];
        if (selected !== undefined) onFocus(String(selected));
        else onClear();
      });
      network.once("stabilizationIterationsDone", () => {
        initialViewRef.current = {
          position: network.getViewPosition(),
          scale: network.getScale()
        };
        onStabilized();
      });
      return () => {
        network.destroy();
        networkRef.current = null;
      };
    }, [edgeData, nodeData, onClear, onFocus, onStabilized]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network) return;
      network.setOptions({ physics: { enabled: physicsRunning } });
      if (physicsRunning) network.startSimulation();
      else network.stopSimulation();
    }, [physicsRunning]);

    useEffect(() => {
      const hiddenNodes = new Set(
        model.nodes
          .filter((node) => hiddenCommunities.has(node.community))
          .map((node) => node.id)
      );
      nodeData.update(model.nodes.map((node) => ({
        id: node.id,
        hidden: hiddenNodes.has(node.id)
      })));
      edgeData.update(model.edges.map((edge) => ({
        id: edge.id,
        hidden: hiddenNodes.has(edge.source) || hiddenNodes.has(edge.target)
      })));
    }, [edgeData, hiddenCommunities, model.edges, model.nodes, nodeData]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network) return;
      const connected = focusedNodeId
        ? new Set(network.getConnectedNodes(focusedNodeId).map(String))
        : new Set<string>();
      nodeData.update(model.nodes.map((node) => {
        const isFocused = node.id === focusedNodeId;
        const isVisible = !focusedNodeId || isFocused || connected.has(node.id);
        return {
          id: node.id,
          opacity: isVisible ? 1 : 0.12,
          borderWidth: isFocused ? 4 : 1.5,
          shadow: isFocused
            ? { enabled: true, color: "rgba(79,156,249,.65)", size: 20, x: 0, y: 0 }
            : { enabled: false }
        };
      }));
      edgeData.update(model.edges.map((edge) => ({
        id: edge.id,
        color: {
          color: "#66788e",
          opacity: !focusedNodeId
            || edge.source === focusedNodeId
            || edge.target === focusedNodeId ? 0.9 : 0.08
        }
      })));
      if (focusedNodeId) {
        network.selectNodes([focusedNodeId]);
        network.focus(focusedNodeId, {
          scale: 1.25,
          animation: window.matchMedia("(prefers-reduced-motion: reduce)").matches
            ? false
            : { duration: 240, easingFunction: "easeInOutQuad" }
        });
      } else {
        network.unselectAll();
      }
    }, [edgeData, focusedNodeId, model.edges, model.nodes, nodeData]);

    useEffect(() => {
      nodeData.update(model.nodes.map((node) => ({
        id: node.id,
        font: { size: forceLabels ? 13 : 11 }
      })));
    }, [forceLabels, model.nodes, nodeData]);

    useImperativeHandle(ref, () => ({
      fit() {
        networkRef.current?.fit({
          animation: window.matchMedia("(prefers-reduced-motion: reduce)").matches
            ? false
            : { duration: 220, easingFunction: "easeInOutQuad" }
        });
      },
      reset() {
        const network = networkRef.current;
        const initial = initialViewRef.current;
        if (!network) return;
        if (!initial) {
          network.fit({ animation: false });
          return;
        }
        network.moveTo({
          position: initial.position,
          scale: initial.scale,
          animation: false
        });
      }
    }), []);

    return (
      <div
        ref={containerRef}
        className="compass-canvas"
        role="region"
        aria-label="Interactive Compass code graph"
      />
    );
  }
);
