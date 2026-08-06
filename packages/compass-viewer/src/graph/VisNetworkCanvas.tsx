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
import type { GraphEdgeHover } from "./EdgeHoverCard";
import type { GraphHover } from "./NodeHoverCard";
import {
  bindGraphNetworkEvents,
  type GraphNetworkHandlers
} from "./networkEvents";
import { graphRenderingProfile, seedStaticGraphPositions } from "./renderingProfile";
import type { GraphChangeType } from "./state";

export type GraphCanvasHandle = {
  fit(): void;
  reset(): void;
};

export type GraphCanvasPosition = {
  x: number;
  y: number;
};

type Props = {
  model: GraphViewModel;
  focusedNodeId: string | null;
  physicsRunning: boolean;
  initialPositions?: ReadonlyMap<string, GraphCanvasPosition>;
  forceLabels: boolean;
  hiddenCommunities: ReadonlySet<number>;
  hiddenChanges: ReadonlySet<GraphChangeType>;
  onFocus(nodeId: string): void;
  onOpenSource(nodeId: string): void;
  onOpenRelationshipSource(edgeId: string): void;
  onHover(change: GraphHover | null): void;
  onHoverEdge(change: GraphEdgeHover | null): void;
  onClear(): void;
  onStabilized(): void;
};

type ComparisonColor = {
  background: string;
  border: string;
};

type ComparisonPalette = Record<GraphChangeType, ComparisonColor>;

const fallbackComparisonPalette: ComparisonPalette = {
  added: { background: "#163d24", border: "#56d364" },
  removed: { background: "#4b1f24", border: "#ff7b72" },
  changed: { background: "#3d3015", border: "#d7a72b" },
  unchanged: { background: "#29313b", border: "#8b949e" }
};

const defaultOptions: Options = {
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

const comparisonOptions: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: { enabled: true }
  },
  layout: {
    improvedLayout: true,
    randomSeed: 17
  },
  nodes: {
    borderWidth: 2,
    shape: "dot"
  },
  edges: {
    arrows: { to: { enabled: true, scaleFactor: 0.38 } },
    smooth: { enabled: true, type: "continuous", roundness: 0.14 },
    selectionWidth: 3
  },
  physics: {
    enabled: true,
    solver: "barnesHut",
    stabilization: { enabled: true, iterations: 520, fit: true, updateInterval: 25 },
    maxVelocity: 35,
    minVelocity: 0.55,
    barnesHut: {
      theta: 0.45,
      gravitationalConstant: -12000,
      centralGravity: 0.12,
      springLength: 180,
      springConstant: 0.025,
      damping: 0.3,
      avoidOverlap: 0.85
    }
  }
};

const staticOptions: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    hideEdgesOnZoom: true,
    navigationButtons: false,
    keyboard: { enabled: true }
  },
  layout: {
    improvedLayout: false,
    randomSeed: 17
  },
  nodes: {
    borderWidth: 1.5,
    shape: "dot"
  },
  edges: {
    arrows: { to: { enabled: false } },
    smooth: false,
    selectionWidth: 3
  },
  physics: {
    enabled: false,
    solver: "barnesHut"
  }
};

export function graphNodeColor(
  model: GraphViewModel,
  node: GraphNode,
  contrastBorder?: string,
  comparisonPalette?: ComparisonPalette,
  communityColors?: ReadonlyMap<number, string>
) {
  const comparisonColor = node.change && comparisonPalette
    ? comparisonPalette[node.change]
    : undefined;
  const background = comparisonColor
    ? comparisonColor.background
    : node.color?.background
    ?? communityColors?.get(node.community)
    ?? model.communities.find((candidate) => candidate.id === node.community)?.color
    ?? "#6688aa";
  return {
    background,
    border: contrastBorder ?? comparisonColor?.border ?? node.color?.border ?? background
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
    return { color: palette.added.border, dashes: false, width: 1.7, opacity: 0.6 };
  }
  if (change === "removed") {
    return { color: palette.removed.border, dashes: [6, 5], width: 1.6, opacity: 0.56 };
  }
  if (change === "changed") {
    return { color: palette.changed.border, dashes: false, width: 1.65, opacity: 0.48 };
  }
  if (change === "unchanged") {
    return { color: palette.unchanged.border, dashes: true, width: 1, opacity: 0.2 };
  }
  const appearance = edgeAppearance(confidence);
  return { color: fallback, ...appearance };
}

