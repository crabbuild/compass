import { useMemo, useRef, useState } from "react";
import {
  BracesIcon,
  Clock3Icon,
  FileCode2Icon,
  NetworkIcon,
  PlayIcon,
  SearchIcon,
  SquareIcon
} from "lucide-react";
import type { SourceLocation } from "../contracts/graph";
import { WorkspaceState } from "../components/workbench/WorkspaceState";
import {
  normalizeStructuredResult,
  parseNaturalQueryResult,
  type NaturalQueryResult
} from "./state";

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
  openSource(source: SourceLocation): void;
  openGraph(): void;
};

const NATURAL_EXAMPLES = [
  "How does authentication reach storage?",
  "Which modules depend on the query engine?",
  "Where are errors converted into API responses?"
];

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
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const [mode, setMode] = useState<QueryMode>("natural");
  const [query, setQuery] = useState("");
  const [params, setParams] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const parsedParams = useMemo(() => parseParams(params), [params]);
  const structured = useMemo(
    () => result?.json === undefined ? undefined : normalizeStructuredResult(result.json),
    [result?.json]
  );
  const naturalResult = useMemo(
    () => result?.text === undefined ? undefined : parseNaturalQueryResult(result.text),
    [result?.text]
  );
  const execute = () => {
    const trimmed = query.trim();
    if (!trimmed || running) return;
    setHistory((current) =>
      [trimmed, ...current.filter((item) => item !== trimmed)].slice(0, 20)
    );
    host.execute({
      mode,
      query: trimmed,
      params: parsedParams,
      timeoutMs: 5000,
      maxRows: 1000
    });
  };

  return (
    <div className="query-shell">
      <header className="query-header">
        <div className="query-heading-row">
          <div>
            <span className="query-context">
              Compass query · {revision ? revision.slice(0, 12) : "working tree"}
            </span>
            <h1>Query the codebase</h1>
          </div>
        </div>
        <div className="query-mode" role="tablist" aria-label="Query mode">
          <button
            id="query-mode-natural"
            type="button"
            role="tab"
            aria-selected={mode === "natural"}
            aria-controls="query-composer-panel"
            onClick={() => setMode("natural")}
          >
            <SearchIcon aria-hidden="true" />
            <span>
              <strong>Ask the codebase</strong>
              <small>Explore systems and relationships in plain language</small>
            </span>
          </button>
          <button
            id="query-mode-cql"
            type="button"
            role="tab"
            aria-selected={mode === "cql"}
            aria-controls="query-composer-panel"
            onClick={() => setMode("cql")}
          >
            <BracesIcon aria-hidden="true" />
            <span>
              <strong>CompassQL</strong>
              <small>Run precise, repeatable graph queries</small>
            </span>
          </button>
        </div>
        <div
          id="query-composer-panel"
          className="query-composer-panel"
          data-mode={mode}
          role="tabpanel"
          aria-labelledby={`query-mode-${mode}`}
        >
          <div className="query-composer">
            <div className="query-editor">
              <textarea
                ref={editorRef}
                value={query}
                placeholder={mode === "natural"
                  ? "Ask how a subsystem works, where a symbol is used, or how two modules connect…"
                  : "MATCH (n) RETURN n LIMIT 20"}
                aria-label={mode === "natural" ? "Natural-language query" : "CompassQL query"}
                aria-keyshortcuts="Control+Enter Meta+Enter"
                spellCheck={mode === "natural"}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                    event.preventDefault();
                    execute();
                  }
                }}
              />
              <span className="query-shortcut">
                {navigator.platform.toLocaleLowerCase().includes("mac") ? "⌘" : "Ctrl"} Enter
              </span>
            </div>
            <button
              type="button"
              className="query-run"
              aria-label={running ? "Cancel query" : "Run query"}
              disabled={!running && !query.trim()}
              onClick={running ? host.cancel : execute}
            >
              {running ? <SquareIcon aria-hidden="true" /> : <PlayIcon aria-hidden="true" />}
              {running ? "Cancel" : "Run"}
            </button>
          </div>
          {mode === "cql" ? (
            <label className="query-params">
              <span>Parameters</span>
              <input
                value={params}
                placeholder="kind=Function, module=api"
                aria-label="CompassQL parameters"
                onChange={(event) => setParams(event.target.value)}
              />
            </label>
          ) : (
            <div className="query-examples" aria-label="Example questions">
              <span>Try</span>
              {NATURAL_EXAMPLES.map((example) => (
                <button key={example} type="button" onClick={() => {
                  setQuery(example);
                  editorRef.current?.focus();
                }}>
                  {example}
                </button>
              ))}
            </div>
          )}
        </div>
      </header>

      <main className="query-results">
        {running ? (
          <WorkspaceState
            kind="running"
            title="Traversing the code graph"
            description="Compass is resolving symbols and following relationships for this query."
          />
        ) : error ? (
          <WorkspaceState
            kind="error"
            title="Query failed"
            description={error}
            action={{ label: "Revise query", onClick: () => editorRef.current?.focus() }}
          />
        ) : result ? (
          <section className="query-result" aria-labelledby="query-result-heading">
            <header>
              <div>
                <span>Result</span>
                <h2 id="query-result-heading">
                  {naturalResult?.summary
                    ? `${naturalResult.summary.total.toLocaleString()} graph matches`
                    : result.mode === "natural" ? "Codebase answer" : "CompassQL rows"}
                </h2>
              </div>
              <span><Clock3Icon aria-hidden="true" /> {result.durationMs.toLocaleString()} ms</span>
            </header>
            {result.text !== undefined ? (
              naturalResult && (naturalResult.summary || naturalResult.entries.length > 0) ? (
                <TraversalResult result={naturalResult} host={host} />
              ) : (
                <ProseResult text={naturalResult?.prose ?? result.text} />
              )
            ) : structured ? (
              <div className="query-table">
                <table>
                  <thead>
                    <tr>{structured.columns.map((column) => <th key={column}>{column}</th>)}</tr>
                  </thead>
                  <tbody>
                    {structured.rows.map((row, rowIndex) => (
                      <tr key={rowIndex}>
                        {row.map((cell, columnIndex) => (
                          <td key={`${rowIndex}:${structured.columns[columnIndex]}`}>{cell}</td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <pre className="query-json-result">{JSON.stringify(result.json, null, 2)}</pre>
            )}
          </section>
        ) : (
          <div className="query-empty">
            <WorkspaceState
              kind="empty"
              title="Ask how this codebase works"
              description="Use natural language for an explanation or CompassQL for deterministic graph rows."
            />
            {history.length > 0 && (
              <section className="query-history" aria-label="Recent queries">
                <h2>Recent in this tab</h2>
                {history.slice(0, 6).map((item) => (
                  <button key={item} type="button" onClick={() => {
                    setQuery(item);
                    editorRef.current?.focus();
                  }}>
                    {item}
                  </button>
                ))}
              </section>
            )}
          </div>
        )}
      </main>
    </div>
  );
}

function TraversalResult({
  result,
  host
}: {
  result: NaturalQueryResult;
  host: QueryHost;
}) {
  const communities = new Set(
    result.entries.map((entry) => entry.community).filter(Boolean)
  ).size;
  const strategy = result.summary?.strategy.toLocaleUpperCase() === "BFS"
    ? "Breadth-first"
    : result.summary?.strategy.toLocaleUpperCase() === "DFS"
      ? "Depth-first"
      : result.summary?.strategy;

  return (
    <div className="query-traversal-result">
      <div className="query-traversal-summary">
        <div>
          {result.summary && (
            <p>
              {strategy} · depth {result.summary.depth}
              {result.summary.starts.length > 0
                ? ` · started at ${result.summary.starts.join(", ")}`
                : ""}
            </p>
          )}
          <div className="query-result-metrics" aria-label="Traversal summary">
            <span><strong>{result.entries.length.toLocaleString()}</strong> listed</span>
            <span><strong>{communities.toLocaleString()}</strong> communities</span>
            <span>
              <strong>
                {result.entries.filter((entry) => entry.source).length.toLocaleString()}
              </strong>{" "}
              source links
            </span>
          </div>
        </div>
        <button type="button" className="query-graph-action" onClick={host.openGraph}>
          <NetworkIcon aria-hidden="true" />
          Open code graph
        </button>
      </div>
      {result.prose && <ProseResult text={result.prose} />}
      <div className="query-node-results" role="list" aria-label="Graph query matches">
        {result.entries.map((entry, index) => (
          <article
            key={`${entry.kind}:${entry.label}:${entry.source?.file ?? index}`}
            className="query-node-result"
            role="listitem"
          >
            <FileCode2Icon aria-hidden="true" />
            <div className="query-node-copy">
              <h3 title={entry.label}>{entry.label}</h3>
              <p>
                {entry.kind.toLocaleLowerCase()}
                {entry.community ? ` · ${entry.community}` : ""}
              </p>
            </div>
            {entry.source ? (
              <button
                type="button"
                className="query-source-action"
                aria-label={sourceActionLabel(entry.label, entry.source)}
                title={sourceActionLabel(entry.label, entry.source)}
                onClick={() => host.openSource(entry.source!)}
              >
                <span>{entry.source.file}</span>
                {entry.source.startLine && <small>L{entry.source.startLine}</small>}
              </button>
            ) : (
              <span className="query-source-missing">Source not recorded</span>
            )}
          </article>
        ))}
      </div>
    </div>
  );
}

function ProseResult({ text }: { text: string }) {
  const blocks = text.trim().split(/\n{2,}/).filter(Boolean);
  return (
    <div className="query-prose-result">
      {blocks.map((block, index) => {
        const lines = block.split(/\r?\n/);
        if (lines.every((line) => /^[-*]\s+/.test(line))) {
          return (
            <ul key={index}>
              {lines.map((line) => <li key={line}>{line.replace(/^[-*]\s+/, "")}</li>)}
            </ul>
          );
        }
        return <p key={index}>{lines.join(" ")}</p>;
      })}
    </div>
  );
}

function sourceActionLabel(label: string, source: SourceLocation): string {
  return `Open ${label} at ${source.file}${
    source.startLine ? ` line ${source.startLine}` : ""
  }`;
}

function parseParams(value: string): Record<string, string> {
  return Object.fromEntries(
    value.split(",")
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => {
        const [name, ...rest] = entry.split("=");
        return [name ?? "", rest.join("=")];
      })
      .filter(([name]) => name)
  );
}
