import { useEffect, useMemo, useRef, useState } from "react";
import { AlertCircleIcon, CheckCircle2Icon, CircleDashedIcon, LoaderCircleIcon } from "lucide-react";
import type { HistoryEntry } from "../contracts/history";

const ROW_HEIGHT = 68;

export function CommitRail({
  entries,
  selected,
  hasMore = false,
  loadingMore = false,
  onLoadMore,
  onSelect
}: {
  entries: HistoryEntry[];
  selected: string;
  hasMore?: boolean;
  loadingMore?: boolean;
  onLoadMore?(): void;
  onSelect(commit: string): void;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(520);
  const railRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const rail = railRef.current;
    if (!rail) return;
    const updateHeight = () => setHeight(Math.max(1, rail.clientHeight));
    updateHeight();
    const observer = new ResizeObserver(updateHeight);
    observer.observe(rail);
    return () => observer.disconnect();
  }, []);
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 5);
  const end = Math.min(entries.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + 5);
  const visible = useMemo(() => entries.slice(start, end), [end, entries, start]);
  return (
    <div
      ref={railRef}
      role="listbox"
      aria-label="Git commit timeline"
      tabIndex={0}
      className="history-rail"
      onScroll={(event) => {
        const rail = event.currentTarget;
        setScrollTop(rail.scrollTop);
        if (
          hasMore
          && !loadingMore
          && rail.scrollHeight - rail.scrollTop - rail.clientHeight <= ROW_HEIGHT * 5
        ) {
          onLoadMore?.();
        }
      }}
      onKeyDown={(event) => {
        const index = entries.findIndex((entry) => entry.commit === selected);
        const next = entries[index + 1];
        const previous = entries[index - 1];
        if (event.key === "ArrowDown" && next) {
          event.preventDefault();
          onSelect(next.commit);
        }
        if (event.key === "ArrowUp" && previous) {
          event.preventDefault();
          onSelect(previous.commit);
        }
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
            <span className="history-commit-row-copy">
              <span className="history-commit-subject">{entry.subject || "(no subject)"}</span>
              <span className="history-commit-byline">
                <code>{entry.commit.slice(0, 9)}</code>
                <span>{entry.authorName}</span>
                <time>{formatRelativeDate(entry.authoredAtSeconds)}</time>
              </span>
            </span>
            <span className="sr-only">{stateLabel(entry.graphState)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function StateIcon({ state }: { state: HistoryEntry["graphState"] }) {
  if (state === "graph_available") return <CheckCircle2Icon aria-hidden="true" data-state="available" />;
  if (state === "building") return <LoaderCircleIcon aria-hidden="true" data-state="building" className="animate-spin" />;
  if (state === "failed") return <AlertCircleIcon aria-hidden="true" data-state="failed" />;
  return <CircleDashedIcon aria-hidden="true" data-state="unavailable" />;
}

function stateLabel(state: HistoryEntry["graphState"]): string {
  return state.replaceAll("_", " ");
}

function formatRelativeDate(authoredAtSeconds: number): string {
  const timestamp = new Date(authoredAtSeconds * 1000);
  const elapsed = Date.now() - timestamp.getTime();
  const days = Math.floor(elapsed / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "1 day ago";
  if (days < 30) return `${days} days ago`;
  return timestamp.toLocaleDateString();
}
