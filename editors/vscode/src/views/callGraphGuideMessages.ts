import type { CallDirection } from "@compass/viewer/contracts/callGraph";

export const MAX_CALL_GRAPH_SYMBOL_LENGTH = 512;

export type CallGraphSymbolRequest = {
  symbol: string;
  direction: CallDirection;
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

function isDirection(value: unknown): value is CallDirection {
  return value === "callers" || value === "callees" || value === "both";
}
