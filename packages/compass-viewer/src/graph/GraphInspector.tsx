import { useEffect, useId, useMemo, useState, type KeyboardEvent } from "react";
import {
  ArrowDownToLineIcon,
  ArrowUpFromLineIcon,
  BoxIcon,
  ChevronRightIcon,
  CompassIcon,
  ExternalLinkIcon,
  PanelRightCloseIcon,
  PanelRightOpenIcon,
  RadarIcon,
  SearchIcon
} from "lucide-react";
import type {
  GraphEdge,
  GraphNode,
  GraphViewModel,
  SourceLocation
} from "../contracts/graph";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import { ChangeEvidence, type GraphSourceRevisions } from "./ChangeEvidence";
import { ChangedSymbolList } from "./ChangedSymbolList";
import { CodeEvidence } from "./CodeEvidence";
import {
  documentContextForNode,
  DocumentOcrPanel
} from "./DocumentOcrPanel";
import { navigableSource } from "./sourceNavigation";
import type { CommunityFacts } from "./communityFacts";

export const COMMUNITY_CONTROL_LIMIT = 200;

export type DirectionalNodeGroup = {
  node: GraphNode;
  edges: GraphEdge[];
};

export type DirectionalRelationships = {
  incoming: DirectionalNodeGroup[];
  outgoing: DirectionalNodeGroup[];
};

export function groupDirectionalRelationships(
  selectedId: string,
  connectedEdges: GraphEdge[],
  nodes: ReadonlyMap<string, GraphNode>
): DirectionalRelationships {
  const incoming = new Map<string, DirectionalNodeGroup>();
  const outgoing = new Map<string, DirectionalNodeGroup>();

  const add = (
    groups: Map<string, DirectionalNodeGroup>,
    nodeId: string,
    edge: GraphEdge
  ) => {
    const node = nodes.get(nodeId);
    if (!node) return;
    const existing = groups.get(nodeId);
    if (existing) {
      existing.edges.push(edge);
    } else {
      groups.set(nodeId, { node, edges: [edge] });
    }
  };

  for (const edge of connectedEdges) {
    if (edge.target === selectedId) add(incoming, edge.source, edge);
    if (edge.source === selectedId) add(outgoing, edge.target, edge);
  }

  const sorted = (groups: Map<string, DirectionalNodeGroup>) => [...groups.values()]
    .map((group) => ({
      ...group,
      edges: group.edges.sort((left, right) => left.relation.localeCompare(right.relation)
        || left.id.localeCompare(right.id))
    }))
    .sort((left, right) => left.node.label.localeCompare(right.node.label)
      || left.node.id.localeCompare(right.node.id));

  return {
    incoming: sorted(incoming),
    outgoing: sorted(outgoing)
  };
}

function relationshipSummary(edges: GraphEdge[]): string {
  const counts = new Map<string, number>();
  for (const edge of edges) {
    const relation = edge.relation || "related";
    counts.set(relation, (counts.get(relation) ?? 0) + (edge.weight ?? 1));
  }
  return [...counts.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([relation, count]) => count > 1 ? `${relation} ×${count}` : relation)
    .join(" · ");
}

/**
 * The published community a bubble can open. A level projection numbers its
 * nodes by group index, so only the id the hierarchy pairs with the group names
 * a community a detail can be fetched for.
 */
function communityToOpen(
  facts: CommunityFacts | undefined,
  selected: GraphNode | undefined
): number | undefined {
  if (!selected) return undefined;
  return facts?.groupId !== undefined ? facts.communityId : selected.community;
}

/** True when the hierarchy carries more than the bubble card already says. */
function communityEvidenceApplies(facts: CommunityFacts): boolean {
  return facts.groupId !== undefined
    || facts.level !== undefined
    || facts.couplings.length > 0
    || facts.boundaryKinds.length > 0;
}

function communityLevelLabel(facts: CommunityFacts): string {
  if (facts.level === undefined) return "Published hierarchy";
  const rule = facts.merge === "locationAffinity"
    ? "grouped by shared location"
    : facts.resolution === undefined
      ? "grouped by relationship evidence"
      : `relationship evidence at ${facts.resolution}`;
  return `Level ${facts.level} · ${rule}`;
}

function formatShare(share: number): string {
  return `${(share * 100).toFixed(share >= 0.1 ? 0 : 1)}%`;
}

