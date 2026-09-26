import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState
} from "react";
import { DataSet, Network, type Edge, type Node, type Options } from "vis-network/standalone";
import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import type { GraphEdgeHover } from "./EdgeHoverCard";
import type { GraphHover } from "./NodeHoverCard";
import {
  bindGraphNetworkEvents,
  type GraphNetworkHandlers
} from "./networkEvents";
import {
  centerCommunityOverviewPositions,
  graphRenderingProfile,
  relaxCommunityOverviewPositions,
  seedCommunityOverviewPositions,
  seedGraphLayoutPositions,
  seedStaticGraphPositions,
  visibleGraphEdges,
  type GraphLayoutStyle
} from "./renderingProfile";
import {
  COMMUNITY_LABEL_FONT_SIZE,
  communityOverviewLabelText,
  communityOverviewLabelledIds
} from "./communityOverview";
import { blendColor, isDarkColor } from "../lib/color";
import { cssColor, useThemeRevision } from "../lib/theme";
import type { GraphChangeType } from "./state";
import type { GraphLayoutSpacing } from "./state";
import {
  GraphMinimap,
  type GraphMinimapSnapshot
} from "./GraphMinimap";
import {
  edgeSemanticCategory,
  nodeSemanticCategory,
  nodeSemanticShape,
  type EdgeSemanticCategory,
  type NodeSemanticCategory
} from "./semanticAppearance";

export type GraphCanvasHandle = {
  fit(): void;
  fitSelection(nodeIds: readonly string[]): void;
  reset(): void;
  resetZoom(): void;
  zoomIn(): void;
  zoomOut(): void;
};

export type GraphCanvasPosition = {
  x: number;
  y: number;
};

type Props = {
  model: GraphViewModel;
  focusedNodeId: string | null;
  /** Node under the pointer, so hovering a context bubble can reveal its hue. */
  hoveredNodeId?: string | null | undefined;
  physicsRunning: boolean;
  layoutStyle?: GraphLayoutStyle;
  initialPositions?: ReadonlyMap<string, GraphCanvasPosition>;
  forceLabels: boolean;
  showEdgeLabels?: boolean;
  isolatedNodeIds?: ReadonlySet<string> | undefined;
  isolatedEdgeIds?: ReadonlySet<string> | undefined;
  layoutSpacing?: GraphLayoutSpacing;
  showMinimap?: boolean;
  semanticDetail?: boolean;
  /** Importance per community bubble id, for label ranking in an overview. */
  communityImportance?: ReadonlyMap<string, number> | undefined;
  /** Dominant relationship category per aggregated edge id, for edge colour. */
  edgeSemanticHints?: ReadonlyMap<string, EdgeSemanticCategory> | undefined;
  hiddenCommunities: ReadonlySet<number>;
  hiddenChanges: ReadonlySet<GraphChangeType>;
  onFocus(nodeId: string): void;
  onOpenSource(nodeId: string): void;
  onOpenRelationshipSource(edgeId: string): void;
  onInteractionStart?(): void;
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
type SemanticNodePalette = Record<NodeSemanticCategory, ComparisonColor>;
type SemanticEdgePalette = Record<EdgeSemanticCategory, string>;

/**
 * Fallbacks used until the host theme resolves. They follow the shared Compass
 * palette: deep, desaturated fills with a brighter companion border, so a graph
 * looks the same in a standalone export and inside an editor.
 */
const fallbackComparisonPalette: ComparisonPalette = {
  added: { background: "#14351f", border: "#4fbe6b" },
  removed: { background: "#3d1c20", border: "#e06c75" },
  changed: { background: "#332a15", border: "#c9902c" },
  unchanged: { background: "#20262e", border: "#7a838e" }
};
const fallbackSemanticNodePalette: SemanticNodePalette = {
  callable: { background: "#123048", border: "#4e9bd6" },
  type: { background: "#382a16", border: "#c4814c" },
  module: { background: "#0f322e", border: "#43b39e" },
  boundary: { background: "#3a2320", border: "#d9755f" },
  other: { background: "#222831", border: "#8593a3" }
};
const fallbackSemanticEdgePalette: SemanticEdgePalette = {
  execution: "#4e9bd6",
  dependency: "#43b39e",
  structure: "#d2a15c",
  flow: "#c07bb4",
  other: "#8593a3"
};
const STATIC_VISIBLE_LABEL_LIMIT = 200;
const MINIMAP_POSITION_LIMIT = 1_500;
/**
 * Neutral fill for the long tail of a community overview. Hue is spent on the
 * communities the reader is meant to notice (the labelled, coupled, and
 * boundary-rich ones); everything else is context, and colouring it would turn
 * the map into confetti.
 */
const CONTEXT_NODE_FILL = "#8D97A3";
const CONTEXT_NODE_BORDER = "#6B7581";
const MIN_VIEW_SCALE = 0.1;
const MAX_VIEW_SCALE = 3;
const MIN_LAYOUT_REHEAT_DISTANCE = 18;
const MAX_LAYOUT_REHEAT_DISTANCE = 96;
const LAYOUT_REHEAT_NODE_LIMIT = 1_500;

function reheatGraphLayout(
  network: Network,
  visibleNodeIds: ReadonlySet<string>
): void {
  const nodeIds = [...visibleNodeIds].sort().slice(0, LAYOUT_REHEAT_NODE_LIMIT);
  const positions = network.getPositions(nodeIds);
  const coordinates = Object.values(positions);
  const graphSpan = coordinates.length === 0 ? 0 : Math.max(
    Math.max(...coordinates.map(({ x }) => x)) - Math.min(...coordinates.map(({ x }) => x)),
    Math.max(...coordinates.map(({ y }) => y)) - Math.min(...coordinates.map(({ y }) => y))
  );
  const reheatDistance = Math.min(
    MAX_LAYOUT_REHEAT_DISTANCE,
    Math.max(MIN_LAYOUT_REHEAT_DISTANCE, graphSpan * 0.012)
  );
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  nodeIds.forEach((id, index) => {
    const position = positions[id];
    if (!position) return;
    const angle = index * goldenAngle;
    network.moveNode(
      id,
      position.x + Math.cos(angle) * reheatDistance,
      position.y + Math.sin(angle) * reheatDistance
    );
  });
}

function cameraAnimation(duration: number) {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ? false
    : { duration, easingFunction: "easeInOutQuad" as const };
}

function withPhysicsEnabled(options: Options, enabled: boolean): Options {
  if (!options.physics || typeof options.physics !== "object") {
    return { ...options, physics: { enabled } };
  }
  return {
    ...options,
    physics: {
      ...options.physics,
      enabled
    }
  };
}

const defaultOptions: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: { enabled: true, bindToWindow: false, autoFocus: false }
  },
  layout: { improvedLayout: false, randomSeed: 17 },
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

