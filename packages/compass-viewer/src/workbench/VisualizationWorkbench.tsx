import { useEffect, useMemo, useState } from "react";
import {
  BoxesIcon,
  BracesIcon,
  CircleDotDashedIcon,
  GitCompareArrowsIcon,
  GitForkIcon,
  NetworkIcon,
  RouteIcon,
  SearchCodeIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  WaypointsIcon
} from "lucide-react";
import type { ArchitectureOverview } from "../contracts/architecture";
import type { CallflowViewModel } from "../contracts/callflow";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import type { WorkbenchModel, WorkbenchView } from "../contracts/workbench";
import { ArchitectureMap, type ArchitectureSelection } from "../architecture/ArchitectureMap";
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
    <div className="visualization-workbench">
      <aside className="visualization-rail">
        <header>
          <span className="visualization-bearing" aria-hidden="true"><WaypointsIcon /></span>
          <span>
            <strong>Compass</strong>
            <small>Graph workbench</small>
          </span>
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
  }, [activeGraphKey]);
  const activeFilterCount = [relation, evidence, kind, language].filter(Boolean).length;
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
        preferredLayout={preferredLayout}
        toolbarLeading={(
          <details className="workbench-filter-menu">
            <summary aria-label="Graph filters">
              <SlidersHorizontalIcon aria-hidden="true" />
              <span>Filters</span>
              {activeFilterCount > 0 ? <b>{activeFilterCount}</b> : null}
              <small>
                {filtered.nodes.length.toLocaleString()} / {activeModel.nodes.length.toLocaleString()}
              </small>
            </summary>
            <div className="workbench-filter-popover" role="region" aria-label="Graph filter options">
              <header>
                <span>
                  <strong>Filter graph</strong>
                  <small>Refine visible nodes and relationships</small>
                </span>
                <button
                  type="button"
                  disabled={activeFilterCount === 0}
                  onClick={() => {
                    setRelation("");
                    setEvidence("");
                    setKind("");
                    setLanguage("");
                  }}
                >
                  Clear all
                </button>
              </header>
              <Filter label="Relationship" value={relation} values={options.relations} onChange={setRelation} />
              <Filter label="Evidence" value={evidence} values={options.evidence} onChange={setEvidence} />
              <Filter label="Node kind" value={kind} values={options.kinds} onChange={setKind} />
              <Filter label="Language" value={language} values={options.languages} onChange={setLanguage} />
              <footer role="status">
                Showing {filtered.nodes.length.toLocaleString()} of {activeModel.nodes.length.toLocaleString()} nodes
              </footer>
            </div>
          </details>
        )}
      />
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
  model: CallflowViewModel;
  host: VisualizationWorkbenchHost;
}) {
  const [scope, setScope] = useState<"production" | "all">("production");
  const [evidence, setEvidence] = useState<"all" | "extracted" | "inferred" | "ambiguous">("all");
  const overview = useMemo(() => callflowOverview(model, scope, evidence), [evidence, model, scope]);
  const first = overview.sections.find((section) => section.nodeCount > 0);
  const [selection, setSelection] = useState<ArchitectureSelection>(
    first ? { kind: "section", id: first.id } : undefined
  );
  const selectedSection = selection?.kind === "section"
    ? model.sections.find((section) => section.id === selection.id)
    : undefined;
  const selectedRoute = selection?.kind === "route"
    ? overview.routes.find((route) => route.id === selection.id)
    : undefined;
  return (
    <div className="workbench-architecture">
      <div className="workbench-filter-strip" aria-label="Architecture filters">
        <Filter label="Source scope" value={scope} values={["production", "all"]} onChange={(value) => setScope(value as typeof scope)} />
        <Filter label="Evidence" value={evidence} values={["all", "extracted", "inferred", "ambiguous"]} onChange={(value) => setEvidence(value as typeof evidence)} />
        <span role="status">{overview.statistics.visibleCalls.toLocaleString()} visible calls</span>
      </div>
      <div className="workbench-architecture-body">
        <ArchitectureMap overview={overview} selection={selection} onSelect={setSelection} />
        <aside aria-label="Architecture details">
          {selectedSection ? (
            <>
              <span>Subsystem</span>
              <h2>{selectedSection.name}</h2>
              <p>{selectedSection.nodes.length.toLocaleString()} symbols · {selectedSection.edges.length.toLocaleString()} internal calls</p>
              <div className="workbench-symbol-list">
                {selectedSection.nodes.slice(0, 100).map((node) => (
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
              <h2>{sectionName(overview, selectedRoute.sourceSection)} → {sectionName(overview, selectedRoute.targetSection)}</h2>
              <p>{selectedRoute.calls.toLocaleString()} calls cross this boundary.</p>
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

function callflowOverview(
  model: CallflowViewModel,
  scope: "production" | "all",
  evidence: "all" | "extracted" | "inferred" | "ambiguous"
): ArchitectureOverview {
  const sections = model.sections.filter((section) => section.id !== "overview");
  const includeNode = (scopeValue: string) => scope === "all" || scopeValue === "production";
  const nodeById = new Map(sections.flatMap((section) => section.nodes).map((node) => [node.id, node]));
  const includeCall = (call: { source: string; target: string; confidence: string }) =>
    (evidence === "all" || call.confidence === evidence)
    && includeNode(nodeById.get(call.source)?.scope ?? "unknown")
    && includeNode(nodeById.get(call.target)?.scope ?? "unknown");
  const routes = new Map<string, typeof model.crossSectionCalls>();
  for (const call of model.crossSectionCalls.filter(includeCall)) {
    const id = `${call.sourceSection}->${call.targetSection}`;
    const group = routes.get(id) ?? [];
    group.push(call);
    routes.set(id, group);
  }
  const routeModels = [...routes.entries()].map(([id, calls]) => ({
    id,
    sourceSection: calls[0]!.sourceSection,
    targetSection: calls[0]!.targetSection,
    calls: calls.length,
    extracted: calls.filter((call) => call.confidence === "extracted").length,
    inferred: calls.filter((call) => call.confidence === "inferred").length,
    ambiguous: calls.filter((call) => call.confidence === "ambiguous").length
  })).sort((left, right) => right.calls - left.calls || left.id.localeCompare(right.id));
  const incoming = new Map<string, number>();
  const outgoing = new Map<string, number>();
  for (const route of routeModels) {
    outgoing.set(route.sourceSection, (outgoing.get(route.sourceSection) ?? 0) + route.calls);
    incoming.set(route.targetSection, (incoming.get(route.targetSection) ?? 0) + route.calls);
  }
  const sectionModels = sections.map((section) => {
    const scopes = { production: 0, test: 0, generated: 0, vendor: 0, unknown: 0 };
    for (const node of section.nodes) scopes[node.scope] += 1;
    return {
      id: section.id,
      name: section.name,
      nodeCount: section.nodes.filter((node) => includeNode(node.scope)).length,
      totalNodeCount: section.nodes.length,
      internalCallCount: section.edges.filter(includeCall).length,
      incomingCalls: incoming.get(section.id) ?? 0,
      outgoingCalls: outgoing.get(section.id) ?? 0,
      scopes
    };
  });
  const visibleNodes = sectionModels.reduce((sum, section) => sum + section.nodeCount, 0);
  const visibleInternal = sectionModels.reduce((sum, section) => sum + section.internalCallCount, 0);
  const visibleCross = routeModels.reduce((sum, route) => sum + route.calls, 0);
  return {
    title: model.title,
    scope,
    evidence,
    sections: sectionModels,
    routes: routeModels,
    statistics: {
      visibleNodes,
      totalNodes: model.statistics.nodes,
      visibleCalls: visibleInternal + visibleCross,
      totalCalls: model.statistics.edges,
      communities: model.statistics.communities,
      extracted: model.statistics.extracted,
      inferred: model.statistics.inferred,
      ambiguous: model.statistics.ambiguous
    },
    coverage: model.coverage,
    provenance: model.provenance
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
  return view.coverage.status === "complete" ? "Complete"
    : view.coverage.status === "summary" ? "Community summary" : "Bounded result";
}

function humanize(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function sectionName(overview: ArchitectureOverview, id: string): string {
  return overview.sections.find((section) => section.id === id)?.name ?? id;
}

export function dispatchOpenSource(source: SourceLocation): void {
  window.dispatchEvent(new CustomEvent("compass:open-source", { detail: source }));
}
