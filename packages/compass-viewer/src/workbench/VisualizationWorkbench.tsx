import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  BoxesIcon,
  BracesIcon,
  CircleDotDashedIcon,
  GitCompareArrowsIcon,
  GitForkIcon,
  NetworkIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  RouteIcon,
  SearchCodeIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  WaypointsIcon
} from "lucide-react";
import type { ArchitectureLens, ArchitectureOverview, ArchitectureViewModel } from "../contracts/architecture";
import type { GraphViewModel } from "../contracts/graph";
import type { WorkbenchModel, WorkbenchView } from "../contracts/workbench";
import { ArchitectureMap, type ArchitectureSelection } from "../architecture/ArchitectureMap";
import { architectureOverview } from "../architecture/projection";
import { CallGraph } from "../calls/CallGraph";
import { CompassGraph, type GraphHost } from "../graph/CompassGraph";
import { codeQueryGraphViewModel } from "../graph/codeQueryGraph";
import type { InspectorLayout } from "../graph/inspectorLayout";
import { compareGraphs } from "../history/ComparisonOverlay";

export type VisualizationWorkbenchHost = GraphHost & {
  expandCall?(symbol: string, direction: "callers" | "callees" | "both", depth: number): void;
  changeCallDirection?(direction: "callers" | "callees" | "both"): void;
};

