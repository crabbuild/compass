import { GitCompareIcon, HammerIcon, NetworkIcon, SearchIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type { HistoryEntry } from "../contracts/history";
import type { HistoryChangeCounts } from "../contracts/history";

export function CommitDetails({
  entry,
  building,
  onLoad,
  onBuild,
  onCompare,
  onQuery,
  changeCounts
}: {
  entry: HistoryEntry;
  building: boolean;
  onLoad(): void;
  onBuild(): void;
  onCompare(parent: string): void;
  onQuery(): void;
  changeCounts?: HistoryChangeCounts | undefined;
}) {
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
          <Button size="sm" variant="outline" disabled={building} onClick={onBuild}>
            <HammerIcon /> {building ? "Building…" : "Build graph"}
          </Button>
        )}
        {entry.parents.map((parent, index) => (
          <Button key={parent} size="sm" variant="ghost" onClick={() => onCompare(parent)}>
            <GitCompareIcon /> Compare parent {index + 1}
          </Button>
        ))}
      </div>
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
