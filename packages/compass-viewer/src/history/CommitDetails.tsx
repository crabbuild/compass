import { GitCompareIcon, SearchIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type {
  HistoryBuildState,
  HistoryChangeCounts,
  HistoryEntry,
  HistoryOperationError
} from "../contracts/history";

export function CommitDetails({
  entry,
  operationError,
  comparisonEntries,
  comparisonCommit,
  selectedBuildState,
  comparisonBuildState,
  hasMore,
  onComparisonCommit,
  onCompare,
  onBuildRevision,
  onQuery,
  changeCounts
}: {
  entry: HistoryEntry;
  operationError?: HistoryOperationError | undefined;
  comparisonEntries: HistoryEntry[];
  comparisonCommit: string;
  selectedBuildState?: HistoryBuildState | undefined;
  comparisonBuildState?: HistoryBuildState | undefined;
  hasMore: boolean;
  onComparisonCommit(commit: string): void;
  onCompare(): void;
  onBuildRevision(commit: string): void;
  onQuery(): void;
  changeCounts?: HistoryChangeCounts | undefined;
}) {
  const comparisonEntry = comparisonEntries.find(
    (candidate) => candidate.commit === comparisonCommit
  );
  const buildTarget = !entry.presentationAvailable
    ? { entry, state: selectedBuildState, label: "selected" }
    : comparisonEntry && !comparisonEntry.presentationAvailable
      ? { entry: comparisonEntry, state: comparisonBuildState, label: "baseline" }
      : undefined;
  const buildInProgress = buildTarget?.state?.status === "requesting"
    || buildTarget?.state?.status === "running";
  const actionLabel = buildTarget
    ? buildInProgress
      ? `Building ${buildTarget.label} graph…`
      : buildTarget.state?.status === "failed"
        ? `Retry ${buildTarget.label} graph build`
        : `Build ${buildTarget.label} graph`
    : "Compare revisions";
  const canAct = comparisonEntry !== undefined && !buildInProgress;

  function performComparisonAction() {
    if (!comparisonEntry) return;
    if (buildTarget) {
      onBuildRevision(buildTarget.entry.commit);
      return;
    }
    onCompare();
  }

  return (
    <section className="history-commit-details" aria-labelledby="history-selected-title">
      <div className="history-commit-heading">
        <div className="history-commit-copy">
          <span className="history-eyebrow">Selected revision</span>
          <h2 id="history-selected-title">{entry.subject || "(no subject)"}</h2>
          <div className="history-commit-metadata">
            <code title={entry.commit}>{entry.commit.slice(0, 12)}</code>
            <span>{entry.authorName}</span>
            <time dateTime={new Date(entry.authoredAtSeconds * 1000).toISOString()}>
              {new Date(entry.authoredAtSeconds * 1000).toLocaleString()}
            </time>
          </div>
        </div>
        <Badge
          className="history-state-badge"
          variant={entry.graphState === "failed" ? "destructive" : "outline"}
        >
          {entry.graphState.replaceAll("_", " ")}
        </Badge>
      </div>
      <div className="history-commit-actions">
        {entry.presentationAvailable && (
          <Button size="sm" variant="outline" onClick={onQuery}>
            <SearchIcon /> Query this revision
          </Button>
        )}
        <div className="history-comparison-control">
          <label htmlFor="history-comparison-revision">Compare against</label>
          <select
            id="history-comparison-revision"
            aria-label="Comparison revision"
            value={comparisonCommit}
            disabled={comparisonEntries.length === 0 || buildInProgress}
            onChange={(event) => onComparisonCommit(event.target.value)}
          >
            {comparisonEntries.length === 0 && (
              <option value="">No other loaded revisions</option>
            )}
            {comparisonEntries.map((candidate) => (
              <option key={candidate.commit} value={candidate.commit}>
                {candidate.subject || "(no subject)"} · {candidate.commit.slice(0, 9)}
                {candidate.presentationAvailable ? "" : " · graph not built"}
              </option>
            ))}
          </select>
          <Button
            size="sm"
            variant="outline"
            disabled={!canAct}
            onClick={performComparisonAction}
          >
            <GitCompareIcon /> {actionLabel}
          </Button>
        </div>
      </div>
      {comparisonEntries.length === 0 ? (
        <p className="history-comparison-help">
          Load another revision to compare with this one.
        </p>
      ) : buildTarget?.state?.status === "failed" && buildTarget.label === "baseline" ? (
        <p className="history-inline-error" role="alert">
          Baseline graph build failed: {buildTarget.state.message}
        </p>
      ) : buildTarget?.state?.status === "failed" ? null : buildTarget ? (
        <p className="history-comparison-help">
          Build the {buildTarget.label} revision graph, then compare without changing your
          selection.
        </p>
      ) : hasMore ? (
        <p className="history-comparison-help">
          Choose any loaded revision, or load more commits to reach older history.
        </p>
      ) : null}
      {operationError && (
        <p className="history-inline-error" role="alert">
          {operationError.operation}: {operationError.message}
        </p>
      )}
      {changeCounts && (
        <>
          <div className="history-change-counts" aria-label="Structural change counts">
            <ChangeCount label="nodes" counts={changeCounts.counts.nodes} />
            <ChangeCount label="edges" counts={changeCounts.counts.edges} />
            <ChangeCount label="hyperedges" counts={changeCounts.counts.hyperedges} />
          </div>
          {isEmptyChangeCounts(changeCounts) && (
            <p className="history-change-counts-help">
              No structural changes from the comparison baseline. Source or configuration
              changes may still exist; compare the revisions to inspect them.
            </p>
          )}
        </>
      )}
    </section>
  );
}

function isEmptyChangeCounts(changeCounts: HistoryChangeCounts): boolean {
  return Object.values(changeCounts.counts).every(
    (counts) => counts.added === 0 && counts.removed === 0 && counts.changed === 0
  );
}

function ChangeCount({
  label,
  counts
}: {
  label: string;
  counts: { added: number; removed: number; changed: number };
}) {
  return (
    <span>
      <strong>{label}</strong> +{counts.added} −{counts.removed} ~{counts.changed}
    </span>
  );
}