function DirectionalRelationshipGroup({
  direction,
  groups,
  communityColors,
  onFocus
}: {
  direction: "incoming" | "outgoing";
  groups: DirectionalNodeGroup[];
  communityColors: ReadonlyMap<number, string>;
  onFocus(nodeId: string): void;
}) {
  const incoming = direction === "incoming";
  const edgeCount = groups.reduce((count, group) => count + group.edges.length, 0);
  const title = incoming ? "Incoming" : "Outgoing";
  const description = incoming
    ? "Nodes that point to this node"
    : "Nodes this node points to";
  const DirectionIcon = incoming ? ArrowDownToLineIcon : ArrowUpFromLineIcon;

  return (
    <section className="compass-direction-group" data-direction={direction}>
      <div className="compass-direction-heading">
        <span className="compass-direction-icon" aria-hidden="true">
          <DirectionIcon />
        </span>
        <span className="compass-direction-copy">
          <strong>{title}</strong>
          <small>{description}</small>
        </span>
        <span className="compass-direction-count">
          <strong>{groups.length}</strong>
          <small>{edgeCount} {edgeCount === 1 ? "edge" : "edges"}</small>
        </span>
      </div>
      <div className="compass-direction-list">
        {groups.length ? groups.map((group) => {
          const summary = relationshipSummary(group.edges);
          return (
            <button
              key={group.node.id}
              type="button"
              className="compass-direction-link"
              title={`Focus ${group.node.label}`}
              aria-label={`Focus ${group.node.label}; ${title.toLocaleLowerCase()}; ${summary}`}
              onClick={() => onFocus(group.node.id)}
            >
              <span
                className="compass-neighbor-dot"
                aria-hidden="true"
                style={{ background: group.node.color?.background
                  ?? communityColors.get(group.node.community)
                  ?? "var(--border)" }}
              />
              <span className="compass-direction-node">
                <strong>{group.node.label}</strong>
                <small>{summary}</small>
              </span>
              <ChevronRightIcon aria-hidden="true" />
            </button>
          );
        }) : (
          <span className="compass-direction-empty">
            {incoming ? "No incoming relationships" : "No outgoing relationships"}
          </span>
        )}
      </div>
    </section>
  );
}

type RelationshipDirection = "incoming" | "outgoing";

function RelationshipTabs({
  selectedId,
  relationships,
  communityColors,
  onFocus
}: {
  selectedId: string;
  relationships: DirectionalRelationships;
  communityColors: ReadonlyMap<number, string>;
  onFocus(nodeId: string): void;
}) {
  const tabId = useId();
  const [choice, setChoice] = useState<{
    selectedId: string;
    direction: RelationshipDirection;
  }>();
  const direction = choice?.selectedId === selectedId
    ? choice.direction
    : relationships.outgoing.length > 0 ? "outgoing" : "incoming";
  const select = (next: RelationshipDirection, focus: boolean) => {
    setChoice({ selectedId, direction: next });
    if (focus) document.getElementById(`${tabId}-${next}-tab`)?.focus();
  };
  const onTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      select(direction === "incoming" ? "outgoing" : "incoming", true);
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      select(event.key === "Home" ? "incoming" : "outgoing", true);
    }
  };
  return (
    <div className="compass-relationship-directions">
      <div className="compass-relationship-tabs" role="tablist" aria-label="Relationship direction" onKeyDown={onTabKeyDown}>
        {(["incoming", "outgoing"] as const).map((option) => {
          const groups = relationships[option];
          const edgeCount = groups.reduce((count, group) => count + group.edges.length, 0);
          const active = direction === option;
          const title = option === "incoming" ? "Incoming" : "Outgoing";
          const DirectionIcon = option === "incoming" ? ArrowDownToLineIcon : ArrowUpFromLineIcon;
          return (
            <button
              key={option}
              id={`${tabId}-${option}-tab`}
              type="button"
              role="tab"
              aria-label={`${title}, ${edgeCount} ${edgeCount === 1 ? "edge" : "edges"}`}
              aria-selected={active}
              aria-controls={`${tabId}-${option}-panel`}
              tabIndex={active ? 0 : -1}
              data-direction={option}
              onClick={() => select(option, false)}
            >
              <DirectionIcon aria-hidden="true" />
              <span>{title}</span>
              <small>{edgeCount}</small>
            </button>
          );
        })}
      </div>
      {(["incoming", "outgoing"] as const).map((option) => (
        <div
          key={option}
          id={`${tabId}-${option}-panel`}
          role="tabpanel"
          aria-labelledby={`${tabId}-${option}-tab`}
          tabIndex={direction === option ? 0 : -1}
          hidden={direction !== option}
        >
          {direction === option && (
            <DirectionalRelationshipGroup
              direction={option}
              groups={relationships[option]}
              communityColors={communityColors}
              onFocus={onFocus}
            />
          )}
        </div>
      ))}
    </div>
  );
}

export function visibleCommunityControls(
  communities: GraphViewModel["communities"],
  query: string,
  order?: readonly number[] | undefined
): GraphViewModel["communities"] {
  const normalized = query.trim().toLocaleLowerCase();
  const matches = normalized
    ? communities.filter((community) =>
      community.label.toLocaleLowerCase().includes(normalized)
      || String(community.id).includes(normalized))
    : communities;
  if (!order) return matches.slice(0, COMMUNITY_CONTROL_LIMIT);
  const rank = new Map(order.map((communityId, index) => [communityId, index]));
  return [...matches]
    .sort((left, right) =>
      (rank.get(left.id) ?? Number.MAX_SAFE_INTEGER)
      - (rank.get(right.id) ?? Number.MAX_SAFE_INTEGER))
    .slice(0, COMMUNITY_CONTROL_LIMIT);
}

