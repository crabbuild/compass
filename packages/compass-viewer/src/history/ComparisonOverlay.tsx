import { Badge } from "../components/ui/badge";

export type GraphComparison = {
  addedNodes: number;
  removedNodes: number;
  addedEdges: number;
  removedEdges: number;
};

export function ComparisonOverlay({ comparison }: { comparison: GraphComparison }) {
  return (
    <div className="flex flex-wrap gap-2" aria-label="Structural graph comparison">
      <Badge className="bg-emerald-600">+{comparison.addedNodes} nodes</Badge>
      <Badge variant="destructive">−{comparison.removedNodes} nodes</Badge>
      <Badge className="bg-emerald-600">+{comparison.addedEdges} edges</Badge>
      <Badge variant="destructive">−{comparison.removedEdges} edges</Badge>
    </div>
  );
}

export function compareGraphs(
  parent: { nodes: { id: string }[]; edges: { id: string }[] },
  current: { nodes: { id: string }[]; edges: { id: string }[] }
): GraphComparison {
  const parentNodes = new Set(parent.nodes.map((node) => node.id));
  const currentNodes = new Set(current.nodes.map((node) => node.id));
  const parentEdges = new Set(parent.edges.map((edge) => edge.id));
  const currentEdges = new Set(current.edges.map((edge) => edge.id));
  return {
    addedNodes: [...currentNodes].filter((id) => !parentNodes.has(id)).length,
    removedNodes: [...parentNodes].filter((id) => !currentNodes.has(id)).length,
    addedEdges: [...currentEdges].filter((id) => !parentEdges.has(id)).length,
    removedEdges: [...parentEdges].filter((id) => !currentEdges.has(id)).length
  };
}
