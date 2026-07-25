import { GitCompareIcon, HammerIcon, NetworkIcon, SearchIcon } from "lucide-react";
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
  buildState,
  operationError,
  availableCommits,
  onLoad,
  onBuild,
  onCompare,
  onQuery,
  changeCounts
}: {
  entry: HistoryEntry;
  buildState?: HistoryBuildState | undefined;
  operationError?: HistoryOperationError | undefined;
  availableCommits: ReadonlySet<string>;
  onLoad(): void;
  onBuild(): void;
  onCompare(parent: string): void;
  onQuery(): void;
  changeCounts?: HistoryChangeCounts | undefined;
}) {
  const buildBusy = buildState?.status === "requesting" || buildState?.status === "running";
  const buildLabel = buildState?.status === "requesting"
    ? "Choosing profile…"
    : buildState?.status === "running"
      ? "Building…"
      : buildState?.status === "failed"
        ? "Retry build"
        : "Build graph";
  const unavailableParents = entry.parents.filter((parent) => !availableCommits.has(parent));
  const comparisonUnavailable = !entry.presentationAvailable || unavailableParents.length > 0;
  return (
    <section className="rounded-md border bg-card p-4 text-card-foreground">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold">{entry.subject || "(no subject)"}</h2>
          <p className="mt-1 font-mono text-xs text-muted-foreground">{entry.commit}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            {entry.authorName} · {new Date(entry.authoredAtSeconds * 1000).toLocaleString()}
          </p>
        </div>
        <Badge variant={entry.graphState === "failed" ? "destructive" : "outline"}>
          {entry.graphState.replaceAll("_", " ")}
        </Badge>
      </div>
      <div className="mt-4 flex flex-wrap gap-2">
        {entry.presentationAvailable && (
          <Button size="sm" onClick={onLoad}><NetworkIcon /> Open graph</Button>
        )}
        {entry.presentationAvailable && (
          <Button size="sm" variant="outline" onClick={onQuery}>
            <SearchIcon /> Query this revision
          </Button>
        )}
        {!entry.presentationAvailable && (
          <Button size="sm" variant="outline" disabled={buildBusy} onClick={onBuild}>
            <HammerIcon /> {buildLabel}
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
        <p className="mt-2 text-xs text-muted-foreground">
          Comparison unavailable: {!entry.presentationAvailable
            ? "build this revision first."
            : "one or more parent graphs are not available."}
        </p>
      )}
      {buildState?.status === "failed" && (
        <p className="mt-3 text-sm text-destructive" role="alert">
          Build failed: {buildState.message}
        </p>
      )}
      {operationError && (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {operationError.operation}: {operationError.message}
        </p>
      )}
      {changeCounts && (
        <div className="mt-3 flex flex-wrap gap-2 text-xs" aria-label="Structural change counts">
          <Badge variant="secondary">
            nodes +{changeCounts.counts.nodes.added} −{changeCounts.counts.nodes.removed} ~{changeCounts.counts.nodes.changed}
          </Badge>
          <Badge variant="secondary">
            edges +{changeCounts.counts.edges.added} −{changeCounts.counts.edges.removed} ~{changeCounts.counts.edges.changed}
          </Badge>
          <Badge variant="secondary">
            hyperedges +{changeCounts.counts.hyperedges.added} −{changeCounts.counts.hyperedges.removed} ~{changeCounts.counts.hyperedges.changed}
          </Badge>
        </div>
      )}
    </section>
  );
}
