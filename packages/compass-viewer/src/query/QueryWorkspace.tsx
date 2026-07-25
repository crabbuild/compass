import { useMemo, useRef, useState } from "react";
import {
  BracesIcon,
  Clock3Icon,
  PlayIcon,
  SearchIcon,
  SquareIcon
} from "lucide-react";
import { WorkspaceState } from "../components/workbench/WorkspaceState";
import { normalizeStructuredResult } from "./state";

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
            <h1>Ask the codebase</h1>
          </div>
          <div className="query-mode" role="group" aria-label="Query language">
            <button
              type="button"
              aria-pressed={mode === "natural"}
              onClick={() => setMode("natural")}
            >
              <SearchIcon aria-hidden="true" /> Natural language
            </button>
            <button
              type="button"
              aria-pressed={mode === "cql"}
              onClick={() => setMode("cql")}
            >
              <BracesIcon aria-hidden="true" /> CompassQL
            </button>
          </div>
        </div>
        <div className="query-composer">
          <div className="query-editor">
            <textarea
              ref={editorRef}
              value={query}
              placeholder={mode === "natural"
                ? "Ask how code, modules, or systems connect"
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
                  {result.mode === "natural" ? "Codebase answer" : "CompassQL rows"}
                </h2>
              </div>
              <span><Clock3Icon aria-hidden="true" /> {result.durationMs.toLocaleString()} ms</span>
            </header>
            {result.text !== undefined ? (
              <pre className="query-text-result">{result.text}</pre>
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