function lineRange(node: GraphNode): string | undefined {
  const start = node.source?.startLine;
  const end = node.source?.endLine;
  if (start === undefined) return undefined;
  return end !== undefined && end !== start ? `${start}–${end}` : String(start);
}

function sourceDisplayRange(node: GraphNode): {
  text: string;
  action: string;
} | undefined {
  const startLine = node.source?.startLine;
  const endLine = node.source?.endLine;
  if (startLine !== undefined) {
    return endLine !== undefined && endLine !== startLine
      ? { text: `Lines ${startLine}–${endLine}`, action: `at lines ${startLine}–${endLine}` }
      : { text: `Line ${startLine}`, action: `at line ${startLine}` };
  }

  const startByte = node.source?.startByte;
  const endByte = node.source?.endByte;
  if (startByte === undefined) return undefined;
  return endByte !== undefined && endByte !== startByte
    ? { text: `Bytes ${startByte}–${endByte}`, action: `at bytes ${startByte}–${endByte}` }
    : { text: `Byte ${startByte}`, action: `at byte ${startByte}` };
}

function sourceActionLabel(node: GraphNode, source: SourceLocation): string {
  const range = sourceDisplayRange(node);
  return `Open source ${source.file}${range ? ` ${range.action}` : ""}`;
}

function changeLabel(change: GraphNode["change"]): string | undefined {
  return change === "unchanged"
    ? "Context"
    : change ? `${change[0]?.toLocaleUpperCase()}${change.slice(1)}` : undefined;
}

function changeColor(change: GraphNode["change"]): string | undefined {
  if (!change) return undefined;
  return {
    added: "var(--vscode-gitDecoration-addedResourceForeground, #2ea043)",
    removed: "var(--vscode-gitDecoration-deletedResourceForeground, #f85149)",
    changed: "var(--vscode-gitDecoration-modifiedResourceForeground, #d29922)",
    unchanged: "var(--vscode-descriptionForeground, #6e7781)"
  }[change];
}

