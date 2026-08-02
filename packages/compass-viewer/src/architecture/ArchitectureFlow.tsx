import { useEffect, useState } from "react";
import {
  AlertTriangleIcon,
  ArrowDownLeftIcon,
  ArrowUpRightIcon,
  BoxIcon,
  FileCodeIcon,
  PanelRightCloseIcon,
  PanelRightOpenIcon,
  SearchIcon,
  SlidersHorizontalIcon
} from "lucide-react";
import type {
  ArchitectureEvidence,
  ArchitectureOverview,
  ArchitectureRoutePage,
  ArchitectureScope,
  ArchitectureSearchPage,
  ArchitectureSectionPage
} from "../contracts/architecture";
import { Pagination } from "../components/workbench/Pagination";
import { ArchitectureMap, type ArchitectureSelection } from "./ArchitectureMap";

export type ArchitectureHost = {
  setFilters(scope: ArchitectureScope, evidence: ArchitectureEvidence): void;
  requestSection(
    sectionId: string,
    kind: "symbols" | "calls",
    page: number,
    query: string
  ): void;
  requestRoute(routeId: string, page: number, query: string): void;
  search(query: string, page: number): void;
  openSource(file: string): void;
};

export function ArchitectureFlow({
  overview,
  sectionPage,
  routePage,
  searchPage,
  loadingMessage,
  host
}: {
  overview: ArchitectureOverview;
  sectionPage: ArchitectureSectionPage | undefined;
  routePage: ArchitectureRoutePage | undefined;
  searchPage: ArchitectureSearchPage | undefined;
  loadingMessage: string | undefined;
  host: ArchitectureHost;
}) {
  const firstSection = overview.sections.find((section) => section.nodeCount > 0);
  const [selection, setSelection] = useState<ArchitectureSelection>(
    firstSection ? { kind: "section", id: firstSection.id } : undefined
  );
  const [sectionTab, setSectionTab] = useState<"symbols" | "calls">("symbols");
  const [detailQuery, setDetailQuery] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [inspectorOpen, setInspectorOpen] = useState(true);

  useEffect(() => {
    const selectionExists = selection?.kind === "section"
      ? overview.sections.some((section) => section.id === selection.id)
      : selection?.kind === "route"
        ? overview.routes.some((route) => route.id === selection.id)
        : false;
    if (selection && !selectionExists) {
      setSelection(firstSection ? { kind: "section", id: firstSection.id } : undefined);
      return;
    }
    if (!selection && firstSection) {
      setSelection({ kind: "section", id: firstSection.id });
      return;
    }
    if (selection?.kind === "section") {
      host.requestSection(selection.id, sectionTab, 1, detailQuery);
    } else if (selection?.kind === "route") {
      host.requestRoute(selection.id, 1, detailQuery);
    }
  }, [detailQuery, firstSection, host, overview.routes, overview.sections, sectionTab, selection]);

  const selectedSection = selection?.kind === "section"
    ? overview.sections.find((section) => section.id === selection.id)
    : undefined;
  const selectedRoute = selection?.kind === "route"
    ? overview.routes.find((route) => route.id === selection.id)
    : undefined;
  const displayedSectionPage = sectionPage && sectionPage.sectionId === selectedSection?.id
    && sectionPage.kind === sectionTab ? sectionPage : undefined;
  const displayedRoutePage = routePage?.routeId === selectedRoute?.id ? routePage : undefined;

  const select = (next: Exclude<ArchitectureSelection, undefined>) => {
    setSelection(next);
    setDetailQuery("");
    if (next.kind === "section") {
      setSectionTab("symbols");
    }
  };

  const updateSearch = (value: string) => {
    setSearchQuery(value);
    host.search(value, 1);
  };

  return (
    <div className="architecture-workspace" data-inspector-open={inspectorOpen}>
      <aside className="architecture-rail">
        <header className="architecture-rail-header">
          <span>Architecture flow</span>
          <h1>{overview.provenance.projectName}</h1>
          <p>
            {overview.statistics.visibleNodes.toLocaleString()} of{" "}
            {overview.statistics.totalNodes.toLocaleString()} symbols visible
          </p>
        </header>
        <nav aria-label="Architecture subsystems">
          {overview.sections.map((section) => (
            <button
              key={section.id}
              type="button"
              aria-current={
                selection?.kind === "section" && selection.id === section.id
                  ? "page"
                  : undefined
              }
              data-empty={section.nodeCount === 0 || undefined}
              onClick={() => select({ kind: "section", id: section.id })}
            >
              <BoxIcon aria-hidden="true" />
              <span>
                <strong>{section.name}</strong>
                <small>
                  {section.nodeCount.toLocaleString()} visible ·{" "}
                  {section.totalNodeCount.toLocaleString()} total
                </small>
              </span>
              <i>{section.incomingCalls + section.outgoingCalls}</i>
            </button>
          ))}
        </nav>
        <footer className="architecture-scope-totals">
          <span><strong>{overview.statistics.totalNodes.toLocaleString()}</strong>symbols</span>
          <span><strong>{overview.statistics.totalCalls.toLocaleString()}</strong>calls</span>
          <span><strong>{overview.statistics.communities.toLocaleString()}</strong>groups</span>
        </footer>
      </aside>

      <main className="architecture-stage">
        <header className="architecture-command-bar">
          <div className="architecture-search">
            <SearchIcon aria-hidden="true" />
            <input
              type="search"
              value={searchQuery}
              placeholder="Search the complete architecture"
              aria-label="Search the complete architecture"
              onChange={(event) => updateSearch(event.target.value)}
            />
            {searchQuery && (
              <span role="status">{searchPage?.total.toLocaleString() ?? "…"} matches</span>
            )}
            {searchQuery && searchPage && (
              <div className="architecture-search-popover" role="listbox">
                {searchPage.items.length > 0 ? searchPage.items.map((result) => (
                  <button
                    key={result.id}
                    type="button"
                    role="option"
                    aria-selected="false"
                    onClick={() => {
                      if (result.routeId) select({ kind: "route", id: result.routeId });
                      else if (result.sectionId) {
                        select({ kind: "section", id: result.sectionId });
                      }
                      if (result.sourceFile) host.openSource(result.sourceFile);
                      setSearchQuery("");
                    }}
                  >
                    <strong>{result.label}</strong>
                    <small>{result.kind} · {result.detail}</small>
                  </button>
                )) : <p>No matches for “{searchQuery}”</p>}
              </div>
            )}
          </div>
          <div className="architecture-scope-switch" aria-label="Architecture source scope">
            <button
              type="button"
              aria-pressed={overview.scope === "production"}
              onClick={() => host.setFilters("production", overview.evidence)}
            >
              Production
            </button>
            <button
              type="button"
              aria-pressed={overview.scope === "all"}
              onClick={() => host.setFilters("all", overview.evidence)}
            >
              All code
            </button>
          </div>
          <label className="architecture-evidence-filter">
            <SlidersHorizontalIcon aria-hidden="true" />
            <span className="sr-only">Evidence</span>
            <select
              value={overview.evidence}
              aria-label="Filter architecture evidence"
              onChange={(event) =>
                host.setFilters(overview.scope, event.target.value as ArchitectureEvidence)
              }
            >
              <option value="all">All evidence</option>
              <option value="extracted">Extracted</option>
              <option value="inferred">Inferred</option>
              <option value="ambiguous">Ambiguous</option>
            </select>
          </label>
          <button
            className="architecture-inspector-toggle"
            type="button"
            aria-label={inspectorOpen ? "Hide architecture details" : "Show architecture details"}
            aria-pressed={inspectorOpen}
            title={inspectorOpen ? "Hide details" : "Show details"}
            onClick={() => setInspectorOpen((value) => !value)}
          >
            {inspectorOpen
              ? <PanelRightCloseIcon aria-hidden="true" />
              : <PanelRightOpenIcon aria-hidden="true" />}
          </button>
        </header>

        <div className="architecture-context-strip">
          <strong>
            {overview.scope === "production" ? "Production" : "All code"} ·{" "}
            {overview.statistics.visibleNodes.toLocaleString()} of{" "}
            {overview.statistics.totalNodes.toLocaleString()} symbols
          </strong>
          <span>
            {overview.statistics.visibleCalls.toLocaleString()} visible calls ·{" "}
            {overview.routes.length.toLocaleString()} subsystem routes
          </span>
          {loadingMessage && <span role="status">{loadingMessage}</span>}
          {overview.coverage.unassigned > 0 && (
            <span className="architecture-coverage-warning" role="status">
              <AlertTriangleIcon aria-hidden="true" />
              {overview.coverage.unassigned.toLocaleString()} unassigned calls disclosed
            </span>
          )}
        </div>

        <ArchitectureMap
          overview={overview}
          selection={selection}
          onSelect={select}
        />
      </main>

      {inspectorOpen && (
        <aside className="architecture-inspector" aria-label="Architecture selection details">
          {selectedSection ? (
            <SectionInspector
              section={selectedSection}
              page={displayedSectionPage}
              tab={sectionTab}
              query={detailQuery}
              onTab={(tab) => {
                setSectionTab(tab);
              }}
              onQuery={setDetailQuery}
              onPage={(page) =>
                host.requestSection(selectedSection.id, sectionTab, page, detailQuery)
              }
              onOpenSource={host.openSource}
            />
          ) : selectedRoute ? (
            <RouteInspector
              route={selectedRoute}
              page={displayedRoutePage}
              query={detailQuery}
              sectionName={(id) =>
                overview.sections.find((section) => section.id === id)?.name ?? id
              }
              onQuery={setDetailQuery}
              onPage={(page) => host.requestRoute(selectedRoute.id, page, detailQuery)}
              onOpenSource={host.openSource}
            />
          ) : (
            <div className="architecture-inspector-empty">
              Select a subsystem or directed route to inspect its complete evidence.
            </div>
          )}
        </aside>
      )}
    </div>
  );
}

