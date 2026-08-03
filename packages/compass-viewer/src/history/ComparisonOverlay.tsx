import { GitCompareIcon, XIcon } from "lucide-react";
import type {
  GraphEdge,
  GraphNode,
  GraphRecordEvidence,
  GraphViewModel
} from "../contracts/graph";
import {
  compareRecord,
  compareStructuralRecord,
  structuralRecordProjection
} from "./recordDiff";

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
  const nodes = new Map<string, GraphNode>();
  const edges: GraphEdge[] = [];
  const addedNodeIds = difference(currentNodes, parentNodes);
  const removedNodeIds = difference(parentNodes, currentNodes);
  const nodeEvidence = new Map(intersection(currentNodes, parentNodes).map((id) => [
    id,
    compareStructuralRecord(parentNodes.get(id), currentNodes.get(id), "node")
  ]));
  const changedNodeIds = [...nodeEvidence.entries()]
    .filter(([, evidence]) => evidence.fields.length > 0)
    .map(([id]) => id);
  const edgeMatches = matchEdges(parent.edges, current.edges);
  const changedEdgeMatches = edgeMatches.matched
    .map(({ before, after }) => ({
      before,
      after,
      evidence: compareMatchedEdges(before, after)
    }))
    .filter(({ evidence }) => evidence.fields.length > 0);

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
  for (const edge of edgeMatches.added) {
    edges.push({
      ...edge,
      change: "added",
      evidence: compareRecord(undefined, edge)
    });
    addEdgeContext(nodes, edge, parentNodes, currentNodes);
  }
  for (const edge of edgeMatches.removed) {
    edges.push({
      ...edge,
      change: "removed",
      evidence: compareRecord(edge, undefined)
    });
    addEdgeContext(nodes, edge, parentNodes, currentNodes);
  }
  for (const { after, evidence } of changedEdgeMatches) {
    edges.push({ ...after, change: "changed", evidence });
    addEdgeContext(nodes, after, parentNodes, currentNodes);
  }
  const orderedNodes = [...nodes.values()].sort((left, right) => left.id.localeCompare(right.id));
  edges.sort((left, right) => left.id.localeCompare(right.id));
  const orderedEdges = uniqueEdgeIds(edges);
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
    addedEdges: edgeMatches.added.length,
    removedEdges: edgeMatches.removed.length,
    changedEdges: changedEdgeMatches.length,
    graph: {
      ...current,
      title: `Graph delta · ${current.title}`,
      stats: {
        ...current.stats,
        nodes: orderedNodes.length,
        edges: orderedEdges.length,
        communities: communities.length,
        aggregated: parent.stats.aggregated || current.stats.aggregated
      },
      nodes: orderedNodes,
      edges: orderedEdges,
      communities,
      hyperedges: []
    }
  };
}