export function GraphInspector({
  model,
  selected,
  communityFacts,
  neighbors,
  connectedEdges,
  query,
  matches,
  searchSpansCommunities,
  searchCoverage,
  searchOnly,
  communityOrder,
  hiddenCommunities,
  communityDrilldown,
  subgraphAvailable,
  comparisonMode,
  sourceRevisions,
  queryResult,
  renderedEdgeCount,
  showHeader,
  onQueryChange,
  onFocus,
  onOpenSource,
  onOpenCommunity,
  onQueryNode,
  onToggleCommunity,
  onSetAllVisible,
  collapsed,
  onToggleCollapsed
}: {
  model: GraphViewModel;
  selected: GraphNode | undefined;
  /**
   * Set while the selected node is a community bubble: the panel then answers
   * what the community holds and how it couples, not what a bubble's degree is.
   */
  communityFacts?: CommunityFacts | undefined;
  neighbors: GraphNode[];
  connectedEdges: GraphEdge[];
  query: string;
  matches: Array<GraphNode & { previewAvailable?: boolean }>;
  /**
   * Set when matches come from every community rather than only the graph on
   * screen, so a result names the community that holds it.
   */
  searchSpansCommunities?: boolean | undefined;
  searchCoverage?: { indexed: number; total: number } | undefined;
  searchOnly?: boolean | undefined;
  /** Community ids ordered by reader importance, when the viewer derived them. */
  communityOrder?: readonly number[] | undefined;
  hiddenCommunities: ReadonlySet<number>;
  /** Set while one community is open; its details take the side column. */
  communityDrilldown?: boolean | undefined;
  /** A published child projection can be opened from the selected group. */
  subgraphAvailable?: boolean | undefined;
  comparisonMode: boolean;
  sourceRevisions?: GraphSourceRevisions | undefined;
  queryResult?: CodeQueryResponse | undefined;
  renderedEdgeCount: number;
  showHeader: boolean;
  onQueryChange(query: string): void;
  onFocus(nodeId: string): void;
  onOpenSource(source: SourceLocation, revision?: string): void;
  onOpenCommunity?: ((communityId: number) => void) | undefined;
  onQueryNode?: ((
    operation: "callers" | "callees" | "impact",
    symbol: string
  ) => void) | undefined;
  onToggleCommunity(communityId: number): void;
  onSetAllVisible(visible: boolean): void;
  collapsed: boolean;
  onToggleCollapsed(): void;
}) {
  const [activeResult, setActiveResult] = useState(0);
  // The selected node's evidence owns the side column in every graph view.
  const hideCommunities = Boolean(communityDrilldown) || selected !== undefined;
  const [communitiesOpen, setCommunitiesOpen] = useState(!communityDrilldown);
  useEffect(() => {
    setCommunitiesOpen(!communityDrilldown);
  }, [communityDrilldown]);
  const communitiesCollapsed = comparisonMode && !communitiesOpen;
  const source = selected ? navigableSource(selected) : undefined;
  const range = selected ? lineRange(selected) : undefined;
  const sourceRange = selected ? sourceDisplayRange(selected) : undefined;
  const canOpenSubgraph = subgraphAvailable === true
    && (communityFacts?.childGroups ?? 0) > 0;
  const openCommunityId = canOpenSubgraph
    ? selected?.community
    : communityToOpen(communityFacts, selected);
  const canOpenCommunity = !canOpenSubgraph
    && model.stats.aggregated
    && selected?.memberCount !== undefined
    && selected.detailAvailable !== false;
  const canOpenSelected = onOpenCommunity !== undefined
    && openCommunityId !== undefined
    && (canOpenSubgraph || canOpenCommunity);
  const communityCounts = useMemo(() => {
    const counts = new Map<number, number>();
    for (const node of model.nodes) {
      // An aggregated overview holds one bubble per community, so the panel
      // reports the community's symbols rather than counting that bubble.
      counts.set(
        node.community,
        (counts.get(node.community) ?? 0) + (node.memberCount ?? 1)
      );
    }
    return counts;
  }, [model.nodes]);
  const aggregatedSymbols = useMemo(() => model.stats.aggregated
    ? model.nodes.reduce((sum, node) => sum + (node.memberCount ?? 0), 0)
    : undefined, [model.nodes, model.stats.aggregated]);
  const allVisible = hiddenCommunities.size === 0;
  const nodeLookup = useMemo(
    () => new Map(model.nodes.map((node) => [node.id, node])),
    [model.nodes]
  );
  const searchResultContext = (node: GraphNode): string => {
    const base = node.source?.file ?? node.kind ?? "Graph node";
    if (!searchSpansCommunities || nodeLookup.has(node.id)) return base;
    const community = node.communityName
      ?? model.communities.find((item) => item.id === node.community)?.label
      ?? `Community ${node.community}`;
    return `${community} · ${base}${node.previewAvailable === false ? " · not in preview" : ""}`;
  };
  const communityColors = useMemo(
    () => new Map(model.communities.map((community) => [community.id, community.color])),
    [model.communities]
  );
  const relationshipGroups = useMemo(
    () => selected
      ? groupDirectionalRelationships(selected.id, connectedEdges, nodeLookup)
      : { incoming: [], outgoing: [] },
    [connectedEdges, nodeLookup, selected?.id]
  );
  const selectedQueryNode = selected
    ? queryResult?.nodes.find((node) => node.id === selected.id)
    : undefined;
  const documentContext = selected ? documentContextForNode(model, selected) : undefined;
  const selectedCodeEvidence = selectedQueryNode?.evidence ?? selected?.codeEvidence ?? [];
  const relationshipCodeEvidence = selected
    ? (queryResult
      ? queryResult.edges
        .filter((edge) => edge.source === selected.id || edge.target === selected.id)
        .flatMap((edge) => edge.evidence)
      : connectedEdges.flatMap((edge) => edge.codeEvidence ?? []))
    : [];
  const communityControls = (
    <CommunityControls
      model={model}
      communityCounts={communityCounts}
      hiddenCommunities={hiddenCommunities}
      allVisible={allVisible}
      onSetAllVisible={onSetAllVisible}
      onToggleCommunity={onToggleCommunity}
      communityOrder={communityOrder}
      onOpenCommunity={onOpenCommunity}
    />
  );

  const choose = (node: GraphNode) => {
    onFocus(node.id);
    onQueryChange("");
    setActiveResult(0);
  };
  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (!matches.length && event.key !== "Escape") return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveResult((activeResult + 1) % matches.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveResult((activeResult - 1 + matches.length) % matches.length);
    } else if (event.key === "Enter" && matches[activeResult]) {
      event.preventDefault();
      choose(matches[activeResult]);
    } else if (event.key === "Escape") {
      onQueryChange("");
      setActiveResult(0);
    }
  };

  if (collapsed) {
    return (
      <aside
        className="compass-graph-inspector compass-graph-inspector-collapsed"
        aria-label="Graph inspector"
      >
        <button
          className="compass-inspector-disclosure compass-inspector-expand"
          type="button"
          aria-label="Expand graph inspector"
          title="Expand graph inspector"
          onClick={onToggleCollapsed}
        >
          <PanelRightOpenIcon aria-hidden="true" />
        </button>
        <span className="compass-inspector-rail-label" aria-hidden="true">Inspector</span>
      </aside>
    );
  }

  return (
    <aside
      className="compass-graph-inspector"
      aria-label="Graph inspector"
      data-focused={hideCommunities}
    >
      {showHeader && (
        <header className="compass-inspector-header">
          <span className="compass-product-mark" aria-hidden="true"><CompassIcon /></span>
          <span className="compass-inspector-title">
            <strong>Compass</strong>
            <small>{model.title}</small>
          </span>
          <button
            className="compass-inspector-disclosure"
            type="button"
            aria-label="Collapse graph inspector"
            title="Collapse graph inspector"
            onClick={onToggleCollapsed}
          >
            <PanelRightCloseIcon aria-hidden="true" />
          </button>
        </header>
      )}
      <div className="compass-inspector-search" role="search">
        <label className="sr-only" htmlFor="compass-node-search">Search graph nodes</label>
        <div className="compass-inspector-search-row">
          <div className="compass-search-field">
            <SearchIcon aria-hidden="true" />
            <input
              id="compass-node-search"
              type="search"
              role="combobox"
              value={query}
              placeholder="Search nodes and files"
              autoComplete="off"
              aria-controls="compass-search-results"
              aria-autocomplete="list"
              aria-expanded={matches.length > 0}
              aria-activedescendant={matches[activeResult]
                ? `compass-search-result-${activeResult}`
                : undefined}
              onChange={(event) => {
                onQueryChange(event.target.value);
                setActiveResult(0);
              }}
              onKeyDown={onSearchKeyDown}
            />
          </div>
          {!showHeader && (
            <button
              className="compass-inspector-disclosure compass-inspector-search-disclosure"
              type="button"
              aria-label="Collapse graph inspector"
              title="Collapse graph inspector"
              onClick={onToggleCollapsed}
            >
              <PanelRightCloseIcon aria-hidden="true" />
            </button>
          )}
        </div>
        {matches.length > 0 && (
          <div
            id="compass-search-results"
            className="compass-search-results"
            role="listbox"
            aria-label="Matching nodes"
          >
            {matches.map((node, index) => (
              <button
                id={`compass-search-result-${index}`}
                key={node.id}
                type="button"
                role="option"
                aria-selected={index === activeResult}
                className="compass-search-item"
                onMouseEnter={() => setActiveResult(index)}
                onClick={() => choose(node)}
              >
                <strong>{node.label}</strong>
                <span>{searchResultContext(node)}</span>
              </button>
            ))}
          </div>
        )}
        {searchCoverage && (
          <small className="compass-search-coverage" role="status">
            {searchCoverage.indexed === searchCoverage.total
              ? `Search all ${searchCoverage.total.toLocaleString()} graph nodes and files`
              : `Search indexes ${searchCoverage.indexed.toLocaleString()} of ${searchCoverage.total.toLocaleString()} graph nodes`}
          </small>
        )}
      </div>

      <section className="compass-info-panel" aria-labelledby="compass-info-title">
        <div className="compass-section-heading">
          <h2 id="compass-info-title">Inspector</h2>
          <span>{selected ? "Pinned" : "Node details"}</span>
        </div>
        {selected ? (
          <div className="compass-info-content">
            <div className="compass-node-identity">
              <span
                className="compass-node-swatch"
                aria-hidden="true"
                style={{ background: changeColor(selected.change)
                  ?? selected.color?.background
                  ?? communityColors.get(selected.community) }}
              />
              <span>
                <strong>{selected.label}</strong>
                <small>{selected.kind ?? "Symbol"}</small>
              </span>
              {changeLabel(selected.change) && (
                <span className="compass-change-badge" data-change={selected.change}>
                  {changeLabel(selected.change)}
                </span>
              )}
            </div>
            {searchOnly && (
              <p className="compass-search-only-notice" role="status">
                Found in the full graph. This node is outside the bounded preview, so its
                relationships are unavailable here. Open the source when a link is available,
                or export its community for complete details.
              </p>
            )}
            <dl className="compass-metadata-grid">
              <div>
                <dt>Community</dt>
                <dd>{communityFacts?.label
                  ?? selected.communityName
                  ?? model.communities.find((item) => item.id === selected.community)?.label
                  ?? selected.community}</dd>
              </div>
              {communityFacts ? (
                <>
                  <div>
                    <dt>Symbols</dt>
                    <dd>{communityFacts.symbols.toLocaleString()}</dd>
                  </div>
                  {communityFacts.childGroups !== undefined && (
                    <div>
                      <dt>Sub-groups</dt>
                      <dd>{communityFacts.childGroups.toLocaleString()}</dd>
                    </div>
                  )}
                  <div>
                    <dt>Couplings</dt>
                    <dd>{communityFacts.couplingCount.toLocaleString()}</dd>
                  </div>
                </>
              ) : (
                <>
                  <div>
                    <dt>Degree</dt>
                    <dd>{selected.degree ?? neighbors.length}</dd>
                  </div>
                  {!searchOnly && (
                    <>
                      <div>
                        <dt>Incoming</dt>
                        <dd>{relationshipGroups.incoming.length}</dd>
                      </div>
                      <div>
                        <dt>Outgoing</dt>
                        <dd>{relationshipGroups.outgoing.length}</dd>
                      </div>
                    </>
                  )}
                </>
              )}
              {selected.language && <div><dt>Language</dt><dd>{selected.language}</dd></div>}
              {range && <div><dt>Lines</dt><dd>{range}</dd></div>}
              <div
                className="compass-metadata-wide compass-source-metadata"
                data-interactive={source !== undefined}
              >
                {source ? (
                  <>
                    <dt className="sr-only">Source</dt>
                    <dd>
                      <button
                        className="compass-source-card"
                        type="button"
                        aria-label={sourceActionLabel(selected, source)}
                        title={sourceActionLabel(selected, source)}
                        onClick={() => onOpenSource(
                          source,
                          selected.change === "removed"
                            ? sourceRevisions?.before
                            : sourceRevisions?.after
                        )}
                      >
                        <span className="compass-source-copy">
                          <span className="compass-source-eyebrow" aria-hidden="true">
                            Source
                          </span>
                          <span className="compass-source-path">{source.file}</span>
                          {sourceRange && (
                            <span className="compass-source-range">
                              {sourceRange.text}
                            </span>
                          )}
                        </span>
                        <ExternalLinkIcon aria-hidden="true" />
                      </button>
                    </dd>
                  </>
                ) : (
                  <>
                    <dt>Source</dt>
                    <dd title={selected.source?.file ?? "Not recorded"}>
                      {selected.source?.file ?? "Not recorded"}
                    </dd>
                  </>
                )}
              </div>
            </dl>
            {selected.signature && (
              <code className="compass-signature-block">{selected.signature}</code>
            )}
            {canOpenSelected && (
              <button
                className="compass-inspector-action"
                type="button"
                onClick={() => {
                  if (openCommunityId !== undefined) onOpenCommunity?.(openCommunityId);
                }}
              >
                <span className="compass-inspector-action-icon" aria-hidden="true">
                  <BoxIcon />
                </span>
                <span className="compass-inspector-action-copy">
                  <strong>{canOpenSubgraph
                    ? "Open subgraph"
                    : comparisonMode ? "Inspect changes" : "Open community"}</strong>
                  <small>{canOpenSubgraph
                    ? `${communityFacts?.childGroups ?? 0} sub-groups`
                    : `${selected.memberCount?.toLocaleString() ?? 0} ${comparisonMode ? "current symbols" : "members"}`}</small>
                </span>
                <ChevronRightIcon aria-hidden="true" />
              </button>
            )}
            {communityFacts && communityEvidenceApplies(communityFacts) && (
              <section
                className="compass-community-evidence"
                aria-labelledby="compass-community-evidence-title"
              >
                <div className="compass-inspector-subheading">
                  <h3 id="compass-community-evidence-title">Community evidence</h3>
                  <span>{communityLevelLabel(communityFacts)}</span>
                </div>
                <dl className="compass-metadata-grid">
                  {communityFacts.communityId !== undefined && (
                    <div>
                      <dt>Community id</dt>
                      <dd>{communityFacts.communityId}</dd>
                    </div>
                  )}
                  {communityFacts.cohesion !== undefined && (
                    <div>
                      <dt>Cohesion</dt>
                      <dd>{formatShare(communityFacts.cohesion)}</dd>
                    </div>
                  )}
                  {communityFacts.conductance !== undefined && (
                    <div>
                      <dt>Conductance</dt>
                      <dd>{communityFacts.conductance.toFixed(2)}</dd>
                    </div>
                  )}
                  {communityFacts.boundaryKinds.length > 0 && (
                    <div className="compass-metadata-wide">
                      <dt>Boundary kinds</dt>
                      <dd>
                        {communityFacts.boundaryKinds
                          .map(([kind, count]) => `${kind} ×${count}`)
                          .join(" · ")}
                      </dd>
                    </div>
                  )}
                </dl>
                {communityFacts.couplings.length > 0 && (
                  <>
                    <div className="compass-inspector-subheading">
                      <h3>Couplings</h3>
                      <span>
                        {communityFacts.couplingCount.toLocaleString()}{" "}
                        {communityFacts.couplingCount === 1 ? "edge" : "edges"}
                      </span>
                    </div>
                    <ul className="compass-community-couplings">
                      {communityFacts.couplings.map((coupling) => (
                        <li key={coupling.label}>
                          <span>{coupling.label}</span>
                          <small>{coupling.edges.toLocaleString()}</small>
                        </li>
                      ))}
                    </ul>
                    {!communityFacts.couplingsComplete && (
                      <p className="compass-empty">
                        The strongest couplings are listed; the level projection bounds the rest.
                      </p>
                    )}
                  </>
                )}
                {communityFacts.groupId && (
                  <code className="compass-signature-block">{communityFacts.groupId}</code>
                )}
              </section>
            )}
            {documentContext && (
              <DocumentOcrPanel
                context={documentContext}
                selectedId={selected.id}
                onFocus={onFocus}
              />
            )}
            {onQueryNode && (
              <section className="compass-node-actions" aria-labelledby="compass-node-actions-title">
                <div className="compass-inspector-subheading">
                  <h3 id="compass-node-actions-title">Actions</h3>
                  <span>3 available</span>
                </div>
                <div className="compass-code-query-actions" aria-label="Code graph queries">
                  <button
                    type="button"
                    title="Find code that calls this node"
                    onClick={() => onQueryNode("callers", selected.id)}
                  >
                    <ArrowDownToLineIcon aria-hidden="true" />
                    <span>Callers</span>
                  </button>
                  <button
                    type="button"
                    title="Find code called by this node"
                    onClick={() => onQueryNode("callees", selected.id)}
                  >
                    <ArrowUpFromLineIcon aria-hidden="true" />
                    <span>Callees</span>
                  </button>
                  <button
                    type="button"
                    title="Trace code affected by this node"
                    onClick={() => onQueryNode("impact", selected.id)}
                  >
                    <RadarIcon aria-hidden="true" />
                    <span>Impact</span>
                  </button>
                </div>
              </section>
            )}
            <CodeEvidence
              evidence={selectedCodeEvidence}
              diagnostics={queryResult?.diagnostics}
              truncated={queryResult?.truncated}
              title="Node evidence"
              onOpenSource={onOpenSource}
            />
            <CodeEvidence
              evidence={relationshipCodeEvidence}
              title="Relationship evidence"
              onOpenSource={onOpenSource}
            />
            {model.stats.aggregated
              && selected.memberCount !== undefined
              && selected.detailAvailable === false
              && !canOpenSubgraph
              && openCommunityId !== undefined && (
                <p className="compass-empty">
                  This community detail was omitted to keep the standalone HTML export bounded.
                  Open the graph in VS Code or export this community as JSON for full inspection.
                </p>
              )}
            {searchOnly ? null : comparisonMode ? (
              <ChangeEvidence
                node={selected}
                edges={connectedEdges}
                nodes={nodeLookup}
                sourceRevisions={sourceRevisions}
                onFocus={onFocus}
                onOpenSource={onOpenSource}
              />
            ) : (
              <section className="compass-relationships" aria-labelledby="compass-relationships-title">
                <div className="compass-inspector-subheading">
                  <h3 id="compass-relationships-title">Relationships</h3>
                  <span>{connectedEdges.length} {connectedEdges.length === 1 ? "edge" : "edges"}</span>
                </div>
                <RelationshipTabs
                  selectedId={selected.id}
                  relationships={relationshipGroups}
                  communityColors={communityColors}
                  onFocus={onFocus}
                />
              </section>
            )}
          </div>
        ) : (
          <p className="compass-empty">Select a node to inspect its relationships.</p>
        )}
      </section>

      {comparisonMode && !model.stats.aggregated && (
        <ChangedSymbolList
          nodes={model.nodes}
          query={query}
          selectedId={selected?.id}
          onFocus={onFocus}
        />
      )}

      {model.effectiveGraph && (
        <section className="compass-info-panel" aria-labelledby="compass-agent-overlay-title">
          <div className="compass-section-heading">
            <h2 id="compass-agent-overlay-title">Agent overlay</h2>
            <span>Exact revision</span>
          </div>
          <dl className="compass-metadata-grid">
            <div>
              <dt>Pinned profile</dt>
              <dd>{model.effectiveGraph.compositionProfile}</dd>
            </div>
            <div>
              <dt>Retractions</dt>
              <dd>{model.effectiveGraph.retractions.total.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Omissions</dt>
              <dd>{model.effectiveGraph.omissions.total.toLocaleString()}</dd>
            </div>
            <div className="compass-metadata-wide">
              <dt>Overlay revision</dt>
              <dd title={model.effectiveGraph.overlayRevision}>
                <code>{model.effectiveGraph.overlayRevision.slice(0, 12)}</code>
              </dd>
            </div>
          </dl>
          {model.effectiveGraph.retractions.examples.length > 0 && (
            <details>
              <summary>
                Retraction history
                <span>{model.effectiveGraph.retractions.examples.length}</span>
              </summary>
              <ul>
                {model.effectiveGraph.retractions.examples.map((retraction) => (
                  <li key={`${retraction.kind}:${retraction.id}`}>
                    <strong>{retraction.kind}</strong>{" "}
                    <code>{retraction.id}</code>{" — "}
                    {retraction.reasonCode}: {retraction.explanation}
                  </li>
                ))}
              </ul>
              {model.effectiveGraph.retractions.omittedExamples > 0 && (
                <p className="compass-empty">
                  {model.effectiveGraph.retractions.omittedExamples.toLocaleString()} additional
                  Retractions omitted by the response bound.
                </p>
              )}
            </details>
          )}
          <p className="compass-empty">
            Changing augment or curated profile requires a new exact Effective Graph read;
            this viewer never relabels one identity as another.
          </p>
        </section>
      )}

      {!hideCommunities && (
        <section
          className="compass-community-panel"
          aria-labelledby="compass-communities-title"
          data-secondary={comparisonMode}
          data-collapsed={String(communitiesCollapsed)}
        >
          {comparisonMode ? (
            <details
              open={communitiesOpen}
              onToggle={(event) => setCommunitiesOpen(event.currentTarget.open)}
            >
              <summary id="compass-communities-title">
                Communities
                <span>{model.communities.length}</span>
              </summary>
              {communityControls}
            </details>
          ) : (
            <>
              <h2 id="compass-communities-title">Communities</h2>
              {communityControls}
            </>
          )}
        </section>
      )}
      {!hideCommunities && <footer className="compass-graph-stats">
        {model.stats.aggregated ? (
          <>
            {model.stats.communities.toLocaleString()} communities ·{" "}
            {(aggregatedSymbols ?? 0).toLocaleString()} symbols ·{" "}
            {model.stats.edges.toLocaleString()} cross-community relationships
            {renderedEdgeCount < model.stats.edges
              ? ` (${renderedEdgeCount.toLocaleString()} shown)`
              : ""}
          </>
        ) : (
          <>
            {model.stats.nodes.toLocaleString()} nodes ·{" "}
            {model.stats.edges.toLocaleString()} edges
            {renderedEdgeCount < model.stats.edges
              ? ` (${renderedEdgeCount.toLocaleString()} shown)`
              : ""} ·{" "}
            {model.stats.communities.toLocaleString()} communities
          </>
        )}
      </footer>}
    </aside>
  );
}