export function VisualizationWorkbench({
  workbench,
  host,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  initialInspectorLayout,
  onInspectorLayoutChange
}: {
  workbench: WorkbenchModel;
  host: VisualizationWorkbenchHost;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  initialInspectorLayout?: InspectorLayout | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
}) {
  const [activeViewId, setActiveViewId] = useState(() => hashView(workbench) ?? workbench.defaultView);
  const [navigationCollapsed, setNavigationCollapsed] = useState(false);
  const activeView = workbench.views.find((view) => view.id === activeViewId)
    ?? workbench.views[0];

  useEffect(() => {
    const onHash = () => {
      const selected = hashView(workbench);
      if (selected) setActiveViewId(selected);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, [workbench]);

  const selectView = (id: string) => {
    setActiveViewId(id);
    const next = `view=${encodeURIComponent(id)}`;
    if (window.location.hash.slice(1) !== next) window.location.hash = next;
  };

  if (!activeView) return null;
  return (
    <div
      className="visualization-workbench"
      data-navigation-collapsed={navigationCollapsed}
    >
      <aside
        className="visualization-rail"
        data-collapsed={navigationCollapsed}
        aria-label="Compass navigation"
      >
        <header>
          <span className="visualization-bearing" aria-hidden="true"><WaypointsIcon /></span>
          <span>
            <strong>Compass</strong>
            <small title={workbench.title}>{workbench.title}</small>
          </span>
          <button
            className="visualization-rail-disclosure"
            type="button"
            aria-label={navigationCollapsed
              ? "Expand graph navigation"
              : "Collapse graph navigation"}
            title={navigationCollapsed
              ? "Expand graph navigation"
              : "Collapse graph navigation"}
            onClick={() => setNavigationCollapsed((collapsed) => !collapsed)}
          >
            {navigationCollapsed
              ? <PanelLeftOpenIcon aria-hidden="true" />
              : <PanelLeftCloseIcon aria-hidden="true" />}
          </button>
        </header>
        <nav aria-label="Graph views">
          {workbench.views.map((view) => (
            <button
              key={view.id}
              type="button"
              aria-current={view.id === activeView.id ? "page" : undefined}
              onClick={() => selectView(view.id)}
            >
              <ViewIcon view={view} />
              <span>
                <strong>{view.title}</strong>
                <small>{view.description}</small>
              </span>
              <i data-status={view.coverage.status} title={`${view.coverage.status} view`} />
            </button>
          ))}
        </nav>
        <footer>
          <span>Snapshot</span>
          <code>{shortIdentity(workbench.graphIdentity)}</code>
        </footer>
      </aside>
      <main className="visualization-main">
        <header className="visualization-context">
          <div>
            <span>{viewEyebrow(activeView)}</span>
            <strong>{activeView.title}</strong>
          </div>
          <div className="visualization-coverage" data-status={activeView.coverage.status}>
            {activeView.kind !== "call" && (
              <>
                <span>{activeView.coverage.nodes.toLocaleString()} nodes</span>
                <span>{activeView.coverage.edges.toLocaleString()} relationships</span>
              </>
            )}
            <strong>{coverageLabel(activeView)}</strong>
          </div>
        </header>
        <section className="visualization-stage" aria-label={`${activeView.title} visualization`}>
          <WorkbenchView
            key={activeView.id}
            view={activeView}
            host={host}
            communityDetail={communityDetail}
            communityLoading={communityLoading}
            communityError={communityError}
            onBackToOverview={onBackToOverview}
            initialInspectorLayout={initialInspectorLayout}
            onInspectorLayoutChange={onInspectorLayoutChange}
          />
        </section>
      </main>
    </div>
  );
}

function WorkbenchView({
  view,
  host,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  initialInspectorLayout,
  onInspectorLayoutChange
}: {
  view: WorkbenchView;
  host: VisualizationWorkbenchHost;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  initialInspectorLayout?: InspectorLayout | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
}) {
  if (view.kind === "call") {
    return (
      <CallGraph
        graph={view.graph}
        host={{
          openSource: host.openSource,
          ...(host.expandCall ? { expand: host.expandCall } : {}),
          ...(host.changeCallDirection ? { changeDirection: host.changeCallDirection } : {})
        }}
      />
    );
  }
  if (view.kind === "architecture") {
    return <WorkbenchArchitecture model={view.model} host={host} />;
  }
  if (view.kind === "impact") {
    return (
      <FilteredGraph
        model={codeQueryGraphViewModel(view.result, view.title)}
        host={host}
        queryResult={view.result}
        preferredLayout="hierarchical"
      />
    );
  }
  if (view.kind === "history") {
    const comparison = compareGraphs(view.before, view.after);
    return (
      <div className="workbench-history-view">
        <div className="workbench-history-banner">
          <GitCompareArrowsIcon aria-hidden="true" />
          <span>
            <strong>{view.baseRevision} → {view.targetRevision}</strong>
            <small>
              +{comparison.addedNodes} / −{comparison.removedNodes} / Δ{comparison.changedNodes} nodes
            </small>
          </span>
        </div>
        <FilteredGraph
          model={comparison.graph}
          host={host}
          preferredLayout="grid"
          sourceRevisions={{
            before: view.baseRevision,
            after: view.targetRevision
          }}
        />
      </div>
    );
  }
  const preferredLayout = view.kind === "affected"
    ? "hierarchical" as const
    : view.kind === "artifact"
      ? view.lens === "routes" ? "hierarchical" as const : "grid" as const
      : "automatic" as const;
  return (
    <FilteredGraph
      model={view.model}
      host={host}
      preferredLayout={preferredLayout}
      communityDetails={view.kind === "code" ? view.communityDetails : undefined}
      communityDetail={view.kind === "code" ? communityDetail : undefined}
      communityLoading={view.kind === "code" ? communityLoading : undefined}
      communityError={view.kind === "code" ? communityError : undefined}
      onBackToOverview={view.kind === "code" ? onBackToOverview : undefined}
      initialInspectorLayout={initialInspectorLayout}
      onInspectorLayoutChange={onInspectorLayoutChange}
    />
  );
}

function FilteredGraph({
  model,
  host,
  queryResult,
  sourceRevisions,
  preferredLayout,
  communityDetails,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  initialInspectorLayout,
  onInspectorLayoutChange
}: {
  model: GraphViewModel;
  host: VisualizationWorkbenchHost;
  queryResult?: Parameters<typeof CompassGraph>[0]["queryResult"];
  sourceRevisions?: Parameters<typeof CompassGraph>[0]["sourceRevisions"];
  preferredLayout: Parameters<typeof CompassGraph>[0]["preferredLayout"];
  communityDetails?: Record<string, GraphViewModel> | undefined;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  initialInspectorLayout?: InspectorLayout | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
}) {
  const [relation, setRelation] = useState("");
  const [evidence, setEvidence] = useState("");
  const [kind, setKind] = useState("");
  const [language, setLanguage] = useState("");
  const [communityId, setCommunityId] = useState<number>();
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filterPanelId = useId();
  const filterButtonRef = useRef<HTMLButtonElement>(null);
  const filterPanelRef = useRef<HTMLDivElement>(null);
  const embeddedCommunityDetail = communityId === undefined ? undefined : {
    communityId,
    model: communityDetails?.[String(communityId)] ?? model
  };
  const activeCommunityDetail = embeddedCommunityDetail ?? communityDetail;
  const activeModel = activeCommunityDetail?.model ?? model;
  const activeGraphKey = activeCommunityDetail
    ? `community-${activeCommunityDetail.communityId}`
    : "overview";
  const options = useMemo(() => graphFilterOptions(activeModel), [activeModel]);
  const filtered = useMemo(
    () => filterGraph(activeModel, { relation, evidence, kind, language }),
    [activeModel, evidence, kind, language, relation]
  );
  useEffect(() => {
    setRelation("");
    setEvidence("");
    setKind("");
    setLanguage("");
    setFiltersOpen(false);
  }, [activeGraphKey]);
  useEffect(() => {
    if (!filtersOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setFiltersOpen(false);
      filterButtonRef.current?.focus();
    };
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (filterButtonRef.current?.contains(target) || filterPanelRef.current?.contains(target)) {
        return;
      }
      setFiltersOpen(false);
    };
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [filtersOpen]);
  const clearFilters = useCallback(() => {
    setRelation("");
    setEvidence("");
    setKind("");
    setLanguage("");
  }, []);
  const closeFilters = useCallback(() => setFiltersOpen(false), []);
  const activeFilterCount = [relation, evidence, kind, language].filter(Boolean).length;
  const filtersHideEveryNode = activeFilterCount > 0 && filtered.nodes.length === 0;
  return (
    <div className="workbench-graph-lens">
      <CompassGraph
        model={activeCommunityDetail ? model : filtered}
        communityDetail={activeCommunityDetail ? {
          ...activeCommunityDetail,
          model: filtered
        } : undefined}
        communityLoading={communityLoading}
        communityError={communityError}
        onBackToOverview={communityId === undefined ? onBackToOverview : () => setCommunityId(undefined)}
        initialInspectorLayout={initialInspectorLayout}
        onInspectorLayoutChange={onInspectorLayoutChange}
        host={{
          ...host,
          openCommunity: (id) => {
            if (communityDetails?.[String(id)]) setCommunityId(id);
            else host.openCommunity?.(id);
          }
        }}
        queryResult={queryResult}
        sourceRevisions={sourceRevisions}
        showInspectorHeader={false}
        preferredLayout={preferredLayout}
        toolbarLeading={(
          <button
            ref={filterButtonRef}
            className="workbench-filter-trigger"
            type="button"
            aria-label="Graph filters"
            aria-expanded={filtersOpen}
            aria-controls={filterPanelId}
            onClick={() => setFiltersOpen((open) => !open)}
          >
            <SlidersHorizontalIcon aria-hidden="true" />
            <span>Filters</span>
            {activeFilterCount > 0 ? <b>{activeFilterCount}</b> : null}
            <small>
              {filtered.nodes.length.toLocaleString()} / {activeModel.nodes.length.toLocaleString()}
            </small>
          </button>
        )}
        toolbarLeadingPanel={filtersOpen ? (
          <div
            ref={filterPanelRef}
            id={filterPanelId}
            className="workbench-filter-popover"
            role="region"
            aria-label="Graph filter options"
          >
            <header>
              <span>
                <strong>Filter graph</strong>
                <small>Refine visible nodes and relationships</small>
              </span>
              <button type="button" disabled={activeFilterCount === 0} onClick={clearFilters}>
                Clear filters
              </button>
            </header>
            <Filter label="Relationship" value={relation} values={options.relations} onChange={setRelation} />
            <Filter label="Evidence" value={evidence} values={options.evidence} onChange={setEvidence} />
            <Filter label="Node kind" value={kind} values={options.kinds} onChange={setKind} />
            <Filter label="Language" value={language} values={options.languages} onChange={setLanguage} />
            <footer role="status">
              {filtersHideEveryNode
                ? "No nodes match these filters"
                : `Showing ${filtered.nodes.length.toLocaleString()} of ${activeModel.nodes.length.toLocaleString()} nodes`}
            </footer>
          </div>
        ) : undefined}
        toolbarLeadingOpen={filtersOpen}
        onToolbarLeadingClose={closeFilters}
        stageOverlay={activeModel.nodes.length === 0 ? (
          <GraphEmptyState
            title="This view has no graph nodes"
            detail="The exported graph lens did not contain any nodes to visualize."
          />
        ) : filtersHideEveryNode && !filtersOpen ? (
          <GraphEmptyState
            title="No nodes match these filters"
            detail="Clear one or more filters to restore the graph."
            onClear={clearFilters}
          />
        ) : undefined}
      />
    </div>
  );
}

function GraphEmptyState({
  title,
  detail,
  onClear
}: {
  title: string;
  detail: string;
  onClear?: (() => void) | undefined;
}) {
  return (
    <div className="workbench-graph-empty" role="status">
      <NetworkIcon aria-hidden="true" />
      <strong>{title}</strong>
      <span>{detail}</span>
      {onClear ? <button type="button" onClick={onClear}>Clear filters</button> : null}
    </div>
  );
}

function Filter({
  label,
  value,
  values,
  onChange
}: {
  label: string;
  value: string;
  values: string[];
  onChange(value: string): void;
}) {
  return (
    <label>
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        <option value="">All</option>
        {values.map((item) => <option key={item} value={item}>{humanize(item)}</option>)}
      </select>
    </label>
  );
}

function WorkbenchArchitecture({
  model,
  host
}: {
  model: ArchitectureViewModel;
  host: VisualizationWorkbenchHost;
}) {
  const [scope, setScope] = useState<"production" | "all">("production");
  const [evidence, setEvidence] = useState<"all" | "extracted" | "inferred" | "ambiguous">("all");
  const [lens, setLens] = useState<ArchitectureLens>("architecture");
  const overview = useMemo(
    () => architectureOverview(model, { scope, evidence, lens }),
    [evidence, lens, model, scope]
  );
  const first = overview.groups.find((section) => section.nodeCount > 0);
  const [selection, setSelection] = useState<ArchitectureSelection>(
    first ? { kind: "group", id: first.id } : undefined
  );
  const projection = model.projections.find((candidate) =>
    candidate.scope === (scope === "all" ? "all_code" : "production"));
  const selectedGroup = selection?.kind === "group"
    ? projection?.groups.find((group) => group.id === selection.id)
    : undefined;
  const childIds = new Set(projection?.groups
    .filter((group) => group.parentId === selectedGroup?.id)
    .map((group) => group.id) ?? []);
  const selectedNodeIds = new Set(projection?.memberships
    .filter((membership) => {
      const groupId = projection.groups[membership.groupIndex]?.id;
      return groupId === selectedGroup?.id || (groupId !== undefined && childIds.has(groupId));
    })
    .map((membership) => model.nodes[membership.nodeIndex]?.id)
    .filter((nodeId): nodeId is string => nodeId !== undefined) ?? []);
  const selectedNodes = model.nodes.filter((node) => selectedNodeIds.has(node.id));
  const selectedRoute = selection?.kind === "route"
    ? overview.routes.find((route) => route.id === selection.id)
    : undefined;
  return (
    <div className="workbench-architecture">
      <div className="workbench-filter-strip" aria-label="Architecture filters">
        <Filter label="Source scope" value={scope} values={["production", "all"]} onChange={(value) => setScope(value as typeof scope)} />
        <Filter label="Evidence" value={evidence} values={["all", "extracted", "inferred", "ambiguous"]} onChange={(value) => setEvidence(value as typeof evidence)} />
        <Filter label="Lens" value={lens} values={["architecture", "execution", "dependency", "type", "structure", "all"]} onChange={(value) => setLens(value as ArchitectureLens)} />
        <label>
          Subsystem directory
          <select
            aria-label="Subsystem directory"
            value={selectedGroup?.id ?? ""}
            onChange={(event) => event.target.value
              && setSelection({ kind: "group", id: event.target.value })}
          >
            <option value="">Select a subsystem</option>
            {projection?.groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name.value} · {group.nodeCount.toLocaleString()} symbols
              </option>
            ))}
          </select>
        </label>
        <span role="status">{overview.statistics.visibleRelationships.toLocaleString()} visible relationships</span>
        <span role="status">Architecture quality: {overview.quality.status}</span>
        <span role="status">
          {overview.omissions.shownGroups.toLocaleString()} of {overview.omissions.totalGroups.toLocaleString()} groups shown
          {overview.omissions.omittedGroups > 0
            ? ` · ${overview.omissions.omittedGroups.toLocaleString()} available in directory`
            : ""}
        </span>
      </div>
      <div className="workbench-architecture-body">
        <ArchitectureMap overview={overview} selection={selection} onSelect={setSelection} />
        <aside aria-label="Architecture details">
          {selectedGroup ? (
            <>
              <span>Subsystem</span>
              <h2>{selectedGroup.name.value}</h2>
              <p>{selectedNodes.length.toLocaleString()} symbols · {selectedGroup.relationshipCount.toLocaleString()} related relationships</p>
              <small>Name source: {selectedGroup.name.provenance} · quality {selectedGroup.name.quality}/100</small>
              <div className="workbench-symbol-list">
                {selectedNodes.slice(0, 100).map((node) => (
                  <button
                    key={node.id}
                    type="button"
                    disabled={!node.sourceFile}
                    onClick={() => node.sourceFile && host.openSource({ file: node.sourceFile })}
                  >
                    <strong>{node.label}</strong>
                    <small>{node.kind}{node.sourceFile ? ` · ${node.sourceFile}` : ""}</small>
                  </button>
                ))}
              </div>
            </>
          ) : selectedRoute ? (
            <>
              <span>Subsystem route</span>
              <h2>{groupName(overview, selectedRoute.sourceGroup)} → {groupName(overview, selectedRoute.targetGroup)}</h2>
              <p>{selectedRoute.relationships.toLocaleString()} relationships cross this boundary.</p>
            </>
          ) : <p>Select a subsystem or route.</p>}
        </aside>
      </div>
    </div>
  );
}

