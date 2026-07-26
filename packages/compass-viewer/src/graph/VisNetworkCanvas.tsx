import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState
} from "react";
import { DataSet, Network, type Edge, type Node, type Options } from "vis-network/standalone";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import type { GraphHover } from "./NodeHoverCard";
import { bindGraphNetworkEvents } from "./networkEvents";

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
  onOpenSource(nodeId: string): void;
  onHover(change: GraphHover | null): void;
  onClear(): void;
  onStabilized(): void;
};

const options: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: { enabled: true }
  },
  layout: { improvedLayout: true },
  nodes: {
    borderWidth: 1.5,
    shape: "dot"
  },
  edges: {
    arrows: { to: { enabled: true, scaleFactor: 0.5 } },
    smooth: { enabled: true, type: "continuous", roundness: 0.2 },
    selectionWidth: 3
  },
  physics: {
    enabled: true,
    solver: "forceAtlas2Based",
    stabilization: { enabled: true, iterations: 200, fit: true, updateInterval: 20 },
    forceAtlas2Based: {
      gravitationalConstant: -60,
      centralGravity: 0.005,
      springLength: 120,
      springConstant: 0.08,
      damping: 0.4,
      avoidOverlap: 0.8
    }
  }
};

export function graphNodeColor(
  model: GraphViewModel,
  node: GraphNode,
  contrastBorder?: string
) {
  const background = node.color?.background
    ?? model.communities.find((candidate) => candidate.id === node.community)?.color
    ?? "#6688aa";
  return {
    background,
    border: contrastBorder ?? node.color?.border ?? background
  };
}

function cssColor(name: string, fallback: string): string {
  if (typeof window === "undefined") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function edgeAppearance(confidence: string | undefined) {
  if (confidence === "extracted") return { dashes: false, width: 2, opacity: 0.7 };
  if (confidence === "ambiguous") return { dashes: [3, 4], width: 2, opacity: 0.62 };
  return { dashes: true, width: 1, opacity: 0.35 };
}

function comparisonEdgeColor(change: string | undefined, fallback: string): string {
  if (change === "added") return "#2ea043";
  if (change === "removed") return "#f85149";
  if (change === "changed") return "#d29922";
  return fallback;
}

function useThemeRevision(): number {
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const refresh = () => setRevision((current) => current + 1);
    const observer = new MutationObserver(refresh);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    colorScheme.addEventListener("change", refresh);
    return () => {
      observer.disconnect();
      colorScheme.removeEventListener("change", refresh);
    };
  }, []);
  return revision;
}

