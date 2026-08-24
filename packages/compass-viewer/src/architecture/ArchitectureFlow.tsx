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
  ArchitectureLens,
  ArchitectureOverview,
  ArchitectureRoutePage,
  ArchitectureScope,
  ArchitectureSearchPage,
  ArchitectureGroupPage
} from "../contracts/architecture";
import { Pagination } from "../components/workbench/Pagination";
import { ArchitectureMap, type ArchitectureSelection } from "./ArchitectureMap";

export type ArchitectureHost = {
  setFilters(scope: ArchitectureScope, evidence: ArchitectureEvidence, lens: ArchitectureLens): void;
  requestGroup(
    groupId: string,
    kind: "symbols" | "relationships",
    page: number,
    query: string
  ): void;
  requestRoute(routeId: string, page: number, query: string): void;
  search(query: string, page: number): void;
  openSource(file: string): void;
};

export function ArchitectureFlow({
  overview,
  groupPage,
  routePage,
  searchPage,
  loadingMessage,
  host
}: {
  overview: ArchitectureOverview;
  groupPage: ArchitectureGroupPage | undefined;
  routePage: ArchitectureRoutePage | undefined;
  searchPage: ArchitectureSearchPage | undefined;
  loadingMessage: string | undefined;
  host: ArchitectureHost;
}) {
  const firstGroup = overview.groups.find((section) => section.nodeCount > 0);
  const [selection, setSelection] = useState<ArchitectureSelection>(
    firstGroup ? { kind: "group", id: firstGroup.id } : undefined
  );
  const [groupTab, setSectionTab] = useState<"symbols" | "relationships">("symbols");
  const [detailQuery, setDetailQuery] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [directoryOpen, setDirectoryOpen] = useState(false);
  const [hiddenSelection, setHiddenSelection] = useState<{ id: string; name: string }>();
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const relationshipNoun = "relationships";

  useEffect(() => {
    const selectionExists = selection?.kind === "group"
      ? overview.groups.some((section) => section.id === selection.id)
        || hiddenSelection?.id === selection.id
      : selection?.kind === "route"
        ? overview.routes.some((route) => route.id === selection.id)
        : false;
    if (selection && !selectionExists) {
      setSelection(firstGroup ? { kind: "group", id: firstGroup.id } : undefined);
      return;
    }
    if (!selection && firstGroup) {
      setSelection({ kind: "group", id: firstGroup.id });
      return;
    }
    if (selection?.kind === "group") {
      host.requestGroup(selection.id, groupTab, 1, detailQuery);
    } else if (selection?.kind === "route") {
      host.requestRoute(selection.id, 1, detailQuery);
    }
  }, [detailQuery, firstGroup, hiddenSelection, host, overview.routes, overview.groups, groupTab, selection]);

  const selectedGroup = selection?.kind === "group"
    ? overview.groups.find((section) => section.id === selection.id)
      ?? (hiddenSelection?.id === selection.id ? {
        id: hiddenSelection.id,
        name: hiddenSelection.name,
        nodeCount: 0,
        totalNodeCount: 0,
        internalRelationshipCount: 0,
        incomingRelationships: 0,
        outgoingRelationships: 0,
        scopes: { production: 0, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 }
      } : undefined)
    : undefined;
  const selectedRoute = selection?.kind === "route"
    ? overview.routes.find((route) => route.id === selection.id)
    : undefined;
  const displayedGroupPage = groupPage && groupPage.groupId === selectedGroup?.id
    && groupPage.kind === groupTab ? groupPage : undefined;
  const displayedRoutePage = routePage?.routeId === selectedRoute?.id ? routePage : undefined;

  const select = (next: Exclude<ArchitectureSelection, undefined>) => {
    setHiddenSelection(undefined);
    setSelection(next);
    setDetailQuery("");
    if (next.kind === "group") {
      setSectionTab("symbols");
    }
  };

  const updateSearch = (value: string) => {
    setDirectoryOpen(false);
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
          {overview.groups.map((section) => (
            <button
              key={section.id}
              type="button"
              aria-current={
                selection?.kind === "group" && selection.id === section.id
                  ? "page"
                  : undefined
              }
              data-empty={section.nodeCount === 0 || undefined}
              onClick={() => select({ kind: "group", id: section.id })}
            >
              <BoxIcon aria-hidden="true" />
              <span>
                <strong>{section.name}</strong>
                <small>
                  {section.nodeCount.toLocaleString()} visible ·{" "}
                  {section.totalNodeCount.toLocaleString()} total
                </small>
              </span>
              <i>{section.incomingRelationships + section.outgoingRelationships}</i>
            </button>
          ))}
        </nav>
        <footer className="architecture-scope-totals">
          <span><strong>{overview.statistics.totalNodes.toLocaleString()}</strong>symbols</span>
          <span><strong>{overview.statistics.totalRelationships.toLocaleString()}</strong>{relationshipNoun}</span>
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
            {(searchQuery || directoryOpen) && (
              <span role="status">{searchPage?.total.toLocaleString() ?? "…"} matches</span>
            )}
            {(searchQuery || directoryOpen) && searchPage && (
              <div className="architecture-search-popover" role="listbox">
                {searchPage.items.length > 0 ? searchPage.items.map((result) => (
                  <button
                    key={result.id}
                    type="button"
                    role="option"
                    aria-selected="false"
                    onClick={() => {
                      if (result.routeId) select({ kind: "route", id: result.routeId });
                      else if (result.groupId) {
                        if (!overview.groups.some((section) => section.id === result.groupId)) {
                          setHiddenSelection({ id: result.groupId, name: result.label });
                          setSelection({ kind: "group", id: result.groupId });
                          setDetailQuery("");
                          setSectionTab("symbols");
                        } else {
                          select({ kind: "group", id: result.groupId });
                        }
                      }
                      if (result.sourceFile) host.openSource(result.sourceFile);
                      setSearchQuery("");
                      setDirectoryOpen(false);
                    }}
                  >
                    <strong>{result.label}</strong>
                    <small>{result.kind} · {result.detail}</small>
                  </button>
                )) : <p>No matches for “{searchQuery}”</p>}
                {searchPage.pageCount > 1 && (
                  <div className="architecture-directory-pagination" role="group" aria-label="Group directory pages">
                    <button
                      type="button"
                      disabled={searchPage.page <= 1}
                      onClick={() => host.search(searchQuery, searchPage.page - 1)}
                    >Previous</button>
                    <span>Page {searchPage.page} of {searchPage.pageCount}</span>
                    <button
                      type="button"
                      disabled={searchPage.page >= searchPage.pageCount}
                      onClick={() => host.search(searchQuery, searchPage.page + 1)}
                    >Next</button>
                  </div>
                )}
              </div>
            )}
          </div>
          <div className="architecture-scope-switch" aria-label="Architecture source scope">
            <button
              type="button"
              aria-pressed={overview.scope === "production"}
              onClick={() => host.setFilters("production", overview.evidence, overview.lens)}
            >
              Production
            </button>
            <button
              type="button"
              aria-pressed={overview.scope === "all"}
              onClick={() => host.setFilters("all", overview.evidence, overview.lens)}
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
                host.setFilters(
                  overview.scope,
                  event.target.value as ArchitectureEvidence,
                  overview.lens
                )
              }
            >
              <option value="all">All evidence</option>
              <option value="extracted">Extracted</option>
              <option value="inferred">Inferred</option>
              <option value="ambiguous">Ambiguous</option>
            </select>
          </label>
          <label className="architecture-evidence-filter">
            <span className="sr-only">Relationship lens</span>
            <select
              value={overview.lens}
              aria-label="Architecture relationship lens"
              onChange={(event) => host.setFilters(
                overview.scope,
                overview.evidence,
                event.target.value as ArchitectureLens
              )}
            >
              <option value="architecture">Architecture</option>
              <option value="execution">Execution</option>
              <option value="dependency">Dependency</option>
              <option value="type">Type</option>
              <option value="structure">Structure</option>
              <option value="all">All typed</option>
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
            {overview.statistics.visibleRelationships.toLocaleString()} visible {relationshipNoun} ·{" "}
            {overview.routes.length.toLocaleString()} subsystem routes
          </span>
          {loadingMessage && <span role="status">{loadingMessage}</span>}
          {overview.coverage.unassigned > 0 && (
            <span className="architecture-coverage-warning" role="status">
              <AlertTriangleIcon aria-hidden="true" />
              {overview.coverage.unassigned.toLocaleString()} unassigned relationships disclosed
            </span>
          )}
          <span className="architecture-quality-status" data-status={overview.quality.status}>
            Architecture quality: {overview.quality.status}
          </span>
          {overview.omissions.omittedGroups > 0 && (
            <span role="status">
              Overview shows {overview.omissions.shownGroups} of {overview.omissions.totalGroups} groups;{" "}
              {overview.omissions.omittedGroups} remain searchable.{" "}
              <button type="button" onClick={() => {
                setDirectoryOpen(true);
                host.search("", 1);
              }}>Browse all groups</button>
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
          {selectedGroup ? (
            <>
              {hiddenSelection?.id === selectedGroup.id && (
                <p className="architecture-omitted-selection" role="status">
                  This group is outside the bounded map overview.
                </p>
              )}
              <GroupInspector
                section={selectedGroup}
                page={displayedGroupPage}
                tab={groupTab}
                query={detailQuery}
                onTab={(tab) => {
                  setSectionTab(tab);
                }}
                onQuery={setDetailQuery}
                onPage={(page) =>
                  host.requestGroup(selectedGroup.id, groupTab, page, detailQuery)
                }
                onOpenSource={host.openSource}
                relationshipNoun={relationshipNoun}
              />
            </>
          ) : selectedRoute ? (
            <RouteInspector
              route={selectedRoute}
              page={displayedRoutePage}
              query={detailQuery}
              groupName={(id) =>
                overview.groups.find((section) => section.id === id)?.name ?? id
              }
              onQuery={setDetailQuery}
              onPage={(page) => host.requestRoute(selectedRoute.id, page, detailQuery)}
              onOpenSource={host.openSource}
              relationshipNoun={relationshipNoun}
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

function GroupInspector({
  section,
  page,
  tab,
  query,
  onTab,
  onQuery,
  onPage,
  onOpenSource,
  relationshipNoun
}: {
  section: ArchitectureOverview["groups"][number];
  page: ArchitectureGroupPage | undefined;
  tab: "symbols" | "relationships";
  query: string;
  onTab(tab: "symbols" | "relationships"): void;
  onQuery(query: string): void;
  onPage(page: number): void;
  onOpenSource(file: string): void;
  relationshipNoun: string;
}) {
  return (
    <>
      <header className="architecture-inspector-header">
        <span>Subsystem</span>
        <h2>{section.name}</h2>
        <div className="architecture-route-metrics">
          <span><ArrowDownLeftIcon aria-hidden="true" /> {section.incomingRelationships} incoming</span>
          <span><ArrowUpRightIcon aria-hidden="true" /> {section.outgoingRelationships} outgoing</span>
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
          aria-selected={tab === "relationships"}
          onClick={() => onTab("relationships")}
        >
          Internal {relationshipNoun}
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
        ) : page.kind === "relationships" ? (
          <RelationshipList relationships={page.items} onOpenSource={onOpenSource} relationshipNoun={relationshipNoun} />
        ) : (
          <div className="architecture-inspector-empty">
            No symbols match this filter.
          </div>
        )}
      </div>
      {page && (
        <Pagination
          {...page}
          label={page.kind === "symbols" ? "symbols" : relationshipNoun}
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
  groupName,
  onQuery,
  onPage,
  onOpenSource,
  relationshipNoun
}: {
  route: ArchitectureOverview["routes"][number];
  page: ArchitectureRoutePage | undefined;
  query: string;
  groupName(id: string): string;
  onQuery(query: string): void;
  onPage(page: number): void;
  onOpenSource(file: string): void;
  relationshipNoun: string;
}) {
  return (
    <>
      <header className="architecture-inspector-header">
        <span>Directed route</span>
        <h2>{groupName(route.sourceGroup)} → {groupName(route.targetGroup)}</h2>
        <p>{route.relationships.toLocaleString()} complete cross-subsystem {relationshipNoun}</p>
        <div className="architecture-evidence-counts">
          <span>{route.extracted} extracted</span>
          <span>{route.inferred} inferred</span>
          <span>{route.ambiguous} ambiguous</span>
        </div>
      </header>
      <DetailFilter value={query} onChange={onQuery} />
      <div className="architecture-inspector-scroll">
        {page ? <RelationshipList relationships={page.items} onOpenSource={onOpenSource} relationshipNoun={relationshipNoun} /> : <LoadingRows />}
      </div>
      {page && <Pagination {...page} label={relationshipNoun} onPageChange={onPage} />}
    </>
  );
}

function RelationshipList({
  relationships,
  onOpenSource,
  relationshipNoun
}: {
  relationships: readonly {
    id: string;
    sourceLabel: string;
    targetLabel: string;
    sourceFile: string | null;
    targetFile: string | null;
    relation: string;
    relationClass: string;
    confidence: string;
  }[];
  onOpenSource(file: string): void;
  relationshipNoun: string;
}) {
  return relationships.length > 0 ? (
    <div className="architecture-relationship-list">
      {relationships.map((relationship) => (
        <article key={relationship.id}>
          <div>
            <strong title={relationship.sourceLabel}>{relationship.sourceLabel}</strong>
            <span>→</span>
            <strong title={relationship.targetLabel}>{relationship.targetLabel}</strong>
          </div>
          <small>{relationship.relation} · {relationship.relationClass} · {relationship.confidence}</small>
          <div>
            {relationship.sourceFile && (
              <button type="button" onClick={() => onOpenSource(relationship.sourceFile!)}>
                source
              </button>
            )}
            {relationship.targetFile && (
              <button type="button" onClick={() => onOpenSource(relationship.targetFile!)}>
                target
              </button>
            )}
          </div>
        </article>
      ))}
    </div>
  ) : (
    <div className="architecture-inspector-empty">No {relationshipNoun} match this filter.</div>
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
