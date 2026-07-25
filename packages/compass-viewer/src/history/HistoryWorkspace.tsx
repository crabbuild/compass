import { useMemo, useState } from "react";
import { HistoryIcon, SearchIcon } from "lucide-react";
import { CompassGraph } from "../graph/CompassGraph";
import { Input } from "../components/ui/input";
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
  buildState?: HistoryBuildState | undefined;
  operationError?: HistoryOperationError | undefined;
  onSelectCommit(commit: string): void;
  host: HistoryHost;
}) {
  const [query, setQuery] = useState("");
  const entries = useMemo(() => {
    const normalizedQuery = query.toLocaleLowerCase();
    return timeline.entries.filter((entry) => !normalizedQuery
      || entry.commit.includes(normalizedQuery)
      || entry.subject.toLocaleLowerCase().includes(normalizedQuery)
      || entry.authorName.toLocaleLowerCase().includes(normalizedQuery)
      || entry.graphState.includes(normalizedQuery));
  }, [query, timeline.entries]);
  const selected = timeline.entries.find((entry) => entry.commit === selectedCommit)
    ?? entries[0];
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
        <header className="border-b p-3">
          <div className="mb-2 flex items-center gap-2">
            <HistoryIcon />
            <div>
              <h1 className="text-sm font-semibold">Codebase evolution</h1>
              <p className="text-xs text-muted-foreground">{timeline.entries.length} reachable commits</p>
            </div>
          </div>
          <div className="relative">
            <SearchIcon className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="pl-8"
              value={query}
              placeholder="Search commits and states"
              aria-label="Search commit history"
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
        </header>
        <CommitRail
          entries={entries}
          selected={selected?.commit ?? ""}
          onSelect={onSelectCommit}
        />
      </aside>
      <main className="min-w-0 overflow-auto p-4">
        {selected && (
          <CommitDetails
            entry={selected}
            buildState={buildState}
            operationError={operationError}
            availableCommits={availableCommits}
            onLoad={() => host.loadRevision(selected.commit)}
            onBuild={() => host.buildRevision(selected.commit)}
            onCompare={(parent) => host.compare(selected.commit, parent)}
            onQuery={() => host.queryRevision(selected.commit)}
            changeCounts={changeCounts?.commit === selected.commit ? changeCounts : undefined}
          />
        )}
        <div className="mt-4 h-[calc(100vh-12rem)] min-h-96 overflow-hidden rounded-md border">
          {visibleGraph ? (
            <div className="flex h-full min-h-0 flex-col">
              <div
                className="border-b bg-muted/40 px-3 py-2 text-xs text-muted-foreground"
                role="status"
              >
                Viewing graph for <span className="font-mono text-foreground">
                  {selected?.commit.slice(0, 9)}
                </span>
              </div>
              <div className="min-h-0 flex-1">
                <CompassGraph
                  model={visibleGraph}
                  communityDetail={communityDetail}
                  communityLoading={communityLoading}
                  communityError={communityError}
                  onBackToOverview={onBackToOverview}
                  host={{
                    openSource(source) {
                      if (selected) host.openSource(selected.commit, source);
                    },
                    openCommunity(communityId) {
                      if (graphCommit) host.openCommunity(graphCommit, communityId);
                    }
                  }}
                />
              </div>
            </div>
          ) : (
            <div className="grid h-full place-items-center text-center text-sm text-muted-foreground">
              Select an available commit and choose Open graph. Missing commits are never built implicitly.
            </div>
          )}
        </div>
        {semanticDiff !== undefined && <SemanticFindings report={semanticDiff} />}
      </main>
    </div>
  );
}
