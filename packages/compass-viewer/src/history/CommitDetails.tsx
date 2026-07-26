import { GitCompareIcon, SearchIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type {
  HistoryChangeCounts,
  HistoryEntry,
  HistoryOperationError
} from "../contracts/history";

export function CommitDetails({
  entry,
  operationError,
  availableCommits,
  onCompare,
  onQuery,
  changeCounts
}: {
  entry: HistoryEntry;
  operationError?: HistoryOperationError | undefined;
  availableCommits: ReadonlySet<string>;
  onCompare(parent: string): void;
  onQuery(): void;
  changeCounts?: HistoryChangeCounts | undefined;
}) {
  const unavailableParents = entry.parents.filter((parent) => !availableCommits.has(parent));
  const comparisonUnavailable = !entry.presentationAvailable || unavailableParents.length > 0;
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
        {entry.parents.map((parent, index) => (
          <Button
            key={parent}
            size="sm"
            variant="ghost"
            disabled={!entry.presentationAvailable || !availableCommits.has(parent)}
            title={!entry.presentationAvailable
              ? "Build this revision first"
              : !availableCommits.has(parent)
                ? "Parent graph is not available"
                : undefined}
            onClick={() => onCompare(parent)}
          >
            <GitCompareIcon /> Compare parent {index + 1}
          </Button>
        ))}
      </div>
      {comparisonUnavailable && entry.parents.length > 0 && (
        <p className="history-comparison-help">
          Comparison unavailable: {!entry.presentationAvailable
            ? "build this revision first."
            : "one or more parent graphs are not available."}
        </p>
      )}
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
              No structural changes from the first parent. Source or configuration changes may
              still exist; compare the revisions to inspect them.
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
