import {
  InitializationWizard,
  OcrModelStatusSchema,
  type InitializationRequest,
  type InitializationStatus,
  type OcrModelStatus
} from "@compass/viewer";
import { createRoot } from "react-dom/client";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass initialization root is missing");

const root = createRoot(element);
let repositoryName = "repository";
let repositoryRoot = "";
let configurationExists = false;
let scopeFiles: string[] = [];
let scopeFilesTruncated = false;
let status: InitializationStatus | undefined;
let ocrModel: OcrModelStatus = { kind: "checking", profile: "pp-ocrv6-small" };

function render(): void {
  root.render(
    <InitializationWizard
      repositoryName={repositoryName}
      repositoryRoot={repositoryRoot}
      configurationExists={configurationExists}
      scopeFiles={scopeFiles}
      scopeFilesTruncated={scopeFilesTruncated}
      ocrModel={ocrModel}
      {...(status ? { status } : {})}
      host={{
        start(request: InitializationRequest) {
          status = {
            kind: "building",
            phase: "configuring",
            message: "Preparing repository scope"
          };
          render();
          vscode.postMessage({ type: "start", request });
        },
        cancel() {
          vscode.postMessage({ type: "cancel" });
        },
        reset() {
          status = undefined;
          render();
          vscode.postMessage({ type: "reset" });
        },
        openGraph() {
          vscode.postMessage({ type: "openGraph" });
        },
        showOutput() {
          vscode.postMessage({ type: "showOutput" });
        },
        installOcrModel() {
          vscode.postMessage({ type: "installOcrModel" });
        },
        verifyOcrModel() {
          vscode.postMessage({ type: "verifyOcrModel" });
        }
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  const message = event.data;
  if (message?.type === "hydrate") {
    repositoryName = String(message.repositoryName);
    repositoryRoot = String(message.repositoryRoot);
    configurationExists = message.configurationExists === true;
    scopeFiles = Array.isArray(message.scopeFiles)
      ? message.scopeFiles.filter((value: unknown): value is string => typeof value === "string")
      : [];
    scopeFilesTruncated = message.scopeFilesTruncated === true;
    const parsedOcrModel = OcrModelStatusSchema.safeParse(message.ocrModel);
    if (parsedOcrModel.success) ocrModel = parsedOcrModel.data;
  } else if (message?.type === "progress") {
    const progress = message.event;
    status = {
      kind: "building",
      phase: String(progress.phase),
      current: typeof progress.current === "number" ? progress.current : undefined,
      total: typeof progress.total === "number" ? progress.total : undefined,
      message: String(progress.message)
    };
  } else if (message?.type === "succeeded") {
    status = {
      kind: "success",
      message: typeof message.message === "string" ? message.message : undefined
    };
  } else if (message?.type === "failed") {
    status = { kind: "error", message: String(message.message) };
  } else if (message?.type === "cancelled") {
    status = { kind: "cancelled" };
  } else if (message?.type === "configurationChanged") {
    configurationExists = message.configurationExists === true;
  } else if (message?.type === "ocrModel") {
    const parsedOcrModel = OcrModelStatusSchema.safeParse(message.status);
    if (!parsedOcrModel.success) return;
    ocrModel = parsedOcrModel.data;
  } else {
    return;
  }
  render();
});

render();
vscode.postMessage({ type: "ready" });