function SectionInspector({
  section,
  page,
  tab,
  query,
  onTab,
  onQuery,
  onPage,
  onOpenSource
}: {
  section: ArchitectureOverview["sections"][number];
  page: ArchitectureSectionPage | undefined;
  tab: "symbols" | "calls";
  query: string;
  onTab(tab: "symbols" | "calls"): void;
  onQuery(query: string): void;
  onPage(page: number): void;
  onOpenSource(file: string): void;
}) {
  return (
    <>
      <header className="architecture-inspector-header">
        <span>Subsystem</span>
        <h2>{section.name}</h2>
        <div className="architecture-route-metrics">
          <span><ArrowDownLeftIcon aria-hidden="true" /> {section.incomingCalls} incoming</span>
          <span><ArrowUpRightIcon aria-hidden="true" /> {section.outgoingCalls} outgoing</span>
        </div>
      </header>
      <div className="architecture-inspector-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "symbols"}
          onClick={() => onTab("symbols")}
        >
          Symbols
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "calls"}
          onClick={() => onTab("calls")}
        >
          Internal calls
        </button>
      </div>
      <DetailFilter value={query} onChange={onQuery} />
      <div className="architecture-inspector-scroll">
        {!page ? <LoadingRows /> : page.kind === "symbols" && page.items.length > 0 ? (
          <div className="architecture-symbol-list">
            {page.items.map((symbol) => (
              <article key={symbol.id}>
                <FileCodeIcon aria-hidden="true" />
                <div>
                  <strong title={symbol.label}>{symbol.label}</strong>
                  <small>{symbol.kind || "symbol"} · {symbol.scope}</small>
                  {symbol.sourceFile ? (
                    <button type="button" onClick={() => onOpenSource(symbol.sourceFile!)}>
                      {symbol.sourceFile}
                    </button>
                  ) : <span>Source not recorded</span>}
                </div>
              </article>
            ))}
          </div>
        ) : page.kind === "calls" ? (
          <CallList calls={page.items} onOpenSource={onOpenSource} />
        ) : (
          <div className="architecture-inspector-empty">
            No symbols match this filter.
          </div>
        )}
      </div>
      {page && (
        <Pagination
          {...page}
          label={page.kind === "symbols" ? "symbols" : "calls"}
          onPageChange={onPage}
        />
      )}
    </>
  );
}

