import { useEffect, useMemo, useReducer, useRef } from "react";
import { HistoryIcon, SearchIcon } from "lucide-react";
import { CompassGraph } from "../graph/CompassGraph";
import { Input } from "../components/ui/input";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import type { HistoryChangeCounts, HistoryTimeline } from "../contracts/history";
import { CommitDetails } from "./CommitDetails";
import { CommitRail } from "./CommitRail";
import { historyReducer, initialHistoryState } from "./state";
import { SemanticFindings } from "./SemanticFindings";

export type HistoryHost = {
  loadRevision(commit: string): void;
  buildRevision(commit: string): void;
  compare(commit: string, parent: string): void;
  queryRevision(commit: string): void;
  loadChangeCounts(commit: string): void;
  openSource(source: SourceLocation): void;
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
  host: HistoryHost;
}) {
  const [state, dispatch] = useReducer(historyReducer, timeline, initialHistoryState);
  const entries = useMemo(() => {
    const query = state.query.toLocaleLowerCase();
    return timeline.entries.filter((entry) => !query
      || entry.commit.includes(query)
      || entry.subject.toLocaleLowerCase().includes(query)
      || entry.authorName.toLocaleLowerCase().includes(query)
      || entry.graphState.includes(query));
  }, [state.query, timeline.entries]);
  const selected = timeline.entries.find((entry) => entry.commit === state.selected)
    ?? entries[0];
  const requestedCounts = useRef(new Set<string>());
  useEffect(() => {
    if (!selected?.presentationAvailable || selected.parents.length === 0
      || requestedCounts.current.has(selected.commit)) return;
    requestedCounts.current.add(selected.commit);
    host.loadChangeCounts(selected.commit);
  }, [host, selected]);
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
              value={state.query}
              placeholder="Search commits and states"
              aria-label="Search commit history"
              onChange={(event) => dispatch({ type: "search", query: event.target.value })}
            />
          </div>
        </header>
        <CommitRail
          entries={entries}
          selected={selected?.commit ?? ""}
          onSelect={(commit) => dispatch({ type: "select", commit })}
        />
      </aside>
      <main className="min-w-0 overflow-auto p-4">
        {selected && (
          <CommitDetails
            entry={selected}
            building={state.building.has(selected.commit)}
            onLoad={() => host.loadRevision(selected.commit)}
            onBuild={() => {
              dispatch({ type: "building", commit: selected.commit, building: true });
              host.buildRevision(selected.commit);
            }}
            onCompare={(parent) => host.compare(selected.commit, parent)}
            onQuery={() => host.queryRevision(selected.commit)}
            changeCounts={changeCounts?.commit === selected.commit ? changeCounts : undefined}
          />
        )}
        <div className="mt-4 h-[calc(100vh-12rem)] min-h-96 overflow-hidden rounded-md border">
          {graph ? (
            <CompassGraph
              model={graph}
              communityDetail={communityDetail}
              communityLoading={communityLoading}
              communityError={communityError}
              onBackToOverview={onBackToOverview}
              host={{
                openSource: host.openSource,
                openCommunity(communityId) {
                  if (graphCommit) host.openCommunity(graphCommit, communityId);
                }
              }}
            />
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
