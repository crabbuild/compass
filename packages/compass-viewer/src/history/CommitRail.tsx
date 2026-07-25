import { useMemo, useState } from "react";
import { AlertCircleIcon, CheckCircle2Icon, CircleDashedIcon, LoaderCircleIcon } from "lucide-react";
import type { HistoryEntry } from "../contracts/history";

const ROW_HEIGHT = 58;

export function CommitRail({
  entries,
  selected,
  onSelect
}: {
  entries: HistoryEntry[];
  selected: string;
  onSelect(commit: string): void;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const height = 520;
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 5);
  const end = Math.min(entries.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + 5);
  const visible = useMemo(() => entries.slice(start, end), [end, entries, start]);
  return (
    <div
      role="listbox"
      aria-label="Git commit timeline"
      tabIndex={0}
      className="history-rail"
      style={{ height }}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      onKeyDown={(event) => {
        const index = entries.findIndex((entry) => entry.commit === selected);
        const next = entries[index + 1];
        const previous = entries[index - 1];
        if (event.key === "ArrowDown" && next) onSelect(next.commit);
        if (event.key === "ArrowUp" && previous) onSelect(previous.commit);
      }}
    >
      <div style={{ height: entries.length * ROW_HEIGHT, position: "relative" }}>
        {visible.map((entry, offset) => (
          <button
            key={entry.commit}
            type="button"
            role="option"
            aria-selected={entry.commit === selected}
            className="history-commit"
            style={{ top: (start + offset) * ROW_HEIGHT, height: ROW_HEIGHT }}
            onClick={() => onSelect(entry.commit)}
          >
            <StateIcon state={entry.graphState} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm">{entry.subject || "(no subject)"}</span>
              <span className="block truncate font-mono text-xs text-muted-foreground">
                {entry.commit.slice(0, 9)} · {entry.authorName}
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function StateIcon({ state }: { state: HistoryEntry["graphState"] }) {
  if (state === "graph_available") return <CheckCircle2Icon aria-label="Graph available" className="text-emerald-500" />;
  if (state === "building") return <LoaderCircleIcon aria-label="Building" className="animate-spin text-blue-500" />;
  if (state === "failed") return <AlertCircleIcon aria-label="Failed" className="text-destructive" />;
  return <CircleDashedIcon aria-label="Not materialized" className="text-muted-foreground" />;
}
