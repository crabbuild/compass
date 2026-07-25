import { useMemo, useState } from "react";
import { BracesIcon, PlayIcon, SearchIcon, SquareIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";

export type QueryMode = "natural" | "cql";
export type QueryRequest = {
  mode: QueryMode;
  query: string;
  params: Record<string, string>;
  timeoutMs: number;
  maxRows: number;
};
export type QueryResult = {
  mode: QueryMode;
  text?: string;
  json?: unknown;
  durationMs: number;
};
export type QueryHost = {
  execute(request: QueryRequest): void;
  cancel(): void;
};

export function QueryWorkspace({
  result,
  running,
  error,
  revision,
  host
}: {
  result?: QueryResult | undefined;
  running: boolean;
  error?: string | undefined;
  revision?: string | undefined;
  host: QueryHost;
}) {
  const [mode, setMode] = useState<QueryMode>("natural");
  const [query, setQuery] = useState("");
  const [params, setParams] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const parsedParams = useMemo(() => parseParams(params), [params]);
  const execute = () => {
    const trimmed = query.trim();
    if (!trimmed) return;
    setHistory((current) => [trimmed, ...current.filter((item) => item !== trimmed)].slice(0, 100));
    host.execute({ mode, query: trimmed, params: parsedParams, timeoutMs: 5000, maxRows: 1000 });
  };
  return (
    <div className="query-shell">
      <header className="border-b p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <span className="font-mono text-xs uppercase tracking-wider text-muted-foreground">
              Compass query {revision ? `· ${revision.slice(0, 12)}` : "· working tree"}
            </span>
            <h1 className="text-lg font-semibold">Ask the codebase</h1>
          </div>
          <div
            className="inline-flex h-8 items-center rounded-lg bg-muted p-[3px]"
            role="group"
            aria-label="Query language"
          >
            <Button
              size="xs"
              variant={mode === "natural" ? "secondary" : "ghost"}
              aria-pressed={mode === "natural"}
              onClick={() => setMode("natural")}
            >
              <SearchIcon /> Natural language
            </Button>
            <Button
              size="xs"
              variant={mode === "cql" ? "secondary" : "ghost"}
              aria-pressed={mode === "cql"}
              onClick={() => setMode("cql")}
            >
              <BracesIcon /> CompassQL
            </Button>
          </div>
        </div>
        <div className="mt-4 flex gap-2">
          <textarea
            className="min-h-24 flex-1 resize-y rounded-md border bg-input/30 px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            value={query}
            placeholder={mode === "natural"
              ? "How does authentication reach the data layer?"
              : "MATCH (n) RETURN n LIMIT 20"}
            aria-label={mode === "natural" ? "Natural-language query" : "CompassQL query"}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") execute();
            }}
          />
          <Button className="self-end" onClick={running ? host.cancel : execute}>
            {running ? <SquareIcon /> : <PlayIcon />}
            {running ? "Cancel" : "Run"}
          </Button>
        </div>
        {mode === "cql" && (
          <Input
            className="mt-2 font-mono"
            value={params}
            placeholder="Parameters: kind=Function, module=api"
            aria-label="CompassQL parameters"
            onChange={(event) => setParams(event.target.value)}
          />
        )}
      </header>
      <main className="min-h-0 overflow-auto p-4">
        {error && <div role="alert" className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm">{error}</div>}
        {!result && !running && !error && (
          <div className="grid min-h-64 place-items-center text-center text-muted-foreground">
            <div>
              <SearchIcon className="mx-auto mb-2 size-8" />
              <p>Run a question or deterministic CompassQL query.</p>
              {history.length > 0 && (
                <div className="mt-4 flex max-w-xl flex-wrap justify-center gap-1">
                  {history.slice(0, 8).map((item) => (
                    <Button key={item} size="xs" variant="outline" onClick={() => setQuery(item)}>
                      {item}
                    </Button>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
        {running && <div role="status" className="text-sm text-muted-foreground">Compass is traversing the graph…</div>}
        {result && (
          <section>
            <div className="mb-2 text-xs text-muted-foreground">Completed in {result.durationMs} ms</div>
            {result.text !== undefined ? (
              <pre className="whitespace-pre-wrap rounded-md border bg-card p-4 text-sm">{result.text}</pre>
            ) : (
              <StructuredResult value={result.json} />
            )}
          </section>
        )}
      </main>
    </div>
  );
}

function StructuredResult({ value }: { value: unknown }) {
  const rows = value && typeof value === "object" && "rows" in value
    && Array.isArray((value as { rows?: unknown }).rows)
    ? (value as { rows: unknown[] }).rows
    : undefined;
  if (!rows) {
    return <pre className="overflow-auto rounded-md border bg-card p-4 text-xs">{JSON.stringify(value, null, 2)}</pre>;
  }
  return (
    <div className="overflow-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <tbody>
          {rows.map((row, index) => (
            <tr key={index} className="border-t">
              <td className="p-2 font-mono text-xs">{JSON.stringify(row)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function parseParams(value: string): Record<string, string> {
  return Object.fromEntries(value.split(",").map((entry) => entry.trim()).filter(Boolean).map((entry) => {
    const [name, ...rest] = entry.split("=");
    return [name ?? "", rest.join("=")];
  }).filter(([name]) => name));
}