function graphFilterOptions(model: GraphViewModel) {
  const unique = (values: Array<string | undefined>) => [...new Set(values.filter((value): value is string => Boolean(value)))].sort();
  return {
    relations: unique(model.edges.map((edge) => edge.relation)),
    evidence: unique(model.edges.map((edge) => edge.confidence)),
    kinds: unique(model.nodes.map((node) => node.kind)),
    languages: unique(model.nodes.map((node) => node.language))
  };
}

function filterGraph(
  model: GraphViewModel,
  filters: { relation: string; evidence: string; kind: string; language: string }
): GraphViewModel {
  const nodes = model.nodes.filter((node) =>
    (!filters.kind || node.kind === filters.kind)
    && (!filters.language || node.language === filters.language));
  let ids = new Set(nodes.map((node) => node.id));
  const edges = model.edges.filter((edge) =>
    ids.has(edge.source)
    && ids.has(edge.target)
    && (!filters.relation || edge.relation === filters.relation)
    && (!filters.evidence || edge.confidence === filters.evidence));
  if (filters.relation || filters.evidence) {
    const connected = new Set(edges.flatMap((edge) => [edge.source, edge.target]));
    for (const node of nodes) if (node.root) connected.add(node.id);
    ids = connected;
  }
  const visibleNodes = nodes.filter((node) => ids.has(node.id));
  const communityIds = new Set(visibleNodes.map((node) => node.community));
  return {
    ...model,
    stats: {
      ...model.stats,
      nodes: visibleNodes.length,
      edges: edges.length,
      communities: communityIds.size
    },
    nodes: visibleNodes,
    edges,
    communities: model.communities.filter((community) => communityIds.has(community.id))
  };
}