export const VisNetworkCanvas = forwardRef<GraphCanvasHandle, Props>(
  function VisNetworkCanvas({
    model,
    focusedNodeId,
    physicsRunning,
    forceLabels,
    hiddenCommunities,
    onFocus,
    onOpenSource,
    onHover,
    onClear,
    onStabilized
  }, ref) {
    const containerRef = useRef<HTMLDivElement>(null);
    const networkRef = useRef<Network | null>(null);
    const physicsRunningRef = useRef(physicsRunning);
    physicsRunningRef.current = physicsRunning;
    const initialViewRef = useRef<{ position: { x: number; y: number }; scale: number } | null>(null);
    const themeRevision = useThemeRevision();
    const maxDegree = useMemo(
      () => Math.max(1, ...model.nodes.map((node) => node.degree ?? 1)),
      [model.nodes]
    );
    const labelColor = useMemo(
      () => cssColor("--vscode-editor-foreground", "#eef5ff"),
      [themeRevision]
    );
    const edgeColor = useMemo(
      () => cssColor("--vscode-descriptionForeground", "#60728b"),
      [themeRevision]
    );
    const contrastBorder = useMemo(() => {
      const highContrast = document.body.classList.contains("vscode-high-contrast")
        || document.body.classList.contains("vscode-high-contrast-light");
      return highContrast
        ? cssColor("--vscode-contrastBorder", "#ffffff")
        : undefined;
    }, [themeRevision]);
    const nodeData = useMemo(() => new DataSet<Node>(
      model.nodes.map((node) => ({
        id: node.id,
        label: node.label,
        color: graphNodeColor(model, node),
        size: node.size ?? Math.min(40, 10 + 30 * (node.degree ?? 1) / maxDegree),
        font: {
          color: "#eef5ff",
          face: "system-ui",
          size: (node.degree ?? 1) >= maxDegree * 0.15 ? 12 : 0
        }
      }))
      // Styling changes are applied in place below so the Network, its paused
      // physics state, and its saved reset view survive theme and label changes.
    ), [maxDegree, model]);
    const edgeData = useMemo(() => new DataSet<Edge>(
      model.edges.map((edge) => {
        const appearance = edgeAppearance(edge.confidence);
        const color = comparisonEdgeColor(edge.change, "#60728b");
        return {
          id: edge.id,
          from: edge.source,
          to: edge.target,
          title: `${edge.relation}${edge.confidence ? ` · ${edge.confidence}` : ""}`,
          dashes: appearance.dashes,
          width: appearance.width,
          color: { color, opacity: edge.change ? 0.88 : appearance.opacity }
        };
      })
    ), [model.edges]);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;
      initialViewRef.current = null;
      const network = new Network(container, {
        nodes: nodeData,
        edges: edgeData
      }, options);
      network.setOptions({ physics: { enabled: physicsRunningRef.current } });
      if (!physicsRunningRef.current) network.stopSimulation();
      networkRef.current = network;
      bindGraphNetworkEvents(network, {
        onFocus,
        onOpenSource,
        onHover,
        onClear
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
    }, [
      edgeData,
      nodeData,
      onClear,
      onFocus,
      onHover,
      onOpenSource,
      onStabilized
    ]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network) return;
      network.setOptions({ physics: { enabled: physicsRunning } });
      if (physicsRunning) network.startSimulation();
      else network.stopSimulation();
    }, [edgeData, nodeData, physicsRunning]);

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
          opacity: isVisible ? 1 : 0.14,
          borderWidth: isFocused ? 4 : contrastBorder ? 2.5 : 1.5,
          color: graphNodeColor(model, node, contrastBorder),
          shadow: isFocused
            ? {
                enabled: true,
                color: node.color?.background
                  ?? model.communities.find((item) => item.id === node.community)?.color
                  ?? "#76b7ff",
                size: 24,
                x: 0,
                y: 0
              }
            : { enabled: false }
        };
      }));
      edgeData.update(model.edges.map((edge) => {
        const appearance = edgeAppearance(edge.confidence);
        const baseOpacity = edge.change ? 0.88 : appearance.opacity;
        const connectedEdge = edge.source === focusedNodeId || edge.target === focusedNodeId;
        const color = comparisonEdgeColor(edge.change, edgeColor);
        return {
          id: edge.id,
          color: {
            color,
            opacity: !focusedNodeId ? baseOpacity : connectedEdge ? 0.9 : 0.06
          },
          width: connectedEdge ? Math.max(2.5, appearance.width) : appearance.width
        };
      }));
      if (focusedNodeId) {
        network.selectNodes([focusedNodeId]);
        network.focus(focusedNodeId, {
          scale: 1.35,
          animation: window.matchMedia("(prefers-reduced-motion: reduce)").matches
            ? false
            : { duration: 260, easingFunction: "easeInOutQuad" }
        });
      } else {
        network.unselectAll();
      }
    }, [
      contrastBorder,
      edgeColor,
      edgeData,
      focusedNodeId,
      model,
      nodeData
    ]);

    useEffect(() => {
      nodeData.update(model.nodes.map((node) => ({
        id: node.id,
        font: {
          color: labelColor,
          size: forceLabels || (node.degree ?? 1) >= maxDegree * 0.15 ? 12 : 0
        }
      })));
    }, [forceLabels, labelColor, maxDegree, model.nodes, nodeData]);

    useImperativeHandle(ref, () => ({
      fit() {
        networkRef.current?.fit({
          animation: window.matchMedia("(prefers-reduced-motion: reduce)").matches
            ? false
            : { duration: 280, easingFunction: "easeInOutQuad" }
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
