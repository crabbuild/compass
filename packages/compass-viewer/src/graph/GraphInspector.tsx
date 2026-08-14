import { useMemo, useState, type KeyboardEvent } from "react";
import {
  CompassIcon,
  ExternalLinkIcon,
  PanelRightCloseIcon,
  PanelRightOpenIcon,
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
import { navigableSource } from "./sourceNavigation";

export const COMMUNITY_CONTROL_LIMIT = 200;

export function visibleCommunityControls(
  communities: GraphViewModel["communities"],
  query: string
): GraphViewModel["communities"] {
  const normalized = query.trim().toLocaleLowerCase();
  const matches = normalized
    ? communities.filter((community) =>
      community.label.toLocaleLowerCase().includes(normalized)
      || String(community.id).includes(normalized))
    : communities;
  return matches.slice(0, COMMUNITY_CONTROL_LIMIT);
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
  neighbors,
  connectedEdges,
  query,
  matches,
  hiddenCommunities,
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
  neighbors: GraphNode[];
  connectedEdges: GraphEdge[];
  query: string;
  matches: GraphNode[];
  hiddenCommunities: ReadonlySet<number>;
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
  const source = selected ? navigableSource(selected) : undefined;
  const range = selected ? lineRange(selected) : undefined;
  const sourceRange = selected ? sourceDisplayRange(selected) : undefined;
  const communityCounts = useMemo(() => {
    const counts = new Map<number, number>();
    for (const node of model.nodes) {
      counts.set(node.community, (counts.get(node.community) ?? 0) + 1);
    }
    return counts;
  }, [model.nodes]);
  const allVisible = hiddenCommunities.size === 0;
  const nodeLookup = useMemo(
    () => new Map(model.nodes.map((node) => [node.id, node])),
    [model.nodes]
  );
  const selectedQueryNode = selected
    ? queryResult?.nodes.find((node) => node.id === selected.id)
    : undefined;
  const selectedCodeEvidence = selectedQueryNode?.evidence ?? selected?.codeEvidence ?? [];
  const relationshipCodeEvidence = selected
    ? (queryResult
      ? queryResult.edges
        .filter((edge) => edge.source === selected.id || edge.target === selected.id)
        .flatMap((edge) => edge.evidence)
      : connectedEdges.flatMap((edge) => edge.codeEvidence ?? []))
    : [];

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
    <aside className="compass-graph-inspector" aria-label="Graph inspector">
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
                <span>{node.source?.file ?? node.kind ?? "Graph node"}</span>
              </button>
            ))}
          </div>
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
                  ?? model.communities.find((item) => item.id === selected.community)?.color }}
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
            <dl className="compass-metadata-grid">
              <div>
                <dt>Community</dt>
                <dd>{selected.communityName
                  ?? model.communities.find((item) => item.id === selected.community)?.label
                  ?? selected.community}</dd>
              </div>
              <div>
                <dt>Degree</dt>
                <dd>{selected.degree ?? neighbors.length}</dd>
              </div>
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
            {onQueryNode && (
              <div className="compass-code-query-actions" aria-label="Code graph queries">
                <button type="button" onClick={() => onQueryNode("callers", selected.id)}>
                  Callers
                </button>
                <button type="button" onClick={() => onQueryNode("callees", selected.id)}>
                  Callees
                </button>
                <button type="button" onClick={() => onQueryNode("impact", selected.id)}>
                  Impact
                </button>
              </div>
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
              && selected.detailAvailable !== false
              && onOpenCommunity && (
                <button
                  className="compass-inspector-action"
                  type="button"
                  onClick={() => onOpenCommunity(selected.community)}
                >
                  {comparisonMode ? "Inspect changes" : "Open community"}
                  <span>
                    {selected.memberCount.toLocaleString()}{" "}
                    {comparisonMode ? "current symbols" : "members"}
                  </span>
                </button>
              )}
            {model.stats.aggregated
              && selected.memberCount !== undefined
              && selected.detailAvailable === false && (
                <p className="compass-empty">
                  This community detail was omitted to keep the standalone HTML export bounded.
                  Open the graph in VS Code or export this community as JSON for full inspection.
                </p>
              )}
            {comparisonMode ? (
              <ChangeEvidence
                node={selected}
                edges={connectedEdges}
                nodes={nodeLookup}
                sourceRevisions={sourceRevisions}
                onFocus={onFocus}
                onOpenSource={onOpenSource}
              />
            ) : (
              <>
                <div className="compass-neighbors-heading">
                  <span>Connected nodes</span>
                  <strong>{neighbors.length}</strong>
                </div>
                <div className="compass-neighbors-list">
                  {neighbors.length ? neighbors.map((neighbor) => (
                    <button
                      key={neighbor.id}
                      type="button"
                      className="compass-neighbor-link"
                      title={neighbor.label}
                      onClick={() => onFocus(neighbor.id)}
                    >
                      <span
                        className="compass-neighbor-dot"
                        aria-hidden="true"
                        style={{ background: neighbor.color?.background
                          ?? model.communities.find((item) => item.id === neighbor.community)?.color
                          ?? "var(--border)" }}
                      />
                      <span className="compass-neighbor-label">{neighbor.label}</span>
                    </button>
                  )) : <span className="compass-empty">No connected nodes</span>}
                </div>
              </>
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

      <section
        className="compass-community-panel"
        aria-labelledby="compass-communities-title"
        data-secondary={comparisonMode}
      >
        {comparisonMode ? (
          <details>
            <summary id="compass-communities-title">
              Communities
              <span>{model.communities.length}</span>
            </summary>
            <CommunityControls
              model={model}
              communityCounts={communityCounts}
              hiddenCommunities={hiddenCommunities}
              allVisible={allVisible}
              onSetAllVisible={onSetAllVisible}
              onToggleCommunity={onToggleCommunity}
            />
          </details>
        ) : (
          <>
            <h2 id="compass-communities-title">Communities</h2>
            <CommunityControls
              model={model}
              communityCounts={communityCounts}
              hiddenCommunities={hiddenCommunities}
              allVisible={allVisible}
              onSetAllVisible={onSetAllVisible}
              onToggleCommunity={onToggleCommunity}
            />
          </>
        )}
      </section>
      <footer className="compass-graph-stats">
        {model.stats.nodes.toLocaleString()} nodes · {model.stats.edges.toLocaleString()} edges
        {renderedEdgeCount < model.stats.edges
          ? ` (${renderedEdgeCount.toLocaleString()} shown)`
          : ""} ·{" "}
        {model.stats.communities.toLocaleString()} communities
      </footer>
    </aside>
  );
}

function CommunityControls({
  model,
  communityCounts,
  hiddenCommunities,
  allVisible,
  onSetAllVisible,
  onToggleCommunity
}: {
  model: GraphViewModel;
  communityCounts: ReadonlyMap<number, number>;
  hiddenCommunities: ReadonlySet<number>;
  allVisible: boolean;
  onSetAllVisible(visible: boolean): void;
  onToggleCommunity(communityId: number): void;
}) {
  const [query, setQuery] = useState("");
  const visibleCommunities = useMemo(
    () => visibleCommunityControls(model.communities, query),
    [model.communities, query]
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
            <label
              key={community.id}
              className="compass-community-item"
              data-hidden={!visible}
            >
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
              <small>{communityCounts.get(community.id) ?? 0}</small>
            </label>
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
