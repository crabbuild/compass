import { useMemo, useRef, useState } from "react";
import {
  AlertTriangleIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  BracesIcon,
  Clock3Icon,
  FileCode2Icon,
  NetworkIcon,
  PlayIcon,
  SearchIcon,
  SquareIcon,
  XIcon
} from "lucide-react";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import type { SourceLocation } from "../contracts/graph";
import { WorkspaceState } from "../components/workbench/WorkspaceState";
import { normalizeStructuredResult, parseExplanationResult } from "./state";

export type QueryCommand = "ask" | "explain" | "cql";
export type QuerySubmission = {
  command: QueryCommand;
  query: string;
  params: Record<string, string>;
  timeoutMs: number;
  maxRows: number;
};
export type QueryOutput =
  | { kind: "code-query"; value: CodeQueryResponse }
  | { kind: "explanation"; text: string }
  | { kind: "rows"; value: unknown };
export type QueryRun = {
  id: string;
  request: QuerySubmission;
  status: "running" | "success" | "error" | "cancelled";
  durationMs?: number | undefined;
  output?: QueryOutput | undefined;
  error?: string | undefined;
};
export type QueryHost = {
  execute(request: QuerySubmission): void;
  cancel(runId: string): void;
  selectRun(runId: string): void;
  closeRun(runId: string): void;
  openSource(source: SourceLocation): void;
  openGraph(): void;
};

const COMMANDS: Array<{
  id: QueryCommand;
  label: string;
  description: string;
  placeholder: string;
}> = [{
  id: "ask",
  label: "Ask",
  description: "Route a question to the typed graph",
  placeholder: "Who calls PaymentService.charge?"
}, {
  id: "explain",
  label: "Explain",
  description: "Inspect one symbol and its relationships",
  placeholder: "crate::PaymentService::charge"
}, {
  id: "cql",
  label: "CompassQL",
  description: "Run a precise, repeatable graph query",
  placeholder: "MATCH (n) RETURN n LIMIT 20"
}];

const EXAMPLES: Record<QueryCommand, string[]> = {
  ask: [
    "Who calls PaymentService.charge?",
    "What depends on the query engine?",
    "Path from request authentication to storage"
  ],
  explain: [
    "PaymentService",
    "crate::Parser::parse",
    "CheckoutController.create"
  ],
  cql: [
    "MATCH (n) RETURN n LIMIT 20",
    "MATCH (a)-[r:CALLS]->(b) RETURN a, r, b LIMIT 20"
  ]
};

