import { createRoot, type Root } from "react-dom/client";
import {
  CompassGraph,
  type GraphViewModel
} from "@compass/viewer";
import { HostToGraphMessageSchema } from "../transport/messages";

declare function acquireVsCodeApi(): {
  postMessage(message: unknown): void;
};

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass graph root is missing");
const root: Root = createRoot(element);
let repositoryId = "";
let overview: GraphViewModel | undefined;
let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";

function render(): void {
  if (!overview) return;
  root.render(
    <CompassGraph
      model={overview}
      communityDetail={communityDetail}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={() => {
        communityDetail = undefined;
        communityLoading = null;
        communityError = undefined;
        activeCommunityRequest = "";
        render();
      }}
      host={{
        openSource(source) {
          vscode.postMessage({ type: "openSource", repositoryId, source });
        },
        openCommunity(communityId) {
          if (communityLoading !== null) return;
          communityLoading = communityId;
          communityError = undefined;
          activeCommunityRequest = crypto.randomUUID();
          vscode.postMessage({
            type: "openCommunity",
            requestId: activeCommunityRequest,
            repositoryId,
            communityId
          });
          render();
        }
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  const parsed = HostToGraphMessageSchema.safeParse(event.data);
  if (!parsed.success) return;
  if (parsed.data.type === "error") {
    root.render(
      <main className="grid min-h-screen place-items-center p-8 text-center">
        <div>
          <h1 className="text-lg font-semibold">Compass could not load this graph</h1>
          <p className="mt-2 text-sm text-muted-foreground">{parsed.data.message}</p>
        </div>
      </main>
    );
    return;
  }
  if (parsed.data.type === "hydrateGraph") {
    repositoryId = parsed.data.repositoryId;
    overview = parsed.data.model;
    communityDetail = undefined;
    communityLoading = null;
    communityError = undefined;
    activeCommunityRequest = "";
  } else if (parsed.data.requestId === activeCommunityRequest) {
    communityLoading = null;
    if (parsed.data.type === "communityGraph") {
      communityDetail = {
        communityId: parsed.data.communityId,
        model: parsed.data.model
      };
      communityError = undefined;
    } else {
      communityError = parsed.data.message;
    }
  }
  render();
});

vscode.postMessage({ type: "ready" });