/**
 * Community overviews are already laid out as a packed map, so they open
 * without physics: the packed bubbles stay put, labels remain readable, and
 * pressing Run layout still reheats the map when the reader wants it.
 */
const communityOverviewOptions: Options = {
  autoResize: true,
  interaction: {
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: { enabled: true, bindToWindow: false, autoFocus: false }
  },
  layout: {
    improvedLayout: false,
    randomSeed: 17
  },
  nodes: {
    borderWidth: 2,
    shape: "dot"
  },
  edges: {
    arrows: { to: { enabled: false } },
    chosen: false,
    smooth: { enabled: true, type: "continuous", roundness: 0.24 },
    selectionWidth: 3
  },
  physics: {
    enabled: false,
    solver: "forceAtlas2Based",
    stabilization: { enabled: true, iterations: 260, fit: true, updateInterval: 25 },
    forceAtlas2Based: {
      // Overview bubbles are small and carry labels, so the arrangement stays
      // compact: coupled communities pull together, everything keeps a body's
      // width of separation, and the fitted view keeps labels legible.
      gravitationalConstant: -34,
      centralGravity: 0.02,
      springLength: 96,
      springConstant: 0.08,
      damping: 0.45,
      avoidOverlap: 1
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
    keyboard: { enabled: true, bindToWindow: false, autoFocus: false }
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
    hoverConnectedEdges: false,
    selectConnectedEdges: false,
    navigationButtons: false,
    keyboard: { enabled: true, bindToWindow: false, autoFocus: false }
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
    chosen: false,
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
  communityColors?: ReadonlyMap<number, string>,
  semanticPalette?: SemanticNodePalette
) {
  if (node.groundingStatus === "GROUNDED") {
    return {
      background: "#153d3a",
      border: node.challenged ? "#fbbf24" : "#5eead4"
    };
  }
  const comparisonColor = node.change && comparisonPalette
    ? comparisonPalette[node.change]
    : undefined;
  const semanticColor = semanticPalette?.[nodeSemanticCategory(node.kind)];
  const background = comparisonColor
    ? comparisonColor.background
    : semanticColor?.background
    ?? node.color?.background
    ?? communityColors?.get(node.community)
    ?? model.communities.find((candidate) => candidate.id === node.community)?.color
    ?? "#6688aa";
  return {
    background,
    border: contrastBorder
      ?? comparisonColor?.border
      ?? semanticColor?.border
      ?? node.color?.border
      ?? background
  };
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

function edgeAppearance(confidence: string | undefined, weight?: number | undefined) {
  if (confidence === "extracted") return { dashes: false, width: 1, opacity: 0.7 };
  if (confidence === "ambiguous") return { dashes: [3, 4], width: 1, opacity: 0.62 };
  if (confidence === "aggregated") {
    // Aggregated relationships carry the exact number of crossed relationships
    // as their weight: heavier community routes read thicker and darker.
    const crossings = Math.max(1, weight ?? 1);
    const presence = Math.log2(1 + crossings);
    return {
      dashes: false,
      width: Math.min(2.5, 0.5 + 0.75 * presence),
      opacity: Math.min(0.5, 0.16 + 0.09 * presence)
    };
  }
  return { dashes: true, width: 0.5, opacity: 0.35 };
}

function comparisonEdgeAppearance(
  change: GraphChangeType | undefined,
  confidence: string | undefined,
  fallback: string,
  palette: ComparisonPalette,
  weight?: number | undefined
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
  const appearance = edgeAppearance(confidence, weight);
  return { color: fallback, ...appearance };
}

function effectiveEdgeAppearance(
  edge: GraphEdge,
  appearance: ReturnType<typeof comparisonEdgeAppearance>
) {
  if (edge.challenged) {
    return { color: "#fbbf24", dashes: [3, 3], width: 2.5, opacity: 0.9 };
  }
  if (edge.groundingStatus === "GROUNDED") {
    return { color: "#5eead4", dashes: [7, 4], width: 2.3, opacity: 0.86 };
  }
  return appearance;
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

/**
 * Canvas label for a community bubble: the bounded community name with the
 * exact member count underneath, because bubble area alone cannot be read.
 */
function communityBubbleLabel(node: GraphNode): string {
  const name = communityOverviewLabelText(node.label);
  const members = node.memberCount ?? 0;
  return members > 0 ? `${name}\n${members.toLocaleString()} symbols` : name;
}

function communityBubbleFontSize(node: GraphNode): number {
  return Math.round((node.size ?? 12) + 12);
}

/**
 * Soft halo under a signal bubble, in the community's own hue. It separates the
 * hubs from the neutral context field on a dark canvas without adding chrome.
 */
function signalGlow(node: GraphNode) {
  return {
    enabled: true,
    color: node.color?.background ?? "#8D97A3",
    size: Math.round((node.size ?? 12) * 0.6),
    x: 0,
    y: 0
  };
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

export const VisNetworkCanvas = forwardRef<GraphCanvasHandle, Props>(
  function VisNetworkCanvas({
    model,
    focusedNodeId,
    hoveredNodeId,
    physicsRunning,
    layoutStyle = "automatic",
    initialPositions,
    forceLabels,
    showEdgeLabels = false,
    isolatedNodeIds,
    isolatedEdgeIds,
    layoutSpacing = 2,
    showMinimap = false,
    semanticDetail = false,
    communityImportance,
    edgeSemanticHints,
    hiddenCommunities,
    hiddenChanges,
    onFocus,
    onOpenSource,
    onOpenRelationshipSource,
    onInteractionStart = () => undefined,
    onHover,
    onHoverEdge,
    onClear,
    onStabilized
  }, ref) {
    const containerRef = useRef<HTMLDivElement>(null);
    const networkRef = useRef<Network | null>(null);
    const [minimapSnapshot, setMinimapSnapshot] = useState<GraphMinimapSnapshot | null>(null);
    const physicsRunningRef = useRef(physicsRunning);
    physicsRunningRef.current = physicsRunning;
    const previousPhysicsRunningRef = useRef(physicsRunning);
    const eventHandlersRef = useRef<GraphNetworkHandlers>({
      onFocus,
      onOpenSource,
      onOpenRelationshipSource,
      onInteractionStart,
      onHover,
      onHoverEdge,
      onClear
    });
    eventHandlersRef.current = {
      onFocus,
      onOpenSource,
      onOpenRelationshipSource,
      onInteractionStart,
      onHover,
      onHoverEdge,
      onClear
    };
    const onStabilizedRef = useRef(onStabilized);
    onStabilizedRef.current = onStabilized;
    const initialViewRef = useRef<{ position: { x: number; y: number }; scale: number } | null>(null);
    const previousLayoutSpacingRef = useRef(layoutSpacing);
    const themeRevision = useThemeRevision();
    const renderingProfile = useMemo(
      () => graphRenderingProfile(model),
      [model.edges.length, model.nodes.length]
    );
    const communityOverview = model.stats.aggregated;
    const renderedEdges = useMemo(
      () => visibleGraphEdges(model),
      [model]
    );
    const visibleNodeIds = useMemo(() => new Set(model.nodes
      .filter((node) => !hiddenCommunities.has(node.community)
        && !hiddenChanges.has(node.change ?? "unchanged")
        && (!isolatedNodeIds || isolatedNodeIds.has(node.id)))
      .map((node) => node.id)), [
      hiddenChanges,
      hiddenCommunities,
      isolatedNodeIds,
      model.nodes
    ]);
    const visibleEdgeIds = useMemo(() => new Set(renderedEdges
      .filter((edge) => visibleNodeIds.has(edge.source)
        && visibleNodeIds.has(edge.target)
        && (!isolatedEdgeIds || isolatedEdgeIds.has(edge.id)))
      .map((edge) => edge.id)), [isolatedEdgeIds, renderedEdges, visibleNodeIds]);
    const visibleNodeIdsRef = useRef(visibleNodeIds);
    visibleNodeIdsRef.current = visibleNodeIds;
    const refreshMinimap = useCallback(() => {
      const network = networkRef.current;
      const container = containerRef.current;
      if (!network || !container) return;
      const positions = network.getPositions(
        [...visibleNodeIdsRef.current].sort().slice(0, MINIMAP_POSITION_LIMIT)
      );
      const center = network.getViewPosition();
      const scale = Math.max(MIN_VIEW_SCALE, network.getScale());
      const width = Math.max(1, container.clientWidth) / scale;
      const height = Math.max(1, container.clientHeight) / scale;
      setMinimapSnapshot({
        positions: new Map(Object.entries(positions)),
        viewport: {
          left: center.x - width / 2,
          top: center.y - height / 2,
          width,
          height
        }
      });
    }, []);
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
    const edgeLabelColor = useMemo(
      () => cssColor(
        "--vscode-editor-foreground",
        cssColor("--foreground", "#d8e4f2")
      ),
      [themeRevision]
    );
    const canvasBackground = useMemo(
      () => cssColor(
        "--vscode-editor-background",
        cssColor("--background", "#08111f")
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
    const semanticNodePalette = useMemo<SemanticNodePalette>(() => {
      const background = cssColor(
        "--vscode-editor-background",
        cssColor("--background", "#08111f")
      );
      const dark = isDarkColor(background);
      const semanticColor = (name: string, darkFallback: string, lightFallback: string) =>
        comparisonColor(background, cssColor(name, dark ? darkFallback : lightFallback), dark);
      return {
        callable: semanticColor("--vscode-symbolIcon-functionForeground", "#5fa8ff", "#0969da"),
        type: semanticColor("--vscode-symbolIcon-classForeground", "#e3b341", "#9a6700"),
        module: semanticColor("--vscode-symbolIcon-moduleForeground", "#56d4b4", "#168b76"),
        boundary: semanticColor("--vscode-symbolIcon-eventForeground", "#ff9b87", "#cf4c35"),
        other: comparisonColor(
          background,
          cssColor("--vscode-descriptionForeground", dark ? "#8b949e" : "#656d76"),
          dark,
          true
        )
      };
    }, [themeRevision]);
    const semanticEdgePalette = useMemo<SemanticEdgePalette>(() => ({
      execution: cssColor("--vscode-symbolIcon-functionForeground", "#5fa8ff"),
      dependency: cssColor("--vscode-symbolIcon-moduleForeground", "#56d4b4"),
      structure: cssColor("--vscode-symbolIcon-classForeground", "#e3b341"),
      flow: cssColor("--vscode-symbolIcon-eventForeground", "#ff9b87"),
      other: edgeColor
    }), [edgeColor, themeRevision]);
    const comparisonMode = useMemo(
      () => model.nodes.some((node) => node.change !== undefined)
        || model.edges.some((edge) => edge.change !== undefined),
      [model.edges, model.nodes]
    );
    const automaticLabelIds = useMemo(() => communityOverview
      ? new Set(communityOverviewLabelledIds(model.nodes, communityImportance))
      : new Set(
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
      ), [communityImportance, communityOverview, comparisonMode, model.nodes, renderingProfile]);

    // A community bubble carries its hue when it is signal: labelled by the
    // importance budget, or the bubble the reader is pointing at or has selected.
    // The rest is context, so hue marks what matters instead of decorating
    // every dot on the map.
    const signalBubble = useCallback((nodeId: string) => !communityOverview
      || automaticLabelIds.has(nodeId)
      || nodeId === focusedNodeId
      || nodeId === hoveredNodeId, [
      automaticLabelIds,
      communityOverview,
      focusedNodeId,
      hoveredNodeId
    ]);
    const expandedLabelIds = useMemo(() => renderingProfile === "static"
      ? new Set([...model.nodes]
        .sort((left, right) =>
          (right.degree ?? 0) - (left.degree ?? 0) || left.id.localeCompare(right.id))
        .slice(0, STATIC_VISIBLE_LABEL_LIMIT)
        .map((node) => node.id))
      : new Set<string>(), [model.nodes, renderingProfile]);
    const comparisonPositions = useMemo(
      () => comparisonMode ? seedComparisonPositions(model.nodes) : new Map(),
      [comparisonMode, model.nodes]
    );
    const fixedLayoutSpacing = layoutStyle === "automatic" && renderingProfile !== "static"
      ? 2
      : layoutSpacing;
    const selectedLayoutPositions = useMemo(
      () => layoutStyle === "automatic"
        ? new Map<string, { x: number; y: number }>()
        : seedGraphLayoutPositions(model.nodes, layoutStyle, model.stats.aggregated),
      [layoutStyle, model.nodes, model.stats.aggregated]
    );
    const staticPositions = useMemo(
      () => communityOverview
        ? seedCommunityOverviewPositions(model.nodes, communityImportance)
        : !comparisonMode
        ? seedStaticGraphPositions(model.nodes, model.stats.aggregated)
        : new Map(),
      [
        communityImportance,
        communityOverview,
        comparisonMode,
        model.nodes,
        model.stats.aggregated,
        renderingProfile
      ]
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
        const position = selectedLayoutPositions.get(node.id)
          ?? comparisonPositions.get(node.id)
          ?? initialPositions?.get(node.id)
          ?? staticPositions.get(node.id);
        const spacedPosition = position
          ? { x: position.x * fixedLayoutSpacing, y: position.y * fixedLayoutSpacing }
          : undefined;
        // Unlabelled long-tail bubbles sit back so the labelled communities and
        // their relationships stay the first thing a reader sees.
        const restOpacity = communityOverview && !automaticLabelIds.has(node.id)
          ? 0.82
          : node.change === "unchanged" ? 0.58 : 1;
        return {
          id: node.id,
          label: communityOverview
            ? communityBubbleLabel(node)
            : node.label,
          color: signalBubble(node.id)
            ? graphNodeColor(
              model,
              node,
              undefined,
              fallbackComparisonPalette,
              communityColors,
              semanticDetail ? fallbackSemanticNodePalette : undefined
            )
            : { background: CONTEXT_NODE_FILL, border: CONTEXT_NODE_BORDER },
          shape: semanticDetail
            ? nodeSemanticShape(nodeSemanticCategory(node.kind))
            : "dot",
          size,
          ...(spacedPosition ?? {}),
          opacity: restOpacity,
          ...(communityOverview ? { borderWidth: 2 } : {}),
          ...(communityOverview && signalBubble(node.id)
            ? { shadow: signalGlow(node) }
            : {}),
          font: {
            color: "#eef5ff",
            face: "system-ui",
            size: communityOverview
              ? automaticLabelIds.has(node.id) ? COMMUNITY_LABEL_FONT_SIZE : 0
              : comparisonMode
                ? automaticLabelIds.has(node.id) ? 12 : 0
                : renderingProfile === "static"
                  ? automaticLabelIds.has(node.id) ? 12 : 0
                  : (node.degree ?? 1) >= maxDegree * 0.15 ? 12 : 0,
            vadjust: communityOverview ? Math.round(size + 12) : 0,
            ...(communityOverview
              ? { strokeWidth: 3, strokeColor: canvasBackground }
              : {})
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
      communityOverview,
      initialPositions,
      fixedLayoutSpacing,
      maxDegree,
      model,
      renderingProfile,
      semanticDetail,
      selectedLayoutPositions,
      staticPositions
    ]);
    const edgeData = useMemo(() => new DataSet<Edge>(
      renderedEdges.map((edge) => {
        const hinted = edgeSemanticHints?.get(edge.id);
        const appearance = effectiveEdgeAppearance(edge, comparisonEdgeAppearance(
          edge.change,
          edge.confidence,
          hinted
            ? fallbackSemanticEdgePalette[hinted]
            : semanticDetail
              ? fallbackSemanticEdgePalette[edgeSemanticCategory(edge.relation)]
              : "#60728b",
          fallbackComparisonPalette,
          edge.weight
        ));
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
    // The mount-time palette is theme-independent on purpose: the focus effect
    // restyles edges in place when the theme changes, so a theme switch never
    // destroys and rebuilds the Network.
    ), [comparisonMode, edgeSemanticHints, renderedEdges, renderingProfile, semanticDetail]);
    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;
      initialViewRef.current = null;
      previousLayoutSpacingRef.current = fixedLayoutSpacing;
      const options = communityOverview
        ? communityOverviewOptions
        : renderingProfile === "static"
          ? staticOptions
          : comparisonMode ? comparisonOptions : defaultOptions;
      const network = new Network(container, {
        nodes: nodeData,
        edges: edgeData
      }, withPhysicsEnabled(options, physicsRunningRef.current));
      if (!physicsRunningRef.current) network.stopSimulation();
      networkRef.current = network;
      bindGraphNetworkEvents(network, {
        onFocus: (nodeId) => eventHandlersRef.current.onFocus(nodeId),
        onOpenSource: (nodeId) => eventHandlersRef.current.onOpenSource(nodeId),
        onOpenRelationshipSource: (edgeId) => eventHandlersRef.current.onOpenRelationshipSource(edgeId),
        onInteractionStart: () => {
          network.stopSimulation();
          eventHandlersRef.current.onInteractionStart();
        },
        onHover: (change) => eventHandlersRef.current.onHover(change),
        onHoverEdge: (change) => eventHandlersRef.current.onHoverEdge(change),
        onClear: () => eventHandlersRef.current.onClear()
      });
      network.on("stabilizationIterationsDone", () => {
        // vis-network can continue its dynamic phase after the configured
        // stabilization iterations. Freeze synchronously before React removes
        // the loading screen so the first interactive frame cannot drift.
        network.stopSimulation();
        if (communityOverview) {
          // The simulation is good at structure and bad at labels and balance:
          // re-centre it on importance so the big communities hold the middle
          // and the tail scatters outside, separate the bubbles so every label
          // keeps its room, then re-fit.
          const settled = new Map(Object.entries(network.getPositions()));
          const relaxed = relaxCommunityOverviewPositions(
            model.nodes,
            centerCommunityOverviewPositions(model.nodes, settled, communityImportance),
            communityImportance
          );
          nodeData.update([...relaxed].map(([id, position]) => ({
            id,
            x: position.x,
            y: position.y
          })));
          network.redraw();
        }
        // Fit the final positions before revealing any automatically arranged graph.
        if (physicsRunningRef.current) network.fit({ animation: false });
        initialViewRef.current = {
          position: network.getViewPosition(),
          scale: network.getScale()
        };
        onStabilizedRef.current();
        refreshMinimap();
      });
      network.on("stabilized", refreshMinimap);
      network.on("dragEnd", refreshMinimap);
      network.on("zoom", refreshMinimap);
      network.on("resize", refreshMinimap);
      network.on("animationFinished", refreshMinimap);
      const hasSeededPositions = comparisonPositions.size > 0
        || selectedLayoutPositions.size > 0
        || (initialPositions?.size ?? 0) > 0
        || staticPositions.size > 0;
      if (!physicsRunningRef.current && hasSeededPositions) {
        network.stopSimulation();
        // The stage can still be unsized on the first frame, which would fit the
        // seeded layout against a zero-sized canvas. Retry on the container's
        // first real resize so the opening camera always frames the whole graph.
        const fitSeededLayout = (): boolean => {
          const element = containerRef.current;
          if (!element || element.clientWidth <= 1 || element.clientHeight <= 1) {
            return false;
          }
          network.fit({ animation: false });
          initialViewRef.current = {
            position: network.getViewPosition(),
            scale: network.getScale()
          };
          return true;
        };
        if (!fitSeededLayout()) {
          const fitOnResize = () => {
            if (fitSeededLayout()) network.off("resize", fitOnResize);
          };
          network.on("resize", fitOnResize);
        }
      }
      const minimapTimer = window.setTimeout(refreshMinimap, 0);
      return () => {
        window.clearTimeout(minimapTimer);
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
      selectedLayoutPositions,
      staticPositions,
      refreshMinimap
    ]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network) return;
      const wasRunning = previousPhysicsRunningRef.current;
      previousPhysicsRunningRef.current = physicsRunning;
      network.setOptions({ physics: { enabled: physicsRunning } });
      if (physicsRunning) {
        if (!wasRunning) reheatGraphLayout(network, visibleNodeIdsRef.current);
        network.startSimulation();
      } else {
        network.stopSimulation();
      }
    }, [edgeData, nodeData, physicsRunning]);

    useEffect(() => {
      const network = networkRef.current;
      if (!network || layoutStyle !== "automatic" || renderingProfile === "static") return;
      const previousSpacing = previousLayoutSpacingRef.current;
      previousLayoutSpacingRef.current = layoutSpacing;
      network.setOptions({
        physics: comparisonMode
          ? {
              enabled: physicsRunning,
              barnesHut: { springLength: 180 * layoutSpacing }
            }
          : {
              enabled: physicsRunning,
              forceAtlas2Based: { springLength: (communityOverview ? 96 : 120) * layoutSpacing }
            }
      });
      if (!physicsRunning && previousSpacing !== layoutSpacing) {
        const ratio = layoutSpacing / previousSpacing;
        const positions = network.getPositions();
        nodeData.update(Object.entries(positions).map(([id, position]) => ({
          id,
          x: position.x * ratio,
          y: position.y * ratio
        })));
        network.redraw();
        refreshMinimap();
      }
    }, [
      communityOverview,
      comparisonMode,
      layoutSpacing,
      layoutStyle,
      nodeData,
      physicsRunning,
      refreshMinimap,
      renderingProfile
    ]);

    useEffect(() => {
      nodeData.update(model.nodes.map((node) => ({
        id: node.id,
        hidden: !visibleNodeIds.has(node.id)
      })));
      edgeData.update(renderedEdges.map((edge) => ({
        id: edge.id,
        hidden: !visibleEdgeIds.has(edge.id)
      })));
      refreshMinimap();
    }, [
      edgeData,
      refreshMinimap,
      renderedEdges,
      model.nodes,
      nodeData,
      visibleEdgeIds,
      visibleNodeIds
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
        const comparisonOpacity = communityOverview && !automaticLabelIds.has(node.id)
          ? 0.82
          : node.change === "unchanged" ? 0.58 : 1;
        return {
          id: node.id,
          opacity: !focusedNodeId
            ? comparisonOpacity
            : isVisible ? Math.max(comparisonOpacity, 0.72) : 0.32,
          borderWidth: isFocused ? 4 : contrastBorder ? 2.5 : communityOverview ? 2 : 1.5,
          color: signalBubble(node.id) || isVisible && focusedNodeId !== null
            ? graphNodeColor(
              model,
              node,
              contrastBorder,
              comparisonPalette,
              communityColors,
              semanticDetail ? semanticNodePalette : undefined
            )
            : { background: CONTEXT_NODE_FILL, border: CONTEXT_NODE_BORDER },
          shadow: isFocused
            ? {
                enabled: true,
                color: node.change
                  ? comparisonPalette[node.change].border
                  : semanticDetail
                    ? semanticNodePalette[nodeSemanticCategory(node.kind)].border
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
      edgeData.update(renderedEdges.map((edge) => {
        const hinted = edgeSemanticHints?.get(edge.id);
        const appearance = effectiveEdgeAppearance(edge, comparisonEdgeAppearance(
          edge.change,
          edge.confidence,
          hinted
            ? semanticEdgePalette[hinted]
            : semanticDetail
              ? semanticEdgePalette[edgeSemanticCategory(edge.relation)]
              : edgeColor,
          comparisonPalette,
          edge.weight
        ));
        const connectedEdge = edge.source === focusedNodeId || edge.target === focusedNodeId;
        return {
          id: edge.id,
          dashes: appearance.dashes,
          color: {
            color: appearance.color,
            opacity: !focusedNodeId ? appearance.opacity : connectedEdge ? 0.92 : 0.12
          },
          width: connectedEdge ? Math.max(3, appearance.width) : appearance.width
        };
      }));
      if (focusedNodeId) {
        network.selectNodes([focusedNodeId]);
        if (!physicsRunningRef.current) {
          const position = network.getPositions([focusedNodeId])[focusedNodeId];
          if (position) {
            network.moveTo({ position, scale: 1.35, animation: false });
          }
        } else {
          network.focus(focusedNodeId, {
            scale: 1.35,
            animation: window.matchMedia("(prefers-reduced-motion: reduce)").matches
              ? false
              : { duration: 260, easingFunction: "easeInOutQuad" }
          });
        }
      } else {
        network.unselectAll();
      }
    }, [
      automaticLabelIds,
      communityOverview,
      contrastBorder,
      comparisonPalette,
      communityColors,
      edgeColor,
      edgeData,
      edgeSemanticHints,
      focusedNodeId,
      model,
      nodeData,
      renderedEdges,
      semanticDetail,
      semanticEdgePalette,
      semanticNodePalette
    ]);

    useEffect(() => {
      nodeData.update(model.nodes.map((node) => ({
        id: node.id,
        font: {
          color: labelColor,
          size: communityOverview
            ? (forceLabels
              || node.id === focusedNodeId
              || automaticLabelIds.has(node.id))
              ? COMMUNITY_LABEL_FONT_SIZE
              : 0
            : (forceLabels && (renderingProfile !== "static" || expandedLabelIds.has(node.id)))
              || node.id === focusedNodeId
              || (comparisonMode
                ? automaticLabelIds.has(node.id)
                : renderingProfile === "static"
                  ? automaticLabelIds.has(node.id)
                  : (node.degree ?? 1) >= maxDegree * 0.15)
              ? 12
              : 0,
          ...(communityOverview
            ? {
              vadjust: communityBubbleFontSize(node),
              strokeWidth: 3,
              strokeColor: canvasBackground
            }
            : {})
        }
      })));
    }, [
      automaticLabelIds,
      canvasBackground,
      comparisonMode,
      communityOverview,
      expandedLabelIds,
      focusedNodeId,
      forceLabels,
      labelColor,
      maxDegree,
      model.nodes,
      nodeData,
      renderingProfile
    ]);

    useEffect(() => {
      edgeData.update(renderedEdges.map((edge) => ({
        id: edge.id,
        label: showEdgeLabels ? edge.relation : "",
        font: {
          align: "middle",
          color: edgeLabelColor,
          face: "system-ui",
          size: 10,
          strokeWidth: 3,
          strokeColor: cssColor(
            "--vscode-editor-background",
            cssColor("--background", "#08111f")
          )
        }
      })));
    }, [edgeData, edgeLabelColor, renderedEdges, showEdgeLabels]);

    useImperativeHandle(ref, () => ({
      fit() {
        networkRef.current?.fit({
          animation: cameraAnimation(280)
        });
      },
      fitSelection(nodeIds) {
        const network = networkRef.current;
        if (!network || nodeIds.length === 0) return;
        network.fit({
          nodes: [...new Set(nodeIds)],
          animation: cameraAnimation(240)
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
      },
      resetZoom() {
        networkRef.current?.moveTo({
          scale: 1,
          animation: cameraAnimation(180)
        });
      },
      zoomIn() {
        const network = networkRef.current;
        if (!network) return;
        network.moveTo({
          scale: Math.min(MAX_VIEW_SCALE, network.getScale() * 1.25),
          animation: cameraAnimation(160)
        });
      },
      zoomOut() {
        const network = networkRef.current;
        if (!network) return;
        network.moveTo({
          scale: Math.max(MIN_VIEW_SCALE, network.getScale() / 1.25),
          animation: cameraAnimation(160)
        });
      }
    }), []);

    return (
      <>
        <div
          ref={containerRef}
          className="compass-canvas"
          data-rendering-profile={renderingProfile}
          data-layout-style={layoutStyle}
          data-physics-running={physicsRunning ? "true" : "false"}
          data-isolated={isolatedNodeIds ? "true" : "false"}
          data-layout-spacing={layoutSpacing}
          role="region"
          aria-label="Interactive Compass code graph"
          onMouseLeave={() => {
            onHover(null);
            onHoverEdge(null);
          }}
        />
        {showMinimap && minimapSnapshot ? (
          <GraphMinimap
            model={model}
            snapshot={minimapSnapshot}
            visibleNodeIds={visibleNodeIds}
            visibleEdgeIds={visibleEdgeIds}
            focusedNodeId={focusedNodeId}
            semanticDetail={semanticDetail}
            onNavigate={(position) => networkRef.current?.moveTo({
              position,
              animation: cameraAnimation(180)
            })}
          />
        ) : null}
      </>
    );
  }
);