function comparisonEdgeCurve(change: GraphChangeType | undefined) {
  if (change === "added") {
    return { enabled: true, type: "curvedCW" as const, roundness: 0.13 };
  }
  if (change === "removed") {
    return { enabled: true, type: "curvedCCW" as const, roundness: 0.13 };
  }
  return { enabled: true, type: "continuous" as const, roundness: 0.1 };
}

function seedComparisonPositions(nodes: GraphNode[]): ReadonlyMap<string, { x: number; y: number }> {
  const groups = new Map<GraphChangeType, GraphNode[]>();
  for (const node of [...nodes].sort((left, right) => left.id.localeCompare(right.id))) {
    const change = node.change ?? "unchanged";
    const group = groups.get(change) ?? [];
    group.push(node);
    groups.set(change, group);
  }
  const laneOffset: Record<GraphChangeType, number> = {
    added: -300,
    changed: 0,
    removed: 300,
    unchanged: 0
  };
  const positions = new Map<string, { x: number; y: number }>();
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  for (const [change, group] of groups) {
    group.forEach((node, index) => {
      const angle = index * goldenAngle;
      const radius = change === "unchanged"
        ? 210 + Math.sqrt(index) * 34
        : 42 + Math.sqrt(index) * 40;
      positions.set(node.id, {
        x: laneOffset[change] + Math.cos(angle) * radius,
        y: Math.sin(angle) * radius
      });
    });
  }
  return positions;
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

function parseRgb(color: string): [number, number, number] | undefined {
  const hex = color.match(/^#([\da-f]{3}|[\da-f]{6})$/i)?.[1];
  if (hex) {
    const normalized = hex.length === 3
      ? [...hex].map((value) => `${value}${value}`).join("")
      : hex;
    return [
      Number.parseInt(normalized.slice(0, 2), 16),
      Number.parseInt(normalized.slice(2, 4), 16),
      Number.parseInt(normalized.slice(4, 6), 16)
    ];
  }
  const rgb = color.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
  if (!rgb) return undefined;
  return [
    Number.parseFloat(rgb[1] ?? "0"),
    Number.parseFloat(rgb[2] ?? "0"),
    Number.parseFloat(rgb[3] ?? "0")
  ];
}

function blendColor(background: string, foreground: string, foregroundRatio: number): string {
  const backgroundRgb = parseRgb(background);
  const foregroundRgb = parseRgb(foreground);
  if (!backgroundRgb || !foregroundRgb) return foreground;
  const values = backgroundRgb.map((value, index) =>
    Math.round(value * (1 - foregroundRatio) + foregroundRgb[index]! * foregroundRatio));
  return `rgb(${values[0]}, ${values[1]}, ${values[2]})`;
}

function isDarkColor(color: string): boolean {
  const rgb = parseRgb(color);
  if (!rgb) return true;
  return (rgb[0] * 299 + rgb[1] * 587 + rgb[2] * 114) / 1000 < 145;
}

function comparisonColor(
  background: string,
  border: string,
  dark: boolean,
  context = false
): ComparisonColor {
  return {
    background: blendColor(background, border, context
      ? dark ? 0.18 : 0.1
      : dark ? 0.26 : 0.14),
    border
  };
}

export const VisNetworkCanvas = forwardRef<GraphCanvasHandle, Props>(
  function VisNetworkCanvas({
    model,
    focusedNodeId,
    physicsRunning,
    initialPositions,
    forceLabels,
    hiddenCommunities,
    hiddenChanges,
    onFocus,
    onOpenSource,
    onOpenRelationshipSource,
    onHover,
    onHoverEdge,
    onClear,
    onStabilized
  }, ref) {
    const containerRef = useRef<HTMLDivElement>(null);
    const networkRef = useRef<Network | null>(null);
    const physicsRunningRef = useRef(physicsRunning);
    physicsRunningRef.current = physicsRunning;
    const eventHandlersRef = useRef<GraphNetworkHandlers>({
      onFocus,
      onOpenSource,
      onOpenRelationshipSource,
      onHover,
      onHoverEdge,
      onClear
    });
    eventHandlersRef.current = {
      onFocus,
      onOpenSource,
      onOpenRelationshipSource,
      onHover,
      onHoverEdge,
      onClear
    };
    const onStabilizedRef = useRef(onStabilized);
    onStabilizedRef.current = onStabilized;
    const initialViewRef = useRef<{ position: { x: number; y: number }; scale: number } | null>(null);
    const themeRevision = useThemeRevision();
    const renderingProfile = useMemo(
      () => graphRenderingProfile(model),
      [model.edges.length, model.nodes.length]
    );
    const maxDegree = useMemo(() => {
      let maximum = 1;
      for (const node of model.nodes) maximum = Math.max(maximum, node.degree ?? 1);
      return maximum;
    }, [model.nodes]);
    const communityColors = useMemo(
      () => new Map(model.communities.map((community) => [community.id, community.color])),
      [model.communities]
    );
    const labelColor = useMemo(
      () => cssColor(
        "--vscode-editor-foreground",
        cssColor("--foreground", "#eef5ff")
      ),
      [themeRevision]
    );
    const edgeColor = useMemo(
      () => cssColor(
        "--vscode-descriptionForeground",
        cssColor("--muted-foreground", "#60728b")
      ),
      [themeRevision]
    );
    const comparisonPalette = useMemo<ComparisonPalette>(() => {
      const background = cssColor(
        "--vscode-editor-background",
        cssColor("--background", "#08111f")
      );
      const dark = isDarkColor(background);
      return {
        added: comparisonColor(
          background,
          cssColor(
            "--vscode-gitDecoration-addedResourceForeground",
            dark ? "#56d364" : "#1a7f37"
          ),
          dark
        ),
        removed: comparisonColor(
          background,
          cssColor(
            "--vscode-gitDecoration-deletedResourceForeground",
            dark ? "#ff7b72" : "#cf222e"
          ),
          dark
        ),
        changed: comparisonColor(
          background,
          cssColor(
            "--vscode-gitDecoration-modifiedResourceForeground",
            dark ? "#d7a72b" : "#9a6700"
          ),
          dark
        ),
        unchanged: comparisonColor(
          background,
          cssColor("--vscode-descriptionForeground", dark ? "#8b949e" : "#656d76"),
          dark,
          true
        )
      };
    }, [themeRevision]);
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
        : renderingProfile === "static"
          ? [...model.nodes]
            .sort((left, right) =>
              (right.degree ?? 0) - (left.degree ?? 0) || left.id.localeCompare(right.id))
            .slice(0, 20)
            .map((node) => node.id)
          : []
    ), [comparisonMode, model.nodes, renderingProfile]);
    const comparisonPositions = useMemo(
      () => comparisonMode ? seedComparisonPositions(model.nodes) : new Map(),
      [comparisonMode, model.nodes]
    );
    const staticPositions = useMemo(
      () => renderingProfile === "static" && !comparisonMode
        ? seedStaticGraphPositions(model.nodes)
        : new Map(),
      [comparisonMode, model.nodes, renderingProfile]
    );
    const contrastBorder = useMemo(() => {
      if (typeof document === "undefined") return undefined;
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
          ? node.change === "unchanged"
            ? 7 + 5 * Math.sqrt((node.degree ?? 1) / maxDegree)
            : 11 + 12 * Math.sqrt((node.degree ?? 1) / maxDegree)
          : baseSize;
        const position = comparisonPositions.get(node.id)
          ?? initialPositions?.get(node.id)
          ?? staticPositions.get(node.id);
        return {
          id: node.id,
          label: node.label,
          color: graphNodeColor(
            model,
            node,
            undefined,
            fallbackComparisonPalette,
            communityColors
          ),
          size,
          ...(position ?? {}),
          opacity: node.change === "unchanged" ? 0.58 : 1,
          font: {
            color: "#eef5ff",
            face: "system-ui",
            size: comparisonMode
              ? automaticLabelIds.has(node.id) ? 12 : 0
              : renderingProfile === "static"
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
      comparisonPositions,
      communityColors,
      initialPositions,
      maxDegree,
      model,
      renderingProfile,
      staticPositions
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
          dashes: appearance.dashes,
          width: appearance.width,
          ...(comparisonMode && renderingProfile !== "static"
            ? { smooth: comparisonEdgeCurve(edge.change) }
            : {}),
          color: { color: appearance.color, opacity: appearance.opacity }
        };
      })
    ), [comparisonMode, model.edges, renderingProfile]);
    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;
      initialViewRef.current = null;
      const network = new Network(container, {
        nodes: nodeData,
        edges: edgeData
      }, renderingProfile === "static"
        ? staticOptions
        : comparisonMode ? comparisonOptions : defaultOptions);
      network.setOptions({ physics: { enabled: physicsRunningRef.current } });
      if (!physicsRunningRef.current) network.stopSimulation();
      networkRef.current = network;
      bindGraphNetworkEvents(network, {
        onFocus: (nodeId) => eventHandlersRef.current.onFocus(nodeId),
        onOpenSource: (nodeId) => eventHandlersRef.current.onOpenSource(nodeId),
        onOpenRelationshipSource: (edgeId) => eventHandlersRef.current.onOpenRelationshipSource(edgeId),
        onHover: (change) => eventHandlersRef.current.onHover(change),
        onHoverEdge: (change) => eventHandlersRef.current.onHoverEdge(change),
        onClear: () => eventHandlersRef.current.onClear()
      });
      network.once("stabilizationIterationsDone", () => {
        initialViewRef.current = {
          position: network.getViewPosition(),
          scale: network.getScale()
        };
        onStabilizedRef.current();
      });
      const hasSeededPositions = comparisonPositions.size > 0
        || (initialPositions?.size ?? 0) > 0
        || staticPositions.size > 0;
      if (!physicsRunningRef.current && hasSeededPositions) {
        network.stopSimulation();
        network.fit({ animation: false });
        initialViewRef.current = {
          position: network.getViewPosition(),
          scale: network.getScale()
        };
      }
      return () => {
        network.destroy();
        networkRef.current = null;
      };
    }, [
      edgeData,
      nodeData,
      comparisonMode,
      comparisonPositions,
      initialPositions,
      renderingProfile,
      staticPositions
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
        const comparisonOpacity = node.change === "unchanged" ? 0.58 : 1;
        return {
          id: node.id,
          opacity: !focusedNodeId
            ? comparisonOpacity
            : isVisible ? Math.max(comparisonOpacity, 0.72) : 0.08,
          borderWidth: isFocused ? 4 : contrastBorder ? 2.5 : 1.5,
          color: graphNodeColor(
            model,
            node,
            contrastBorder,
            comparisonPalette,
            communityColors
          ),
          shadow: isFocused
            ? {
                enabled: true,
                color: node.change
                  ? comparisonPalette[node.change].border
                  : node.color?.background
                    ?? communityColors.get(node.community)
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
      communityColors,
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
              : renderingProfile === "static"
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
      nodeData,
      renderingProfile
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
        data-rendering-profile={renderingProfile}
        role="region"
        aria-label="Interactive Compass code graph"
        onMouseLeave={() => {
          onHover(null);
          onHoverEdge(null);
        }}
      />
    );
  }
);
