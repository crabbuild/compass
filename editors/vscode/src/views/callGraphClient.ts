import {
  CallGraphResponseSchema,
  type CallDirection,
  type CallGraphResponse
} from "@compass/viewer/contracts/callGraph";
import { z } from "zod";
import { buildCqlArgs } from "../commands/queryArguments";
import type { RepositorySession } from "../workspace/repositorySession";
import { runCodeQuery } from "./codeQueryClient";
import {
  callGraphCommandArguments,
  callGraphExpansionArguments,
  callGraphRootArguments,
  type CallGraphRoot
} from "./callGraphArguments";
import { codeQueryCallGraph } from "./codeQueryCallGraph";

const CqlStringSchema = z.strictObject({
  type: z.literal("string"),
  value: z.string()
});
const CqlIntegerSchema = z.strictObject({
  type: z.literal("integer"),
  value: z.number().int().nonnegative()
});
const CursorQuerySchema = z.object({
  schema: z.literal("compass.cql.result/1"),
  rows: z.array(z.strictObject({
    "n.id": CqlStringSchema,
    "n.kind": CqlStringSchema,
    "n.source": z.strictObject({
      type: z.literal("map"),
      value: z.strictObject({
        file: CqlStringSchema,
        startByte: CqlIntegerSchema,
        endByte: CqlIntegerSchema,
        startLine: CqlIntegerSchema,
        startColumn: CqlIntegerSchema,
        endLine: CqlIntegerSchema,
        endColumn: CqlIntegerSchema
      })
    })
  }))
});

export async function runCallGraph(
  session: RepositorySession,
  request: readonly string[],
  signal?: AbortSignal
): Promise<CallGraphResponse> {
  return session.processes.runJson(
    session.root,
    callGraphCommandArguments(request, session.graphPath),
    CallGraphResponseSchema,
    signal
  );
}

export async function runCallGraphAtCursor(
  session: RepositorySession,
  root: CallGraphRoot,
  direction: CallDirection,
  depth: number,
  signal?: AbortSignal
): Promise<CallGraphResponse> {
  if (requiresTypedFallback(session)) {
    const symbol = await resolveCursorSymbol(session, root, signal);
    if (!symbol) throw missingCursorError(root);
    return runTypedCallGraph(session, symbol, direction, signal);
  }
  try {
    return await runCallGraph(
      session,
      callGraphRootArguments(root, direction, depth),
      signal
    );
  } catch (error) {
    if (!message(error).includes("no callable graph node matches")) throw error;
    let symbol: string | undefined;
    try {
      symbol = await resolveCursorSymbol(session, root, signal);
    } catch (fallbackError) {
      throw new Error(
        `${message(error)} Compatibility lookup failed: ${message(fallbackError)}`
      );
    }
    if (!symbol) throw error;
    return runCallGraphForSymbol(
      session,
      symbol,
      direction,
      depth,
      signal
    );
  }
}

export async function runCallGraphForSymbol(
  session: RepositorySession,
  symbol: string,
  direction: CallDirection,
  depth: number,
  signal?: AbortSignal
): Promise<CallGraphResponse> {
  if (requiresTypedFallback(session)) {
    return runTypedCallGraph(session, symbol, direction, signal);
  }
  try {
    return await runCallGraph(
      session,
      callGraphExpansionArguments(symbol, direction, depth),
      signal
    );
  } catch (error) {
    if (!message(error).includes("no callable graph node matches")) throw error;
    return runTypedCallGraph(session, symbol, direction, signal);
  }
}

async function runTypedCallGraph(
  session: RepositorySession,
  symbol: string,
  direction: CallDirection,
  signal?: AbortSignal
): Promise<CallGraphResponse> {
  const operations = direction === "both"
    ? ["callers", "callees"] as const
    : [direction] as const;
  const responses = await Promise.all(operations.map((operation) =>
    runCodeQuery(session, { operation, symbol }, signal)
  ));
  return codeQueryCallGraph(symbol, direction, responses);
}

async function resolveCursorSymbol(
  session: RepositorySession,
  root: CallGraphRoot,
  signal?: AbortSignal
): Promise<string | undefined> {
  if (!Number.isSafeInteger(root.byte) || root.byte < 0) {
    throw new Error("The cursor byte must be a non-negative safe integer.");
  }
  if (!Number.isSafeInteger(root.line) || root.line < 1) {
    throw new Error("The cursor line must be a positive safe integer.");
  }
  const query = [
    "MATCH (n)",
    "WHERE n.source.file = $file",
    `AND ((n.source.startByte <= ${root.byte} AND n.source.endByte >= ${root.byte})`,
    `OR (n.source.startLine <= ${root.line} AND n.source.endLine >= ${root.line}))`,
    "AND n.kind IN ['function', 'method', 'constructor', 'procedure', 'subroutine']",
    "RETURN n.id, n.kind, n.source"
  ].join(" ");
  const result = await session.processes.runJson(
    session.root,
    buildCqlArgs({
      query,
      params: { file: root.file },
      timeoutMs: 5000,
      maxRows: 64,
      graph: session.graphPath
    }),
    CursorQuerySchema,
    signal
  );
  return result.rows
    .filter((row) => row["n.source"].value.file.value === root.file)
    .sort((left, right) => {
      const leftSource = left["n.source"].value;
      const rightSource = right["n.source"].value;
      return (leftSource.endByte.value - leftSource.startByte.value)
        - (rightSource.endByte.value - rightSource.startByte.value)
        || (leftSource.endLine.value - leftSource.startLine.value)
        - (rightSource.endLine.value - rightSource.startLine.value)
        || left["n.id"].value.localeCompare(right["n.id"].value);
    })[0]?.["n.id"].value;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function requiresTypedFallback(session: RepositorySession): boolean {
  return /^0\.3\.0(?:\D|$)/.test(session.capabilities?.compass_version ?? "");
}

function missingCursorError(root: CallGraphRoot): Error {
  return new Error(
    `error: no callable graph node matches ${root.file}:${root.byte} (line ${root.line})`
  );
}
