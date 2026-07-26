import { parsePatchFiles, type FileDiffMetadata } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import {
  Component,
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
  type ErrorInfo,
  type ReactNode
} from "react";

export type SourceChange = {
  old_path?: string;
  new_path?: string;
  status?: string;
  patch?: string;
};

type DiffStyle = "split" | "unified";

const DIFF_THEME_CSS = `:host {
  --diffs-font-family: var(--compass-font-mono);
  --diffs-font-size: 11px;
  --diffs-line-height: 19px;
  --diffs-gap-inline: 6px;
  --diffs-min-number-column-width: 3ch;
}`;
const DIFF_LINE_HEIGHT = 19;
const DIFF_HUNK_SEPARATOR_HEIGHT = 32;
const DIFF_VERTICAL_PADDING = 16;

export function SourceChanges({ changes }: { changes: SourceChange[] }) {
  const [preferredStyle, setPreferredStyle] = useState<DiffStyle>("split");
  const [wrap, setWrap] = useState(false);
  const [narrow, setNarrow] = useState(
    () => typeof matchMedia === "function" && matchMedia("(max-width: 760px)").matches
  );
  const [openFiles, setOpenFiles] = useState<ReadonlySet<number>>(
    () => new Set(changes.length ? [0] : [])
  );

  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const media = matchMedia("(max-width: 760px)");
    const update = () => setNarrow(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    setOpenFiles((current) => new Set(
      [...current].filter((index) => index < changes.length)
    ));
  }, [changes.length]);

  const style: DiffStyle = narrow ? "unified" : preferredStyle;
  const allOpen = changes.length > 0 && openFiles.size === changes.length;

  return (
    <>
      <div className="history-diff-toolbar" aria-label="Source diff controls">
        <div className="history-diff-layout" role="group" aria-label="Diff layout">
          <button
            type="button"
            aria-pressed={style === "split"}
            disabled={narrow}
            title={narrow ? "Split view needs a wider panel" : "Show side-by-side diff"}
            onClick={() => setPreferredStyle("split")}
          >
            Split
          </button>
          <button
            type="button"
            aria-pressed={style === "unified"}
            onClick={() => setPreferredStyle("unified")}
          >
            Unified
          </button>
        </div>
        <label className="history-diff-wrap">
          <input
            type="checkbox"
            checked={wrap}
            onChange={(event) => setWrap(event.target.checked)}
          />
          <span>Wrap lines</span>
        </label>
        <button
          type="button"
          className="history-diff-expand"
          onClick={() => setOpenFiles(
            allOpen ? new Set() : new Set(changes.map((_, index) => index))
          )}
        >
          {allOpen ? "Collapse all" : "Expand all"}
        </button>
      </div>
      <div className="history-source-changes">
        {changes.map((change, index) => {
          const open = openFiles.has(index);
          return (
            <details
              key={`${change.new_path ?? change.old_path}-${index}`}
              open={open}
              onToggle={(event) => {
                const nextOpen = event.currentTarget.open;
                setOpenFiles((current) => {
                  const next = new Set(current);
                  if (nextOpen) next.add(index);
                  else next.delete(index);
                  return next;
                });
              }}
            >
              <summary>
                <span className="history-source-path">
                  <code>{change.new_path ?? change.old_path ?? "(unknown path)"}</code>
                  <small>{change.status ?? "changed"}</small>
                </span>
                <ChangeStats patch={change.patch} />
              </summary>
              {open && (
                <SourcePatch
                  change={change}
                  cacheKey={`compass-history-${index}`}
                  style={style}
                  wrap={wrap}
                />
              )}
            </details>
          );
        })}
      </div>
    </>
  );
}

function ChangeStats({ patch }: { patch: string | undefined }) {
  const counts = useMemo(() => {
    if (!patch) return { additions: 0, deletions: 0 };
    let additions = 0;
    let deletions = 0;
    for (const line of patch.split("\n")) {
      if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
      if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
    }
    return { additions, deletions };
  }, [patch]);
  return (
    <span className="history-source-stats" aria-label={
      `${counts.additions} additions and ${counts.deletions} deletions`
    }>
      <i data-change="added">+{counts.additions}</i>
      <i data-change="removed">−{counts.deletions}</i>
    </span>
  );
}

function SourcePatch({
  change,
  cacheKey,
  style,
  wrap
}: {
  change: SourceChange;
  cacheKey: string;
  style: DiffStyle;
  wrap: boolean;
}) {
  const parsed = useMemo(() => {
    const patch = normalizeSourcePatch(change);
    if (!patch) return { valid: false, reason: "missing" } as const;
    try {
      const files = parsePatchFiles(patch, cacheKey, true)
        .flatMap((parsed) => parsed.files ?? []);
      const fileDiff = files[0];
      return files.length === 1 && fileDiff
        ? { valid: true, fileDiff } as const
        : { valid: false, reason: "unparseable" } as const;
    } catch {
      return { valid: false, reason: "unparseable" } as const;
    }
  }, [cacheKey, change]);
  const { themeType, style: themeStyle } = useVscodeDiffTheme();

  if (!change.patch) {
    return <p>Compass recorded this file change without an inline patch.</p>;
  }
  if (!parsed.valid) return <PatchFallback patch={change.patch} />;

  return (
    <DiffErrorBoundary fallback={<PatchFallback patch={change.patch} />}>
      <FileDiff
        key={`${cacheKey}-${themeType}`}
        fileDiff={parsed.fileDiff}
        disableWorkerPool
        className="history-source-diff"
        style={{
          ...themeStyle,
          minHeight: minimumDiffHeight(parsed.fileDiff, style)
        }}
        options={{
          theme: themeType === "light" ? "pierre-light" : "pierre-dark",
          themeType,
          diffStyle: style,
          diffIndicators: "classic",
          hunkSeparators: "metadata",
          lineDiffType: "word-alt",
          overflow: wrap ? "wrap" : "scroll",
          disableFileHeader: true,
          unsafeCSS: DIFF_THEME_CSS
        }}
      />
    </DiffErrorBoundary>
  );
}