/** Build an exact changed-subgraph when revision exports are community aggregates. */
export function comparisonFromSemanticDiff(
  report: unknown,
  reference: GraphViewModel
): GraphComparison | undefined {
  if (!isObject(report) || !isObject(report.graph_delta)) return undefined;
  const delta = report.graph_delta;
  const addedNodes = graphNodeDeltas(delta.added_nodes);
  const removedNodes = graphNodeDeltas(delta.removed_nodes);
  const changedNodes = graphNodeDeltas(delta.changed_nodes);
  const addedEdges = graphEdgeDeltas(delta.added_edges);
  const removedEdges = graphEdgeDeltas(delta.removed_edges);
  const changedEdges = graphEdgeDeltas(delta.changed_edges);
  if ([addedNodes, removedNodes, changedNodes, addedEdges, removedEdges, changedEdges]
    .some((records) => records === undefined)) return undefined;

  const displayNames = isObject(report.entity_display_names)
    ? report.entity_display_names
    : {};
  const nodes = new Map<string, GraphNode>();
  const addDeltaNode = (
    record: SemanticNodeDelta,
    change: "added" | "removed" | "changed"
  ): void => {
    nodes.set(record.id, {
      id: record.id,
      label: record.label || stringField(displayNames, record.id) || record.id,
      community: 0,
      ...(record.kind ? { kind: record.kind } : {}),
      ...(record.sourceFile ? { source: { file: record.sourceFile } } : {}),
      change,
      evidence: { fields: record.changedFields.map((field) => ({ field })) }
    });
  };
  addedNodes!.forEach((node) => addDeltaNode(node, "added"));
  removedNodes!.forEach((node) => addDeltaNode(node, "removed"));
  changedNodes!.forEach((node) => addDeltaNode(node, "changed"));

  const edges: GraphEdge[] = [];
  const addDeltaEdges = (
    records: SemanticEdgeDelta[],
    change: "added" | "removed" | "changed"
  ): void => records.forEach((record, index) => {
    for (const id of [record.source, record.target]) {
      if (!nodes.has(id)) nodes.set(id, {
        id,
        label: stringField(displayNames, id) || id,
        community: 0,
        change: "unchanged"
      });
    }
    edges.push({
      id: record.key || `semantic-${change}-${index}-${record.source}-${record.target}`,
      source: record.source,
      target: record.target,
      relation: record.relation,
      ...(record.sourceFile ? { relationshipSite: { file: record.sourceFile } } : {}),
      change,
      evidence: { fields: record.changedFields.map((field) => ({ field })) }
    });
  });
  addDeltaEdges(addedEdges!, "added");
  addDeltaEdges(removedEdges!, "removed");
  addDeltaEdges(changedEdges!, "changed");
  const orderedNodes = [...nodes.values()].sort((left, right) => left.id.localeCompare(right.id));
  const orderedEdges = uniqueEdgeIds(edges.sort((left, right) => left.id.localeCompare(right.id)));
  return {
    addedNodes: addedNodes!.length,
    removedNodes: removedNodes!.length,
    changedNodes: changedNodes!.length,
    addedEdges: addedEdges!.length,
    removedEdges: removedEdges!.length,
    changedEdges: changedEdges!.length,
    graph: {
      ...reference,
      title: `Graph delta · ${reference.title}`,
      stats: {
        ...reference.stats,
        nodes: orderedNodes.length,
        edges: orderedEdges.length,
        communities: orderedNodes.length > 0 ? 1 : 0,
        aggregated: false
      },
      nodes: orderedNodes,
      edges: orderedEdges,
      communities: orderedNodes.length > 0
        ? [{ id: 0, label: "Changed subgraph", color: "#6688aa", hidden: false }]
        : [],
      hyperedges: []
    }
  };
}

type SemanticNodeDelta = {
  id: string;
  label: string;
  kind: string;
  sourceFile: string;
  changedFields: string[];
};

type SemanticEdgeDelta = {
  source: string;
  target: string;
  relation: string;
  key: string;
  sourceFile: string;
  changedFields: string[];
};

function graphNodeDeltas(value: unknown): SemanticNodeDelta[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const records = value.map((item) => {
    if (!isObject(item) || typeof item.id !== "string") return undefined;
    return {
      id: item.id,
      label: stringField(item, "label"),
      kind: stringField(item, "kind"),
      sourceFile: stringField(item, "source_file"),
      changedFields: stringArray(item.changed_fields)
    };
  });
  return records.every((record) => record !== undefined)
    ? records as SemanticNodeDelta[]
    : undefined;
}

