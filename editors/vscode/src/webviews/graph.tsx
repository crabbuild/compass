import { createRoot, type Root } from "react-dom/client";
import { CompassGraph } from "@compass/viewer";
import { HostToGraphMessageSchema } from "../transport/messages";

declare function acquireVsCodeApi(): {
  postMessage(message: unknown): void;
};

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass graph root is missing");
const root: Root = createRoot(element);
let repositoryId = "";

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
  repositoryId = parsed.data.repositoryId;
  root.render(
    <CompassGraph
      model={parsed.data.model}
      host={{
        openSource(source) {
          vscode.postMessage({ type: "openSource", repositoryId, source });
        }
      }}
    />
  );
});

vscode.postMessage({ type: "ready" });
