import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode
} from "react";
import { BoxesIcon, BracesIcon } from "lucide-react";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import type { CommunityHierarchyView } from "../contracts/hierarchy";
import { descendModel, scopeModel } from "./hierarchyLevels";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import {
  communityDetailModel,
  communityOverviewApplies,
  communityOverviewModel
} from "./communityOverview";
import {
  communityVariantData,
  type CommunityVariantData
} from "./communityVariants";
import { THEME_PREFERENCES, type ThemePreference } from "../lib/theme";
import { CommunityMatrix } from "./CommunityMatrix";
import { CommunityTreemap } from "./CommunityTreemap";
import { CommunityLanes } from "./CommunityLanes";
import type { EdgeSemanticCategory } from "./semanticAppearance";
import {
  Grid3x3Icon,
  LayoutDashboardIcon,
  MonitorIcon,
  MoonIcon,
  Rows3Icon,
  ScatterChartIcon,
  SunIcon
} from "lucide-react";

export type CommunityVariant = "bubbles" | "matrix" | "treemap" | "lanes";

/**
 * One icon per design so the switch reads at a glance and fits the rail: the
 * scattered map, the coupling grid, the area map, and the tier rows.
 */
const COMMUNITY_VARIANTS: ReadonlyArray<{
  value: CommunityVariant;
  label: string;
  hint: string;
  Icon: typeof ScatterChartIcon;
}> = [
  {
    value: "bubbles",
    label: "Bubbles",
    hint: "Packed community map on the canvas",
    Icon: ScatterChartIcon
  },
  {
    value: "matrix",
    label: "Matrix",
    hint: "Who couples to whom, cell by cell",
    Icon: Grid3x3Icon
  },
  {
    value: "treemap",
    label: "Area",
    hint: "Every community sized by symbol count",
    Icon: LayoutDashboardIcon
  },
  {
    value: "lanes",
    label: "Tiers",
    hint: "Importance tiers with coupling ribbons",
    Icon: Rows3Icon
  }
];
import { GraphInspector } from "./GraphInspector";
import { GraphTransitionScreen } from "./GraphTransitionScreen";
import { GraphToolbar } from "./GraphToolbar";
import { GraphSemanticLegend } from "./GraphSemanticLegend";
import { InspectorResizeHandle } from "./InspectorResizeHandle";
import {
  normalizeInspectorLayout,
  type InspectorLayout
} from "./inspectorLayout";
import { graphNodeActivation } from "./nodeActivation";
import { NodeHoverCard, type GraphHover } from "./NodeHoverCard";
import { EdgeHoverCard, type GraphEdgeHover } from "./EdgeHoverCard";
import { navigableRelationshipSource } from "./sourceNavigation";
import type { GraphSourceRevisions } from "./ChangeEvidence";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";
import {
  graphNeighborhood,
  MAX_NEIGHBORHOOD_DEPTH,
  MIN_NEIGHBORHOOD_DEPTH,
  type GraphEdgeDirection
} from "./neighborhood";
import {
  autoLayoutApplies,
  visibleGraphEdges,
  type GraphLayoutStyle
} from "./renderingProfile";
import {
  graphReducer,
  initialGraphStateForModel,
  type GraphChangeType
} from "./state";

const CHANGE_TYPES: Array<{
  value: GraphChangeType;
  label: string;
}> = [
  { value: "added", label: "Added" },
  { value: "removed", label: "Removed" },
  { value: "changed", label: "Changed" },
  { value: "unchanged", label: "Context" }
];

const FIXED_LAYOUT_STATUS: Record<Exclude<GraphLayoutStyle, "automatic">, string> = {
  hierarchical: "Depth-layer layout",
  circle: "Circle layout",
  concentric: "Concentric layout",
  spiral: "Spiral layout",
  grid: "Square grid layout"
};

const EDGE_DIRECTIONS: readonly GraphEdgeDirection[] = ["both", "outgoing", "incoming"];

function isEditableKeyboardTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement
    && (target.isContentEditable
      || target.tagName === "INPUT"
      || target.tagName === "TEXTAREA"
      || target.tagName === "SELECT");
}

export type GraphHost = {
  openSource(source: SourceLocation, revision?: string): void;
  openCommunity?(communityId: number): void;
  queryNode?(operation: "callers" | "callees" | "impact", symbol: string): void;
};

export type CommunityGraphDetail = {
  communityId: number;
  model: GraphViewModel;
  bounded?: {
    limit: number;
    parentMembers: number;
    currentMembers: number;
    /**
     * `viewer` marks a bound the viewer chose for a drill-down it computed
     * itself; omitting it keeps the exported `compass.graphNodeLimit` wording.
     */
    scope?: "viewer" | undefined;
  } | undefined;
};

export type CompassGraphProps = {
  model: GraphViewModel;
  host: GraphHost;
  /**
   * The published community hierarchy, when the export embedded one. Levels are
   * the reader's overview: level 0 opens first and double-clicking a group
   * descends one level instead of opening a community.
   */
  hierarchy?: CommunityHierarchyView | undefined;
  communityDetail?: CommunityGraphDetail | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  sourceRevisions?: GraphSourceRevisions | undefined;
  queryResult?: CodeQueryResponse | undefined;
  initialInspectorLayout?: Partial<InspectorLayout> | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
  preferredLayout?: GraphLayoutStyle | undefined;
  toolbarLeading?: ReactNode;
  toolbarLeadingPanel?: ReactNode;
  toolbarLeadingOpen?: boolean | undefined;
  onToolbarLeadingClose?: (() => void) | undefined;
  stageOverlay?: ReactNode;
  showInspectorHeader?: boolean | undefined;
  themePreference?: ThemePreference | undefined;
  onThemePreferenceChange?: ((next: ThemePreference) => void) | undefined;
};

