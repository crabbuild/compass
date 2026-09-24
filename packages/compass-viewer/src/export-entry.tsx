import { createRoot } from "react-dom/client";
import {
  GraphViewModelSchema,
  type GraphViewModel
} from "./contracts/graph";
import { WorkbenchModelSchema } from "./contracts/workbench";
import { CommunityHierarchyViewSchema } from "./contracts/hierarchy";
import { CompassGraph } from "./graph/CompassGraph";
import { VisualizationWorkbench } from "./workbench/VisualizationWorkbench";
import {
  openExportSource,
  SourceNavigationSchema,
  type SourceNavigation
} from "./sourceLinks";
import {
  applyThemePreference,
  type ThemePreference
} from "./lib/theme";
import "./theme.css";

function mount() {
  const rootElement = document.getElementById("compass-viewer-root");
  const modelElement = document.getElementById("compass-viewer-model");
  if (!rootElement || !modelElement) {
    throw new Error("Compass viewer root or model is missing");
  }
  const untrusted = JSON.parse(modelElement.textContent ?? "");
  const sourceNavigation = parseSourceNavigation();
  const root = createRoot(rootElement);
  // A standalone document carries its own theme choice: readers may want the
  // dark palette for a screenshot even when their system is light, or the
  // reverse. Hosted viewers never get this control; their editor theme wins.
  let theme: ThemePreference = "auto";
  applyThemePreference(theme);
  let renderStandalone: () => void = () => undefined;
  const setTheme = (next: ThemePreference) => {
    theme = next;
    applyThemePreference(next);
    renderStandalone();
  };
  const workbench = WorkbenchModelSchema.safeParse(untrusted);
  if (workbench.success) {
    renderStandalone = () => root.render(
      <VisualizationWorkbench
        workbench={workbench.data}
        host={{
          openSource(source, revision) {
            openStandaloneSource(sourceNavigation, source, revision);
          }
        }}
        themePreference={theme}
        onThemePreferenceChange={setTheme}
      />
    );
    renderStandalone();
    return;
  }
  const overview = GraphViewModelSchema.parse(untrusted);
  const hierarchy = parseEmbeddedHierarchy();
  const detailCache = new Map<number, GraphViewModel>();
  let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
  let communityLoading: number | null = null;
  let communityError: string | undefined;

  const render = () => {
    root.render(
      <CompassGraph
        model={overview}
        hierarchy={hierarchy}
        initialLevel={hierarchy?.initialLevel}
        themePreference={theme}
        onThemePreferenceChange={setTheme}
        communityDetail={communityDetail}
        communityLoading={communityLoading}
        communityError={communityError}
        onBackToOverview={communityDetail ? () => {
          communityDetail = undefined;
          communityError = undefined;
          render();
        } : undefined}
        host={{
          openSource(source, revision) {
            openStandaloneSource(sourceNavigation, source, revision);
          },
          openCommunity(communityId) {
            if (communityLoading !== null) return;
            communityLoading = communityId;
            communityError = undefined;
            render();
            window.setTimeout(() => {
              try {
                let model = detailCache.get(communityId);
                if (!model) {
                  const detailElement = document.querySelector<HTMLScriptElement>(
                    `script[data-compass-community="${communityId}"]`
                  );
                  if (!detailElement) {
                    throw new Error(`Community ${communityId} detail is unavailable in this export.`);
                  }
                  model = GraphViewModelSchema.parse(
                    JSON.parse(detailElement.textContent ?? "")
                  );
                  detailCache.set(communityId, model);
                }
                communityDetail = { communityId, model };
                window.dispatchEvent(new CustomEvent("compass:open-community", {
                  detail: { communityId }
                }));
              } catch (error) {
                communityError = error instanceof Error ? error.message : String(error);
              } finally {
                communityLoading = null;
                render();
              }
            }, 0);
          }
        }}
      />
    );
  };
  renderStandalone = render;
  render();
}

/**
 * The published community hierarchy, when the export embedded one. An export
 * from an older Compass or an unclustered build carries no script, and a level
 * the export could not afford to draw has no model: both render the overview
 * the export already had.
 */
function parseEmbeddedHierarchy() {
  const element = document.getElementById("compass-viewer-hierarchy");
  if (!element?.textContent) return undefined;
  const parsed = CommunityHierarchyViewSchema.safeParse(JSON.parse(element.textContent));
  return parsed.success ? parsed.data : undefined;
}

function parseSourceNavigation(): SourceNavigation | undefined {
  const element = document.getElementById("compass-source-navigation");
  if (!element?.textContent) return undefined;
  const parsed = SourceNavigationSchema.safeParse(JSON.parse(element.textContent));
  return parsed.success ? parsed.data : undefined;
}

function openStandaloneSource(
  navigation: SourceNavigation | undefined,
  source: Parameters<typeof openExportSource>[1],
  revision?: string
): void {
  const result = openExportSource(navigation, source, revision);
  if (result.kind === "unavailable") showSourceUnavailable(source.file);
}

function showSourceUnavailable(file: string): void {
  const existing = document.getElementById("compass-source-notice");
  const notice = existing ?? document.createElement("div");
  notice.id = "compass-source-notice";
  notice.className = "compass-source-notice";
  notice.setAttribute("role", "status");
  notice.setAttribute("aria-live", "polite");
  notice.textContent = [
    "Source link unavailable.",
    `The exported revision for ${file} is not published by the configured remote.`,
    "Open this graph in VS Code to navigate to the local source."
  ].join(" ");
  if (!existing) document.body.append(notice);
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", mount, { once: true });
} else {
  mount();
}
