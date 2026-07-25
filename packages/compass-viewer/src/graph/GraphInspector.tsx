import { useMemo, useState, type KeyboardEvent } from "react";
import {
  CompassIcon,
  PanelRightCloseIcon,
  PanelRightOpenIcon,
  SearchIcon
} from "lucide-react";
import type { GraphNode, GraphViewModel, SourceLocation } from "../contracts/graph";
import { navigableSource } from "./sourceNavigation";

function lineRange(node: GraphNode): string | undefined {
  const start = node.source?.startLine;
  const end = node.source?.endLine;
  if (start === undefined) return undefined;
  return end !== undefined && end !== start ? `${start}–${end}` : String(start);
}

export function GraphInspector({
  model,
  selected,
  neighbors,
  query,
  matches,
  hiddenCommunities,
  onQueryChange,
  onFocus,
  onOpenSource,
  onOpenCommunity,
  onToggleCommunity,
  onSetAllVisible,
  collapsed,
  onToggleCollapsed
}: {
  model: GraphViewModel;
  selected: GraphNode | undefined;
  neighbors: GraphNode[];
  query: string;
  matches: GraphNode[];
  hiddenCommunities: ReadonlySet<number>;
  onQueryChange(query: string): void;
  onFocus(nodeId: string): void;
  onOpenSource(source: SourceLocation): void;
  onOpenCommunity?: ((communityId: number) => void) | undefined;
  onToggleCommunity(communityId: number): void;
  onSetAllVisible(visible: boolean): void;
  collapsed: boolean;
  onToggleCollapsed(): void;
}) {
  const [activeResult, setActiveResult] = useState(0);
  const source = selected ? navigableSource(selected) : undefined;
  const range = selected ? lineRange(selected) : undefined;
  const communityCounts = useMemo(() => {
    const counts = new Map<number, number>();
    for (const node of model.nodes) {
      counts.set(node.community, (counts.get(node.community) ?? 0) + 1);
    }
    return counts;
  }, [model.nodes]);
  const allVisible = hiddenCommunities.size === 0;

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

      <div className="compass-inspector-search" role="search">
        <label className="sr-only" htmlFor="compass-node-search">Search graph nodes</label>
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
                style={{ background: selected.color?.background
                  ?? model.communities.find((item) => item.id === selected.community)?.color }}
              />
              <span>
                <strong>{selected.label}</strong>
                <small>{selected.kind ?? "Symbol"}</small>
              </span>
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
              <div className="compass-metadata-wide">
                <dt>Source</dt>
                <dd title={selected.source?.file ?? "Not recorded"}>
                  {selected.source?.file ?? "Not recorded"}
                </dd>
              </div>
            </dl>
            {selected.signature && (
              <code className="compass-signature-block">{selected.signature}</code>
            )}
            {source && (
              <button
                className="compass-inspector-action"
                type="button"
                onClick={() => onOpenSource(source)}
              >
                Open source
                <span>{source.file}{range ? `:${range}` : ""}</span>
              </button>
            )}
            {model.stats.aggregated
              && selected.memberCount !== undefined
              && onOpenCommunity && (
                <button
                  className="compass-inspector-action"
                  type="button"
                  onClick={() => onOpenCommunity(selected.community)}
                >
                  Open community
                  <span>{selected.memberCount.toLocaleString()} members</span>
                </button>
              )}
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
                  style={{ borderLeftColor: neighbor.color?.background
                    ?? model.communities.find((item) => item.id === neighbor.community)?.color }}
                  onClick={() => onFocus(neighbor.id)}
                >
                  {neighbor.label}
                </button>
              )) : <span className="compass-empty">No connected nodes</span>}
            </div>
          </div>
        ) : (
          <p className="compass-empty">Select a node to inspect its relationships.</p>
        )}
      </section>

      <section className="compass-community-panel" aria-labelledby="compass-communities-title">
        <h2 id="compass-communities-title">Communities</h2>
        <label className="compass-community-control">
          <input
            type="checkbox"
            checked={allVisible}
            onChange={(event) => onSetAllVisible(event.target.checked)}
          />
          <span>Select all</span>
        </label>
        <div className="compass-community-list">
          {model.communities.map((community) => {
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
      </section>
      <footer className="compass-graph-stats">
        {model.stats.nodes.toLocaleString()} nodes · {model.stats.edges.toLocaleString()} edges ·{" "}
        {model.stats.communities.toLocaleString()} communities
      </footer>
    </aside>
  );
}
