import { useMemo, useState } from "react";
import {
  ArrowRightIcon,
  BoxIcon,
  FileCodeIcon,
  NetworkIcon,
  SearchIcon
} from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../components/ui/tabs";
import { CollectionToolbar } from "../components/workbench/CollectionToolbar";
import { Pagination } from "../components/workbench/Pagination";
import { WorkspaceState } from "../components/workbench/WorkspaceState";
import type { CallflowViewModel } from "../contracts/callflow";
import { paginate } from "../lib/collectionView";
import {
  filterSectionCalls,
  filterSectionSymbols,
  nodeNameMap,
  searchArchitecture,
  sortCalls,
  type ArchitectureResult,
  type CallSort
} from "./state";

const SYMBOL_PAGE_SIZE = 24;
const CALL_PAGE_SIZE = 25;

export type ArchitectureHost = {
  openSource(file: string): void;
};

export function ArchitectureFlow({
  model,
  host
}: {
  model: CallflowViewModel;
  host: ArchitectureHost;
}) {
  const sections = useMemo(
    () => model.sections.filter((section) => section.id !== "overview"),
    [model.sections]
  );
  const first = sections[0]?.id ?? "overview";
  const [sectionId, setSectionId] = useState(first);
  const [activeTab, setActiveTab] = useState<"symbols" | "calls">("symbols");
  const [globalQuery, setGlobalQuery] = useState("");
  const [symbolQuery, setSymbolQuery] = useState("");
  const [callQuery, setCallQuery] = useState("");
  const [symbolPage, setSymbolPage] = useState(1);
  const [callPage, setCallPage] = useState(1);
  const [callSort, setCallSort] = useState<CallSort>({
    column: "caller",
    direction: "ascending"
  });
  const [showAllFlows, setShowAllFlows] = useState(false);
  const section = sections.find((candidate) => candidate.id === sectionId);
  const names = useMemo(() => nodeNameMap(model), [model]);
  const globalResults = useMemo(
    () => searchArchitecture(model, globalQuery),
    [globalQuery, model]
  );
  const globalResultCount = globalResults.reduce(
    (total, group) => total + group.results.length,
    0
  );
  const visibleOverviewLinks = showAllFlows
    ? model.overviewLinks
    : model.overviewLinks.slice(0, 24);
  const symbols = useMemo(
    () => section ? filterSectionSymbols(section, symbolQuery) : [],
    [section, symbolQuery]
  );
  const calls = useMemo(
    () => section
      ? sortCalls(filterSectionCalls(section, names, callQuery), names, callSort)
      : [],
    [callQuery, callSort, names, section]
  );
  const symbolResults = paginate(symbols, symbolPage, SYMBOL_PAGE_SIZE);
  const callResults = paginate(calls, callPage, CALL_PAGE_SIZE);

  const selectSection = (id: string) => {
    setSectionId(id);
    setSymbolPage(1);
    setCallPage(1);
  };
  const selectGlobalResult = (result: ArchitectureResult) => {
    selectSection(result.sectionId);
    setActiveTab(result.tab);
    if (result.tab === "symbols") {
      setSymbolQuery(result.query);
    } else {
      setCallQuery(result.query);
    }
    setGlobalQuery("");
  };
  const toggleSort = (column: CallSort["column"]) => {
    setCallSort((current) => ({
      column,
      direction: current.column === column && current.direction === "ascending"
        ? "descending"
        : "ascending"
    }));
    setCallPage(1);
  };

  return (
    <div className="architecture-shell">
      <aside className="architecture-nav">
        <header className="architecture-nav-header">
          <span>Architecture flow</span>
          <h1>{model.title}</h1>
        </header>
        <nav className="architecture-section-list" aria-label="Architecture sections">
          {sections.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-current={item.id === sectionId ? "page" : undefined}
              onClick={() => selectSection(item.id)}
            >
              <BoxIcon aria-hidden="true" />
              <span>
                <strong>{item.name}</strong>
                <small>
                  {item.nodes.length.toLocaleString()} symbols ·{" "}
                  {item.edges.length.toLocaleString()} calls
                </small>
              </span>
            </button>
          ))}
        </nav>
        <footer className="architecture-stats">
          <span><strong>{model.statistics.nodes.toLocaleString()}</strong>nodes</span>
          <span><strong>{model.statistics.edges.toLocaleString()}</strong>edges</span>
          <span><strong>{model.statistics.communities.toLocaleString()}</strong>groups</span>
        </footer>
      </aside>

      <main className="architecture-main">
        <div className="architecture-global-search">
          <label>
            <SearchIcon aria-hidden="true" />
            <input
              type="search"
              value={globalQuery}
              placeholder="Search symbols, calls, paths, and subsystems"
              aria-label="Search architecture"
              onChange={(event) => setGlobalQuery(event.target.value)}
            />
            {globalQuery && (
              <span role="status">{globalResultCount.toLocaleString()} matches</span>
            )}
          </label>
          {globalQuery && (
            <div className="architecture-search-results" role="listbox" aria-label="Architecture search results">
              {globalResults.length > 0 ? globalResults.slice(0, 8).map((group) => (
                <section
                  key={group.sectionId}
                  role="group"
                  aria-label={`${group.sectionName} search results`}
                >
                  <h2>{group.sectionName}</h2>
                  {group.results.slice(0, 8).map((result) => (
                    <button
                      key={result.id}
                      type="button"
                      role="option"
                      aria-selected="false"
                      aria-label={`${result.label} ${result.kind} in ${result.sectionName}`}
                      onClick={() => selectGlobalResult(result)}
                    >
                      <span>{result.label}</span>
                      <small>{result.kind} · {result.detail}</small>
                    </button>
                  ))}
                </section>
              )) : (
                <p>No architecture matches for “{globalQuery}”</p>
              )}
            </div>
          )}
        </div>

        <section className="architecture-overview" aria-labelledby="system-flow-heading">
          <div className="architecture-section-heading">
            <div>
              <h2 id="system-flow-heading">System call flow</h2>
              <p>Cross-subsystem relationships derived from the current Compass graph.</p>
            </div>
            <Badge variant="outline">
              <NetworkIcon aria-hidden="true" /> {model.overviewLinks.length} flows
            </Badge>
          </div>
          <div className="architecture-flow-grid">
            {visibleOverviewLinks.map((link) => (
              <button
                type="button"
                key={`${link.sourceSection}:${link.targetSection}`}
                className="architecture-flow"
                onClick={() => selectSection(link.targetSection)}
                title={`${sectionName(model, link.sourceSection)} to ${sectionName(model, link.targetSection)}: ${link.calls} calls`}
              >
                <span>{sectionName(model, link.sourceSection)}</span>
                <ArrowRightIcon aria-hidden="true" />
                <span>{sectionName(model, link.targetSection)}</span>
                <Badge variant="secondary">{link.calls}</Badge>
              </button>
            ))}
          </div>
          {!showAllFlows && model.overviewLinks.length > visibleOverviewLinks.length && (
            <div className="architecture-flow-footer">
              <p role="status">
                Showing {visibleOverviewLinks.length} of {model.overviewLinks.length} flows
              </p>
              <button type="button" onClick={() => setShowAllFlows(true)}>
                Show all {model.overviewLinks.length} flows
              </button>
            </div>
          )}
        </section>

        {section && (
          <section className="architecture-detail" aria-labelledby="section-heading">
            <div className="architecture-section-heading">
              <div>
                <span className="architecture-eyebrow">Subsystem</span>
                <h2 id="section-heading">{section.name}</h2>
              </div>
              <span className="architecture-detail-count">
                {section.nodes.length.toLocaleString()} symbols ·{" "}
                {section.edges.length.toLocaleString()} calls
              </span>
            </div>
            <Tabs
              value={activeTab}
              onValueChange={(value) => setActiveTab(value as "symbols" | "calls")}
            >
              <TabsList variant="line">
                <TabsTrigger value="symbols">
                  Symbols <span>{section.nodes.length.toLocaleString()}</span>
                </TabsTrigger>
                <TabsTrigger value="calls">
                  Calls <span>{section.edges.length.toLocaleString()}</span>
                </TabsTrigger>
              </TabsList>
              <TabsContent value="symbols" className="architecture-tab-content">
                <CollectionToolbar
                  value={symbolQuery}
                  label={`Filter ${section.name} symbols`}
                  placeholder="Filter names, kinds, and source paths"
                  resultCount={symbols.length}
                  onChange={(value) => {
                    setSymbolQuery(value);
                    setSymbolPage(1);
                  }}
                />
                {symbolResults.items.length > 0 ? (
                  <div className="architecture-symbol-grid">
                    {symbolResults.items.map((node) => (
                      <article key={node.id} className="architecture-symbol-card">
                        <FileCodeIcon aria-hidden="true" />
                        <div>
                          <h3 title={node.label}>{node.label}</h3>
                          <p>{node.kind || "symbol"} · {section.name}</p>
                        </div>
                        {node.sourceFile ? (
                          <button
                            type="button"
                            title={node.sourceFile}
                            onClick={() => host.openSource(node.sourceFile!)}
                          >
                            {node.sourceFile}
                          </button>
                        ) : (
                          <span>Source not recorded</span>
                        )}
                      </article>
                    ))}
                  </div>
                ) : (
                  <WorkspaceState
                    kind="empty"
                    title="No matching symbols"
                    description={`No symbols in ${section.name} match “${symbolQuery}”.`}
                    action={{ label: "Clear filter", onClick: () => setSymbolQuery("") }}
                  />
                )}
                <Pagination
                  {...symbolResults}
                  label="symbols"
                  onPageChange={setSymbolPage}
                />
              </TabsContent>
              <TabsContent value="calls" className="architecture-tab-content">
                <CollectionToolbar
                  value={callQuery}
                  label={`Filter ${section.name} calls`}
                  placeholder="Filter callers, callees, relations, and evidence"
                  resultCount={calls.length}
                  onChange={(value) => {
                    setCallQuery(value);
                    setCallPage(1);
                  }}
                />
                {callResults.items.length > 0 ? (
                  <div className="architecture-call-table">
                    <table>
                      <thead>
                        <tr>
                          <SortableHeader column="caller" sort={callSort} onSort={toggleSort}>Caller</SortableHeader>
                          <SortableHeader column="relation" sort={callSort} onSort={toggleSort}>Relation</SortableHeader>
                          <SortableHeader column="callee" sort={callSort} onSort={toggleSort}>Callee</SortableHeader>
                          <SortableHeader column="confidence" sort={callSort} onSort={toggleSort}>Evidence</SortableHeader>
                        </tr>
                      </thead>
                      <tbody>
                        {callResults.items.map((edge, index) => (
                          <tr key={`${edge.source}:${edge.target}:${index}`}>
                            <td title={names.get(edge.source) ?? edge.source}>
                              {names.get(edge.source) ?? edge.source}
                            </td>
                            <td>{edge.relation}</td>
                            <td title={names.get(edge.target) ?? edge.target}>
                              {names.get(edge.target) ?? edge.target}
                            </td>
                            <td><Badge variant="outline">{edge.confidence}</Badge></td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : (
                  <WorkspaceState
                    kind="empty"
                    title="No matching calls"
                    description={`No calls in ${section.name} match “${callQuery}”.`}
                    action={{ label: "Clear filter", onClick: () => setCallQuery("") }}
                  />
                )}
                <Pagination
                  {...callResults}
                  label="calls"
                  onPageChange={setCallPage}
                />
              </TabsContent>
            </Tabs>
          </section>
        )}
      </main>
    </div>
  );
}

function SortableHeader({
  column,
  sort,
  onSort,
  children
}: {
  column: CallSort["column"];
  sort: CallSort;
  onSort(column: CallSort["column"]): void;
  children: string;
}) {
  const active = sort.column === column;
  return (
    <th aria-sort={active ? sort.direction : "none"}>
      <button type="button" onClick={() => onSort(column)}>
        {children}
        {active && <span aria-hidden="true">{sort.direction === "ascending" ? "↑" : "↓"}</span>}
      </button>
    </th>
  );
}

function sectionName(model: CallflowViewModel, id: string): string {
  return model.sections.find((section) => section.id === id)?.name ?? id;
}
