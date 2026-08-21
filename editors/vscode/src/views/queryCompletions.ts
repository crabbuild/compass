import type {
  CodeQueryResponse,
  QueryCompletion
} from "@compass/viewer";

const MAX_COMPLETIONS = 8;
const MAX_INSERT_TEXT = 512;
const MAX_DETAIL = 240;
const CALLABLE_NODE_KINDS = new Set([
  "function",
  "method",
  "constructor",
  "property"
]);

export function validGraphCompletionTerm(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length < 2 || value.length > 160
    || value.trim() !== value || /^\d+$/.test(value)
    || value.startsWith("-") || value.startsWith("$")
    || !/^[\p{L}\p{N}_$:.#/@-]+$/u.test(value)) {
    return undefined;
  }
  return value;
}

export function validGraphCompletionNodeId(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_INSERT_TEXT
    || value.trim() !== value || value.startsWith("-") || /[\u0000-\u001f\u007f]/u.test(value)) {
    return undefined;
  }
  return value;
}

export function graphCompletionItems(result: CodeQueryResponse): QueryCompletion[] {
  return completionItems(result);
}

export function callGraphCompletionItems(
  result: CodeQueryResponse
): QueryCompletion[] {
  return completionItems(result, (kind) => CALLABLE_NODE_KINDS.has(kind));
}

function completionItems(
  result: CodeQueryResponse,
  include: (kind: string) => boolean = () => true
): QueryCompletion[] {
  const nodes = new Map(result.nodes.map((node) => [node.id, node]));
  const seen = new Set<string>();
  const completions: QueryCompletion[] = [];
  for (const match of result.results) {
    if (completions.length >= MAX_COMPLETIONS) break;
    const node = nodes.get(match.nodeId);
    if (!node || seen.has(node.id) || !include(node.kind)) continue;
    const insertText = node.qualifiedName.trim();
    if (!insertText || insertText.length > MAX_INSERT_TEXT) continue;
    seen.add(node.id);
    const location = node.source
      ? `${node.source.file}:${node.source.startLine}`
      : "source unavailable";
    completions.push({
      nodeId: node.id,
      label: insertText,
      insertText,
      detail: bounded(`${node.kind.replaceAll("_", " ")} · ${location}`, MAX_DETAIL)
    });
  }
  return completions;
}

function bounded(value: string, limit: number): string {
  return value.length <= limit ? value : `${value.slice(0, limit - 1)}…`;
}
