import { parsePatchFiles } from "@pierre/diffs";
import { PatchDiff } from "@pierre/diffs/react";
import {
  Component,
  useEffect,
  useMemo,
  useState,
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
  --diffs-dark-bg: var(--vscode-textCodeBlock-background, #0f141b);
  --diffs-light-bg: var(--vscode-textCodeBlock-background, #f6f8fa);
  --diffs-added-dark: var(--vscode-gitDecoration-addedResourceForeground, #65bd84);
  --diffs-added-light: var(--vscode-gitDecoration-addedResourceForeground, #1a7f37);
  --diffs-deleted-dark: var(--vscode-gitDecoration-deletedResourceForeground, #ff7b86);
  --diffs-deleted-light: var(--vscode-gitDecoration-deletedResourceForeground, #cf222e);
  --diffs-modified-dark: var(--vscode-gitDecoration-modifiedResourceForeground, #d29922);
  --diffs-modified-light: var(--vscode-gitDecoration-modifiedResourceForeground, #9a6700);
  --diffs-bg-context-override: var(--vscode-editor-background);
  --diffs-bg-context-gutter-override: var(--vscode-editorGutter-background);
  --diffs-bg-separator-override: var(--vscode-editorGroupHeader-tabsBackground);
  --diffs-fg-number-override: var(--vscode-editorLineNumber-foreground);
}`;

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
                  patch={change.patch}
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
  patch,
  cacheKey,
  style,
  wrap
}: {
  patch: string | undefined;
  cacheKey: string;
  style: DiffStyle;
  wrap: boolean;
}) {
  const validation = useMemo(() => {
    if (!patch) return { valid: false, reason: "missing" } as const;
    try {
      const files = parsePatchFiles(patch, cacheKey, true)
        .flatMap((parsed) => parsed.files ?? []);
      return files.length
        ? { valid: true } as const
        : { valid: false, reason: "unparseable" } as const;
    } catch {
      return { valid: false, reason: "unparseable" } as const;
    }
  }, [cacheKey, patch]);
  const themeType = useVscodeThemeType();

  if (!patch) {
    return <p>Compass recorded this file change without an inline patch.</p>;
  }
  if (!validation.valid) return <PatchFallback patch={patch} />;

  return (
    <DiffErrorBoundary fallback={<PatchFallback patch={patch} />}>
      <PatchDiff
        patch={patch}
        disableWorkerPool
        className="history-source-diff"
        options={{
          theme: { dark: "pierre-dark", light: "pierre-light" },
          themeType,
          diffStyle: style,
          diffIndicators: "classic",
          hunkSeparators: "metadata",
          lineDiffType: "word-alt",
          overflow: wrap ? "wrap" : "scroll",
          disableFileHeader: true,
          disableVirtualizationBuffers: true,
          unsafeCSS: DIFF_THEME_CSS
        }}
      />
    </DiffErrorBoundary>
  );
}

function PatchFallback({ patch }: { patch: string }) {
  return (
    <div className="history-diff-fallback" role="note">
      <p>Enhanced diff unavailable. Showing the exact Git patch.</p>
      <pre>{patch}</pre>
    </div>
  );
}

function useVscodeThemeType(): "light" | "dark" {
  const read = () => (
    document.body.classList.contains("vscode-light")
    || document.body.classList.contains("vscode-high-contrast-light")
      ? "light"
      : "dark"
  );
  const [theme, setTheme] = useState<"light" | "dark">(read);
  useEffect(() => {
    const observer = new MutationObserver(() => setTheme(read()));
    observer.observe(document.body, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return theme;
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
