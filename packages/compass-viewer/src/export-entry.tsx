import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { GraphViewModelSchema } from "./contracts/graph";
import { CompassGraph } from "./graph/CompassGraph";
import "./theme.css";

function mount() {
  const rootElement = document.getElementById("compass-viewer-root");
  const modelElement = document.getElementById("compass-viewer-model");
  if (!rootElement || !modelElement) {
    throw new Error("Compass viewer root or model is missing");
  }
  const model = GraphViewModelSchema.parse(JSON.parse(modelElement.textContent ?? ""));
  createRoot(rootElement).render(
    <StrictMode>
      <CompassGraph
        model={model}
        host={{
          openSource(source) {
            window.dispatchEvent(new CustomEvent("compass:open-source", {
              detail: source
            }));
          }
        }}
      />
    </StrictMode>
  );
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", mount, { once: true });
} else {
  mount();
}
