import { createRoot } from "react-dom/client";
import {
  GraphViewModelSchema,
  type GraphViewModel
} from "./contracts/graph";
import { WorkbenchModelSchema } from "./contracts/workbench";
import { CompassGraph } from "./graph/CompassGraph";
import { VisualizationWorkbench, dispatchOpenSource } from "./workbench/VisualizationWorkbench";
import "./theme.css";

function mount() {
  const rootElement = document.getElementById("compass-viewer-root");
  const modelElement = document.getElementById("compass-viewer-model");
  if (!rootElement || !modelElement) {
    throw new Error("Compass viewer root or model is missing");
  }
  const untrusted = JSON.parse(modelElement.textContent ?? "");
  const root = createRoot(rootElement);
  const workbench = WorkbenchModelSchema.safeParse(untrusted);
  if (workbench.success) {
    root.render(
      <VisualizationWorkbench
        workbench={workbench.data}
        host={{ openSource: dispatchOpenSource }}
      />
    );
    return;
  }
  const overview = GraphViewModelSchema.parse(untrusted);
  const detailCache = new Map<number, GraphViewModel>();
  let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
  let communityLoading: number | null = null;
  let communityError: string | undefined;

  const render = () => {
    root.render(
      <CompassGraph
        model={overview}
        communityDetail={communityDetail}
        communityLoading={communityLoading}
        communityError={communityError}
        onBackToOverview={communityDetail ? () => {
          communityDetail = undefined;
          communityError = undefined;
          render();
        } : undefined}
        host={{
          openSource(source) {
            window.dispatchEvent(new CustomEvent("compass:open-source", {
              detail: source
            }));
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
  render();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", mount, { once: true });
} else {
  mount();
}
