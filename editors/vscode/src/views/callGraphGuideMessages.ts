import type { CallDirection } from "@compass/viewer/contracts/callGraph";
import type { QueryCompletion } from "@compass/viewer";
import { validGraphCompletionTerm } from "./queryCompletions";

export const MAX_CALL_GRAPH_SYMBOL_LENGTH = 512;

export type CallGraphSymbolRequest = {
  symbol: string;
  direction: CallDirection;
};

export type CallGraphCompletionRequest = {
  requestId: string;
  term: string;
};

export function parseCallGraphSymbolRequest(
  message: unknown
): CallGraphSymbolRequest | undefined {
  if (!message || typeof message !== "object") return undefined;
  const candidate = message as Record<string, unknown>;
  if (candidate.type !== "openSymbol" || !isDirection(candidate.direction)) {
    return undefined;
  }
  if (typeof candidate.symbol !== "string") return undefined;
  const symbol = candidate.symbol.trim();
  if (!symbol || symbol.length > MAX_CALL_GRAPH_SYMBOL_LENGTH) return undefined;
  return { symbol, direction: candidate.direction };
}

export function parseCallGraphCompletionRequest(
  message: unknown
): CallGraphCompletionRequest | undefined {
  if (!message || typeof message !== "object") return undefined;
  const candidate = message as Record<string, unknown>;
  if (candidate.type !== "completeSymbol"
    || typeof candidate.requestId !== "string"
    || candidate.requestId.length < 1
    || candidate.requestId.length > 128
    || !/^[A-Za-z0-9_-]+$/.test(candidate.requestId)
    || typeof candidate.term !== "string") return undefined;
  const term = callGraphCompletionTerm(candidate.term);
  if (!term) return undefined;
  return { requestId: candidate.requestId, term };
}

export function callGraphCompletionTerm(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  return validGraphCompletionTerm(value.trim());
}

export function parseCallGraphCompletionItems(
  value: unknown
): QueryCompletion[] | undefined {
  if (!Array.isArray(value) || value.length > 8) return undefined;
  const items: QueryCompletion[] = [];
  for (const candidate of value) {
    if (!candidate || typeof candidate !== "object") return undefined;
    const item = candidate as Record<string, unknown>;
    if (!boundedString(item.nodeId, 512)
      || !boundedString(item.label, 512)
      || !boundedString(item.insertText, 512)
      || !boundedString(item.detail, 240)) return undefined;
    items.push({
      nodeId: item.nodeId,
      label: item.label,
      insertText: item.insertText,
      detail: item.detail
    });
  }
  return items;
}

function boundedString(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= limit;
}

function isDirection(value: unknown): value is CallDirection {
  return value === "callers" || value === "callees" || value === "both";
}