export function CompassGraph({
  model,
  host,
  hierarchy,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  sourceRevisions,
  queryResult,
  initialInspectorLayout,
  onInspectorLayoutChange,
  preferredLayout = "automatic",
  toolbarLeading,
  toolbarLeadingPanel,
  toolbarLeadingOpen,
  onToolbarLeadingClose,
  stageOverlay,
  showInspectorHeader = true,
  themePreference,
  onThemePreferenceChange
}: CompassGraphProps) {
  const [inspectorLayout, setInspectorLayout] = useState(
    () => normalizeInspectorLayout(initialInspectorLayout)
  );
  const updateInspectorLayout = useCallback((next: InspectorLayout) => {
    const normalized = normalizeInspectorLayout(next);
    setInspectorLayout(normalized);
    onInspectorLayoutChange?.(normalized);
  }, [onInspectorLayoutChange]);
  // A large unaggregated graph opens on a community overview the viewer derives
  // from the same model, so one screen shows readable, labelled communities
  // instead of thousands of unreadable symbols.
  const derivedOverview = useMemo(
    () => communityOverviewApplies(model) ? communityOverviewModel(model) : undefined,
    [model]
  );
  const [scope, setScope] = useState<"hierarchy" | "communities" | "symbols">(
    () => hierarchy ? "hierarchy" : "communities"
  );
  // Which published level the reader is reading, and the groups they descended
  // through to reach it. Descending narrows the level below to one group's
  // children, so the canvas reads as a zoom instead of a redraw.
  const [activeLevel, setActiveLevel] = useState(0);
  const [trail, setTrail] = useState<
    ReadonlyArray<{ level: number; groupIndex: number; label: string }>
  >([]);
  const hierarchyModel = useMemo(() => {
    if (!hierarchy || scope !== "hierarchy") {
      return undefined;
    }
    const descended = trail[trail.length - 1];
    if (descended === undefined) {
      return scopeModel(hierarchy, { kind: "level", level: activeLevel });
    }
    return descendModel(hierarchy, descended.level, descended.groupIndex)
      ?? scopeModel(hierarchy, { kind: "level", level: activeLevel });
  }, [activeLevel, hierarchy, scope, trail]);
  const hierarchyOpen = hierarchyModel !== undefined
    && communityDetail === undefined
    && scope === "hierarchy";
  // Design variants of the community overview. The canvas is one reading; the
  // matrix, area map, and tier map answer different questions about the same
  // communities, and the reader picks.
  const [variant, setVariant] = useState<CommunityVariant>("bubbles");
  const [derivedCommunityId, setDerivedCommunityId] = useState<number | null>(null);
  const [pendingFocusId, setPendingFocusId] = useState<string | null>(null);
  const derivedDetail = useMemo(
    () => derivedOverview && derivedCommunityId !== null
      ? communityDetailModel(model, derivedCommunityId)
      : undefined,
    [derivedCommunityId, derivedOverview, model]
  );
  const overviewOpen = !communityDetail
    && derivedOverview !== undefined
    && scope === "communities";
  const showCommunityOverview = overviewOpen && derivedDetail === undefined;
  const variantData = useMemo(
    () => showCommunityOverview
      ? communityVariantData(model, derivedOverview)
      : hierarchyOpen && hierarchyModel
        ? communityVariantData(hierarchyModel)
        : !derivedDetail && model.stats.aggregated
          ? communityVariantData(model)
          : undefined,
    [derivedDetail, derivedOverview, hierarchyModel, hierarchyOpen, model, showCommunityOverview]
  );
  const activeVariant: CommunityVariant = variantData ? variant : "bubbles";
  const activeModel = communityDetail?.model
    ?? derivedDetail?.model
    ?? hierarchyModel
    ?? (showCommunityOverview ? derivedOverview!.model : model);
  const viewKey = communityDetail
    ? `community-${communityDetail.communityId}`
    : derivedDetail
      ? `community-${derivedCommunityId}`
      : hierarchyOpen ? `level-${activeLevel}-${trail.length}` : showCommunityOverview ? "communities" : "overview";
  const activeDetailCommunityId = communityDetail?.communityId
    ?? (derivedDetail ? derivedCommunityId ?? undefined : undefined);
  const activeBounded = communityDetail?.bounded
    ?? (derivedDetail?.bounded
      ? { ...derivedDetail.bounded, scope: "viewer" as const }
      : undefined);
  const backToOverview = useCallback(() => {
    setDerivedCommunityId(null);
    setTrail((current) => current.slice(0, -1));
    onBackToOverview?.();
  }, [onBackToOverview]);
  // Opening a group descends one level while it has children; the finest level
  // has none, so it opens the community's symbols instead.
  const openGroup = useCallback((groupIndex: number) => {
    const level = hierarchy?.levels.find((entry) => entry.level === activeLevel);
    const group = level?.groups.find((entry) => entry.index === groupIndex);
    if (group && group.childIndices.length > 0) {
      setTrail((current) => [
        ...current,
        { level: activeLevel, groupIndex, label: group.label }
      ]);
      setActiveLevel(activeLevel + 1);
      return;
    }
    setDerivedCommunityId(group?.community ?? groupIndex);
  }, [activeLevel, hierarchy]);
  // Community activation resolves inside the viewer whenever the overview was
  // derived locally; exported or host-provided details keep their own host.
  const viewHost = useMemo<GraphHost>(
    () => {
      if (hierarchy && scope === "hierarchy" && communityDetail === undefined) {
        return { ...host, openCommunity: openGroup };
      }
      return derivedOverview && communityDetail === undefined
        ? { ...host, openCommunity: (communityId) => setDerivedCommunityId(communityId) }
        : host;
    },
    [communityDetail, derivedOverview, hierarchy, host, openGroup, scope]
  );
  useEffect(() => {
    setPendingFocusId(null);
  }, [viewKey]);
  return (
    <CompassGraphView
      key={viewKey}
      model={activeModel}
      host={viewHost}
      detailCommunityId={activeDetailCommunityId}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={activeDetailCommunityId !== undefined || trail.length > 0
        ? backToOverview
        : undefined}
      bounded={activeBounded}
      initialFocusedNodeId={pendingFocusId ?? undefined}
      overviewSummary={showCommunityOverview
        ? {
          communities: derivedOverview!.model.stats.communities,
          symbols: model.nodes.length
        }
        : hierarchyOpen
          ? { communities: activeModel.nodes.length, symbols: model.nodes.length }
          : undefined}
      communityImportance={showCommunityOverview ? derivedOverview!.importance : undefined}
      communityOrder={showCommunityOverview ? derivedOverview!.communityOrder : undefined}
      edgeSemanticHints={showCommunityOverview ? derivedOverview!.edgeCategories : undefined}
      breadcrumbTrail={hierarchy !== undefined && scope === "hierarchy"
        ? [
          {
            label: "Repository",
            onSelect: () => {
              setTrail([]);
              setActiveLevel(0);
            }
          },
          ...trail.map((step, position) => ({
            label: step.label,
            onSelect: () => {
              setTrail(trail.slice(0, position));
              setActiveLevel(step.level + 1);
            }
          }))
        ]
        : undefined}
      variant={activeVariant}
      variantData={variantData}
      onVariantChange={setVariant}
      searchModel={showCommunityOverview ? model : undefined}
      scopeControls={hierarchy && communityDetail === undefined
        ? (
          <div
            className="compass-scope-toggle compass-level-toggle"
            role="group"
            aria-label="Graph scope"
          >
            {hierarchy.levels.map((level) => (
              <button
                key={level.level}
                type="button"
                aria-label={`Level ${level.level}`}
                aria-pressed={scope === "hierarchy" && activeLevel === level.level && trail.length === 0}
                title={`${level.merge === "locationAffinity" ? "Grouped by shared location" : "Grouped by relationship evidence"} · ${
                  level.groupCount.toLocaleString()} groups`}
                onClick={() => {
                  setScope("hierarchy");
                  setTrail([]);
                  setActiveLevel(level.level);
                  setDerivedCommunityId(null);
                }}
              >
                <span>{`Level ${level.level}`}</span>
              </button>
            ))}
            <button
              type="button"
              aria-label="Symbols"
              aria-pressed={scope === "symbols"}
              title={`Show all ${model.nodes.length.toLocaleString()} symbols`}
              onClick={() => {
                setScope("symbols");
                setDerivedCommunityId(null);
              }}
            >
              <BracesIcon aria-hidden="true" />
              <span>Symbols</span>
            </button>
          </div>
        )
        : derivedOverview && communityDetail === undefined
        ? (
          <div
            className="compass-scope-toggle"
            role="group"
            aria-label="Graph scope"
          >
            <button
              type="button"
              aria-label="Communities"
              aria-pressed={scope === "communities"}
              title="Show one labelled bubble per community"
              onClick={() => {
                setScope("communities");
                setDerivedCommunityId(null);
              }}
            >
              <BoxesIcon aria-hidden="true" />
              <span>Communities</span>
            </button>
            <button
              type="button"
              aria-label="Symbols"
              aria-pressed={scope === "symbols"}
              title={`Show all ${model.nodes.length.toLocaleString()} symbols`}
              onClick={() => {
                setScope("symbols");
                setDerivedCommunityId(null);
              }}
            >
              <BracesIcon aria-hidden="true" />
              <span>Symbols</span>
            </button>
          </div>
        )
        : undefined}
      onOpenCommunityFromSearch={derivedOverview && showCommunityOverview
        ? (communityId, nodeId) => {
          setPendingFocusId(nodeId);
          setDerivedCommunityId(communityId);
        }
        : undefined}
      autoLayoutDetail={derivedDetail !== undefined}
      sourceRevisions={sourceRevisions}
      queryResult={queryResult}
      inspectorLayout={inspectorLayout}
      onInspectorLayoutChange={updateInspectorLayout}
      preferredLayout={preferredLayout}
      toolbarLeading={toolbarLeading}
      toolbarLeadingPanel={toolbarLeadingPanel}
      toolbarLeadingOpen={toolbarLeadingOpen}
      onToolbarLeadingClose={onToolbarLeadingClose}
      stageOverlay={stageOverlay}
      showInspectorHeader={showInspectorHeader}
      themePreference={themePreference}
      onThemePreferenceChange={onThemePreferenceChange}
    />
  );
}

