type GraphSelection = {
  graph?: string | undefined;
  revision?: string | undefined;
};

function graphSelection(options: GraphSelection): string[] {
  return options.revision
    ? ["--at", options.revision]
    : options.graph
      ? ["--graph", options.graph]
      : [];
}

export function buildAskArgs(options: {
  query: string;
} & GraphSelection): string[] {
  return [
    "ask",
    options.query,
    ...graphSelection(options),
    "--format",
    "json"
  ];
}

export function buildExplainArgs(options: {
  query: string;
} & GraphSelection): string[] {
  return [
    "explain",
    options.query,
    ...graphSelection(options)
  ];
}

export function buildCqlArgs(options: {
  query: string;
  params: Record<string, string>;
  timeoutMs: number;
  maxRows: number;
} & GraphSelection): string[] {
  return [
    "query",
    "--cql",
    options.query,
    ...Object.entries(options.params)
      .sort(([left], [right]) => left.localeCompare(right))
      .flatMap(([name, value]) => ["--param", `${name}=${value}`]),
    "--timeout-ms",
    String(options.timeoutMs),
    "--max-rows",
    String(options.maxRows),
    ...graphSelection(options),
    "--format",
    "json"
  ];
}
