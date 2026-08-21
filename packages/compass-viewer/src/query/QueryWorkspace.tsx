import { useMemo, useRef, useState } from "react";
import {
  AlertTriangleIcon,
  ArrowRightIcon,
  BracesIcon,
  CheckCircle2Icon,
  Clock3Icon,
  FileCode2Icon,
  GitBranchIcon,
  Layers3Icon,
  NetworkIcon,
  PlayIcon,
  SearchIcon,
  SquareIcon
} from "lucide-react";
import {
  DiscoveryQueryResponseSchema,
  type CodeEvidenceRecord,
  type CodeQueryNode,
  type CodeSourceAnchor,
  type DiscoveryEdge,
  type DiscoveryQueryResponse,
  type DiscoverySeed
} from "../contracts/codeQuery";
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
    () => result?.mode !== "cql" || result.json === undefined
      ? undefined
      : normalizeStructuredResult(result.json),
    [result?.json, result?.mode]
  );
  const discovery = useMemo(
    () => {
      if (result?.mode !== "natural" || result.json === undefined) return undefined;
      const decoded = DiscoveryQueryResponseSchema.safeParse(result.json);
      return decoded.success ? decoded.data : undefined;
    },
    [result?.json, result?.mode]
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
            <div className="query-editor-shell">
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
              </div>
              <div className="query-composer-footer">
                {mode === "cql" && (
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
                    aria-label={running ? "Cancel query" : "Run query"}
                    disabled={!running && !query.trim()}
                    onClick={running ? host.cancel : execute}
                  >
                    {running ? <SquareIcon aria-hidden="true" /> : <PlayIcon aria-hidden="true" />}
                    {running ? "Cancel" : "Run"}
                  </button>
                </div>
              </div>
            </div>
          </div>
          {mode === "natural" && (
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
                  {discovery
                    ? `${discovery.nodes.length.toLocaleString()} ${plural(
                      discovery.nodes.length,
                      "symbol",
                      "symbols"
                    )} found`
                    : structured
                      ? `${structured.rows.length.toLocaleString()} CompassQL ${plural(
                        structured.rows.length,
                        "row",
                        "rows"
                      )}`
                      : naturalResult?.summary
                    ? `${naturalResult.summary.total.toLocaleString()} graph matches`
                    : result.mode === "natural" ? "Codebase answer" : "CompassQL rows"}
                </h2>
              </div>
              <span><Clock3Icon aria-hidden="true" /> {result.durationMs.toLocaleString()} ms</span>
            </header>
            {discovery ? (
              <DiscoveryResult result={discovery} host={host} />
            ) : result.text !== undefined ? (
              naturalResult && (naturalResult.summary || naturalResult.entries.length > 0) ? (
                <TraversalResult result={naturalResult} host={host} />
              ) : (
                <ProseResult text={naturalResult?.prose ?? result.text} />
              )
            ) : structured ? (
              <div className="query-table">
                <table>
                  <thead>
                    <tr>
                      <th className="query-table-index" aria-label="Row number">#</th>
                      {structured.columns.map((column) => <th key={column}>{column}</th>)}
                    </tr>
                  </thead>
                  <tbody>
                    {structured.rows.map((row, rowIndex) => (
                      <tr key={rowIndex}>
                        <th className="query-table-index" scope="row">{rowIndex + 1}</th>
                        {row.map((cell, columnIndex) => (
                          <td
                            key={`${rowIndex}:${structured.columns[columnIndex]}`}
                            title={cell}
                          >
                            {cell || <span className="query-cell-empty">empty</span>}
                          </td>
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

function DiscoveryResult({
  result,
  host
}: {
  result: DiscoveryQueryResponse;
  host: QueryHost;
}) {
  const nodeById = new Map(result.nodes.map((node) => [node.id, node]));
  const seedById = new Map(result.seeds.map((seed) => [seed.nodeId, seed]));
  const ambiguousSeeds = result.seeds.filter((seed) => seed.ambiguous);
  const omissions = Object.entries(result.omissions)
    .filter((entry): entry is [string, number] => typeof entry[1] === "number" && entry[1] > 0);
  const incompleteCoverage = result.diagnostics.some(
    (diagnostic) => diagnostic.code === "incomplete_coverage"
  );
  const coverage = result.truncated
    ? { label: "Partial", tone: "warning" }
    : incompleteCoverage
      ? { label: "Limited", tone: "warning" }
      : { label: "Complete", tone: "exact" };

  return (
    <div className="query-discovery-result">
      <section className="query-discovery-overview" aria-label="Query overview">
        <div className="query-discovery-question">
          <span>Compass followed</span>
          <p>{result.question}</p>
        </div>
        <dl className="query-discovery-metrics">
          <div>
            <dt>Symbols</dt>
            <dd>{result.nodes.length.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Relationships</dt>
            <dd>{result.edges.length.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Starting points</dt>
            <dd>{result.seeds.length.toLocaleString()}</dd>
          </div>
          <div data-tone={coverage.tone}>
            <dt>Coverage</dt>
            <dd>
              {coverage.tone === "exact"
                ? <CheckCircle2Icon aria-hidden="true" />
                : <AlertTriangleIcon aria-hidden="true" />}
              {coverage.label}
            </dd>
          </div>
        </dl>
        <div className="query-discovery-route">
          <GitBranchIcon aria-hidden="true" />
          <div>
            <span>{result.traversal.toLocaleUpperCase()} traversal</span>
            <small>
              {directionDescription(result.selectedDirection)}
              {result.relationContexts.length > 0
                ? ` · ${result.relationContexts.map(humanize).join(", ")}`
                : " · all relationship types"}
            </small>
          </div>
          <button type="button" className="query-graph-action" onClick={host.openGraph}>
            <NetworkIcon aria-hidden="true" />
            Open code graph
          </button>
        </div>
        {result.seeds.length > 0 && (
          <div className="query-discovery-seeds" aria-label="Starting points">
            <span>Started at</span>
            <div>
              {result.seeds.map((seed) => {
                const node = nodeById.get(seed.nodeId);
                return (
                  <span key={seed.nodeId} data-ambiguous={seed.ambiguous || undefined}>
                    {node?.name ?? shortId(seed.nodeId)}
                    <small>{seedSourceLabel(seed)}</small>
                  </span>
                );
              })}
            </div>
          </div>
        )}
      </section>

      {(result.truncated || omissions.length > 0) && (
        <div className="query-discovery-notice" role="status">
          <AlertTriangleIcon aria-hidden="true" />
          <div>
            <strong>This is a bounded result</strong>
            <span>
              Compass reached a query limit
              {omissions.length > 0
                ? `; ${omissions.map(([name, count]) => `${count} ${humanize(name)}`).join(", ")} omitted.`
                : "."}
            </span>
          </div>
        </div>
      )}

      {result.diagnostics.length > 0 && (
        <section className="query-discovery-diagnostics" aria-labelledby="query-diagnostics-heading">
          <h3 id="query-diagnostics-heading">What to know</h3>
          <ul>
            {result.diagnostics.map((diagnostic, index) => (
              <li key={`${diagnostic.code}:${diagnostic.nodeId ?? index}`}>
                <AlertTriangleIcon aria-hidden="true" />
                <div>
                  <strong>{humanize(diagnostic.code)}</strong>
                  <span>{diagnostic.message}</span>
                </div>
                {diagnostic.path && (
                  <SourceAction
                    label={diagnostic.path}
                    source={{ file: diagnostic.path }}
                    host={host}
                    compact
                  />
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {ambiguousSeeds.length > 0 && (
        <section className="query-discovery-ambiguity" aria-labelledby="query-ambiguity-heading">
          <h3 id="query-ambiguity-heading">Possible starting points</h3>
          <p>Compass kept these alternatives visible instead of guessing.</p>
          <ul>
            {ambiguousSeeds.flatMap((seed) => seed.alternatives.map((alternative) => (
              <li key={`${seed.nodeId}:${alternative.nodeId}`}>
                <span>{alternative.qualifiedName}</span>
                {alternative.source && (
                  <SourceAction
                    label={alternative.qualifiedName}
                    source={alternative.source}
                    host={host}
                    compact
                  />
                )}
              </li>
            )))}
          </ul>
        </section>
      )}

      {result.nodes.length > 0 ? (
        <section className="query-discovery-section" aria-labelledby="query-symbols-heading">
          <div className="query-discovery-section-heading">
            <div>
              <span>Itemized result</span>
              <h3 id="query-symbols-heading">Symbols</h3>
            </div>
            <span>{result.nodes.length.toLocaleString()}</span>
          </div>
          <ol className="query-discovery-nodes">
            {result.nodes.map((node, index) => (
              <DiscoveryNode
                key={node.id}
                index={index}
                node={node}
                seed={seedById.get(node.id)}
                host={host}
              />
            ))}
          </ol>
        </section>
      ) : (
        <div className="query-discovery-empty" role="status">
          <SearchIcon aria-hidden="true" />
          <div>
            <strong>No symbols matched this question</strong>
            <span>Try a concrete symbol, file, subsystem, or relationship such as “what calls save?”</span>
          </div>
        </div>
      )}

      {result.edges.length > 0 && (
        <details className="query-discovery-relationships" open={result.edges.length <= 24}>
          <summary>
            <span>
              <strong>Relationships</strong>
              <small>How the listed symbols connect</small>
            </span>
            <span>{result.edges.length.toLocaleString()}</span>
          </summary>
          <ol>
            {result.edges.map((edge, index) => (
              <DiscoveryRelationship
                key={edge.id ?? `${edge.source}:${edge.target}:${index}`}
                edge={edge}
                index={index}
                nodeById={nodeById}
                host={host}
              />
            ))}
          </ol>
        </details>
      )}

      <details className="query-discovery-technical">
        <summary>Query details</summary>
        <dl>
          <div><dt>Direction</dt><dd>{humanize(result.selectedDirection)} ({humanize(result.directionSource)})</dd></div>
          <div><dt>Traversal</dt><dd>{result.traversal.toLocaleUpperCase()}</dd></div>
          <div><dt>Visited</dt><dd>{result.stats.visitedNodes.toLocaleString()} symbols</dd></div>
          <div><dt>Expanded</dt><dd>{result.stats.expandedRelationships.toLocaleString()} relationships</dd></div>
          <div>
            <dt>Scope</dt>
            <dd>{result.scope.length > 0
              ? result.scope.map((scope) => `${humanize(scope.kind)}: ${scope.value}`).join(", ")
              : "Entire graph"}</dd>
          </div>
        </dl>
      </details>
    </div>
  );
}

function DiscoveryNode({
  index,
  node,
  seed,
  host
}: {
  index: number;
  node: CodeQueryNode;
  seed?: DiscoverySeed | undefined;
  host: QueryHost;
}) {
  const confidence = evidenceConfidence(node.evidence);
  const detail = nodeDetailSummary(node);
  return (
    <li className="query-discovery-node" data-confidence={confidence}>
      <article>
        <span className="query-discovery-index" aria-hidden="true">
          {String(index + 1).padStart(2, "0")}
        </span>
        <FileCode2Icon className="query-discovery-node-icon" aria-hidden="true" />
        <div className="query-discovery-node-copy">
          <div className="query-discovery-node-badges">
            <span data-kind={node.kind}>{humanize(node.kind)}</span>
            {seed && <span data-seed="true">Starting point</span>}
            {node.roles.map((role) => <span key={role}>{humanize(role)}</span>)}
          </div>
          <h4>{node.name}</h4>
          {node.qualifiedName !== node.name && <p>{node.qualifiedName}</p>}
          {detail && <code className="query-discovery-signature">{detail}</code>}
          <div className="query-discovery-node-meta">
            {node.language && <span>{humanize(node.language)}</span>}
            {node.framework && <span>{node.framework}</span>}
            <span data-confidence={confidence}>{confidenceLabel(confidence)} evidence</span>
          </div>
        </div>
        {node.source ? (
          <SourceAction label={node.name} source={node.source} host={host} />
        ) : (
          <span className="query-source-missing">Source not recorded</span>
        )}
        <details className="query-discovery-evidence">
          <summary>
            Why this matched
            <span>{node.evidence.length.toLocaleString()} {plural(node.evidence.length, "record", "records")}</span>
          </summary>
          {seed && (
            <div className="query-discovery-match-reason">
              <strong>{seedSourceLabel(seed)}</strong>
              <span>
                {seed.matchedTerms.length > 0
                  ? `Matched ${seed.matchedTerms.join(", ")}`
                  : "Selected as a traversal starting point"}
                {seed.matchedFields.length > 0
                  ? ` in ${seed.matchedFields.map(humanize).join(", ")}`
                  : ""}
              </span>
            </div>
          )}
          <EvidenceList evidence={node.evidence} host={host} />
          <div className="query-discovery-identity">
            <span>Stable identity</span>
            <code title={node.id}>{shortId(node.id)}</code>
          </div>
        </details>
      </article>
    </li>
  );
}

function DiscoveryRelationship({
  edge,
  index,
  nodeById,
  host
}: {
  edge: DiscoveryEdge;
  index: number;
  nodeById: Map<string, CodeQueryNode>;
  host: QueryHost;
}) {
  const sourceNode = nodeById.get(edge.source);
  const targetNode = nodeById.get(edge.target);
  const site = edge.relationshipSite
    ?? edge.evidence.find((record) => record.anchor || record.wiringSite)?.anchor
    ?? edge.evidence.find((record) => record.wiringSite)?.wiringSite;
  const confidence = evidenceConfidence(edge.evidence);
  return (
    <li data-confidence={confidence}>
      <span className="query-discovery-index" aria-hidden="true">
        {String(index + 1).padStart(2, "0")}
      </span>
      <div className="query-relationship-flow">
        <span title={sourceNode?.qualifiedName ?? edge.source}>
          {sourceNode?.name ?? shortId(edge.source)}
        </span>
        <span className="query-relationship-kind">
          {humanize(edge.kind)}
          <ArrowRightIcon aria-hidden="true" />
        </span>
        <span title={targetNode?.qualifiedName ?? edge.target}>
          {targetNode?.name ?? shortId(edge.target)}
        </span>
      </div>
      <div className="query-relationship-meta">
        {edge.context && <span>{humanize(edge.context)}</span>}
        <span data-confidence={confidence}>{confidenceLabel(confidence)}</span>
        {site && <SourceAction label={`${sourceNode?.name ?? "relationship"} relationship`} source={site} host={host} compact />}
      </div>
    </li>
  );
}

function EvidenceList({
  evidence,
  host
}: {
  evidence: CodeEvidenceRecord[];
  host: QueryHost;
}) {
  if (evidence.length === 0) {
    return <p className="query-discovery-no-evidence">No evidence record was published.</p>;
  }
  return (
    <ul className="query-discovery-evidence-list">
      {evidence.map((record, index) => {
        const source = record.anchor ?? record.wiringSite;
        return (
          <li key={`${record.extractor}:${index}`} data-confidence={record.confidence}>
            <Layers3Icon aria-hidden="true" />
            <div>
              <strong>{confidenceLabel(record.confidence)} · {humanize(record.resolution)}</strong>
              <span>{humanize(record.layer)} · {humanize(record.origin)}</span>
              <code>{record.extractor}</code>
              {record.rule && <small>Rule: {humanize(record.rule)}</small>}
            </div>
            {source && <SourceAction label="evidence" source={source} host={host} compact />}
          </li>
        );
      })}
    </ul>
  );
}

function SourceAction({
  label,
  source,
  host,
  compact = false
}: {
  label: string;
  source: SourceLocation | CodeSourceAnchor;
  host: QueryHost;
  compact?: boolean | undefined;
}) {
  return (
    <button
      type="button"
      className="query-source-action"
      data-compact={compact || undefined}
      aria-label={sourceActionLabel(label, source)}
      title={sourceActionLabel(label, source)}
      onClick={() => host.openSource(source)}
    >
      <span>{source.file}</span>
      {source.startLine && <small>{sourceLineLabel(source)}</small>}
    </button>
  );
}

type EvidenceConfidence = CodeEvidenceRecord["confidence"] | "unverified";

function evidenceConfidence(evidence: CodeEvidenceRecord[]): EvidenceConfidence {
  if (evidence.some((record) => record.confidence === "ambiguous")) return "ambiguous";
  if (evidence.some((record) => record.confidence === "inferred")) return "inferred";
  if (evidence.some((record) => record.confidence === "exact")) return "exact";
  return "unverified";
}

function confidenceLabel(confidence: EvidenceConfidence): string {
  return confidence === "unverified" ? "Unverified" : humanize(confidence);
}

function nodeDetailSummary(node: CodeQueryNode): string | undefined {
  const details = node.details;
  if (!details) return undefined;
  switch (details.type) {
    case "symbol": return details.data.signature ?? undefined;
    case "route": return `${details.data.operation.toLocaleUpperCase()} ${details.data.path}`;
    case "import_export": return details.data.specifier;
    case "config": return details.data.keyPath;
    case "messaging": return `${details.data.transport}: ${details.data.subject}`;
    case "job": return details.data.schedule ?? details.data.queue ?? undefined;
    case "schema": return details.data.dialect ?? details.data.logicalDatabase ?? undefined;
    case "query": return details.data.operation ?? details.data.dialect ?? undefined;
    case "database": return [details.data.logicalDatabase, details.data.databaseSchema]
      .filter(Boolean).join(" · ");
    case "component": return details.data.componentType;
    case "resource": return details.data.mediaType ?? details.data.resourceKind;
    case "file": return `${details.data.byteSize.toLocaleString()} bytes${
      details.data.generated ? " · generated" : ""
    }`;
  }
}

function seedSourceLabel(seed: DiscoverySeed): string {
  const labels: Record<DiscoverySeed["candidateSource"], string> = {
    exact_id: "Exact identity match",
    exact_name: "Exact name match",
    alias: "Alias match",
    term_index: "Term match",
    relation_seed: "Relationship match",
    fuzzy: "Similar name",
    heuristic_fallback: "Heuristic match"
  };
  return labels[seed.candidateSource];
}

function directionDescription(direction: DiscoveryQueryResponse["selectedDirection"]): string {
  switch (direction) {
    case "incoming": return "Following relationships into the starting points";
    case "outgoing": return "Following relationships out from the starting points";
    case "both": return "Following relationships in both directions";
    case "auto": return "Compass selected the relationship direction";
  }
}

function humanize(value: string): string {
  return value.replaceAll("_", " ").replaceAll("-", " ").replace(/\b\w/g, (letter) =>
    letter.toLocaleUpperCase());
}

function shortId(value: string): string {
  const digest = value.startsWith("sha256:") ? value.slice(7) : value;
  return digest.length > 16 ? `${digest.slice(0, 12)}…` : digest;
}

function sourceLineLabel(source: Pick<SourceLocation, "startLine" | "endLine">): string {
  if (!source.startLine) return "";
  return source.endLine && source.endLine !== source.startLine
    ? `L${source.startLine}–${source.endLine}`
    : `L${source.startLine}`;
}

function plural(count: number, singular: string, pluralValue: string): string {
  return count === 1 ? singular : pluralValue;
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