function ViewIcon({ view }: { view: WorkbenchView }) {
  const Icon = view.kind === "code" ? NetworkIcon
    : view.kind === "call" ? GitForkIcon
      : view.kind === "impact" ? CircleDotDashedIcon
        : view.kind === "architecture" ? BoxesIcon
          : view.kind === "history" ? GitCompareArrowsIcon
            : view.kind === "affected" ? ShieldCheckIcon
              : view.lens === "routes" ? RouteIcon
                : view.lens === "provenance" ? BracesIcon : SearchCodeIcon;
  return <Icon aria-hidden="true" />;
}

function hashView(workbench: WorkbenchModel): string | undefined {
  const value = new URLSearchParams(window.location.hash.slice(1)).get("view");
  return value && workbench.views.some((view) => view.id === value) ? value : undefined;
}

function shortIdentity(identity: string): string {
  const value = identity.startsWith("sha256:") ? identity.slice(7) : identity;
  return value.length > 12 ? value.slice(0, 12) : value;
}

function viewEyebrow(view: WorkbenchView): string {
  return view.kind === "artifact" ? `${humanize(view.lens)} lens` : `${humanize(view.kind)} graph`;
}

function coverageLabel(view: WorkbenchView): string {
  if (view.kind === "architecture") {
    return view.coverage.status === "complete" ? "Extraction complete"
      : view.coverage.status === "summary" ? "Extraction summary" : "Extraction bounded";
  }
  return view.coverage.status === "complete" ? "Complete"
    : view.coverage.status === "summary" ? "Community summary" : "Bounded result";
}

function humanize(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function groupName(overview: ArchitectureOverview, id: string): string {
  return overview.groups.find((section) => section.id === id)?.name ?? id;
}
