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
import type { GraphChangeType } from "./state";

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
  hiddenChanges: ReadonlySet<GraphChangeType>;
  onFocus(nodeId: string): void;
  onOpenSource(nodeId: string): void;
  onHover(change: GraphHover | null): void;
  onClear(): void;
  onStabilized(): void;
};

type ComparisonPalette = Record<GraphChangeType, string>;

const fallbackComparisonPalette: ComparisonPalette = {
  added: "#2ea043",
  removed: "#f85149",
  changed: "#d29922",
  unchanged: "#6e7781"
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
  contrastBorder?: string,
  comparisonPalette?: ComparisonPalette
) {
  const background = node.change && comparisonPalette
    ? comparisonPalette[node.change]
    : node.color?.background
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

function comparisonEdgeAppearance(
  change: GraphChangeType | undefined,
  confidence: string | undefined,
  fallback: string,
  palette: ComparisonPalette
) {
  if (change === "added") {
    return { color: palette.added, dashes: false, width: 2.5, opacity: 0.92 };
  }
  if (change === "removed") {
    return { color: palette.removed, dashes: [6, 5], width: 2.3, opacity: 0.9 };
  }
  if (change === "changed") {
    return { color: palette.changed, dashes: false, width: 3, opacity: 0.94 };
  }
  if (change === "unchanged") {
    return { color: palette.unchanged, dashes: true, width: 1, opacity: 0.28 };
  }
  const appearance = edgeAppearance(confidence);
  return { color: fallback, ...appearance };
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
    hiddenChanges,
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
    const comparisonPalette = useMemo<ComparisonPalette>(() => ({
      added: cssColor("--vscode-gitDecoration-addedResourceForeground", "#2ea043"),
      removed: cssColor("--vscode-gitDecoration-deletedResourceForeground", "#f85149"),
      changed: cssColor("--vscode-gitDecoration-modifiedResourceForeground", "#d29922"),
      unchanged: cssColor("--vscode-descriptionForeground", "#6e7781")
    }), [themeRevision]);
    const comparisonMode = useMemo(
      () => model.nodes.some((node) => node.change !== undefined)
        || model.edges.some((edge) => edge.change !== undefined),
      [model.edges, model.nodes]
    );
    const automaticLabelIds = useMemo(() => new Set(
      comparisonMode
        ? model.nodes
          .filter((node) => node.change !== "unchanged")
          .sort((left, right) =>
            (right.degree ?? 0) - (left.degree ?? 0) || left.id.localeCompare(right.id))
          .slice(0, 12)
          .map((node) => node.id)
        : []
    ), [comparisonMode, model.nodes]);
    const contrastBorder = useMemo(() => {
      const highContrast = document.body.classList.contains("vscode-high-contrast")
        || document.body.classList.contains("vscode-high-contrast-light");
      return highContrast
        ? cssColor("--vscode-contrastBorder", "#ffffff")
        : undefined;
    }, [themeRevision]);
    const nodeData = useMemo(() => new DataSet<Node>(
      model.nodes.map((node) => {
        const baseSize = node.size ?? Math.min(40, 10 + 30 * (node.degree ?? 1) / maxDegree);
        const size = comparisonMode
          ? node.change === "unchanged" ? baseSize * 0.72 : baseSize * 1.12
          : baseSize;
        return {
          id: node.id,
          label: node.label,
          color: graphNodeColor(model, node, undefined, fallbackComparisonPalette),
          size,
          opacity: node.change === "unchanged" ? 0.42 : 1,
          font: {
            color: "#eef5ff",
            face: "system-ui",
            size: comparisonMode
              ? automaticLabelIds.has(node.id) ? 12 : 0
              : (node.degree ?? 1) >= maxDegree * 0.15 ? 12 : 0
          }
        };
      })
      // Styling changes are applied in place below so the Network, its paused
      // physics state, and its saved reset view survive theme and label changes.
    ), [
      automaticLabelIds,
      comparisonMode,
      maxDegree,
      model
    ]);
    const edgeData = useMemo(() => new DataSet<Edge>(
      model.edges.map((edge) => {
        const appearance = comparisonEdgeAppearance(
          edge.change,
          edge.confidence,
          "#60728b",
          fallbackComparisonPalette
        );
        return {
          id: edge.id,
          from: edge.source,
          to: edge.target,
          title: `${edge.relation}${edge.confidence ? ` · ${edge.confidence}` : ""}`,
          dashes: appearance.dashes,
          width: appearance.width,
          color: { color: appearance.color, opacity: appearance.opacity }
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
          .filter((node) =>
            hiddenCommunities.has(node.community)
            || hiddenChanges.has(node.change ?? "unchanged"))
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
    }, [
      edgeData,
      hiddenChanges,
      hiddenCommunities,
      model.edges,
      model.nodes,
      nodeData
    ]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network) return;
      const connected = focusedNodeId
        ? new Set(network.getConnectedNodes(focusedNodeId).map(String))
        : new Set<string>();
      nodeData.update(model.nodes.map((node) => {
        const isFocused = node.id === focusedNodeId;
        const isVisible = !focusedNodeId || isFocused || connected.has(node.id);
        const comparisonOpacity = node.change === "unchanged" ? 0.42 : 1;
        return {
          id: node.id,
          opacity: !focusedNodeId
            ? comparisonOpacity
            : isVisible ? Math.max(comparisonOpacity, 0.72) : 0.08,
          borderWidth: isFocused ? 4 : contrastBorder ? 2.5 : 1.5,
          color: graphNodeColor(model, node, contrastBorder, comparisonPalette),
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
        const appearance = comparisonEdgeAppearance(
          edge.change,
          edge.confidence,
          edgeColor,
          comparisonPalette
        );
        const connectedEdge = edge.source === focusedNodeId || edge.target === focusedNodeId;
        return {
          id: edge.id,
          dashes: appearance.dashes,
          color: {
            color: appearance.color,
            opacity: !focusedNodeId ? appearance.opacity : connectedEdge ? 0.92 : 0.05
          },
          width: connectedEdge ? Math.max(3, appearance.width) : appearance.width
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
      comparisonPalette,
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
          size: forceLabels
            || node.id === focusedNodeId
            || (comparisonMode
              ? automaticLabelIds.has(node.id)
              : (node.degree ?? 1) >= maxDegree * 0.15)
            ? 12
            : 0
        }
      })));
    }, [
      automaticLabelIds,
      comparisonMode,
      focusedNodeId,
      forceLabels,
      labelColor,
      maxDegree,
      model.nodes,
      nodeData
    ]);

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
