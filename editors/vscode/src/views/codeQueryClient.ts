import {
  CodeQueryResponseSchema,
  type CodeQueryResponse
} from "@compass/viewer/contracts/codeQuery";
import type { RepositorySession } from "../workspace/repositorySession";

export type CodeQueryRequest =
  | { operation: "search"; query: string }
  | { operation: "callers" | "callees"; symbol: string }
  | { operation: "impact"; symbol: string; includeHeuristic?: boolean }
  | { operation: "explore"; symbols: string[] }
  | {
    operation: "node";
    source: string;
    target: string;
    includeHeuristic?: boolean;
  };

export function codeQueryArguments(
  request: CodeQueryRequest,
  graphPath: string,
  repositoryRoot: string
): string[] {
  const args: string[] = [request.operation];
  if (request.operation === "search") {
    args.push(operand(request.query));
  } else if ("symbol" in request && request.operation !== "impact") {
    args.push(operand(request.symbol));
  } else if (request.operation === "impact") {
    args.push(operand(request.symbol));
    if (request.includeHeuristic) args.push("--include-heuristic");
  } else if (request.operation === "explore") {
    args.push(...request.symbols.map(operand), "--root", repositoryRoot);
  } else if (request.operation === "node") {
    args.push(operand(request.source), operand(request.target));
    if (request.includeHeuristic) args.push("--include-heuristic");
  }
  args.push(
    "--graph",
    graphPath,
    "--max-depth",
    "8",
    "--max-nodes",
    "500",
    "--max-edges",
    "1000",
    "--max-paths",
    "100",
    "--max-source-bytes",
    "1048576",
    "--max-response-bytes",
    "8388608",
    "--format",
    "json"
  );
  return args;
}

function operand(value: string): string {
  const normalized = value.trim();
  if (!normalized || normalized.startsWith("--")) {
    throw new Error("Compass query values must be non-empty and cannot begin with '--'");
  }
  return normalized;
}

export async function runCodeQuery(
  session: RepositorySession,
  request: CodeQueryRequest,
  signal?: AbortSignal
): Promise<CodeQueryResponse> {
  return session.processes.runJson(
    session.root,
    codeQueryArguments(request, session.graphPath, session.root),
    CodeQueryResponseSchema,
    signal
  );
}

export function codeQueryRequiresRebuild(message: string): boolean {
  return message.includes("requires compass.graph/1")
    || message.includes("unsupported graph schema")
    || message.includes("rebuild");
}
