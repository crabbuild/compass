import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import { QueryWorkspace, type QueryHost, type QueryRun } from "./QueryWorkspace";

beforeAll(() => {
  vi.stubGlobal("navigator", { platform: "MacIntel" });
});

function host(): QueryHost {
  return {
    execute: vi.fn(),
    cancel: vi.fn(),
    selectRun: vi.fn(),
    closeRun: vi.fn(),
    openSource: vi.fn(),
    openGraph: vi.fn()
  };
}

function typedAnswer(): CodeQueryResponse {
  return {
    schema: "compass.query/1",
    operation: "search",
    results: [{ nodeId: "checkout", score: 1, matchedFields: ["qualifiedName"] }],
    nodes: [{
      id: "checkout",
      kind: "function",
      roles: [],
      name: "checkout",
      qualifiedName: "crate::checkout",
      language: "rust",
      framework: null,
      source: {
        file: "src/checkout.rs",
        startByte: 0,
        endByte: 8,
        startLine: 12,
        startColumn: 1,
        endLine: 12,
        endColumn: 9
      },
      details: null,
      evidence: []
    }],
    edges: [],
    files: [],
    paths: [],
    diagnostics: [],
    limits: {
      maxDepth: 8,
      maxNodes: 500,
      maxEdges: 1000,
      maxPaths: 100,
      maxCandidates: 20,
      maxSourceBytes: 1048576,
      maxResponseBytes: 8388608
    },
    truncated: false
  };
}

describe("QueryWorkspace", () => {
  it("presents Ask, Explain, and CompassQL as distinct command modes", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <QueryWorkspace runs={[]} host={host()} />
    ));

    expect(Array.from(container.querySelectorAll('[aria-label="Query command"] [role="tab"]'))
      .map((tab) => tab.textContent)).toEqual(expect.arrayContaining([
        expect.stringContaining("Ask"),
        expect.stringContaining("Explain"),
        expect.stringContaining("CompassQL")
      ]));
    root.unmount();
  });

  it("keeps command outputs in separate selectable result tabs", () => {
    const queryHost = host();
    const runs: QueryRun[] = [{
      id: "ask-1",
      request: {
        command: "ask",
        query: "where is checkout?",
        params: {},
        timeoutMs: 5000,
        maxRows: 1000
      },
      status: "success",
      durationMs: 18,
      output: { kind: "code-query", value: typedAnswer() }
    }, {
      id: "explain-1",
      request: {
        command: "explain",
        query: "crate::checkout",
        params: {},
        timeoutMs: 5000,
        maxRows: 1000
      },
      status: "success",
      durationMs: 9,
      output: {
        kind: "explanation",
        text: "Node: checkout\n  ID: checkout\n  Source: src/checkout.rs L12\n  Type: function\n  Community: Checkout\n  Degree: 0"
      }
    }];
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <QueryWorkspace
        runs={runs}
        activeRunId="ask-1"
        host={queryHost}
      />
    ));

    const resultTabs = Array.from(
      container.querySelectorAll<HTMLButtonElement>('[aria-label="Query results"] [role="tab"]')
    );
    expect(resultTabs).toHaveLength(2);
    expect(container.textContent).toContain("crate::checkout");
    flushSync(() => resultTabs[1]?.click());
    expect(queryHost.selectRun).toHaveBeenCalledWith("explain-1");
    root.unmount();
  });
});