function RouteInspector({
  route,
  page,
  query,
  sectionName,
  onQuery,
  onPage,
  onOpenSource
}: {
  route: ArchitectureOverview["routes"][number];
  page: ArchitectureRoutePage | undefined;
  query: string;
  sectionName(id: string): string;
  onQuery(query: string): void;
  onPage(page: number): void;
  onOpenSource(file: string): void;
}) {
  return (
    <>
      <header className="architecture-inspector-header">
        <span>Directed route</span>
        <h2>{sectionName(route.sourceSection)} → {sectionName(route.targetSection)}</h2>
        <p>{route.calls.toLocaleString()} complete cross-subsystem calls</p>
        <div className="architecture-evidence-counts">
          <span>{route.extracted} extracted</span>
          <span>{route.inferred} inferred</span>
          <span>{route.ambiguous} ambiguous</span>
        </div>
      </header>
      <DetailFilter value={query} onChange={onQuery} />
      <div className="architecture-inspector-scroll">
        {page ? <CallList calls={page.items} onOpenSource={onOpenSource} /> : <LoadingRows />}
      </div>
      {page && <Pagination {...page} label="calls" onPageChange={onPage} />}
    </>
  );
}

function CallList({
  calls,
  onOpenSource
}: {
  calls: readonly {
    id: string;
    sourceLabel: string;
    targetLabel: string;
    sourceFile: string | null;
    targetFile: string | null;
    relation: string;
    confidence: string;
  }[];
  onOpenSource(file: string): void;
}) {
  return calls.length > 0 ? (
    <div className="architecture-call-list">
      {calls.map((call) => (
        <article key={call.id}>
          <div>
            <strong title={call.sourceLabel}>{call.sourceLabel}</strong>
            <span>→</span>
            <strong title={call.targetLabel}>{call.targetLabel}</strong>
          </div>
          <small>{call.relation} · {call.confidence}</small>
          <div>
            {call.sourceFile && (
              <button type="button" onClick={() => onOpenSource(call.sourceFile!)}>
                caller source
              </button>
            )}
            {call.targetFile && (
              <button type="button" onClick={() => onOpenSource(call.targetFile!)}>
                callee source
              </button>
            )}
          </div>
        </article>
      ))}
    </div>
  ) : (
    <div className="architecture-inspector-empty">No calls match this filter.</div>
  );
}

function DetailFilter({
  value,
  onChange
}: {
  value: string;
  onChange(value: string): void;
}) {
  return (
    <label className="architecture-detail-filter">
      <SearchIcon aria-hidden="true" />
      <input
        type="search"
        value={value}
        placeholder="Filter this selection"
        aria-label="Filter architecture selection"
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function LoadingRows() {
  return (
    <div
      className="architecture-loading-rows"
      role="status"
      aria-label="Loading architecture details"
    >
      <i /><i /><i /><i />
    </div>
  );
}
