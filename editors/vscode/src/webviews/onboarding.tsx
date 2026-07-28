import { CliOnboarding, type CliOnboardingState } from "@compass/viewer";
import { createRoot } from "react-dom/client";
import { HostToOnboardingMessageSchema } from "../install/messages";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass onboarding root is missing");
const root = createRoot(element);
let state: CliOnboardingState = { kind: "verifying" };

function render(): void {
  root.render(
    <CliOnboarding
      state={state}
      host={{
        install: () => vscode.postMessage({ type: "install" }),
        verifyAgain: () => vscode.postMessage({ type: "verifyAgain" }),
        selectExisting: () => vscode.postMessage({ type: "selectExisting" }),
        initializeRepository: () => vscode.postMessage({ type: "initializeRepository" }),
        openRepository: () => vscode.postMessage({ type: "openRepository" }),
        showTerminal: () => vscode.postMessage({ type: "showTerminal" })
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  const parsed = HostToOnboardingMessageSchema.safeParse(event.data);
  if (!parsed.success) return;
  state = parsed.data.state;
  render();
});

render();
vscode.postMessage({ type: "ready" });