function CompassGraphView({
  model,
  host,
  detailCommunityId,
  communityLoading,
  communityError,
  onBackToOverview,
  bounded,
  initialFocusedNodeId,
  overviewSummary,
  communityImportance,
  communityOrder,
  edgeSemanticHints,
  breadcrumbTrail,
  variant,
  variantData,
  onVariantChange,
  searchModel,
  scopeControls,
  onOpenCommunityFromSearch,
  autoLayoutDetail,
  themePreference,
  onThemePreferenceChange,
  sourceRevisions,
  queryResult,
  inspectorLayout,
  onInspectorLayoutChange,
  preferredLayout,
  toolbarLeading,
  toolbarLeadingPanel,
  toolbarLeadingOpen,
  onToolbarLeadingClose,
  stageOverlay,
  showInspectorHeader
}: {
  model: GraphViewModel;
  host: GraphHost;
  detailCommunityId?: number | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  bounded?: CommunityGraphDetail["bounded"];
  initialFocusedNodeId?: string | undefined;
  overviewSummary?: { communities: number; symbols: number } | undefined;
  communityImportance?: ReadonlyMap<string, number> | undefined;
  communityOrder?: readonly number[] | undefined;
  edgeSemanticHints?: ReadonlyMap<string, EdgeSemanticCategory> | undefined;
  /**
   * The path a reader descended through, rendered as the graph breadcrumb so
   * stepping back up is one click instead of a remembered gesture.
   */
  breadcrumbTrail?: ReadonlyArray<{ label: string; onSelect: () => void }> | undefined;
  variant: CommunityVariant;
  variantData?: CommunityVariantData | undefined;
  onVariantChange(variant: CommunityVariant): void;
  searchModel?: GraphViewModel | undefined;
  scopeControls?: ReactNode;
  onOpenCommunityFromSearch?:
    ((communityId: number, nodeId: string) => void) | undefined;
  /**
   * Set for a drill-down the viewer opened itself: the community is arranged
   * once so the symbols arrive in a readable order instead of a random scatter.
   */
  autoLayoutDetail?: boolean | undefined;
  themePreference?: ThemePreference | undefined;
  onThemePreferenceChange?: ((next: ThemePreference) => void) | undefined;
  sourceRevisions?: GraphSourceRevisions | undefined;
  queryResult?: CodeQueryResponse | undefined;
  inspectorLayout: InspectorLayout;
  onInspectorLayoutChange(layout: InspectorLayout): void;
  preferredLayout: GraphLayoutStyle;
  toolbarLeading?: ReactNode;
  toolbarLeadingPanel?: ReactNode;
  toolbarLeadingOpen?: boolean | undefined;
  onToolbarLeadingClose?: (() => void) | undefined;
  stageOverlay?: ReactNode;
  showInspectorHeader: boolean;
}) {
  const [state, dispatch] = useReducer(
    graphReducer,
    model,
    (initial) => {
      const base = initialGraphStateForModel(initial, preferredLayout);
      // Automatic layout arranges itself for graphs a force simulation can
      // settle within the arranging screen, which covers every interactive
      // graph: symbol canvases, community overviews, and drill-downs. Larger
      // graphs keep their deterministic seeded map and wait for an explicit
      // Layout action so the first frame is never blocked.
      const arrangeNow = preferredLayout === "automatic" && autoLayoutApplies(initial);
      return {
        ...base,
        ...(arrangeNow ? { physicsRunning: true, initialLayoutPending: true } : {}),
        ...(initialFocusedNodeId ? { focusedNodeId: initialFocusedNodeId } : {})
      };
    }
  );
  const [hover, setHover] = useState<GraphHover | null>(null);
  const [edgeHover, setEdgeHover] = useState<GraphEdgeHover | null>(null);
  const canvasRef = useRef<GraphCanvasHandle>(null);
  const hostRef = useRef(host);
  hostRef.current = host;
  const nodeById = useMemo(
    () => new Map(model.nodes.map((node) => [node.id, node])),
    [model.nodes]
  );
  const edgeById = useMemo(
    () => new Map(model.edges.map((edge) => [edge.id, edge])),
    [model.edges]
  );
  const graphIndex = useMemo(() => {
    const neighborIds = new Map<string, Set<string>>();
    const edges = new Map<string, GraphViewModel["edges"]>();
    for (const edge of model.edges) {
      const sourceNeighbors = neighborIds.get(edge.source) ?? new Set<string>();
      sourceNeighbors.add(edge.target);
      neighborIds.set(edge.source, sourceNeighbors);
      const targetNeighbors = neighborIds.get(edge.target) ?? new Set<string>();
      targetNeighbors.add(edge.source);
      neighborIds.set(edge.target, targetNeighbors);
      const sourceEdges = edges.get(edge.source) ?? [];
      sourceEdges.push(edge);
      edges.set(edge.source, sourceEdges);
      const targetEdges = edges.get(edge.target) ?? [];
      targetEdges.push(edge);
      edges.set(edge.target, targetEdges);
    }
    return { neighborIds, edges };
  }, [model.edges]);
  // The community overview searches every symbol in the repository, not only
  // the communities on screen, so search stays the fastest way to reach code.
  const searchNodes = searchModel?.nodes ?? model.nodes;
  const searchIndex = useMemo(() => searchNodes.map((node) => ({
    node,
    text: [node.label, node.source?.file, node.kind]
      .filter((value) => value !== undefined)
      .join("\n")
      .toLocaleLowerCase()
  })), [searchNodes]);
  const renderedEdgeCount = useMemo(
    () => visibleGraphEdges(model).length,
    [model]
  );
  const selected = state.focusedNodeId
    ? nodeById.get(state.focusedNodeId)
    : undefined;
  const selectedNeighborhood = useMemo(
    () => selected
      ? graphNeighborhood(
        model,
        selected.id,
        state.neighborhoodDepth,
        state.edgeDirection
      )
      : null,
    [
      model,
      selected,
      state.edgeDirection,
      state.neighborhoodDepth
    ]
  );
  const hovered = hover ? nodeById.get(hover.nodeId) : undefined;
  const hoveredActivation = hovered
    ? graphNodeActivation(model, hovered, detailCommunityId)
    : undefined;
  const hoveredEdge = edgeHover ? edgeById.get(edgeHover.edgeId) : undefined;
  const hoveredEdgeSource = hoveredEdge ? nodeById.get(hoveredEdge.source) : undefined;
  const hoveredEdgeTarget = hoveredEdge ? nodeById.get(hoveredEdge.target) : undefined;
  const comparisonMode = useMemo(
    () => model.nodes.some((node) => node.change !== undefined)
      || model.edges.some((edge) => edge.change !== undefined),
    [model.edges, model.nodes]
  );
  const changeCounts = useMemo(() => {
    const counts = new Map<GraphChangeType, number>();
    for (const node of model.nodes) {
      const change = node.change ?? "unchanged";
      counts.set(change, (counts.get(change) ?? 0) + 1);
    }
    return counts;
  }, [model.nodes]);
  const neighbors = useMemo(() => {
    if (!selected) return [];
    return [...(graphIndex.neighborIds.get(selected.id) ?? [])]
      .map((id) => nodeById.get(id))
      .filter((node) => node !== undefined)
      .sort((left, right) => left.label.localeCompare(right.label));
  }, [graphIndex.neighborIds, nodeById, selected]);
  const connectedEdges = useMemo(
    () => selected
      ? graphIndex.edges.get(selected.id) ?? []
      : [],
    [graphIndex.edges, selected]
  );
  const deferredQuery = useDeferredValue(state.query);
  const matches = useMemo(() => {
    const query = deferredQuery.trim().toLocaleLowerCase();
    if (!query) return [];
    const found = [];
    for (const entry of searchIndex) {
      if (entry.text.includes(query)) found.push(entry.node);
      if (found.length === 20) break;
    }
    return found;
  }, [deferredQuery, searchIndex]);

  const focus = useCallback((nodeId: string) => {
    setHover(null);
    setEdgeHover(null);
    dispatch({ type: "focus", nodeId });
  }, []);
  const openCommunity = useCallback((communityId: number) => {
    hostRef.current.openCommunity?.(communityId);
  }, []);
  const focusSearchResult = useCallback((nodeId: string) => {
    if (nodeById.has(nodeId)) {
      focus(nodeId);
      return;
    }
    const target = searchModel?.nodes.find((node) => node.id === nodeId);
    if (!target || !onOpenCommunityFromSearch) return;
    onOpenCommunityFromSearch(target.community, target.id);
  }, [focus, nodeById, onOpenCommunityFromSearch, searchModel]);
  const pauseForInteraction = useCallback(() => {
    if (state.physicsRunning) {
      dispatch({ type: "setPhysics", running: false });
    }
  }, [state.physicsRunning]);
  const clear = useCallback(() => {
    setHover(null);
    setEdgeHover(null);
    dispatch({ type: "clearFocus" });
  }, []);
  useEffect(() => {
    if (state.focusedNodeId && !nodeById.has(state.focusedNodeId)) clear();
  }, [clear, nodeById, state.focusedNodeId]);
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.metaKey
        || event.ctrlKey
        || event.altKey
        || isEditableKeyboardTarget(event.target)
      ) return;
      const key = event.key.toLocaleLowerCase();
      let handled = true;
      if (key === "f") {
        if (event.shiftKey && selectedNeighborhood) {
          canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
        } else if (!event.shiftKey) {
          canvasRef.current?.fit();
        } else {
          handled = false;
        }
      } else if (key === "+" || key === "=") {
        canvasRef.current?.zoomIn();
      } else if (key === "-" || key === "_") {
        canvasRef.current?.zoomOut();
      } else if (key === "0") {
        canvasRef.current?.resetZoom();
      } else if (key === "i" && selected) {
        dispatch({ type: "setIsolation", isolated: !state.isolateSelection });
      } else if (key === "[") {
        dispatch({
          type: "setNeighborhoodDepth",
          depth: Math.max(MIN_NEIGHBORHOOD_DEPTH, state.neighborhoodDepth - 1)
        });
      } else if (key === "]") {
        dispatch({
          type: "setNeighborhoodDepth",
          depth: Math.min(MAX_NEIGHBORHOOD_DEPTH, state.neighborhoodDepth + 1)
        });
      } else if (key === "d") {
        const index = EDGE_DIRECTIONS.indexOf(state.edgeDirection);
        dispatch({
          type: "setEdgeDirection",
          direction: EDGE_DIRECTIONS[(index + 1) % EDGE_DIRECTIONS.length] ?? "both"
        });
      } else if (key === "m") {
        dispatch({ type: "setMinimap", visible: !state.showMinimap });
      } else if (key === "l") {
        // Labels moved out of the toolbar to keep the controls row readable;
        // the shortcuts and the graph settings panel are now their home.
        if (event.shiftKey) dispatch({ type: "setEdgeLabels", visible: !state.showEdgeLabels });
        else dispatch({ type: "setLabels", visible: !state.forceLabels });
      } else if (key === "escape" && onBackToOverview) {
        onBackToOverview();
      } else {
        handled = false;
      }
      if (handled) event.preventDefault();
    };
    document.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => document.removeEventListener("keydown", handleKeyDown, { capture: true });
  }, [
    onBackToOverview,
    selected,
    selectedNeighborhood,
    state.edgeDirection,
    state.forceLabels,
    state.isolateSelection,
    state.neighborhoodDepth,
    state.showEdgeLabels,
    state.showMinimap
  ]);
  const handleStabilized = useCallback(() => {
    dispatch({ type: "stabilized" });
  }, []);
  const revealCurrentLayout = useCallback(() => {
    dispatch({ type: "revealLayout" });
  }, []);
  const activateNode = useCallback((nodeId: string) => {
    const node = nodeById.get(nodeId);
    if (!node) return;
    const activation = graphNodeActivation(model, node, detailCommunityId);
    if (activation.type === "community") {
      hostRef.current.openCommunity?.(activation.communityId);
    }
    if (activation.type === "source") {
      hostRef.current.openSource(
        activation.source,
        node.change === "removed" ? sourceRevisions?.before : sourceRevisions?.after
      );
    }
  }, [
    detailCommunityId,
    nodeById,
    model.stats.aggregated,
    sourceRevisions?.after,
    sourceRevisions?.before
  ]);
  const activateRelationship = useCallback((edgeId: string) => {
    const edge = edgeById.get(edgeId);
    if (!edge) return;
    const source = navigableRelationshipSource(edge);
    if (!source) return;
    hostRef.current.openSource(
      source,
      edge.change === "removed" ? sourceRevisions?.before : sourceRevisions?.after
    );
  }, [edgeById, sourceRevisions?.after, sourceRevisions?.before]);
  const detailCommunityLabel = detailCommunityId !== undefined
    ? model.communities.find((community) => community.id === detailCommunityId)?.label
      ?? `Community ${detailCommunityId}`
    : undefined;
  const variantLabel = COMMUNITY_VARIANTS
    .find((option) => option.value === variant)?.label;
  const overviewCounts = overviewSummary
    ?? (variantData
      ? { communities: variantData.communities.length, symbols: variantData.symbols }
      : undefined);
  const overviewStatus = overviewCounts
    ? `${variant === "bubbles" || !variantLabel ? "" : `${variantLabel} · `}${
      overviewCounts.communities.toLocaleString()} communities · ${
      overviewCounts.symbols.toLocaleString()} symbols${
      state.layoutStyle === "automatic" && !autoLayoutApplies(model)
        ? " · press Layout to arrange"
        : ""}`
    : undefined;
  const status = selected && state.isolateSelection && selectedNeighborhood
    ? `Isolated ${selectedNeighborhood.nodeIds.size} nodes · ${state.neighborhoodDepth} hop${state.neighborhoodDepth === 1 ? "" : "s"}`
    : selected
    ? `Inspecting ${selected.label}`
    : detailCommunityLabel
      ? `Community ${detailCommunityLabel} · ${model.stats.nodes.toLocaleString()} symbols`
      : overviewStatus
        ? overviewStatus
        : state.layoutStyle !== "automatic"
          ? FIXED_LAYOUT_STATUS[state.layoutStyle]
          : state.physicsRunning
            ? "Layout running"
            : autoLayoutApplies(model)
              ? "Automatic layout"
              : `Static layout · ${model.nodes.length.toLocaleString()} nodes stay seeded`;
  // Only the canvas arranges itself; the matrix, area, and tier designs have no
  // simulation, so their views must not sit behind an arranging screen.
  const canvasVisible = variant === "bubbles";
  const loadingCommunity = communityLoading !== undefined && communityLoading !== null
    ? model.communities.find((community) => community.id === communityLoading)
    : undefined;
  const transition = communityLoading !== undefined && communityLoading !== null
    ? {
      kind: "community" as const,
      communityLabel: loadingCommunity?.label ?? `Community ${communityLoading}`
    }
    : state.initialLayoutPending && canvasVisible
      ? { kind: "layout" as const }
      : null;

  return (
    <div
      className="compass-workspace"
      style={{
        "--compass-inspector-width": `${inspectorLayout.width}px`
      } as CSSProperties}
    >
      <div
        className="compass-workspace-content"
        data-inspector-collapsed={inspectorLayout.collapsed}
        inert={transition ? true : undefined}
        aria-hidden={transition ? true : undefined}
      >
        <main
          className="compass-graph-stage"
          data-comparison={comparisonMode ? "true" : "false"}
          data-breadcrumb={overviewSummary !== undefined
            || autoLayoutDetail
            || (breadcrumbTrail !== undefined && breadcrumbTrail.length > 0)
            ? "true"
            : undefined}
        >
          {variantData && variant !== "bubbles" ? (
            <div className="compass-variant-stage">
              {variant === "matrix" ? (
                <CommunityMatrix data={variantData} onOpenCommunity={openCommunity} />
              ) : variant === "treemap" ? (
                <CommunityTreemap data={variantData} onOpenCommunity={openCommunity} />
              ) : (
                <CommunityLanes data={variantData} onOpenCommunity={openCommunity} />
              )}
            </div>
          ) : (
          <VisNetworkCanvas
            ref={canvasRef}
            model={model}
            focusedNodeId={selected?.id ?? null}
            hoveredNodeId={hover?.nodeId ?? null}
            physicsRunning={state.physicsRunning}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
            isolatedNodeIds={state.isolateSelection
              ? selectedNeighborhood?.nodeIds
              : undefined}
            isolatedEdgeIds={state.isolateSelection
              ? selectedNeighborhood?.edgeIds
              : undefined}
            layoutSpacing={state.layoutSpacing}
            showMinimap={state.showMinimap}
            semanticDetail={detailCommunityId !== undefined && !comparisonMode}
            communityImportance={communityImportance}
            edgeSemanticHints={edgeSemanticHints}
            hiddenCommunities={state.hiddenCommunities}
            hiddenChanges={state.hiddenChanges}
            onFocus={focus}
            onOpenSource={activateNode}
            onOpenRelationshipSource={activateRelationship}
            onInteractionStart={pauseForInteraction}
            onHover={setHover}
            onHoverEdge={setEdgeHover}
            onClear={clear}
            onStabilized={handleStabilized}
          />
          )}
          {stageOverlay}
          {breadcrumbTrail !== undefined && breadcrumbTrail.length > 0 ? (
            <nav
              className="compass-graph-breadcrumb compass-glass-panel"
              aria-label="Graph path"
            >
              {breadcrumbTrail.map((step, position) => (
                <span key={`${position}-${step.label}`} className="compass-breadcrumb-step">
                  {position > 0 ? (
                    <span className="compass-breadcrumb-separator" aria-hidden="true">
                      ▸
                    </span>
                  ) : null}
                  {position + 1 === breadcrumbTrail.length ? (
                    <span aria-current="page">{step.label}</span>
                  ) : (
                    <button type="button" onClick={step.onSelect}>
                      {step.label}
                    </button>
                  )}
                </span>
              ))}
            </nav>
          ) : overviewSummary !== undefined || autoLayoutDetail ? (
            <nav
              className="compass-graph-breadcrumb compass-glass-panel"
              aria-label="Graph path"
            >
              {detailCommunityId === undefined ? (
                <span aria-current="page">Repository</span>
              ) : (
                <>
                  <button type="button" onClick={onBackToOverview}>
                    Repository
                  </button>
                  <span className="compass-breadcrumb-separator" aria-hidden="true">
                    ▸
                  </span>
                  <span aria-current="page">
                    {detailCommunityLabel ?? `Community ${detailCommunityId}`}
                  </span>
                </>
              )}
            </nav>
          ) : null}
          <GraphToolbar
            status={status}
            physicsRunning={state.physicsRunning && canvasVisible}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
            hasSelection={selected !== undefined}
            isolateSelection={state.isolateSelection}
            neighborhoodDepth={state.neighborhoodDepth}
            edgeDirection={state.edgeDirection}
            layoutSpacing={state.layoutSpacing}
            showMinimap={state.showMinimap}
            themeControls={onThemePreferenceChange ? (
              <div
                className="compass-scope-toggle compass-theme-toggle"
                role="group"
                aria-label="Colour theme"
              >
                {THEME_PREFERENCES.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    aria-label={option.label}
                    aria-pressed={(themePreference ?? "auto") === option.value}
                    title={option.hint}
                    onClick={() => onThemePreferenceChange(option.value)}
                  >
                    {option.value === "auto"
                      ? <MonitorIcon aria-hidden="true" />
                      : option.value === "light"
                        ? <SunIcon aria-hidden="true" />
                        : <MoonIcon aria-hidden="true" />}
                  </button>
                ))}
              </div>
            ) : undefined}
            scopeControls={scopeControls}
            canvasControls={variant === "bubbles"}
            variantControls={variantData ? (
              <div
                className="compass-scope-toggle compass-variant-toggle"
                role="group"
                aria-label="Overview design"
              >
                {COMMUNITY_VARIANTS.map(({ value, label, hint, Icon }) => (
                  <button
                    key={value}
                    type="button"
                    aria-label={label}
                    aria-pressed={variant === value}
                    title={`${label} — ${hint}`}
                    onClick={() => onVariantChange(value)}
                  >
                    <Icon aria-hidden="true" />
                  </button>
                ))}
              </div>
            ) : undefined}
            leadingControls={toolbarLeading}
            leadingPanel={toolbarLeadingPanel}
            leadingPanelOpen={toolbarLeadingOpen}
            onLeadingPanelClose={onToolbarLeadingClose}
            onTogglePhysics={() => dispatch({
              type: "setPhysics",
              running: !state.physicsRunning
            })}
            onLayoutChange={(layout: GraphLayoutStyle) => dispatch({
              type: "setLayout",
              layout,
              runPhysics: false
            })}
            onZoomOut={() => canvasRef.current?.zoomOut()}
            onResetZoom={() => canvasRef.current?.resetZoom()}
            onZoomIn={() => canvasRef.current?.zoomIn()}
            onFit={() => canvasRef.current?.fit()}
            onFitSelection={() => {
              if (selectedNeighborhood) {
                canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
              }
            }}
            onReset={() => {
              clear();
              dispatch({ type: "setIsolation", isolated: false });
              canvasRef.current?.reset();
            }}
            onToggleLabels={() => dispatch({
              type: "setLabels",
              visible: !state.forceLabels
            })}
            onToggleEdgeLabels={() => dispatch({
              type: "setEdgeLabels",
              visible: !state.showEdgeLabels
            })}
            onToggleIsolation={() => {
              const isolated = !state.isolateSelection;
              dispatch({ type: "setIsolation", isolated });
              if (isolated && selectedNeighborhood) {
                window.requestAnimationFrame(() => {
                  canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
                });
              }
            }}
            onNeighborhoodDepthChange={(depth) => dispatch({
              type: "setNeighborhoodDepth",
              depth
            })}
            onEdgeDirectionChange={(direction) => dispatch({
              type: "setEdgeDirection",
              direction
            })}
            onLayoutSpacingChange={(spacing) => dispatch({
              type: "setLayoutSpacing",
              spacing
            })}
            onToggleMinimap={() => dispatch({
              type: "setMinimap",
              visible: !state.showMinimap
            })}
            onBack={onBackToOverview}
          />
          {comparisonMode && (
            <div className="compass-change-legend" aria-label="Graph change filters">
              {CHANGE_TYPES
                .filter(({ value }) => (changeCounts.get(value) ?? 0) > 0)
                .map(({ value, label }) => {
                  const visible = !state.hiddenChanges.has(value);
                  return (
                    <button
                      key={value}
                      type="button"
                      data-change={value}
                      aria-pressed={visible}
                      onClick={() => dispatch({ type: "toggleChange", change: value })}
                    >
                      <span aria-hidden="true" />
                      {label}
                      <small>{changeCounts.get(value) ?? 0}</small>
                    </button>
                  );
                })}
            </div>
          )}
          {detailCommunityId !== undefined && !comparisonMode ? (
            <GraphSemanticLegend model={model} />
          ) : null}
          {communityError && (
            <div
              className="absolute bottom-4 left-4 z-20 max-w-md rounded-md border border-destructive/50 bg-background/95 px-3 py-2 text-sm text-destructive shadow-lg"
              role="alert"
            >
              {communityError}
            </div>
          )}
          {bounded && (
            <div className="compass-bounded-notice" role="status">
              {bounded.scope === "viewer" ? (
                <>
                  <strong>Most connected symbols first</strong>
                  <span>
                    This community holds {bounded.parentMembers.toLocaleString()} symbols;
                    the view shows the {bounded.currentMembers.toLocaleString()} most
                    connected. Search for a symbol to open it directly.
                  </span>
                </>
              ) : (
                <>
                  <strong>Partial community comparison</strong>
                  <span>
                    This view is limited to {bounded.limit.toLocaleString()} nodes. Increase{" "}
                    <code>compass.graphNodeLimit</code> to inspect all{" "}
                    {Math.max(bounded.parentMembers, bounded.currentMembers).toLocaleString()} symbols.
                  </span>
                </>
              )}
            </div>
          )}
          {hover && hovered && hoveredActivation && (
            <NodeHoverCard
              node={hovered}
              hover={hover}
              activation={hoveredActivation}
            />
          )}
          {edgeHover && hoveredEdge && hoveredEdgeSource && hoveredEdgeTarget && (
            <EdgeHoverCard
              edge={hoveredEdge}
              sourceNode={hoveredEdgeSource}
              targetNode={hoveredEdgeTarget}
              hover={edgeHover}
            />
          )}
        </main>
        {!inspectorLayout.collapsed && (
          <InspectorResizeHandle
            width={inspectorLayout.width}
            onResize={(width) => onInspectorLayoutChange({
              ...inspectorLayout,
              width
            })}
          />
        )}
        <GraphInspector
          model={model}
          selected={selected}
          neighbors={neighbors}
          connectedEdges={connectedEdges}
          query={state.query}
          matches={matches}
          communityOrder={communityOrder}
          hiddenCommunities={state.hiddenCommunities}
          comparisonMode={comparisonMode}
          sourceRevisions={sourceRevisions}
          queryResult={queryResult}
          renderedEdgeCount={renderedEdgeCount}
          showHeader={showInspectorHeader}
          onQueryChange={(query) => dispatch({ type: "search", query })}
          onFocus={focusSearchResult}
          onOpenSource={host.openSource}
          onOpenCommunity={detailCommunityId === undefined ? host.openCommunity : undefined}
          searchSpansCommunities={searchModel ? true : undefined}
          onQueryNode={host.queryNode}
          onToggleCommunity={(communityId) => dispatch({
            type: "toggleCommunity",
            communityId
          })}
          onSetAllVisible={(visible) => dispatch({
            type: "setHiddenCommunities",
            communityIds: visible ? [] : model.communities.map((community) => community.id)
          })}
          collapsed={inspectorLayout.collapsed}
          onToggleCollapsed={() => onInspectorLayoutChange({
            ...inspectorLayout,
            collapsed: !inspectorLayout.collapsed
          })}
        />
      </div>
      {transition?.kind === "community" ? (
        <GraphTransitionScreen
          kind="community"
          communityLabel={transition.communityLabel}
        />
      ) : transition?.kind === "layout" ? (
        <GraphTransitionScreen kind="layout" onShowGraph={revealCurrentLayout} />
      ) : null}
    </div>
  );
}
