import { useMemo } from "react";
import type { GraphViewModel } from "../contracts/graph";
import {
  EDGE_SEMANTIC_CATEGORIES,
  NODE_SEMANTIC_CATEGORIES,
  edgeSemanticCategory,
  nodeSemanticCategory,
  type EdgeSemanticCategory,
  type NodeSemanticCategory
} from "./semanticAppearance";

const NODE_LABELS: Record<NodeSemanticCategory, string> = {
  callable: "Callable",
  type: "Type",
  module: "Module / file",
  boundary: "Boundary / data",
  other: "Other"
};

const EDGE_LABELS: Record<EdgeSemanticCategory, string> = {
  execution: "Execution",
  dependency: "Dependency",
  structure: "Structure",
  flow: "Data / event flow",
  other: "Other"
};

export function GraphSemanticLegend({ model }: { model: GraphViewModel }) {
  const nodeCounts = useMemo(() => {
    const counts = new Map<NodeSemanticCategory, number>();
    for (const node of model.nodes) {
      const category = nodeSemanticCategory(node.kind);
      counts.set(category, (counts.get(category) ?? 0) + 1);
    }
    return counts;
  }, [model.nodes]);
  const edgeCounts = useMemo(() => {
    const counts = new Map<EdgeSemanticCategory, number>();
    for (const edge of model.edges) {
      const category = edgeSemanticCategory(edge.relation);
      counts.set(category, (counts.get(category) ?? 0) + 1);
    }
    return counts;
  }, [model.edges]);

  return (
    <aside className="compass-semantic-legend compass-glass-panel" aria-label="Graph visual legend">
      <section aria-label="Node categories">
        <strong>Nodes</strong>
        {NODE_SEMANTIC_CATEGORIES
          .filter((category) => (nodeCounts.get(category) ?? 0) > 0)
          .map((category) => (
            <span key={category} className="compass-semantic-legend-item">
              <i data-node-category={category} aria-hidden="true" />
              {NODE_LABELS[category]}
              <small>{nodeCounts.get(category)}</small>
            </span>
          ))}
      </section>
      {model.edges.length > 0 ? (
        <section aria-label="Relationship categories">
          <strong>Edges</strong>
          {EDGE_SEMANTIC_CATEGORIES
            .filter((category) => (edgeCounts.get(category) ?? 0) > 0)
            .map((category) => (
              <span key={category} className="compass-semantic-legend-item">
                <i data-edge-category={category} aria-hidden="true" />
                {EDGE_LABELS[category]}
                <small>{edgeCounts.get(category)}</small>
              </span>
            ))}
        </section>
      ) : null}
    </aside>
  );
}