function graphEdgeDeltas(value: unknown): SemanticEdgeDelta[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const records = value.map((item) => {
    if (!isObject(item)
      || typeof item.source !== "string"
      || typeof item.target !== "string"
      || typeof item.relation !== "string") return undefined;
    return {
      source: item.source,
      target: item.target,
      relation: item.relation,
      key: stringField(item, "key"),
      sourceFile: stringField(item, "source_file"),
      changedFields: stringArray(item.changed_fields)
    };
  });
  return records.every((record) => record !== undefined)
    ? records as SemanticEdgeDelta[]
    : undefined;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringField(value: Record<string, unknown>, key: string): string {
  return typeof value[key] === "string" ? value[key] : "";
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
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

function addEdgeContext(
  nodes: Map<string, GraphNode>,
  edge: GraphEdge,
  parent: ReadonlyMap<string, GraphNode>,
  current: ReadonlyMap<string, GraphNode>
): void {
  addContextNode(nodes, edge.source, parent, current);
  addContextNode(nodes, edge.target, parent, current);
}

function compareMatchedEdges(
  before: GraphEdge,
  after: GraphEdge
): GraphRecordEvidence {
  return compareStructuralRecord(
    { ...before, id: after.id },
    after,
    "edge"
  );
}

function matchEdges(
  parent: GraphEdge[],
  current: GraphEdge[]
): {
  matched: Array<{ before: GraphEdge; after: GraphEdge }>;
  added: GraphEdge[];
  removed: GraphEdge[];
} {
  const parentUsed = new Set<number>();
  const currentUsed = new Set<number>();
  const matched: Array<{ before: GraphEdge; after: GraphEdge }> = [];
  const pair = (parentIndex: number, currentIndex: number): void => {
    parentUsed.add(parentIndex);
    currentUsed.add(currentIndex);
    matched.push({
      before: parent[parentIndex]!,
      after: current[currentIndex]!
    });
  };

  pairEdgesByKey(
    parent,
    current,
    parentUsed,
    currentUsed,
    pair,
    (edge) => isGeneratedEdgeId(edge.id) ? undefined : edge.id
  );
  pairEdgesByKey(
    parent,
    current,
    parentUsed,
    currentUsed,
    pair,
    (edge) => isGeneratedEdgeId(edge.id) ? edgeSemanticKey(edge) : undefined
  );

  const endpointGroups = new Map<string, { parent: number[]; current: number[] }>();
  parent.forEach((edge, index) => {
    if (parentUsed.has(index) || !isGeneratedEdgeId(edge.id)) return;
    const group = endpointGroups.get(edgeEndpointKey(edge)) ?? { parent: [], current: [] };
    group.parent.push(index);
    endpointGroups.set(edgeEndpointKey(edge), group);
  });
  current.forEach((edge, index) => {
    if (currentUsed.has(index) || !isGeneratedEdgeId(edge.id)) return;
    const group = endpointGroups.get(edgeEndpointKey(edge)) ?? { parent: [], current: [] };
    group.current.push(index);
    endpointGroups.set(edgeEndpointKey(edge), group);
  });
  for (const group of endpointGroups.values()) {
    group.parent.sort((left, right) => compareEdgeOrder(parent[left]!, parent[right]!));
    group.current.sort((left, right) => compareEdgeOrder(current[left]!, current[right]!));
    const pairCount = Math.min(group.parent.length, group.current.length);
    for (let index = 0; index < pairCount; index += 1) {
      pair(group.parent[index]!, group.current[index]!);
    }
  }

  return {
    matched,
    added: current.filter((_, index) => !currentUsed.has(index)),
    removed: parent.filter((_, index) => !parentUsed.has(index))
  };
}

function pairEdgesByKey(
  parent: GraphEdge[],
  current: GraphEdge[],
  parentUsed: ReadonlySet<number>,
  currentUsed: ReadonlySet<number>,
  pair: (parentIndex: number, currentIndex: number) => void,
  keyFor: (edge: GraphEdge) => string | undefined
): void {
  const currentByKey = new Map<string, number[]>();
  current.forEach((edge, index) => {
    if (currentUsed.has(index)) return;
    const key = keyFor(edge);
    if (key === undefined) return;
    const indexes = currentByKey.get(key) ?? [];
    indexes.push(index);
    currentByKey.set(key, indexes);
  });
  parent.forEach((edge, parentIndex) => {
    if (parentUsed.has(parentIndex)) return;
    const key = keyFor(edge);
    if (key === undefined) return;
    const candidates = currentByKey.get(key);
    const currentIndex = candidates?.shift();
    if (currentIndex !== undefined) pair(parentIndex, currentIndex);
  });
}

function edgeSemanticKey(edge: GraphEdge): string {
  return JSON.stringify(structuralRecordProjection(edge, "edge"));
}

function edgeEndpointKey(edge: GraphEdge): string {
  return JSON.stringify([edge.source, edge.target]);
}

function compareEdgeOrder(left: GraphEdge, right: GraphEdge): number {
  return edgeSemanticKey(left).localeCompare(edgeSemanticKey(right))
    || left.id.localeCompare(right.id);
}

function isGeneratedEdgeId(id: string): boolean {
  return /^edge-\d+-/.test(id);
}

function uniqueEdgeIds(edges: GraphEdge[]): GraphEdge[] {
  const counts = new Map<string, number>();
  return edges.map((edge) => {
    const count = counts.get(edge.id) ?? 0;
    counts.set(edge.id, count + 1);
    return count === 0 ? edge : { ...edge, id: `${edge.id}::${edge.change}-${count}` };
  });
}
