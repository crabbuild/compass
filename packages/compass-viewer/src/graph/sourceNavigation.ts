import type { GraphNode, SourceLocation } from "../contracts/graph";

export function navigableSource(node: GraphNode): SourceLocation | undefined {
  const source = node.source;
  if (!source?.file.trim()) return undefined;
  const located = source.startLine !== undefined
    || source.endLine !== undefined
    || source.startByte !== undefined
    || source.endByte !== undefined;
  return located ? source : undefined;
}