export function minimumDiffHeight(
  fileDiff: FileDiffMetadata,
  style: DiffStyle
): number {
  const lineCount = fileDiff.hunks.reduce(
    (total, hunk) => total + (
      style === "split" ? hunk.splitLineCount : hunk.unifiedLineCount
    ),
    0
  );
  return lineCount * DIFF_LINE_HEIGHT
    + fileDiff.hunks.length * DIFF_HUNK_SEPARATOR_HEIGHT
    + DIFF_VERTICAL_PADDING;
}

function PatchFallback({ patch }: { patch: string }) {
  return (
    <div className="history-diff-fallback" role="note">
      <p>Enhanced diff unavailable. Showing the exact Git patch.</p>
      <pre>{patch}</pre>
    </div>
  );
}

export function normalizeSourcePatch(change: SourceChange): string | undefined {
  const patch = change.patch?.trimEnd();
  if (!patch) return undefined;
  if (patch.startsWith("diff --git ")) return `${patch}\n`;
  if (/^--- .+\n\+\+\+ /m.test(patch)) return `${patch}\n`;

  const oldPath = change.old_path ?? change.new_path ?? "unknown";
  const newPath = change.new_path ?? change.old_path ?? "unknown";
  const status = change.status?.toLocaleLowerCase();
  const oldHeader = status === "added" || status === "new"
    ? "/dev/null"
    : `a/${oldPath}`;
  const newHeader = status === "deleted" || status === "removed"
    ? "/dev/null"
    : `b/${newPath}`;
  const hunk = patch.startsWith("@@ ")
    ? patch
    : `${syntheticHunkHeader(patch)}\n${patch}`;
  return [
    `diff --git a/${oldPath} b/${newPath}`,
    `--- ${oldHeader}`,
    `+++ ${newHeader}`,
    hunk,
    ""
  ].join("\n");
}

function syntheticHunkHeader(patch: string): string {
  let oldLines = 0;
  let newLines = 0;
  for (const line of patch.split("\n")) {
    if (!line.startsWith("+")) oldLines += 1;
    if (!line.startsWith("-")) newLines += 1;
  }
  return `@@ -1,${oldLines} +1,${newLines} @@`;
}

function useVscodeDiffTheme(): {
  themeType: "light" | "dark";
  style: CSSProperties;
} {
  const read = () => (
    document.body.classList.contains("vscode-light")
    || document.documentElement.classList.contains("vscode-light")
    || document.body.classList.contains("vscode-high-contrast-light")
    || document.documentElement.classList.contains("vscode-high-contrast-light")
      ? "light"
      : "dark"
  );
  const [theme, setTheme] = useState<"light" | "dark">(read);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const refresh = () => {
      setTheme(read());
      setRevision((current) => current + 1);
    };
    const observer = new MutationObserver(refresh);
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    return () => observer.disconnect();
  }, []);
  const style = useMemo(() => {
    const light = theme === "light";
    const background = cssToken("--vscode-editor-background", light ? "#ffffff" : "#0f141b");
    const foreground = cssToken("--vscode-editor-foreground", light ? "#24292f" : "#e6edf3");
    const gutter = cssToken("--vscode-editorGutter-background", background);
    const separator = cssToken("--vscode-editorGroupHeader-tabsBackground", background);
    const lineNumber = cssToken(
      "--vscode-editorLineNumber-foreground",
      light ? "#656d76" : "#8b949e"
    );
    const added = cssToken(
      "--vscode-gitDecoration-addedResourceForeground",
      light ? "#1a7f37" : "#65bd84"
    );
    const removed = cssToken(
      "--vscode-gitDecoration-deletedResourceForeground",
      light ? "#cf222e" : "#ff7b86"
    );
    const modified = cssToken(
      "--vscode-gitDecoration-modifiedResourceForeground",
      light ? "#9a6700" : "#d29922"
    );
    return {
      colorScheme: theme,
      "--diffs-light-bg": background,
      "--diffs-dark-bg": background,
      "--diffs-light": foreground,
      "--diffs-dark": foreground,
      "--diffs-bg-context-override": background,
      "--diffs-bg-context-gutter-override": gutter,
      "--diffs-bg-separator-override": separator,
      "--diffs-fg-number-override": lineNumber,
      "--diffs-light-addition-color": added,
      "--diffs-dark-addition-color": added,
      "--diffs-light-deletion-color": removed,
      "--diffs-dark-deletion-color": removed,
      "--diffs-light-modified-color": modified,
      "--diffs-dark-modified-color": modified
    } as CSSProperties;
  }, [revision, theme]);
  return { themeType: theme, style };
}

function cssToken(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

class DiffErrorBoundary extends Component<{
  fallback: ReactNode;
  children: ReactNode;
}, { failed: boolean }> {
  override state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  override componentDidCatch(error: Error, info: ErrorInfo) {
    console.warn("Compass could not render an enhanced source diff", error, info);
  }

  override render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}