function CommunityControls({
  model,
  communityCounts,
  hiddenCommunities,
  allVisible,
  onSetAllVisible,
  onToggleCommunity,
  communityOrder,
  onOpenCommunity
}: {
  model: GraphViewModel;
  communityCounts: ReadonlyMap<number, number>;
  hiddenCommunities: ReadonlySet<number>;
  allVisible: boolean;
  onSetAllVisible(visible: boolean): void;
  onToggleCommunity(communityId: number): void;
  communityOrder?: readonly number[] | undefined;
  onOpenCommunity?: ((communityId: number) => void) | undefined;
}) {
  const [query, setQuery] = useState("");
  const visibleCommunities = useMemo(
    () => visibleCommunityControls(model.communities, query, communityOrder),
    [communityOrder, model.communities, query]
  );
  const matchingCount = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return normalized
      ? model.communities.filter((community) =>
        community.label.toLocaleLowerCase().includes(normalized)
        || String(community.id).includes(normalized)).length
      : model.communities.length;
  }, [model.communities, query]);
  return (
    <div className="compass-community-controls">
      <label className="compass-community-control">
        <input
          type="checkbox"
          checked={allVisible}
          onChange={(event) => onSetAllVisible(event.target.checked)}
        />
        <span>Select all</span>
      </label>
      {model.communities.length > COMMUNITY_CONTROL_LIMIT && (
        <label className="compass-community-search">
          <span className="sr-only">Filter communities</span>
          <SearchIcon aria-hidden="true" />
          <input
            type="search"
            value={query}
            placeholder="Filter communities"
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
      )}
      <div className="compass-community-list">
        {visibleCommunities.map((community) => {
          const visible = !hiddenCommunities.has(community.id);
          return (
            <div
              key={community.id}
              className="compass-community-item"
              data-hidden={!visible}
            >
              <label>
                <input
                  type="checkbox"
                  checked={visible}
                  onChange={() => onToggleCommunity(community.id)}
                />
                <span
                  className="compass-community-dot"
                  aria-hidden="true"
                  style={{ background: community.color }}
                />
                <span className="compass-community-label">{community.label}</span>
              </label>
              <small>{communityCounts.get(community.id) ?? 0}</small>
              {onOpenCommunity ? (
                <button
                  className="compass-community-open"
                  type="button"
                  aria-label={`Open group ${community.label}`}
                  title={`Open community ${community.label}`}
                  onClick={() => onOpenCommunity(community.id)}
                >
                  <ChevronRightIcon aria-hidden="true" />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
      {matchingCount > visibleCommunities.length && (
        <p className="compass-community-limit" role="status">
          Showing {visibleCommunities.length.toLocaleString()} of{" "}
          {matchingCount.toLocaleString()} communities. Filter to find another community.
        </p>
      )}
    </div>
  );
}
