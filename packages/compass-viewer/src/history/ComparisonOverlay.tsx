import { GitCompareIcon, XIcon } from "lucide-react";
import type {
  GraphEdge,
  GraphNode,
  GraphRecordEvidence,
  GraphViewModel
} from "../contracts/graph";
import { compareRecord } from "./recordDiff";

export type GraphComparison = {
  addedNodes: number;
  removedNodes: number;
  changedNodes: number;
  addedEdges: number;
  removedEdges: number;
  changedEdges: number;
  graph: GraphViewModel;
};

export function ComparisonOverlay({
  comparison,
  commit,
  parent,
  onExit
}: {
  comparison: GraphComparison;
  commit: string;
  parent: string;
  onExit(): void;
}) {
  const empty = comparison.graph.nodes.length === 0 && comparison.graph.edges.length === 0;
  return (
    <section className="history-comparison" aria-label="Revision graph comparison">
      <div className="history-comparison-heading">
        <GitCompareIcon aria-hidden="true" />
        <div>
          <span className="history-eyebrow">Comparison mode</span>
          <h2>
            Comparing <code>{commit.slice(0, 9)}</code> to <code>{parent.slice(0, 9)}</code>
          </h2>
        </div>
        <button type="button" onClick={onExit} aria-label="Exit comparison">
          <XIcon aria-hidden="true" /> Exit comparison
        </button>
      </div>
      <div className="history-comparison-legend" aria-label="Visible graph delta">
        <Delta label="nodes" added={comparison.addedNodes}
          removed={comparison.removedNodes} changed={comparison.changedNodes} />
        <Delta label="edges" added={comparison.addedEdges}
          removed={comparison.removedEdges} changed={comparison.changedEdges} />
      </div>
      {empty && (
        <div className="history-comparison-empty">
          <strong>No structural graph changes</strong>
          <span>
            The stored topology is identical. Source or configuration changes can still appear below.
          </span>
        </div>
      )}
    </section>
  );
}

function Delta({
  label,
  added,
  removed,
  changed
}: {
  label: string;
  added: number;
  removed: number;
  changed: number;
}) {
  return (
    <span className="history-delta-card">
      <strong>{label}</strong>
      <span>
        <i data-change="added"><small>Added</small> {added}</i>
        <i data-change="removed"><small>Removed</small> {removed}</i>
        <i data-change="changed"><small>Changed</small> {changed}</i>
      </span>
    </span>
  );
}

export function compareGraphs(
  parent: GraphViewModel,
  current: GraphViewModel
): GraphComparison {
  const parentNodes = new Map(parent.nodes.map((node) => [node.id, node]));
  const currentNodes = new Map(current.nodes.map((node) => [node.id, node]));
  const parentEdges = new Map(parent.edges.map((edge) => [edge.id, edge]));
  const currentEdges = new Map(current.edges.map((edge) => [edge.id, edge]));
  const nodes = new Map<string, GraphNode>();
  const edges: GraphEdge[] = [];
  const addedNodeIds = difference(currentNodes, parentNodes);
  const removedNodeIds = difference(parentNodes, currentNodes);
  const nodeEvidence = new Map(intersection(currentNodes, parentNodes).map((id) => [
    id,
    compareRecord(parentNodes.get(id), currentNodes.get(id))
  ]));
  const changedNodeIds = [...nodeEvidence.entries()]
    .filter(([, evidence]) => evidence.fields.length > 0)
    .map(([id]) => id);
  const addedEdgeIds = difference(currentEdges, parentEdges);
  const removedEdgeIds = difference(parentEdges, currentEdges);
  const edgeEvidence = new Map(intersection(currentEdges, parentEdges).map((id) => [
    id,
    compareRecord(parentEdges.get(id), currentEdges.get(id))
  ]));
  const changedEdgeIds = [...edgeEvidence.entries()]
    .filter(([, evidence]) => evidence.fields.length > 0)
    .map(([id]) => id);

  for (const id of addedNodeIds) {
    addNode(
      nodes,
      currentNodes.get(id),
      "added",
      compareRecord(undefined, currentNodes.get(id))
    );
  }
  for (const id of removedNodeIds) {
    addNode(
      nodes,
      parentNodes.get(id),
      "removed",
      compareRecord(parentNodes.get(id), undefined)
    );
  }
  for (const id of changedNodeIds) {
    addNode(nodes, currentNodes.get(id), "changed", nodeEvidence.get(id));
  }
  for (const [ids, source, change] of [
    [addedEdgeIds, currentEdges, "added"],
    [removedEdgeIds, parentEdges, "removed"],
    [changedEdgeIds, currentEdges, "changed"]
  ] as const) {
    for (const id of ids) {
      const edge = source.get(id);
      if (!edge) continue;
      const evidence = change === "added"
        ? compareRecord(undefined, edge)
        : change === "removed"
          ? compareRecord(edge, undefined)
          : edgeEvidence.get(id);
      edges.push({ ...edge, change, ...(evidence ? { evidence } : {}) });
      addContextNode(nodes, edge.source, parentNodes, currentNodes);
      addContextNode(nodes, edge.target, parentNodes, currentNodes);
    }
  }
  const orderedNodes = [...nodes.values()].sort((left, right) => left.id.localeCompare(right.id));
  edges.sort((left, right) => left.id.localeCompare(right.id));
  const communityIds = new Set(orderedNodes.map((node) => node.community));
  const communities = [
    ...current.communities,
    ...parent.communities.filter((community) =>
      !current.communities.some((candidate) => candidate.id === community.id))
  ].filter((community) => communityIds.has(community.id));
  return {
    addedNodes: addedNodeIds.length,
    removedNodes: removedNodeIds.length,
    changedNodes: changedNodeIds.length,
    addedEdges: addedEdgeIds.length,
    removedEdges: removedEdgeIds.length,
    changedEdges: changedEdgeIds.length,
    graph: {
      ...current,
      title: `Graph delta · ${current.title}`,
      stats: {
        ...current.stats,
        nodes: orderedNodes.length,
        edges: edges.length,
        communities: communities.length,
        aggregated: parent.stats.aggregated || current.stats.aggregated
      },
      nodes: orderedNodes,
      edges,
      communities,
      hyperedges: []
    }
  };
}

function difference<T>(left: ReadonlyMap<string, T>, right: ReadonlyMap<string, T>): string[] {
  return [...left.keys()].filter((id) => !right.has(id));
}

function intersection<T>(left: ReadonlyMap<string, T>, right: ReadonlyMap<string, T>): string[] {
  return [...left.keys()].filter((id) => right.has(id));
}

function addNode(
  nodes: Map<string, GraphNode>,
  node: GraphNode | undefined,
  change: "added" | "removed" | "changed",
  evidence: GraphRecordEvidence | undefined
): void {
  if (node) nodes.set(node.id, {
    ...node,
    change,
    ...(evidence ? { evidence } : {})
  });
}

function addContextNode(
  nodes: Map<string, GraphNode>,
  id: string,
  parent: ReadonlyMap<string, GraphNode>,
  current: ReadonlyMap<string, GraphNode>
): void {
  if (nodes.has(id)) return;
  const node = current.get(id) ?? parent.get(id);
  if (node) nodes.set(id, {
    ...node,
    change: "unchanged"
  });
}
