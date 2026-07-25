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
import { SemanticFindings } from "./SemanticFindings";

export type HistoryHost = {
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
  semanticDiff,
  changeCounts,
  graphCommit,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  selectedCommit,
  revisionLoadState,
  buildState,
  operationError,
  onSelectCommit,
  host
}: {
  timeline: HistoryTimeline;
  graph?: GraphViewModel | undefined;
  semanticDiff?: unknown;
  changeCounts?: HistoryChangeCounts | undefined;
  graphCommit?: string | undefined;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  selectedCommit: string;
  revisionLoadState: "idle" | "loading" | "ready";
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
  const visibleGraph = graph && graphCommit === selected?.commit ? graph : undefined;

  return (
    <div className="history-shell">
      <aside className="history-sidebar">
        <header className="history-sidebar-header">
          <div className="history-title">
            <HistoryIcon aria-hidden="true" />
            <div>
              <h1>Codebase evolution</h1>
              <p>{timeline.entries.length.toLocaleString()} reachable commits</p>
            </div>
          </div>
          <label className="history-search">
            <SearchIcon aria-hidden="true" />
            <span className="sr-only">Search commit history</span>
            <input
              type="search"
              value={query}
              placeholder="Search commits and graph states"
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
          onSelect={onSelectCommit}
        />
      </aside>

      <main className="history-content">
        {!timeline.historyEnabled ? (
          <WorkspaceState
            kind="unavailable"
            title="Revision graphs are not enabled"
            description="Enable a Compass history profile for this repository, then reload Codebase Evolution."
          />
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
            <div className="history-graph-frame">
              {visibleGraph ? (
                <div className="history-graph-ready">
                  <div className="history-graph-status" role="status">
                    Viewing graph for <span>{selected.commit.slice(0, 9)}</span>
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
            {semanticDiff !== undefined && <SemanticFindings report={semanticDiff} />}
          </>
        )}
      </main>
    </div>
  );
}
