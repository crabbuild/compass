import { createRoot } from "react-dom/client";
import { ArchitectureFlow, CallflowViewModelSchema } from "@compass/viewer";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass architecture root is missing");
const root = createRoot(element);

window.addEventListener("message", (event) => {
  if (event.data?.type === "error") {
    root.render(<main className="grid min-h-screen place-items-center p-8">{event.data.message}</main>);
    return;
  }
  if (event.data?.type !== "hydrate") return;
  const parsed = CallflowViewModelSchema.safeParse(event.data.model);
  if (!parsed.success || typeof event.data.repositoryId !== "string") return;
  const repositoryId = event.data.repositoryId;
  root.render(
    <ArchitectureFlow
      model={parsed.data}
      host={{
        openSource(file) {
          vscode.postMessage({ type: "openSource", repositoryId, file });
        }
      }}
    />
  );
});
vscode.postMessage({ type: "ready" });
