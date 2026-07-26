import { useMemo, useState } from "react";
import { HistoryIcon, SearchIcon } from "lucide-react";
import { WorkspaceState } from "../components/workbench/WorkspaceState";
import { CompassGraph } from "../graph/CompassGraph";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import type {
  HistoryBuildState,
  HistoryChangeCounts,
  HistoryOperationError,
  HistoryTimeline
} from "../contracts/history";
import { CommitDetails } from "./CommitDetails";
import { CommitRail } from "./CommitRail";
import { ComparisonOverlay, type GraphComparison } from "./ComparisonOverlay";
import { SemanticFindings } from "./SemanticFindings";

export type HistoryHost = {
  enableHistory(): void;
  loadMore(): void;
  loadRevision(commit: string): void;
  buildRevision(commit: string): void;
  compare(commit: string, parent: string): void;
  queryRevision(commit: string): void;
  loadChangeCounts(commit: string): void;
  openSource(commit: string, source: SourceLocation): void;
  openCommunity(commit: string, communityId: number): void;
};

export function HistoryWorkspace({
  timeline,
  graph,
  comparison,
  semanticDiff,
  changeCounts,
  graphCommit,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  onExitComparison,
  selectedCommit,
  revisionLoadState,
  enableState,
  loadingMore = false,
  loadMoreError,
  buildState,
  operationError,
  onSelectCommit,
  host
}: {
  timeline: HistoryTimeline;
  graph?: GraphViewModel | undefined;
  comparison?: (GraphComparison & { parent: string }) | undefined;
  semanticDiff?: unknown;
  changeCounts?: HistoryChangeCounts | undefined;
  graphCommit?: string | undefined;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  onExitComparison?: (() => void) | undefined;
  selectedCommit: string;
  revisionLoadState: "idle" | "loading" | "ready";
  enableState?: HistoryBuildState | undefined;
  loadingMore?: boolean | undefined;
  loadMoreError?: string | undefined;
  buildState?: HistoryBuildState | undefined;
  operationError?: HistoryOperationError | undefined;
  onSelectCommit(commit: string): void;
  host: HistoryHost;
}) {
  const [query, setQuery] = useState("");
  const entries = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return timeline.entries.filter((entry) => !normalizedQuery
      || entry.commit.includes(normalizedQuery)
      || entry.subject.toLocaleLowerCase().includes(normalizedQuery)
      || entry.authorName.toLocaleLowerCase().includes(normalizedQuery)
      || entry.graphState.replaceAll("_", " ").includes(normalizedQuery));
  }, [query, timeline.entries]);
  const selected = timeline.entries.find((entry) => entry.commit === selectedCommit);
  const availableCommits = useMemo(
    () => new Set(timeline.entries
      .filter((entry) => entry.presentationAvailable)
      .map((entry) => entry.commit)),
    [timeline.entries]
  );
  const visibleGraph = comparison?.graph
    ?? (graph && graphCommit === selected?.commit ? graph : undefined);
  const loadedEntries = timeline.entries.length;
  const countLabel = timeline.hasMore
    ? `${loadedEntries.toLocaleString()} loaded commits`
    : `${(timeline.totalEntries ?? loadedEntries).toLocaleString()} reachable commits`;

  return (
    <div className="history-shell">
      <aside className="history-sidebar">
        <header className="history-sidebar-header">
          <div className="history-title">
            <HistoryIcon aria-hidden="true" />
            <div>
              <h1>Codebase evolution</h1>
              <p>{countLabel}</p>
            </div>
          </div>
          <label className="history-search">
            <SearchIcon aria-hidden="true" />
            <span className="sr-only">Search commit history</span>
            <input
              type="search"
              value={query}
              placeholder={timeline.hasMore
                ? "Search loaded commits and graph states"
                : "Search commits and graph states"}
              aria-label="Search commit history"
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <label className="history-mobile-select">
            <span>Select revision</span>
            <select
              value={selected?.commit ?? ""}
              aria-label="Select revision"
              onChange={(event) => onSelectCommit(event.target.value)}
            >
              {entries.map((entry) => (
                <option key={entry.commit} value={entry.commit}>
                  {entry.subject || "(no subject)"} · {entry.commit.slice(0, 9)}
                </option>
              ))}
            </select>
          </label>
        </header>
        <CommitRail
          entries={entries}
          selected={selected?.commit ?? ""}
          hasMore={timeline.hasMore ?? false}
          loadingMore={loadingMore}
          onLoadMore={host.loadMore}
          onSelect={onSelectCommit}
        />
        {timeline.hasMore && (
          <div className="history-pagination" role={loadingMore ? "status" : undefined}>
            {loadingMore ? (
              "Loading more commits…"
            ) : (
              <>
                {loadMoreError && <span role="alert">{loadMoreError}</span>}
                <button type="button" className="workbench-button" onClick={host.loadMore}>
                  {loadMoreError ? "Retry loading commits" : "Load more commits"}
                </button>
              </>
            )}
          </div>
        )}
      </aside>

      <main className="history-content">
        {!timeline.historyEnabled ? (
          enableState?.status === "requesting" ? (
            <WorkspaceState
              kind="running"
              title="Choosing a history profile"
              description="Select the profile Compass should use for future revision graphs."
            />
          ) : enableState?.status === "running" ? (
            <WorkspaceState
              kind="running"
              title="Enabling revision graphs"
              description="Compass is saving the repository history profile and installing its managed hook."
            />
          ) : enableState?.status === "failed" ? (
            <WorkspaceState
              kind="error"
              title="Revision graphs could not be enabled"
              description={enableState.message}
              action={{ label: "Retry enablement", onClick: host.enableHistory }}
            />
          ) : (
            <WorkspaceState
              kind="unavailable"
              title="Revision graphs are not enabled"
              description="Choose a local code-only profile or use the semantic provider detected by Compass."
              action={{ label: "Enable revision graphs", onClick: host.enableHistory }}
            />
          )
        ) : !selected ? (
          <WorkspaceState
            kind="empty"
            title={timeline.entries.length === 0 ? "No commits to show" : "No matching commits"}
            description={timeline.entries.length === 0
              ? "This repository has no reachable commits."
              : "Clear the history search to select a revision."}
            {...(query
              ? { action: { label: "Clear search", onClick: () => setQuery("") } }
              : {})}
          />
        ) : (
          <>
            <CommitDetails
              entry={selected}
              operationError={operationError?.operation === "Load graph" ? undefined : operationError}
              availableCommits={availableCommits}
              onCompare={(parent) => host.compare(selected.commit, parent)}
              onQuery={() => host.queryRevision(selected.commit)}
              changeCounts={changeCounts?.commit === selected.commit ? changeCounts : undefined}
            />
            {comparison && (
              <ComparisonOverlay
                comparison={comparison}
                commit={selected.commit}
                parent={comparison.parent}
                onExit={() => onExitComparison?.()}
              />
            )}
            {comparison && semanticDiff !== undefined && <SemanticFindings report={semanticDiff} />}
            <div className="history-graph-frame">
              {comparison && comparison.graph.nodes.length === 0
                && comparison.graph.edges.length === 0 ? (
                <WorkspaceState
                  kind="empty"
                  title="No graph delta to draw"
                  description="This comparison changes source or configuration without changing visible graph topology."
                />
              ) : visibleGraph ? (
                <div className="history-graph-ready">
                  <div className="history-graph-status" role="status">
                    {comparison ? "Viewing changed subgraph for " : "Viewing graph for "}
                    <span>{selected.commit.slice(0, 9)}</span>
                  </div>
                  <div className="history-graph-canvas">
                    <CompassGraph
                      model={visibleGraph}
                      communityDetail={communityDetail}
                      communityLoading={communityLoading}
                      communityError={communityError}
                      onBackToOverview={onBackToOverview}
                      host={{
                        openSource(source) {
                          host.openSource(selected.commit, source);
                        },
                        openCommunity(communityId) {
                          if (graphCommit) host.openCommunity(graphCommit, communityId);
                        }
                      }}
                    />
                  </div>
                </div>
              ) : buildState?.status === "requesting" ? (
                <WorkspaceState
                  kind="running"
                  title="Choosing a build profile"
                  description="Select how Compass should materialize this revision graph."
                />
              ) : buildState?.status === "running" ? (
                <WorkspaceState
                  kind="running"
                  title="Building revision graph"
                  description={`Compass is materializing ${selected.commit.slice(0, 9)}. You can cancel from the VS Code progress notification.`}
                />
              ) : buildState?.status === "failed" ? (
                <WorkspaceState
                  kind="error"
                  title="Revision build failed"
                  description={buildState.message}
                  action={{
                    label: "Retry build",
                    onClick: () => host.buildRevision(selected.commit)
                  }}
                />
              ) : operationError?.operation === "Load graph" ? (
                <WorkspaceState
                  kind="error"
                  title="Revision graph could not be opened"
                  description={operationError.message}
                  action={{
                    label: "Retry load",
                    onClick: () => host.loadRevision(selected.commit)
                  }}
                />
              ) : !selected.presentationAvailable ? (
                <WorkspaceState
                  kind="unavailable"
                  title="Graph not built for this revision"
                  description="Build this revision explicitly to inspect, compare, or query its code graph."
                  action={{
                    label: "Build graph",
                    onClick: () => host.buildRevision(selected.commit)
                  }}
                />
              ) : revisionLoadState === "loading" ? (
                <WorkspaceState
                  kind="running"
                  title={`Loading ${selected.subject || selected.commit.slice(0, 9)}`}
                  description="Compass is opening the stored graph for this revision."
                />
              ) : (
                <WorkspaceState
                  kind="unavailable"
                  title="Revision graph is ready to open"
                  description="Load the stored graph without rebuilding this revision."
                  action={{
                    label: "Open graph",
                    onClick: () => host.loadRevision(selected.commit)
                  }}
                />
              )}
            </div>
            {!comparison && semanticDiff !== undefined && <SemanticFindings report={semanticDiff} />}
          </>
        )}
      </main>
    </div>
  );
}
