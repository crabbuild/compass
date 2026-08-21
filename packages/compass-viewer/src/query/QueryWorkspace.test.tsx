import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { fireEvent } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import {
  QueryWorkspace,
  querySuggestions,
  type QueryHost,
  type QueryRun
} from "./QueryWorkspace";

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
    const selected = container.querySelector('[aria-label="Query command"] [aria-selected="true"]');
    expect(selected?.textContent).toContain("Ask");
    expect(selected?.querySelector(".query-mode-indicator")).not.toBeNull();
    root.unmount();
  });

  it("runs the active command with Enter and keeps Shift+Enter for a new line", () => {
    const queryHost = host();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <QueryWorkspace runs={[]} host={queryHost} />
    ));
    const input = container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Ask input"]')!;

    fireEvent.change(input, { target: { value: "Who calls checkout?" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(queryHost.execute).not.toHaveBeenCalled();

    fireEvent.keyDown(input, { key: "Enter" });
    expect(queryHost.execute).toHaveBeenCalledWith({
      command: "ask",
      query: "Who calls checkout?",
      params: {},
      timeoutMs: 5000,
      maxRows: 1000
    });
    root.unmount();
  });

  it("offers mode-aware completions and accepts the highlighted option with Tab", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <QueryWorkspace runs={[]} host={host()} />
    ));
    const input = container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Ask input"]')!;

    fireEvent.change(input, { target: { value: "Who" } });
    const suggestions = container.querySelector('[role="listbox"][aria-label="Ask suggestions"]');
    expect(suggestions?.textContent).toContain("Who calls PaymentService.charge?");
    fireEvent.keyDown(input, { key: "Tab" });
    expect(input.value).toBe("Who calls PaymentService.charge?");
    expect(container.querySelector('[aria-label="Ask suggestions"]')).toBeNull();
    root.unmount();
  });

  it("tailors completions to CompassQL and recently returned symbols", () => {
    const run: QueryRun = {
      id: "ask-1",
      request: {
        command: "ask",
        query: "find checkout",
        params: {},
        timeoutMs: 5000,
        maxRows: 1000
      },
      status: "success",
      output: { kind: "code-query", value: typedAnswer() }
    };

    expect(querySuggestions("cql", "MATCH", []))
      .toEqual(expect.arrayContaining([
        expect.objectContaining({ value: "MATCH (n) RETURN n LIMIT 20" })
      ]));
    expect(querySuggestions("explain", "crate::check", [run]))
      .toEqual(expect.arrayContaining([
        expect.objectContaining({
          value: "crate::checkout",
          detail: "Symbol from recent results"
        })
      ]));
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
