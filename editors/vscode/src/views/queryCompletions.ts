import type {
  CodeQueryResponse,
  QueryCompletion
} from "@compass/viewer";

const MAX_COMPLETIONS = 8;
const MAX_INSERT_TEXT = 512;
const MAX_DETAIL = 240;

export function validGraphCompletionTerm(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length < 2 || value.length > 160
    || value.trim() !== value || /^\d+$/.test(value)
    || value.startsWith("-") || value.startsWith("$")
    || !/^[\p{L}\p{N}_$:.#/@-]+$/u.test(value)) {
    return undefined;
  }
  return value;
}

export function graphCompletionItems(result: CodeQueryResponse): QueryCompletion[] {
  const nodes = new Map(result.nodes.map((node) => [node.id, node]));
  const seen = new Set<string>();
  const completions: QueryCompletion[] = [];
  for (const match of result.results) {
    if (completions.length >= MAX_COMPLETIONS) break;
    const node = nodes.get(match.nodeId);
    if (!node || seen.has(node.id)) continue;
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
