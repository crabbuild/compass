import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { CodeEvidenceRecord } from "../contracts/codeQuery";
import { CodeEvidence } from "./CodeEvidence";

const wiringSite = {
  file: "src/routes.ts",
  startByte: 10,
  endByte: 20,
  startLine: 4,
  startColumn: 0,
  endLine: 4,
  endColumn: 10
};

const heuristic: CodeEvidenceRecord = {
  layer: "structural_graph",
  origin: "heuristic",
  extractor: "express.routes",
  confidence: "ambiguous",
  anchor: null,
  rule: "middleware-chain",
  wiringSite,
  resolution: "ambiguous",
  candidates: [{
    nodeId: "handler:a",
    reason: "matching exported name",
    confidence: "ambiguous"
  }]
};

describe("CodeEvidence", () => {
  it("shows attributable heuristic wiring, candidates, diagnostics, and truncation", () => {
    const onOpenSource = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <CodeEvidence
        evidence={[heuristic]}
        diagnostics={[{
          code: "unresolved_handler",
          message: "One middleware target could not be resolved.",
          nodeId: "route:a",
          path: "src/routes.ts"
        }]}
        truncated
        onOpenSource={onOpenSource}
      />
    ));
    expect(container.textContent).toContain("Ambiguous");
    expect(container.textContent).toContain("express.routes");
    expect(container.textContent).toContain("middleware-chain");
    expect(container.textContent).toContain("Wired at src/routes.ts:4");
    expect(container.textContent).toContain("handler:a");
    expect(container.textContent).toContain("matching exported name");
    expect(container.textContent).toContain("configured limit");
    expect(container.querySelector('[data-status="ambiguous"] svg')).not.toBeNull();
    flushSync(() => container.querySelector<HTMLButtonElement>("button")?.click());
    expect(onOpenSource).toHaveBeenCalledWith(wiringSite);
    root.unmount();
  });

  it.each([
    ["ast", "exact", "exact", "Exact"],
    ["config", "exact", "exact", "Configuration"],
    ["convention", "exact", "exact", "Convention"],
    ["heuristic", "inferred", "unresolved", "Unresolved"]
  ] as const)("renders %s evidence as accessible %s state", (
    origin,
    confidence,
    resolution,
    label
  ) => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <CodeEvidence
        evidence={[{
          ...heuristic,
          origin,
          confidence,
          resolution,
          candidates: []
        }]}
      />
    ));
    expect(container.textContent).toContain(label);
    expect(container.querySelector("svg")).not.toBeNull();
    root.unmount();
  });
});