export function QueryWorkspace({
  runs,
  activeRunId,
  revision,
  host
}: {
  runs: QueryRun[];
  activeRunId?: string | undefined;
  revision?: string | undefined;
  host: QueryHost;
}) {
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const [command, setCommand] = useState<QueryCommand>("ask");
  const [drafts, setDrafts] = useState<Record<QueryCommand, string>>({
    ask: "",
    explain: "",
    cql: ""
  });
  const [params, setParams] = useState("");
  const parsedParams = useMemo(() => parseParams(params), [params]);
  const activeRun = runs.find((run) => run.id === activeRunId) ?? runs.at(-1);
  const runningRun = runs.find((run) => run.status === "running");
  const selectedCommand = COMMANDS.find((candidate) => candidate.id === command)!;
  const query = drafts[command];
  const execute = () => {
    const trimmed = query.trim();
    if (!trimmed || runningRun) return;
    host.execute({
      command,
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
        <div className="query-mode" role="tablist" aria-label="Query command">
          {COMMANDS.map((candidate) => {
            const Icon = candidate.id === "cql" ? BracesIcon
              : candidate.id === "explain" ? FileCode2Icon : SearchIcon;
            return (
              <button
                id={`query-mode-${candidate.id}`}
                key={candidate.id}
                type="button"
                role="tab"
                aria-selected={command === candidate.id}
                aria-controls="query-composer-panel"
                onClick={() => {
                  setCommand(candidate.id);
                  requestAnimationFrame(() => editorRef.current?.focus());
                }}
              >
                <Icon aria-hidden="true" />
                <span>
                  <strong>{candidate.label}</strong>
                  <small>{candidate.description}</small>
                </span>
              </button>
            );
          })}
        </div>
        <div
          id="query-composer-panel"
          className="query-composer-panel"
          data-mode={command}
          role="tabpanel"
          aria-labelledby={`query-mode-${command}`}
        >
          <div className="query-editor-shell">
            <div className="query-editor">
              <textarea
                ref={editorRef}
                value={query}
                placeholder={selectedCommand.placeholder}
                aria-label={`${selectedCommand.label} input`}
                aria-keyshortcuts="Control+Enter Meta+Enter"
                spellCheck={command === "ask"}
                onChange={(event) => setDrafts((current) => ({
                  ...current,
                  [command]: event.target.value
                }))}
                onKeyDown={(event) => {
                  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                    event.preventDefault();
                    execute();
                  }
                }}
              />
            </div>
            <div className="query-composer-footer">
              {command === "cql" && (
                <label className="query-params">
                  <span>Parameters</span>
                  <input
                    value={params}
                    placeholder="kind=Function, module=api"
                    aria-label="CompassQL parameters"
                    onChange={(event) => setParams(event.target.value)}
                  />
                </label>
              )}
              <div className="query-footer-actions">
                <span className="query-shortcut">
                  {navigator.platform.toLocaleLowerCase().includes("mac") ? "⌘" : "Ctrl"} Enter
                </span>
                <button
                  type="button"
                  className="query-run"
                  aria-label={runningRun ? `Cancel ${commandLabel(runningRun.request.command)}` : runLabel(command)}
                  disabled={!runningRun && !query.trim()}
                  onClick={runningRun ? () => host.cancel(runningRun.id) : execute}
                >
                  {runningRun ? <SquareIcon aria-hidden="true" /> : <PlayIcon aria-hidden="true" />}
                  {runningRun ? "Cancel" : runLabel(command)}
                </button>
              </div>
            </div>
          </div>
          <div className="query-examples" aria-label={`${selectedCommand.label} examples`}>
            <span>Try</span>
            {EXAMPLES[command].map((example) => (
              <button key={example} type="button" onClick={() => {
                setDrafts((current) => ({ ...current, [command]: example }));
                editorRef.current?.focus();
              }}>
                {example}
              </button>
            ))}
          </div>
        </div>
      </header>

      <main className="query-results">
        {runs.length > 0 && (
          <ResultTabs runs={runs} activeRunId={activeRun?.id} host={host} />
        )}
        <div className="query-result-stage">
          {activeRun ? (
            <RunResult run={activeRun} host={host} focusEditor={() => editorRef.current?.focus()} />
          ) : (
            <WorkspaceState
              kind="empty"
              title="Choose the right lens"
              description="Ask for intent-aware graph evidence, explain one symbol, or run CompassQL for exact rows. Each run opens in its own result tab."
            />
          )}
        </div>
      </main>
    </div>
  );
}

function ResultTabs({
  runs,
  activeRunId,
  host
}: {
  runs: QueryRun[];
  activeRunId?: string | undefined;
  host: QueryHost;
}) {
  return (
    <div className="query-result-tabs" role="tablist" aria-label="Query results">
      {runs.map((run) => (
        <div className="query-result-tab" data-status={run.status} key={run.id}>
          <button
            type="button"
            role="tab"
            aria-selected={run.id === activeRunId}
            aria-controls={`query-run-${run.id}`}
            title={run.request.query}
            onClick={() => host.selectRun(run.id)}
          >
            <span>{commandLabel(run.request.command)}</span>
            <strong>{run.request.query}</strong>
          </button>
          <button
            type="button"
            className="query-result-close"
            aria-label={`Close ${commandLabel(run.request.command)} result`}
            onClick={() => host.closeRun(run.id)}
          >
            <XIcon aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}

function RunResult({
  run,
  host,
  focusEditor
}: {
  run: QueryRun;
  host: QueryHost;
  focusEditor(): void;
}) {
  if (run.status === "running") {
    return (
      <WorkspaceState
        kind="running"
        title={`${commandLabel(run.request.command)} is running`}
        description={runningDescription(run.request.command)}
      />
    );
  }
  if (run.status === "error" || run.status === "cancelled") {
    return (
      <WorkspaceState
        kind={run.status === "error" ? "error" : "unavailable"}
        title={run.status === "error" ? `${commandLabel(run.request.command)} failed` : "Run cancelled"}
        description={run.error ?? "The query was cancelled before Compass returned a result."}
        action={{ label: "Edit input", onClick: focusEditor }}
      />
    );
  }
  if (!run.output) {
    return (
      <WorkspaceState
        kind="unavailable"
        title="Result unavailable"
        description="Compass completed without a readable result. Run the command again."
      />
    );
  }
  return (
    <section
      id={`query-run-${run.id}`}
      className="query-result"
      role="tabpanel"
      aria-label={`${commandLabel(run.request.command)} result`}
    >
      <header>
        <div>
          <span>{commandInvocation(run.request.command)}</span>
          <h2>{resultTitle(run)}</h2>
        </div>
        {run.durationMs !== undefined && (
          <span><Clock3Icon aria-hidden="true" /> {run.durationMs.toLocaleString()} ms</span>
        )}
      </header>
      {run.output.kind === "code-query" ? (
        <CodeQueryResult result={run.output.value} host={host} />
      ) : run.output.kind === "explanation" ? (
        <ExplanationResult text={run.output.text} host={host} />
      ) : (
        <RowsResult value={run.output.value} />
      )}
    </section>
  );
}

function CodeQueryResult({ result, host }: { result: CodeQueryResponse; host: QueryHost }) {
  return (
    <div className="query-typed-result">
      <div className="query-traversal-summary">
        <div>
          <p>{operationLabel(result.operation)}</p>
          <div className="query-result-metrics" aria-label="Typed query summary">
            <span><strong>{result.nodes.length.toLocaleString()}</strong> nodes</span>
            <span><strong>{result.edges.length.toLocaleString()}</strong> edges</span>
            <span><strong>{result.paths.length.toLocaleString()}</strong> paths</span>
            {result.truncated && <span><strong>Bounded</strong> result</span>}
          </div>
        </div>
        {result.nodes.length > 0 && (
          <button type="button" className="query-graph-action" onClick={host.openGraph}>
            <NetworkIcon aria-hidden="true" />
            Open code graph
          </button>
        )}
      </div>
      {result.diagnostics.length > 0 && (
        <section className="query-diagnostics" aria-label="Query guidance">
          <h3>Query guidance</h3>
          {result.diagnostics.map((diagnostic, index) => (
            <article key={`${diagnostic.code}:${diagnostic.nodeId ?? index}`}>
              <AlertTriangleIcon aria-hidden="true" />
              <div>
                <strong>{diagnosticLabel(diagnostic.code)}</strong>
                <p>{diagnostic.message}</p>
              </div>
            </article>
          ))}
        </section>
      )}
      {result.nodes.length > 0 ? (
        <div className="query-node-results" role="list" aria-label="Graph query matches">
          {result.nodes.map((node) => (
            <article key={node.id} className="query-node-result" role="listitem">
              <FileCode2Icon aria-hidden="true" />
              <div className="query-node-copy">
                <h3 title={node.qualifiedName}>{node.name}</h3>
                <p>{node.kind.replaceAll("_", " ")} · {node.qualifiedName}</p>
              </div>
              {node.source ? (
                <button
                  type="button"
                  className="query-source-action"
                  aria-label={sourceActionLabel(node.name, node.source)}
                  title={sourceActionLabel(node.name, node.source)}
                  onClick={() => host.openSource(node.source!)}
                >
                  <span>{node.source.file}</span>
                  <small>L{node.source.startLine}</small>
                </button>
              ) : (
                <span className="query-source-missing">Source not recorded</span>
              )}
            </article>
          ))}
        </div>
      ) : result.diagnostics.length === 0 ? (
        <WorkspaceState
          kind="empty"
          title="No graph evidence matched"
          description="Try a qualified symbol name, a concrete behavior, or Explain for one known node."
        />
      ) : null}
    </div>
  );
}

function ExplanationResult({ text, host }: { text: string; host: QueryHost }) {
  const explanation = parseExplanationResult(text);
  if (explanation.kind === "prose") return <ProseResult text={explanation.text} />;
  if (explanation.kind === "ambiguous") {
    return (
      <div className="query-explanation">
        <section className="query-diagnostics" aria-label="Ambiguous symbol matches">
          <h3>{explanation.title}</h3>
          {explanation.candidates.map((candidate) => (
            <article key={candidate.id}>
              <FileCode2Icon aria-hidden="true" />
              <div>
                <strong>{candidate.source || "Source not recorded"}</strong>
                <p>{candidate.id}</p>
              </div>
            </article>
          ))}
        </section>
        <p className="query-guidance">Choose a full node ID and run Explain again.</p>
      </div>
    );
  }
  return (
    <div className="query-explanation">
      <section className="query-explanation-card">
        <div>
          <span>{explanation.type || "code node"}</span>
          <h3>{explanation.label}</h3>
          <code>{explanation.id}</code>
        </div>
        {explanation.source && (
          <button
            type="button"
            className="query-source-action"
            onClick={() => host.openSource(explanation.source!)}
          >
            <span>{explanation.source.file}</span>
            {explanation.source.startLine && <small>L{explanation.source.startLine}</small>}
          </button>
        )}
        <dl>
          <div><dt>Community</dt><dd>{explanation.community || "Not assigned"}</dd></div>
          <div><dt>Degree</dt><dd>{explanation.degree ?? "Not reported"}</dd></div>
        </dl>
      </section>
      {explanation.connections.length > 0 ? (
        <section className="query-connections">
          <h3>Relationships <span>{explanation.connections.length}</span></h3>
          {explanation.connections.map((connection, index) => (
            <article key={`${connection.direction}:${connection.label}:${index}`}>
              {connection.direction === "outgoing"
                ? <ArrowRightIcon aria-hidden="true" />
                : <ArrowLeftIcon aria-hidden="true" />}
              <div>
                <strong>{connection.label}</strong>
                <p>{connection.relation} · {connection.confidence}</p>
              </div>
              <span>{connection.direction}</span>
            </article>
          ))}
        </section>
      ) : (
        <p className="query-guidance">No incoming or outgoing relationships were recorded.</p>
      )}
    </div>
  );
}

function RowsResult({ value }: { value: unknown }) {
  const structured = normalizeStructuredResult(value);
  if (structured) {
    return (
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
    );
  }
  if (isRecord(value)) {
    return (
      <dl className="query-object-result">
        {Object.entries(value).map(([key, field]) => (
          <div key={key}>
            <dt>{key.replaceAll(/([a-z])([A-Z])/g, "$1 $2")}</dt>
            <dd>{displayValue(field)}</dd>
          </div>
        ))}
      </dl>
    );
  }
  return <ProseResult text={displayValue(value)} />;
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

function resultTitle(run: QueryRun): string {
  if (run.output?.kind === "code-query") {
    const result = run.output.value;
    if (result.nodes.length === 0) return "No graph matches";
    return `${result.nodes.length.toLocaleString()} graph ${result.nodes.length === 1 ? "match" : "matches"}`;
  }
  if (run.output?.kind === "explanation") return "Symbol explanation";
  return "CompassQL rows";
}

function commandLabel(command: QueryCommand): string {
  return command === "cql" ? "CompassQL" : command === "ask" ? "Ask" : "Explain";
}

function commandInvocation(command: QueryCommand): string {
  return command === "cql" ? "compass query" : `compass ${command}`;
}

function runLabel(command: QueryCommand): string {
  return command === "ask" ? "Ask" : command === "explain" ? "Explain" : "Run query";
}

function runningDescription(command: QueryCommand): string {
  if (command === "ask") return "Compass is resolving the question to a typed graph operation.";
  if (command === "explain") return "Compass is loading the symbol and its incoming and outgoing relationships.";
  return "Compass is planning and executing the read-only graph query.";
}

function operationLabel(operation: CodeQueryResponse["operation"]): string {
  return operation === "node_trail"
    ? "Directed path evidence"
    : `${operation[0]!.toLocaleUpperCase()}${operation.slice(1)} evidence`;
}

function diagnosticLabel(code: CodeQueryResponse["diagnostics"][number]["code"]): string {
  return code.split("_").map((part) => part[0]!.toLocaleUpperCase() + part.slice(1)).join(" ");
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function displayValue(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (["number", "boolean", "bigint"].includes(typeof value)) return String(value);
  if (value === undefined) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
