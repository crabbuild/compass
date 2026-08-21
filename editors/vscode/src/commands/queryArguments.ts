export function buildNaturalQueryArgs(options: {
  query: string;
  graph?: string | undefined;
  revision?: string | undefined;
}): string[] {
  return [
    "query",
    options.query,
    ...(options.revision ? ["--at", options.revision] : options.graph ? ["--graph", options.graph] : []),
    "--format",
    "json"
  ];
}

export function buildCqlArgs(options: {
  query: string;
  params: Record<string, string>;
  timeoutMs: number;
  maxRows: number;
  graph?: string | undefined;
  revision?: string | undefined;
}): string[] {
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
    ...(options.revision ? ["--at", options.revision] : options.graph ? ["--graph", options.graph] : []),
    "--format",
    "json"
  ];
}
