import {
  CallGraphResponseSchema,
  type CallDirection,
  type CallEdge,
  type CallGraphResponse,
  type CallNode
} from "@compass/viewer/contracts/callGraph";
import type {
  CodeEvidenceRecord,
  CodeQueryEdge,
  CodeQueryResponse
} from "@compass/viewer/contracts/codeQuery";

export function codeQueryCallGraph(
  rootSymbol: string,
  direction: CallDirection,
  responses: readonly CodeQueryResponse[]
): CallGraphResponse {
  const queryNodes = new Map(
    responses.flatMap((response) => response.nodes).map((node) => [node.id, node])
  );
  const queryEdges = new Map(
    responses
      .flatMap((response) => response.edges)
      .filter((edge) => edge.kind === "calls")
      .map((edge) => [edge.id, edge])
  );
  const nodes = [...queryNodes.values()].map((node): CallNode => ({
    id: node.id,
    symbol: node.id,
    name: node.name,
    file: node.source?.file ?? null,
    startLine: node.source?.startLine ?? null,
    endLine: node.source?.endLine ?? null,
    startByte: node.source?.startByte ?? null,
    endByte: node.source?.endByte ?? null,
    graphNodeId: node.id,
    unresolved: false,
    evidenceLayer: evidenceLayer(node.evidence)
  })).sort((left, right) => left.id.localeCompare(right.id));
  const edges = [...queryEdges.values()].map((edge) => callEdge(edge, queryNodes))
    .sort((left, right) => left.id.localeCompare(right.id));
  const limitations = [
    "typed_query_fallback",
    ...responses.flatMap((response) => response.diagnostics.map((item) => item.code))
  ].filter((value, index, values) => values.indexOf(value) === index).sort();
  const response = {
    schema: "compass.call_graph/1" as const,
    rootSymbol,
    direction,
    depth: 1,
    nodes,
    edges,
    truncated: responses.some((item) => item.truncated),
    continuations: continuations(rootSymbol, responses),
    coverage: {
      resolved: edges.filter((edge) => edge.resolution === "resolved").length,
      inferred: edges.filter((edge) => edge.resolution === "inferred").length,
      ambiguous: edges.filter((edge) => edge.resolution === "ambiguous").length,
      unresolved: edges.filter((edge) => edge.resolution === "unresolved").length,
      evidenceLayer: evidenceLayer(responses.flatMap((item) => [
        ...item.nodes.flatMap((node) => node.evidence),
        ...item.edges.flatMap((edge) => edge.evidence)
      ])),
      partial: true,
      limitations,
      warning: "Showing typed call relationships because source-position call graph lookup is unavailable in the selected Compass CLI."
    }
  };
  return CallGraphResponseSchema.parse(response);
}

function callEdge(
  edge: CodeQueryEdge,
  nodes: Map<string, CodeQueryResponse["nodes"][number]>
): CallEdge {
  const resolution = callResolution(edge.evidence);
  const site = edge.relationshipSite;
  return {
    id: edge.id,
    source: edge.source,
    target: edge.target,
    callee: nodes.get(edge.target)?.name ?? edge.target,
    resolution,
    confidence: resolution === "resolved" ? "EXTRACTED" : resolution.toUpperCase(),
    callSites: site ? [{
      sourceFile: site.file,
      line: site.startLine,
      startByte: site.startByte,
      endByte: site.endByte,
      evidence: [...new Set(edge.evidence.map((item) => item.rule ?? item.extractor))].sort()
    }] : [],
    evidenceLayer: evidenceLayer(edge.evidence)
  };
}

function callResolution(evidence: readonly CodeEvidenceRecord[]): CallEdge["resolution"] {
  if (evidence.some((item) => item.resolution === "unresolved")) return "unresolved";
  if (evidence.some((item) => item.resolution === "ambiguous"
    || item.confidence === "ambiguous")) return "ambiguous";
  if (evidence.some((item) => item.confidence === "inferred")) return "inferred";
  return "resolved";
}

function evidenceLayer(
  evidence: readonly CodeEvidenceRecord[]
): "structural_graph" | "program_ir" | "combined" {
  const structural = evidence.some((item) => item.layer === "structural_graph");
  const program = evidence.some((item) => item.layer === "program_ir");
  return structural && program ? "combined" : program ? "program_ir" : "structural_graph";
}

function continuations(
  rootSymbol: string,
  responses: readonly CodeQueryResponse[]
): CallGraphResponse["continuations"] {
  const values = responses.flatMap((response) => response.nodes
    .filter((node) => node.id !== rootSymbol)
    .map((node) => ({
      symbol: node.id,
      direction: response.operation === "callers" ? "callers" as const : "callees" as const,
      nextDepth: 2
    })));
  return [...new Map(values.map((item) => [
    `${item.symbol}:${item.direction}`,
    item
  ])).values()].sort((left, right) => left.symbol.localeCompare(right.symbol)
    || left.direction.localeCompare(right.direction));
}
