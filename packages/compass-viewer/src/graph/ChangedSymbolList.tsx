import { useState } from "react";
import type { GraphNode } from "../contracts/graph";

const INITIAL_LIMIT = 100;
const CHANGE_ORDER: Record<NonNullable<GraphNode["change"]>, number> = {
  changed: 0,
  added: 1,
  removed: 2,
  unchanged: 3
};

export function ChangedSymbolList({
  nodes,
  query,
  selectedId,
  onFocus
}: {
  nodes: GraphNode[];
  query: string;
  selectedId?: string | undefined;
  onFocus(nodeId: string): void;
}) {
  const [showAll, setShowAll] = useState(false);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const changed = nodes
    .filter((node) => node.change && node.change !== "unchanged")
    .filter((node) => !normalizedQuery
      || node.label.toLocaleLowerCase().includes(normalizedQuery)
      || node.kind?.toLocaleLowerCase().includes(normalizedQuery)
      || node.source?.file.toLocaleLowerCase().includes(normalizedQuery))
    .sort((left, right) =>
      CHANGE_ORDER[left.change ?? "unchanged"] - CHANGE_ORDER[right.change ?? "unchanged"]
      || left.label.localeCompare(right.label)
      || left.id.localeCompare(right.id));
  const visible = showAll ? changed : changed.slice(0, INITIAL_LIMIT);

  return (
    <section className="compass-changed-symbols" aria-labelledby="compass-changed-symbols-title">
      <div className="compass-section-heading">
        <h2 id="compass-changed-symbols-title">Changed symbols</h2>
        <span>{changed.length}</span>
      </div>
      {visible.length ? (
        <div className="compass-changed-symbol-list">
          {visible.map((node) => (
            <button
              key={node.id}
              type="button"
              aria-pressed={node.id === selectedId}
              data-change={node.change}
              onClick={() => onFocus(node.id)}
            >
              <span className="compass-changed-symbol-status">{changeLabel(node.change)}</span>
              <strong>{node.label}</strong>
              <small>{node.source?.file ?? node.kind ?? "Graph symbol"}</small>
            </button>
          ))}
        </div>
      ) : (
        <p className="compass-empty">
          {normalizedQuery ? "No affected symbols match this search." : "No symbol changes."}
        </p>
      )}
      {!showAll && changed.length > INITIAL_LIMIT && (
        <button
          className="compass-changed-symbol-expand"
          type="button"
          onClick={() => setShowAll(true)}
        >
          Show all {changed.length.toLocaleString()} affected symbols
        </button>
      )}
    </section>
  );
}

function changeLabel(change: GraphNode["change"]): string {
  if (!change || change === "unchanged") return "Context";
  return `${change[0]?.toLocaleUpperCase()}${change.slice(1)}`;
}
